#![allow(unused_assignments, dead_code)]
//! ATA/IDE Hard Drive Controller Emulation
//!
//! Ported from Bochs `iodev/harddrv.cc` (AT Attachment with Packet Interface).
//! Reference: T13 ATA/ATAPI specification at www.t13.org.
//!
//! # I/O Port Layout
//!
//! Each ATA channel has two I/O address ranges:
//! - **Command Block** (8 ports): Data, Error/Features, Sector Count, Sector Number,
//!   Cylinder Low, Cylinder High, Drive/Head, Status/Command
//! - **Control Block** (2 ports): Alternate Status / Device Control, Drive Address
//!
//! | Channel   | Command Block | Control Block | IRQ |
//! |-----------|---------------|---------------|-----|
//! | Primary   | 0x1F0-0x1F7   | 0x3F6-0x3F7   | 14  |
//! | Secondary | 0x170-0x177   | 0x376-0x377   | 15  |
//!
//! # Register Descriptions
//!
//! ## Status Register (port+7 read) / Command Register (port+7 write)
//! ```text
//! Bit 7: BSY  - Busy (controller is executing a command)
//! Bit 6: DRDY - Drive Ready (drive is powered up and ready)
//! Bit 5: DWF  - Drive Write Fault
//! Bit 4: DSC  - Drive Seek Complete
//! Bit 3: DRQ  - Data Request (data is ready to transfer)
//! Bit 2: CORR - Corrected Data (ECC correction applied)
//! Bit 1: IDX  - Index (set once per disk revolution, simulated via counter)
//! Bit 0: ERR  - Error (check Error register for details)
//! ```
//!
//! ## Drive/Head Register (port+6)
//! ```text
//! Bit 7:   ECC data field (always 1)
//! Bit 6:   LBA mode (1=LBA, 0=CHS). Historically was sector size bit.
//! Bit 5:   Always 1 (historically 01b = 512 byte sectors)
//! Bit 4:   DRV - Drive select (0=master, 1=slave)
//! Bit 3-0: Head number (CHS mode) or LBA bits 24-27 (LBA mode)
//! ```
//!
//! ## Device Control Register (control port+6 write)
//! ```text
//! Bit 2: SRST - Software Reset (set to reset, then clear)
//! Bit 1: nIEN - Interrupt Enable (0=enabled, 1=disabled)
//! ```
//! Reading the Control Block Status register does NOT clear a pending interrupt.
//! Reading the Command Block Status register (port+7) DOES clear the pending IRQ.
//!
//! # ATA PIO Read Command State Machine (Bochs harddrv.cc)
//!
//! ```text
//! Host writes command (0x20 READ SECTORS) to port+7:
//!   1. Controller sets BSY=1, DRQ=0, clears error
//!   2. Controller reads first sector from disk into internal buffer
//!   3. Seek timer fires (simulated seek delay):
//!      - Sets BSY=0, DRQ=1, DRDY=1, DSC=1
//!      - Raises IRQ (if nIEN=0)
//!   4. Host reads 256 words (512 bytes) from Data register (port+0)
//!   5. After last word read from buffer:
//!      - If num_sectors > 0: read next sector, raise IRQ, goto step 4
//!      - If num_sectors == 0: set DRQ=0, transfer complete
//! ```
//!
//! # ATA PIO Write Command State Machine
//!
//! ```text
//! Host writes command (0x30 WRITE SECTORS) to port+7:
//!   1. Controller sets DRQ=1, BSY=0 immediately (implicit seek)
//!   2. Host writes 256 words to Data register (port+0)
//!   3. After buffer is full:
//!      - Controller writes sector to disk
//!      - Decrements num_sectors via increment_address()
//!      - If num_sectors > 0: keep DRQ=1, raise IRQ, goto step 2
//!      - If num_sectors == 0: set DRQ=0, raise IRQ, transfer complete
//! ```
//!
//! # Key Behavioral Notes from Bochs
//!
//! - **Shared registers**: On real hardware, controller registers are shared between
//!   master and slave on the same channel. The emulator must respond to reads even
//!   if the selected device is not present (e.g., minix2 uses this for drive detection).
//! - **Sector count of 0 means 256**: When sector_count register is 0, the transfer
//!   length is 256 sectors (Bochs `lba48_transform()`).
//! - **Index pulse simulation**: The IDX status bit is set once every
//!   `INDEX_PULSE_CYCLE` (10) status register reads, simulating disk rotation.
//! - **IRQ clearing**: Reading the Status register (port+7) clears the IRQ for that
//!   channel. Reading Alternate Status (control+6) does NOT clear the IRQ.
//! - **Drive/Head register writes go to both drives**: Writes to ports 0x1F2-0x1F5
//!   (sector count, sector number, cylinder low/high) update HOB (High Order Byte)
//!   registers on BOTH drives on the channel, supporting LBA48 addressing.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use bitflags::bitflags;
use alloc::vec::Vec;

#[cfg(feature = "std")]
use std::fs::File;
#[cfg(feature = "std")]
use std::io::{Read, Seek, SeekFrom, Write};

/// Sector size in bytes
pub const SECTOR_SIZE: usize = 512;

/// ATA I/O port offsets (from base address)
pub const ATA_DATA: u16 = 0; // Data register (R/W)
pub const ATA_ERROR: u16 = 1; // Error register (R) / Features (W)
pub const ATA_SECTOR_COUNT: u16 = 2; // Sector count
pub const ATA_SECTOR_NUM: u16 = 3; // Sector number / LBA low
pub const ATA_CYL_LOW: u16 = 4; // Cylinder low / LBA mid
pub const ATA_CYL_HIGH: u16 = 5; // Cylinder high / LBA high
pub const ATA_DRIVE_HEAD: u16 = 6; // Drive/Head / LBA top 4 bits
pub const ATA_STATUS: u16 = 7; // Status (R) / Command (W)
pub const ATA_ALT_STATUS: u16 = 0x206; // Alternate status / Device control

bitflags! {
    /// ATA Status register bits (port+7 read)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AtaStatus: u8 {
        /// Error — check Error register for details
        const ERR  = 0x01;
        /// Index — set once per disk revolution (simulated)
        const IDX  = 0x02;
        /// Corrected data — ECC correction applied (always 0)
        const CORR = 0x04;
        /// Data Request — data is ready to transfer
        const DRQ  = 0x08;
        /// Drive Seek Complete
        const DSC  = 0x10;
        /// Drive Write Fault
        const DWF  = 0x20;
        /// Drive Ready — drive is powered up and ready
        const DRDY = 0x40;
        /// Busy — controller is executing a command
        const BSY  = 0x80;
    }
}

bitflags! {
    /// ATA Error register bits (port+1 read)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AtaError: u8 {
        /// Address mark not found
        const AMNF  = 0x01;
        /// Track 0 not found
        const TK0NF = 0x02;
        /// Command aborted
        const ABRT  = 0x04;
        /// ID not found
        const IDNF  = 0x10;
        /// Uncorrectable data error
        const UNC   = 0x40;
    }
}

/// ATA commands
pub const ATA_CMD_RECALIBRATE: u8 = 0x10;
pub const ATA_CMD_READ_SECTORS: u8 = 0x20;
pub const ATA_CMD_READ_SECTORS_EXT: u8 = 0x24;
pub const ATA_CMD_WRITE_SECTORS: u8 = 0x30;
pub const ATA_CMD_WRITE_SECTORS_EXT: u8 = 0x34;
pub const ATA_CMD_READ_VERIFY: u8 = 0x40;
pub const ATA_CMD_SEEK: u8 = 0x70;
pub const ATA_CMD_EXECUTE_DIAGNOSTICS: u8 = 0x90;
pub const ATA_CMD_INITIALIZE_PARAMS: u8 = 0x91;
pub const ATA_CMD_READ_MULTIPLE: u8 = 0xC4;
pub const ATA_CMD_WRITE_MULTIPLE: u8 = 0xC5;
pub const ATA_CMD_SET_MULTIPLE: u8 = 0xC6;
pub const ATA_CMD_IDENTIFY: u8 = 0xEC;
pub const ATA_CMD_SET_FEATURES: u8 = 0xEF;

// ATAPI commands
pub const ATA_CMD_DEVICE_RESET: u8 = 0x08;
pub const ATA_CMD_PACKET: u8 = 0xA0;
pub const ATA_CMD_IDENTIFY_PACKET: u8 = 0xA1;
pub const PACKET_SIZE: usize = 12;

// CD-ROM sector size
pub const CDROM_SECTOR_SIZE: usize = 2048;

/// Device type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeviceType {
    None,
    Disk,
    Cdrom,
}

/// Drive geometry
#[derive(Debug, Clone)]
pub struct DriveGeometry {
    pub(crate) cylinders: u16,
    pub(crate) heads: u8,
    pub(crate) sectors_per_track: u8,
    pub(crate) total_sectors: u32,
}

impl DriveGeometry {
    /// Create geometry from CHS values
    pub fn from_chs(cylinders: u16, heads: u8, spt: u8) -> Self {
        Self {
            cylinders,
            heads,
            sectors_per_track: spt,
            total_sectors: cylinders as u32 * heads as u32 * spt as u32,
        }
    }

    /// Convert LBA to CHS
    pub fn lba_to_chs(&self, lba: u32) -> (u16, u8, u8) {
        let spt = self.sectors_per_track as u32;
        let heads = self.heads as u32;

        let cylinder = lba / (heads * spt);
        let temp = lba % (heads * spt);
        let head = temp / spt;
        let sector = (temp % spt) + 1;

        (cylinder as u16, head as u8, sector as u8)
    }

    /// Convert CHS to LBA
    pub fn chs_to_lba(&self, cylinder: u16, head: u8, sector: u8) -> u32 {
        let spt = self.sectors_per_track as u32;
        let heads = self.heads as u32;

        (cylinder as u32 * heads * spt) + (head as u32 * spt) + (sector as u32).wrapping_sub(1)
    }
}

/// Controller state for one ATA drive (Bochs `controller_t`, harddrv.h).
///
/// This struct holds the full state of one ATA controller — the task file registers
/// (visible to the host via I/O ports), the internal transfer buffer, and bookkeeping
/// for the current command in progress.
///
/// ## Task File Registers
///
/// The "task file" is the set of 8 registers at I/O offsets 0x00-0x07 that the host
/// CPU uses to communicate with the drive. On real hardware, these registers are
/// shared between master and slave on the same channel.
///
/// ```text
/// Offset 0: Data Register      — 16/32-bit data transfer port
/// Offset 1: Error (R) / Features (W)
/// Offset 2: Sector Count       — number of sectors to transfer (0 = 256)
/// Offset 3: Sector Number      — CHS sector / LBA[7:0]
/// Offset 4: Cylinder Low       — CHS cylinder low / LBA[15:8]
/// Offset 5: Cylinder High      — CHS cylinder high / LBA[23:16]
/// Offset 6: Drive/Head         — drive select + head/LBA[27:24]
/// Offset 7: Status (R) / Command (W)
/// ```
///
/// ## Transfer Tracking
///
/// - `num_sectors`: Internal counter set by `lba48_transform()` at command start.
///   Decremented by `increment_address()` after each sector is transferred.
///   When it reaches 0, the command is complete.
/// - `buffer_size`: For single-sector commands, equals `sect_size` (usually 512).
///   For READ/WRITE MULTIPLE, equals `min(multiple_sectors, num_sectors) * sect_size`.
/// - `buffer_index`: Current byte offset into the buffer. When it reaches
///   `buffer_size`, the next sector batch is loaded or the transfer completes.
#[derive(Debug)]
pub struct AtaController {
    /// Status register (port+7 read).
    pub(crate) status: AtaStatus,
    /// Error register (port+1 read). Set when status ERR bit is set.
    /// After EXECUTE DEVICE DIAGNOSTIC: 0x01 = no error (diagnostic passed).
    pub(crate) error: AtaError,
    /// Features register (port+1 write). Used by SET FEATURES command.
    /// Also serves as Write Precompensation in legacy drives (0xFF = no precomp).
    pub(crate) features: u8,
    /// Sector count register (port+2 read/write). Number of sectors for the
    /// current command. A value of 0 means 256 sectors.
    pub(crate) sector_count: u8,
    /// Sector number (port+3). In CHS mode: sector within track (1-based).
    /// In LBA mode: LBA bits [7:0].
    pub(crate) sector_no: u8,
    /// Cylinder number (port+4 low, port+5 high). In CHS mode: cylinder.
    /// In LBA mode: LBA bits [23:8].
    pub(crate) cylinder_no: u16,
    /// Head number (port+6 bits [3:0]). In CHS mode: head.
    /// In LBA mode: LBA bits [27:24].
    pub(crate) head_no: u8,
    /// LBA mode flag (port+6 bit 6). When set, sector_no/cylinder_no/head_no
    /// are interpreted as a 28-bit LBA address instead of CHS.
    pub(crate) lba_mode: bool,
    /// Device control register (control port+6 write).
    /// Bit 1 (nIEN): 0=interrupts enabled, 1=interrupts disabled.
    /// Bit 2 (SRST): Software reset — set to reset both drives, then clear.
    pub(crate) control: u8,
    /// Interrupt pending flag — set by raise_interrupt(), cleared when the host
    /// reads the Status register (port+7) or writes a new command.
    pub(crate) interrupt_pending: bool,
    /// The ATA command currently being executed (last value written to port+7).
    /// Used to determine behavior when the Data register is read/written.
    pub(crate) current_command: u8,
    /// Multiple sector count, set by SET MULTIPLE MODE (0xC6) command.
    /// Determines how many sectors are transferred per IRQ for READ/WRITE MULTIPLE.
    /// Must be a power of 2 (1, 2, 4, 8, 16, 32, 64, or 128).
    /// A value of 0 means SET MULTIPLE MODE has not been issued — READ/WRITE MULTIPLE
    /// commands will be aborted.
    pub(crate) multiple_sectors: u8,
    /// Internal data buffer for sector transfers. Sized to hold up to 256 sectors
    /// (128KB) for maximum READ/WRITE MULTIPLE transfers.
    pub(crate) buffer: Vec<u8>,
    /// Current byte offset into the buffer. Incremented by each Data register
    /// read or write. When it reaches `buffer_size`, the next batch is processed.
    pub(crate) buffer_index: usize,
    /// DRQ cycle byte counter (Bochs controller_t::drq_index).
    /// Accumulates bytes read across multiple CD blocks within one DRQ cycle.
    /// When it reaches `drq_bytes`, the DRQ cycle is complete.
    pub(crate) drq_index: u32,
    /// Number of valid bytes in the buffer for the current transfer batch.
    /// For single-sector commands: 512 bytes.
    /// For IDENTIFY DEVICE: 512 bytes (256 words of device info).
    /// For multi-sector: `min(multiple_sectors, num_sectors) * sect_size`.
    pub(crate) buffer_size: usize,
    /// Internal remaining-sector counter (Bochs `controller_t::num_sectors`).
    /// Set at command start by `lba48_transform()` from `sector_count` register
    /// (0 means 256 in 28-bit mode, 65536 in 48-bit mode with both zero).
    /// Decremented by `increment_address()` after each sector.
    /// When it reaches 0, the transfer is complete and DRQ is cleared.
    pub(crate) num_sectors: u32,
    /// LBA48 flag (Bochs controller_t::lba48).
    /// Set to true when a 48-bit EXT command is issued (0x24, 0x29, 0x34, 0x39).
    pub(crate) lba48: bool,
    /// Reset in progress — set when SRST bit is written to Device Control register.
    /// Cleared when SRST is deasserted, at which point the drive signature is set.
    pub(crate) reset_in_progress: bool,
    /// Index pulse counter (Bochs harddrv.cc INDEX_PULSE_CYCLE).
    /// Incremented on each status register read. When it reaches 10,
    /// the IDX bit is set in the status byte and counter resets to 0.
    pub(crate) index_pulse_count: u8,
    /// High Order Byte registers for LBA48 (Bochs controller_t::hob).
    /// Stores the previous value of each register before a new write,
    /// allowing 48-bit addressing by reading back the previous values.
    pub(crate) hob: AtaHob,
    /// ATAPI DMA flag (Bochs controller_t::packet_dma).
    /// Set from features register bit 0 when PACKET command (0xA0) is issued.
    pub(crate) packet_dma: bool,
    /// Multiword DMA mode bitmask (Bochs controller_t::mdma_mode).
    /// Set by SET FEATURES (0xEF) sub-command 0x03 transfer mode type 0x04.
    pub(crate) mdma_mode: u8,
    /// Ultra DMA mode bitmask (Bochs controller_t::udma_mode).
    /// Set by SET FEATURES (0xEF) sub-command 0x03 transfer mode type 0x08.
    pub(crate) udma_mode: u8,
}

/// High Order Byte (HOB) registers for LBA48 addressing.
/// Each field stores the previous value of the corresponding task file register.
#[derive(Debug, Default, Clone)]
pub struct AtaHob {
    pub(crate) feature: u8,
    pub(crate) nsector: u8,
    pub(crate) sector: u8,
    pub(crate) lcyl: u8,
    pub(crate) hcyl: u8,
}

/// SCSI sense keys (SPC3r23.pdf, page 41)
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum SenseKey {
    None = 0,
    NotReady = 2,
    MediumError = 3,
    IllegalRequest = 5,
    UnitAttention = 6,
}

/// Additional Sense Code
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum Asc {
    UnrecoveredReadError = 0x11,
    IllegalOpcode = 0x20,
    LogicalBlockOor = 0x21,
    InvFieldInCmdPacket = 0x24,
    MediumMayHaveChanged = 0x28,
    MediumNotPresent = 0x3a,
}

/// ATAPI sense info (Bochs harddrv.h)
#[derive(Debug, Clone, Default)]
pub struct SenseInfo {
    pub sense_key: u8,
    pub information: [u8; 4],
    pub specific_inf: [u8; 4],
    pub key_spec: [u8; 3],
    pub fruc: u8,
    pub asc: u8,
    pub ascq: u8,
}

/// ATAPI command tracking (Bochs harddrv.h)
#[derive(Debug, Clone, Default)]
pub struct AtapiState {
    pub command: u8,
    pub drq_bytes: i32,
    pub total_bytes_remaining: i32,
}

/// CD-ROM state (Bochs harddrv.h)
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct CdromState {
    pub ready: bool,
    pub locked: bool,
    pub max_lba: u32,
    pub curr_lba: u32,
    pub next_lba: u32,
    pub remaining_blocks: i32,
}


impl Default for AtaController {
    fn default() -> Self {
        Self {
            status: AtaStatus::DRDY | AtaStatus::DSC,
            error: AtaError::from_bits_retain(0x01), // Diagnostic passed
            features: 0,
            sector_count: 1,
            sector_no: 1,
            cylinder_no: 0,
            head_no: 0,
            lba_mode: false,
            control: 0,
            interrupt_pending: false,
            current_command: 0,
            multiple_sectors: 0,
            buffer: vec![0; SECTOR_SIZE * 256],
            buffer_index: 0,
            drq_index: 0,
            buffer_size: 0,
            num_sectors: 0,
            lba48: false,
            reset_in_progress: false,
            index_pulse_count: 0,
            hob: AtaHob::default(),
            packet_dma: false,
            mdma_mode: 0,
            udma_mode: 0,
        }
    }
}

/// ATA Drive
#[derive(Debug)]
pub struct AtaDrive {
    /// Device type
    pub(crate) device_type: DeviceType,
    /// Drive geometry
    pub(crate) geometry: DriveGeometry,
    /// Model name
    pub(crate) model: String,
    /// Serial number
    pub(crate) serial: String,
    /// Firmware revision
    pub(crate) firmware: String,
    /// Controller state
    pub(crate) controller: AtaController,
    /// Image file path
    pub(crate) image_path: Option<String>,
    /// Image file (only available with std feature)
    #[cfg(feature = "std")]
    image_file: Option<File>,
    /// Raw disk data (used when std is not available)
    #[cfg(not(feature = "std"))]
    disk_data: Option<Vec<u8>>,
    /// ATAPI sense info
    pub(crate) sense: SenseInfo,
    /// ATAPI command tracking
    pub(crate) atapi: AtapiState,
    /// CD-ROM state
    pub(crate) cdrom: CdromState,
    /// Media status_changed state machine (Bochs harddrv.h)
    /// 0 = no change, 1 = newly inserted (tray open sim), -1 = tray close sim
    pub(crate) status_changed: i32,
    /// IDENTIFY PACKET DEVICE response buffer
    pub(crate) id_drive: [u16; 256],
    /// Whether identify_atapi_drive() has been called
    pub(crate) identify_set: bool,
    /// Device number (0=master, 1=slave)
    pub(crate) device_num: u8,
    /// CD-ROM ISO image file (only available with std feature)
    #[cfg(feature = "std")]
    cdrom_file: Option<File>,
}

#[allow(clippy::new_without_default)]
impl AtaDrive {
    /// Create a new empty drive
    pub fn new() -> Self {
        Self {
            device_type: DeviceType::None,
            geometry: DriveGeometry::from_chs(0, 0, 0),
            model: String::new(),
            serial: String::new(),
            firmware: String::new(),
            controller: AtaController::default(),
            image_path: None,
            #[cfg(feature = "std")]
            image_file: None,
            #[cfg(not(feature = "std"))]
            disk_data: None,
            sense: SenseInfo::default(),
            atapi: AtapiState::default(),
            cdrom: CdromState::default(),
            status_changed: 0,
            id_drive: [0u16; 256],
            identify_set: false,
            device_num: 0,
            #[cfg(feature = "std")]
            cdrom_file: None,
        }
    }

    /// Create a hard disk drive
    pub fn create_disk(geometry: DriveGeometry) -> Self {
        Self {
            device_type: DeviceType::Disk,
            geometry,
            model: String::from("RUSTY_BOX HARDDISK"),
            serial: String::from("RB000001"),
            firmware: String::from("1.0"),
            controller: AtaController::default(),
            image_path: None,
            #[cfg(feature = "std")]
            image_file: None,
            #[cfg(not(feature = "std"))]
            disk_data: None,
            sense: SenseInfo::default(),
            atapi: AtapiState::default(),
            cdrom: CdromState::default(),
            status_changed: 0,
            id_drive: [0u16; 256],
            identify_set: false,
            device_num: 0,
            #[cfg(feature = "std")]
            cdrom_file: None,
        }
    }

    /// Create a CD-ROM drive
    pub fn create_cdrom() -> Self {
        Self {
            device_type: DeviceType::Cdrom,
            geometry: DriveGeometry::from_chs(0, 0, 0),
            model: String::from("RUSTY_BOX CD-ROM"),
            serial: String::from("RBCD0001"),
            firmware: String::from("1.0"),
            controller: AtaController::default(),
            image_path: None,
            #[cfg(feature = "std")]
            image_file: None,
            #[cfg(not(feature = "std"))]
            disk_data: None,
            sense: SenseInfo::default(),
            atapi: AtapiState::default(),
            cdrom: CdromState::default(),
            status_changed: 0,
            id_drive: [0u16; 256],
            identify_set: false,
            device_num: 0,
            #[cfg(feature = "std")]
            cdrom_file: None,
        }
    }

    /// Attach a CD-ROM ISO image (requires std feature)
    #[cfg(feature = "std")]
    pub fn attach_cdrom(&mut self, path: &str) -> std::io::Result<()> {
        let file = File::options().read(true).open(path)?;
        let size = file.metadata()?.len();
        let max_lba = (size / CDROM_SECTOR_SIZE as u64) as u32;

        tracing::info!(
            "ATAPI: Attached CD-ROM '{}' ({} sectors, {} MB)",
            path,
            max_lba,
            size / (1024 * 1024)
        );

        self.image_path = Some(String::from(path));
        self.cdrom.ready = true;
        self.cdrom.max_lba = max_lba.saturating_sub(1);
        self.cdrom.curr_lba = 0;
        self.cdrom_file = Some(file);
        // Bochs cdrom_status_handler (harddrv.cc) sets status_changed=1
        // when media is inserted, so kernel sees a media-change event on first probe
        self.status_changed = 1;
        Ok(())
    }

    /// Attach a disk image file (requires std feature)
    #[cfg(feature = "std")]
    pub fn attach_image(&mut self, path: &str) -> std::io::Result<()> {
        let file = File::options().read(true).write(true).open(path)?;

        let size = file.metadata()?.len() as u32;
        self.geometry.total_sectors = size / SECTOR_SIZE as u32;

        tracing::info!(
            "ATA: Attached image '{}' ({} sectors, {} MB)",
            path,
            self.geometry.total_sectors,
            size / (1024 * 1024)
        );

        self.image_path = Some(String::from(path));
        self.image_file = Some(file);
        Ok(())
    }

    /// Attach disk data directly (for no_std environments)
    #[cfg(not(feature = "std"))]
    pub fn attach_data(&mut self, data: Vec<u8>) {
        self.geometry.total_sectors = (data.len() / SECTOR_SIZE) as u32;
        tracing::info!(
            "ATA: Attached disk data ({} sectors, {} KB)",
            self.geometry.total_sectors,
            data.len() / 1024
        );
        self.disk_data = Some(data);
    }

    /// Attach CD-ROM ISO data directly (for no_std / WASM environments)
    #[cfg(not(feature = "std"))]
    pub fn attach_cdrom_data(&mut self, data: Vec<u8>) {
        let size = data.len() as u64;
        let max_lba = (size / CDROM_SECTOR_SIZE as u64) as u32;
        tracing::info!(
            "ATAPI: Attached CD-ROM data ({} sectors, {} MB)",
            max_lba,
            size / (1024 * 1024)
        );
        self.cdrom.ready = true;
        self.cdrom.max_lba = max_lba.saturating_sub(1);
        self.cdrom.curr_lba = 0;
        self.status_changed = 1;
        self.disk_data = Some(data);
    }

    /// Read a single CD-ROM block (2048 bytes) at the given LBA
    #[cfg(feature = "std")]
    fn read_cdrom_block(&mut self, lba: u32, buf: &mut [u8]) -> bool {
        let offset = lba as u64 * CDROM_SECTOR_SIZE as u64;
        let file = match self.cdrom_file.as_mut() {
            Some(f) => f,
            None => return false,
        };
        if file.seek(SeekFrom::Start(offset)).is_err() {
            return false;
        }
        if file
            .read_exact(&mut buf[..CDROM_SECTOR_SIZE])
            .is_err()
        {
            return false;
        }
        true
    }

    /// Read a single CD-ROM block from in-memory data (no_std)
    #[cfg(not(feature = "std"))]
    fn read_cdrom_block(&mut self, lba: u32, buf: &mut [u8]) -> bool {
        let data = match self.disk_data.as_ref() {
            Some(d) => d,
            None => return false,
        };
        let offset = lba as usize * CDROM_SECTOR_SIZE;
        let end = offset + CDROM_SECTOR_SIZE;
        if end > data.len() {
            return false;
        }
        buf[..CDROM_SECTOR_SIZE].copy_from_slice(&data[offset..end]);
        true
    }

    /// Read multiple consecutive CD-ROM blocks in one file I/O call.
    /// Returns the number of blocks successfully read.
    #[cfg(feature = "std")]
    fn read_cdrom_blocks(&mut self, start_lba: u32, count: usize, buf: &mut [u8]) -> usize {
        let offset = start_lba as u64 * CDROM_SECTOR_SIZE as u64;
        let file = match self.cdrom_file.as_mut() {
            Some(f) => f,
            None => return 0,
        };
        if file.seek(SeekFrom::Start(offset)).is_err() {
            return 0;
        }
        let total_bytes = count * CDROM_SECTOR_SIZE;
        if buf.len() < total_bytes {
            return 0;
        }
        match file.read_exact(&mut buf[..total_bytes]) {
            Ok(()) => count,
            Err(_) => 0,
        }
    }

    /// Read multiple consecutive CD-ROM blocks from in-memory data (no_std)
    #[cfg(not(feature = "std"))]
    fn read_cdrom_blocks(&mut self, start_lba: u32, count: usize, buf: &mut [u8]) -> usize {
        let data = match self.disk_data.as_ref() {
            Some(d) => d,
            None => return 0,
        };
        let offset = start_lba as usize * CDROM_SECTOR_SIZE;
        let total_bytes = count * CDROM_SECTOR_SIZE;
        if buf.len() < total_bytes {
            return 0;
        }
        let end = offset + total_bytes;
        if end > data.len() {
            return 0;
        }
        buf[..total_bytes].copy_from_slice(&data[offset..end]);
        count
    }

    /// Fill the IDENTIFY PACKET DEVICE response (Bochs identify_ATAPI_drive, harddrv.cc)
    fn identify_atapi_drive(&mut self) {
        self.id_drive = [0u16; 256];

        // Word 0: General config — ATAPI device, removable, CMD DRQ, 12-byte packets
        self.id_drive[0] = (2 << 14) | (5 << 8) | (1 << 7) | (2 << 5);

        // Words 10-19: Serial number (ASCII, byte-swapped pairs)
        let serial = b"RBCD0001            "; // 20 chars
        for i in 0..10 {
            let hi = serial[i * 2] as u16;
            let lo = serial[i * 2 + 1] as u16;
            self.id_drive[10 + i] = (hi << 8) | lo;
        }

        // Words 23-26: Firmware revision (8 chars)
        let fw = b"1.0     ";
        for i in 0..4 {
            let hi = fw[i * 2] as u16;
            let lo = fw[i * 2 + 1] as u16;
            self.id_drive[23 + i] = (hi << 8) | lo;
        }

        // Words 27-46: Model name (40 chars)
        let model = b"RUSTY_BOX CD-ROM                        ";
        for i in 0..20 {
            let hi = model[i * 2] as u16;
            let lo = model[i * 2 + 1] as u16;
            self.id_drive[27 + i] = (hi << 8) | lo;
        }

        // Word 49: Capabilities — LBA + DMA supported
        // Bochs harddrv.cc: (1<<9)|(1<<8)
        self.id_drive[49] = (1 << 9) | (1 << 8);

        // Word 53: Field validity (words 64-70 valid, words 54-58 valid, words 88 valid)
        // Bochs harddrv.cc: 7
        self.id_drive[53] = 7;

        // Word 63: Multiword DMA modes supported (bits 0-2) and active (bits 8-10)
        // Bochs harddrv.cc: modes 0-2 supported, active mode from mdma_mode
        self.id_drive[63] = 0x07; // MDMA modes 0-2 supported
        if self.controller.mdma_mode > 0 {
            self.id_drive[63] |= (self.controller.mdma_mode as u16) << 8;
        }

        // Word 64: PIO modes supported — PIO mode 0
        self.id_drive[64] = 1;

        // Word 65: Minimum PIO transfer cycle time
        self.id_drive[65] = 0x02E8; // 746 ns

        // Word 73: ATAPI byte count 0 limit
        self.id_drive[73] = 1; // number of bytes for ATAPI

        // Word 80: Major version — ATA/ATAPI-6
        // Bochs harddrv.cc: 0x7e
        self.id_drive[80] = 0x7E;

        // Word 88: Ultra DMA modes supported (bits 0-5) and active (bits 8-13)
        // Bochs harddrv.cc: modes 0-5 supported, active mode from udma_mode
        self.id_drive[88] = 0x3F; // UDMA modes 0-5 supported
        if self.controller.udma_mode > 0 {
            self.id_drive[88] |= (self.controller.udma_mode as u16) << 8;
        }

        self.identify_set = true;
    }

    /// Initialize `num_sectors` from the `sector_count` register.
    ///
    /// Matches Bochs `lba48_transform()` (harddrv.cc).
    /// Called at the start of every READ/WRITE/VERIFY command to set up the
    /// internal transfer counter.
    ///
    /// In the ATA spec, a sector count of 0 means 256 sectors (for 28-bit commands)
    /// or 65536 sectors (for 48-bit LBA48 commands). We only support 28-bit LBA,
    /// so `num_sectors = sector_count` or 256 if `sector_count == 0`.
    fn lba48_transform(&mut self, lba48: bool) {
        self.controller.lba48 = lba48;
        if !lba48 {
            // 28-bit mode: 0 means 256 sectors
            if self.controller.sector_count == 0 {
                self.controller.num_sectors = 256;
            } else {
                self.controller.num_sectors = self.controller.sector_count as u32;
            }
        } else {
            // 48-bit mode: use HOB (High Order Byte) nsector
            if self.controller.sector_count == 0 && self.controller.hob.nsector == 0 {
                self.controller.num_sectors = 65536;
            } else {
                self.controller.num_sectors = ((self.controller.hob.nsector as u32) << 8)
                    | self.controller.sector_count as u32;
            }
        }
    }

    /// Advance CHS/LBA registers to the next sector and decrement counters.
    ///
    /// Called after each sector is successfully read from or written to disk.
    /// Matches Bochs `increment_address()` (harddrv.cc).
    ///
    /// In **LBA mode**: increments the 28-bit LBA value spread across
    /// `sector_no` (bits 7:0), `cylinder_no` (bits 23:8), and `head_no` (bits 27:24).
    ///
    /// In **CHS mode**: increments sector number. If it exceeds sectors_per_track,
    /// wraps to sector 1 and increments head. If head exceeds max, wraps to head 0
    /// and increments cylinder.
    ///
    /// Both `sector_count` (the visible register) and `num_sectors` (the internal
    /// counter) are decremented. The transfer is complete when `num_sectors` reaches 0.
    fn increment_address(&mut self) {
        self.controller.sector_count = self.controller.sector_count.wrapping_sub(1);
        self.controller.num_sectors = self.controller.num_sectors.wrapping_sub(1);

        if self.controller.lba_mode {
            if !self.controller.lba48 {
                // LBA28: increment the 28-bit LBA value stored across registers
                let logical_sector = self.get_lba() as u64 + 1;
                self.controller.head_no = ((logical_sector >> 24) & 0xf) as u8;
                self.controller.cylinder_no = ((logical_sector >> 8) & 0xffff) as u16;
                self.controller.sector_no = (logical_sector & 0xff) as u8;
            } else {
                // LBA48: update both current and HOB registers (Bochs harddrv.cc)
                let curr_lba = ((self.controller.hob.hcyl as u64) << 40)
                    | ((self.controller.hob.lcyl as u64) << 32)
                    | ((self.controller.hob.sector as u64) << 24)
                    | ((self.controller.cylinder_no as u64) << 8)
                    | (self.controller.sector_no as u64);
                let next_lba = curr_lba + 1;
                self.controller.hob.hcyl = ((next_lba >> 40) & 0xff) as u8;
                self.controller.hob.lcyl = ((next_lba >> 32) & 0xff) as u8;
                self.controller.hob.sector = ((next_lba >> 24) & 0xff) as u8;
                self.controller.cylinder_no = ((next_lba >> 8) & 0xffff) as u16;
                self.controller.sector_no = (next_lba & 0xff) as u8;
            }
        } else {
            // CHS mode: increment sector, wrap to next head/cylinder
            self.controller.sector_no += 1;
            if self.controller.sector_no > self.geometry.sectors_per_track {
                self.controller.sector_no = 1;
                self.controller.head_no += 1;
                if self.controller.head_no >= self.geometry.heads {
                    self.controller.head_no = 0;
                    self.controller.cylinder_no += 1;
                    if self.controller.cylinder_no >= self.geometry.cylinders {
                        self.controller.cylinder_no = self.geometry.cylinders - 1;
                    }
                }
            }
        }
    }

    /// Read one or more sectors into the controller buffer from the disk image.
    ///
    /// Matches Bochs `ide_read_sector()` (harddrv.cc).
    ///
    /// For each sector in the batch (`buffer_size / SECTOR_SIZE`):
    /// 1. Calculates the LBA from the current task file registers (CHS or LBA mode)
    /// 2. Seeks to the byte offset in the disk image file
    /// 3. Reads 512 bytes into the corresponding buffer slice
    /// 4. Calls `increment_address()` to advance registers and decrement `num_sectors`
    ///
    /// Returns `false` on disk I/O error (seek or read failure), which causes
    /// the caller to abort the command.
    #[cfg(feature = "std")]
    fn ide_read_sector(&mut self) -> bool {
        let sector_count = self.controller.buffer_size / SECTOR_SIZE;
        let mut buf_offset = 0;

        for _ in 0..sector_count {
            let lba = self.get_lba();
            let offset = lba as u64 * SECTOR_SIZE as u64;

            let file = match self.image_file.as_mut() {
                Some(f) => f,
                None => return false,
            };

            if file.seek(SeekFrom::Start(offset)).is_err() {
                tracing::error!("ATA: ide_read_sector: seek failed at LBA {}", lba);
                return false;
            }

            if file
                .read_exact(&mut self.controller.buffer[buf_offset..buf_offset + SECTOR_SIZE])
                .is_err()
            {
                tracing::error!("ATA: ide_read_sector: read failed at LBA {}", lba);
                return false;
            }

            tracing::debug!("ATA: read LBA {}", lba);

            self.increment_address();
            buf_offset += SECTOR_SIZE;
        }

        tracing::debug!(
            "ATA: ide_read_sector: read {} sector(s), num_sectors remaining={}",
            sector_count,
            self.controller.num_sectors
        );
        true
    }

    /// Read buffer_size/512 sectors into controller buffer (no_std version).
    #[cfg(not(feature = "std"))]
    fn ide_read_sector(&mut self) -> bool {
        let sector_count = self.controller.buffer_size / SECTOR_SIZE;
        let mut buf_offset = 0;

        for _ in 0..sector_count {
            let lba = self.get_lba();
            let disk_offset = lba as usize * SECTOR_SIZE;

            let data = match self.disk_data.as_ref() {
                Some(d) => d,
                None => return false,
            };

            if disk_offset + SECTOR_SIZE > data.len() {
                return false;
            }

            self.controller.buffer[buf_offset..buf_offset + SECTOR_SIZE]
                .copy_from_slice(&data[disk_offset..disk_offset + SECTOR_SIZE]);

            self.increment_address();
            buf_offset += SECTOR_SIZE;
        }

        true
    }

    /// Write buffer_size/512 sectors from controller buffer to disk at current register position.
    /// Matches Bochs ide_write_sector() (harddrv.cc).
    #[cfg(feature = "std")]
    fn ide_write_sector(&mut self) -> bool {
        let sector_count = self.controller.buffer_size / SECTOR_SIZE;
        let mut buf_offset = 0;

        for _ in 0..sector_count {
            let lba = self.get_lba();
            let offset = lba as u64 * SECTOR_SIZE as u64;

            let file = match self.image_file.as_mut() {
                Some(f) => f,
                None => return false,
            };

            if file.seek(SeekFrom::Start(offset)).is_err() {
                tracing::error!("ATA: ide_write_sector: seek failed at LBA {}", lba);
                return false;
            }

            if file
                .write_all(&self.controller.buffer[buf_offset..buf_offset + SECTOR_SIZE])
                .is_err()
            {
                tracing::error!("ATA: ide_write_sector: write failed at LBA {}", lba);
                return false;
            }

            self.increment_address();
            buf_offset += SECTOR_SIZE;
        }

        if let Some(f) = self.image_file.as_mut() {
            f.flush().ok();
        }

        true
    }

    /// Write buffer_size/512 sectors from controller buffer to disk (no_std version).
    #[cfg(not(feature = "std"))]
    fn ide_write_sector(&mut self) -> bool {
        let sector_count = self.controller.buffer_size / SECTOR_SIZE;
        let mut buf_offset = 0;

        for _ in 0..sector_count {
            let lba = self.get_lba();
            let disk_offset = lba as usize * SECTOR_SIZE;

            let data = match self.disk_data.as_mut() {
                Some(d) => d,
                None => return false,
            };

            if disk_offset + SECTOR_SIZE > data.len() {
                return false;
            }

            data[disk_offset..disk_offset + SECTOR_SIZE]
                .copy_from_slice(&self.controller.buffer[buf_offset..buf_offset + SECTOR_SIZE]);

            self.increment_address();
            buf_offset += SECTOR_SIZE;
        }

        true
    }

    /// Fill identify buffer
    fn fill_identify_buffer(&mut self) {
        let buf = &mut self.controller.buffer;
        buf.fill(0);

        // Word 0: General configuration
        buf[0] = 0x40; // Fixed drive
        buf[1] = 0x00;

        // Word 1: Number of cylinders
        let cyls = self.geometry.cylinders;
        buf[2] = (cyls & 0xFF) as u8;
        buf[3] = (cyls >> 8) as u8;

        // Word 3: Number of heads
        buf[6] = self.geometry.heads;
        buf[7] = 0;

        // Word 4: Unformatted bytes per track (sect_size * spt)
        let bytes_per_track = SECTOR_SIZE as u16 * self.geometry.sectors_per_track as u16;
        buf[8] = (bytes_per_track & 0xFF) as u8;
        buf[9] = (bytes_per_track >> 8) as u8;

        // Word 5: Unformatted bytes per sector (sect_size = 512) — used as blksize by BIOS
        buf[10] = (SECTOR_SIZE & 0xFF) as u8;
        buf[11] = ((SECTOR_SIZE >> 8) & 0xFF) as u8;

        // Word 6: Sectors per track
        buf[12] = self.geometry.sectors_per_track;
        buf[13] = 0;

        // Words 10-19: Serial number (20 ASCII chars)
        let serial_bytes = self.serial.as_bytes();
        for i in 0..10 {
            let idx = 20 + i * 2;
            if i * 2 < serial_bytes.len() {
                buf[idx + 1] = serial_bytes[i * 2];
            } else {
                buf[idx + 1] = b' ';
            }
            if i * 2 + 1 < serial_bytes.len() {
                buf[idx] = serial_bytes[i * 2 + 1];
            } else {
                buf[idx] = b' ';
            }
        }

        // Word 20: buffer type (3 = dual-ported multi-sector with read caching)
        buf[40] = 3;
        buf[41] = 0;

        // Word 21: buffer size in 512-byte increments (512 = 256kB cache)
        buf[42] = 0x00;
        buf[43] = 0x02; // 0x0200 = 512

        // Word 22: # of ECC bytes available on read/write long commands
        buf[44] = 4;
        buf[45] = 0;

        // Words 23-26: Firmware revision (8 ASCII chars)
        let fw_bytes = self.firmware.as_bytes();
        for i in 0..4 {
            let idx = 46 + i * 2;
            if i * 2 < fw_bytes.len() {
                buf[idx + 1] = fw_bytes[i * 2];
            } else {
                buf[idx + 1] = b' ';
            }
            if i * 2 + 1 < fw_bytes.len() {
                buf[idx] = fw_bytes[i * 2 + 1];
            } else {
                buf[idx] = b' ';
            }
        }

        // Words 27-46: Model number (40 ASCII chars)
        let model_bytes = self.model.as_bytes();
        for i in 0..20 {
            let idx = 54 + i * 2;
            if i * 2 < model_bytes.len() {
                buf[idx + 1] = model_bytes[i * 2];
            } else {
                buf[idx + 1] = b' ';
            }
            if i * 2 + 1 < model_bytes.len() {
                buf[idx] = model_bytes[i * 2 + 1];
            } else {
                buf[idx] = b' ';
            }
        }

        // Word 47: Maximum sectors per multiple command
        buf[94] = 16;
        buf[95] = 0x00; // Bochs: id_drive[47] = MAX_MULTIPLE_SECTORS = 16 (high byte must be 0x00)

        // Word 48: PIO32 support (1 = 32-bit PIO supported)
        buf[96] = 0x01;
        buf[97] = 0x00;

        // Word 49: Capabilities
        buf[98] = 0x00;
        buf[99] = 0x02; // LBA supported

        // Word 51: PIO data transfer cycle timing mode (0x0200 = mode 2)
        buf[102] = 0x00;
        buf[103] = 0x02;

        // Word 52: DMA data transfer cycle timing mode (0x0200 = mode 2)
        buf[104] = 0x00;
        buf[105] = 0x02;

        // Word 53: Field validity
        buf[106] = 0x07;
        buf[107] = 0x00;

        // Word 54-56: Current CHS
        buf[108] = (cyls & 0xFF) as u8;
        buf[109] = (cyls >> 8) as u8;
        buf[110] = self.geometry.heads;
        buf[111] = 0;
        buf[112] = self.geometry.sectors_per_track;
        buf[113] = 0;

        // Word 57-58: Current capacity in sectors
        let total = self.geometry.total_sectors;
        buf[114] = (total & 0xFF) as u8;
        buf[115] = ((total >> 8) & 0xFF) as u8;
        buf[116] = ((total >> 16) & 0xFF) as u8;
        buf[117] = ((total >> 24) & 0xFF) as u8;

        // Word 59: Multiple sector setting (Bochs harddrv.cc)
        // Low byte = current multiple sector count, bit 8 = valid if multiple mode active
        if self.controller.multiple_sectors > 0 {
            let w59 = 0x0100u16 | self.controller.multiple_sectors as u16;
            buf[118] = (w59 & 0xFF) as u8;
            buf[119] = (w59 >> 8) as u8;
        }

        // Word 60-61: Total addressable sectors (LBA)
        buf[120] = (total & 0xFF) as u8;
        buf[121] = ((total >> 8) & 0xFF) as u8;
        buf[122] = ((total >> 16) & 0xFF) as u8;
        buf[123] = ((total >> 24) & 0xFF) as u8;

        // Word 64: PIO modes supported (0 = none beyond PIO2)
        // (buf[128-129] = 0 from fill)

        // Words 65-68: PIO/DMA cycle time in nanoseconds (120 ns each)
        buf[130] = 120;
        buf[131] = 0x00;
        buf[132] = 120;
        buf[133] = 0x00;
        buf[134] = 120;
        buf[135] = 0x00;
        buf[136] = 120;
        buf[137] = 0x00;

        // Word 80: Major ATA version number (Bochs harddrv.cc)
        // Bits 1-6: ATA-1 through ATA-6 supported
        buf[160] = 0x7E; // supports ATA-1 through ATA-6
        buf[161] = 0x00;

        // Word 82: Command set supported 1 (Bochs harddrv.cc)
        // Bit 14: NOP supported, bit 5: write cache, bit 4: packet, bit 0: SMART
        buf[164] = 0x00;
        buf[165] = 0x40; // NOP supported

        // Word 83: Command set supported 2 (Bochs harddrv.cc)
        // Bit 14: must be ONE, bit 13: FLUSH CACHE EXT, bit 12: FLUSH CACHE, bit 10: 48-bit LBA
        // = (1<<14)|(1<<13)|(1<<12)|(1<<10) = 0x7400
        buf[166] = 0x00;
        buf[167] = 0x74;

        // Word 84: Command set/feature supported extension (Bochs harddrv.cc)
        // Bit 14: must be 1
        buf[168] = 0x00;
        buf[169] = 0x40;

        // Word 85: Command set enabled 1 (Bochs harddrv.cc)
        buf[170] = 0x00;
        buf[171] = 0x40; // NOP enabled

        // Word 86: Command set enabled 2 (Bochs harddrv.cc)
        // Bit 14: must be ONE, bit 13: FLUSH CACHE EXT enabled, bit 12: FLUSH CACHE, bit 10: 48-bit LBA
        // = (1<<14)|(1<<13)|(1<<12)|(1<<10) = 0x7400
        buf[172] = 0x00;
        buf[173] = 0x74;

        // Word 87: Command set/feature default (Bochs harddrv.cc)
        buf[174] = 0x00;
        buf[175] = 0x40;

        // Word 93: Hardware reset result (Bochs harddrv.cc)
        // = 1 | (1<<14) | 0x2000 = 0x6001
        buf[186] = 0x01;
        buf[187] = 0x60;

        // Words 100-103: 48-bit total number of sectors (Bochs harddrv.cc)
        buf[200] = (total & 0xFF) as u8;
        buf[201] = ((total >> 8) & 0xFF) as u8;
        buf[202] = ((total >> 16) & 0xFF) as u8;
        buf[203] = ((total >> 24) & 0xFF) as u8;
        // buf[204-207] = 0 (total < 2^32 for any reasonable disk)

        self.controller.buffer_size = 512;
        self.controller.buffer_index = 0;
    }

    /// Get current LBA from registers
    fn get_lba(&self) -> u32 {
        if self.controller.lba_mode {
            (self.controller.head_no as u32 & 0x0F) << 24
                | (self.controller.cylinder_no as u32) << 8
                | self.controller.sector_no as u32
        } else {
            self.geometry.chs_to_lba(
                self.controller.cylinder_no,
                self.controller.head_no,
                self.controller.sector_no,
            )
        }
    }

    /// Calculate logical sector address with bounds checking.
    /// Matches Bochs calculate_logical_address() (harddrv.cc).
    /// Returns None if the address is out of bounds.
    fn calculate_logical_address(&self) -> Option<i64> {
        let logical_sector: i64;
        if self.controller.lba_mode {
            if !self.controller.lba48 {
                // 28-bit LBA
                logical_sector = ((self.controller.head_no as i64) << 24)
                    | ((self.controller.cylinder_no as i64) << 8)
                    | (self.controller.sector_no as i64);
            } else {
                // 48-bit LBA
                logical_sector = ((self.controller.hob.hcyl as i64) << 40)
                    | ((self.controller.hob.lcyl as i64) << 32)
                    | ((self.controller.hob.sector as i64) << 24)
                    | ((self.controller.cylinder_no as i64) << 8)
                    | (self.controller.sector_no as i64);
            }
        } else {
            // CHS mode
            logical_sector = (self.controller.cylinder_no as i64
                * self.geometry.heads as i64
                * self.geometry.sectors_per_track as i64)
                + (self.controller.head_no as i64 * self.geometry.sectors_per_track as i64)
                + (self.controller.sector_no as i64 - 1);
        }

        let sector_count = self.geometry.total_sectors as i64;
        if logical_sector >= sector_count {
            tracing::error!(
                "ATA: logical address out of bounds ({}/{}) - aborting command",
                logical_sector,
                sector_count
            );
            return None;
        }
        Some(logical_sector)
    }
}

/// ATA Channel (primary or secondary)
#[derive(Debug)]
pub struct AtaChannel {
    /// Base I/O address
    pub(crate) ioaddr1: u16,
    /// Control I/O address
    pub(crate) ioaddr2: u16,
    /// IRQ number
    pub(crate) irq: u8,
    /// Master and slave drives
    pub(crate) drives: [AtaDrive; 2],
    /// Currently selected drive (0=master, 1=slave)
    pub(crate) drive_select: u8,
}

impl AtaChannel {
    /// Create a new ATA channel
    pub fn new(ioaddr1: u16, ioaddr2: u16, irq: u8) -> Self {
        Self {
            ioaddr1,
            ioaddr2,
            irq,
            drives: [AtaDrive::new(), AtaDrive::new()],
            drive_select: 0,
        }
    }

    /// Get the currently selected drive
    pub fn selected_drive(&self) -> &AtaDrive {
        &self.drives[self.drive_select as usize]
    }

    /// Get the currently selected drive mutably
    pub fn selected_drive_mut(&mut self) -> &mut AtaDrive {
        &mut self.drives[self.drive_select as usize]
    }
}

/// ATA/IDE Hard Drive Controller
#[derive(Debug)]
pub struct BxHardDriveC {
    /// ATA channels
    pub(crate) channels: [AtaChannel; 2],
    /// Deferred seek completion flag per channel (Bochs seek_timer pattern).
    /// When set, the emulator's tick loop calls ready_to_send_atapi().
    /// Matches Bochs start_seek() + seek_timer_handler() flow.
    pub(crate) seek_complete_pending: [bool; 2],
    /// Command history ring buffer (last 32 commands) for diagnostics
    pub(crate) cmd_history: Vec<(u8, u8, u32)>,
}

impl Default for BxHardDriveC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxHardDriveC {
    /// Create a new hard drive controller
    pub fn new() -> Self {
        Self {
            channels: [
                AtaChannel::new(0x1F0, 0x3F0, 14), // Primary
                AtaChannel::new(0x170, 0x370, 15), // Secondary
            ],
            cmd_history: Vec::new(),
            seek_complete_pending: [false; 2],
        }
    }

    /// Initialize the hard drive controller
    pub fn init(&mut self) {
        tracing::info!("HardDrive: Initializing ATA/IDE Controller");
        self.reset();
    }

    /// Reset the controller
    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            for drive in &mut channel.drives {
                drive.controller = AtaController::default();
                // Set drive signature (Bochs set_signature): head_no=0, sector_count=1, sector_no=1
                drive.controller.head_no = 0;
                drive.controller.sector_count = 1;
                drive.controller.sector_no = 1;
                // HD → cylinder_no=0, CDROM → 0xEB14 (ATAPI signature), absent → 0xFFFF
                match drive.device_type {
                    DeviceType::Disk => drive.controller.cylinder_no = 0,
                    DeviceType::Cdrom => drive.controller.cylinder_no = 0xEB14,
                    DeviceType::None => drive.controller.cylinder_no = 0xFFFF,
                }
            }
            channel.drive_select = 0;
        }
    }

    /// Attach a disk image to a drive (requires std feature)
    #[cfg(feature = "std")]
    pub fn attach_disk(
        &mut self,
        channel: usize,
        drive: usize,
        path: &str,
        cylinders: u16,
        heads: u8,
        spt: u8,
    ) -> std::io::Result<()> {
        let geometry = DriveGeometry::from_chs(cylinders, heads, spt);
        self.channels[channel].drives[drive] = AtaDrive::create_disk(geometry);
        self.channels[channel].drives[drive].attach_image(path)?;
        Ok(())
    }

    /// Attach disk data to a drive (for no_std environments)
    #[cfg(not(feature = "std"))]
    pub fn attach_disk_data(
        &mut self,
        channel: usize,
        drive: usize,
        data: Vec<u8>,
        cylinders: u16,
        heads: u8,
        spt: u8,
    ) {
        let geometry = DriveGeometry::from_chs(cylinders, heads, spt);
        self.channels[channel].drives[drive] = AtaDrive::create_disk(geometry);
        self.channels[channel].drives[drive].attach_data(data);
    }

    /// Attach a CD-ROM ISO image to a drive (requires std feature)
    #[cfg(feature = "std")]
    pub fn attach_cdrom_image(
        &mut self,
        channel: usize,
        drive: usize,
        path: &str,
    ) -> std::io::Result<()> {
        self.channels[channel].drives[drive] = AtaDrive::create_cdrom();
        self.channels[channel].drives[drive].device_num = drive as u8;
        self.channels[channel].drives[drive].attach_cdrom(path)?;
        // Set ATAPI signature so kernel detects device type correctly
        // (reset() may have run before attach, so cylinder_no is still default 0)
        let d = &mut self.channels[channel].drives[drive];
        d.controller.head_no = 0;
        d.controller.sector_count = 1;
        d.controller.sector_no = 1;
        d.controller.cylinder_no = 0xEB14; // ATAPI signature
        d.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
        Ok(())
    }

    /// Attach CD-ROM ISO data to a drive (for no_std / WASM environments)
    #[cfg(not(feature = "std"))]
    pub fn attach_cdrom_data(
        &mut self,
        channel: usize,
        drive: usize,
        data: Vec<u8>,
    ) {
        self.channels[channel].drives[drive] = AtaDrive::create_cdrom();
        self.channels[channel].drives[drive].device_num = drive as u8;
        self.channels[channel].drives[drive].attach_cdrom_data(data);
        // Set ATAPI signature so kernel detects device type correctly
        let d = &mut self.channels[channel].drives[drive];
        d.controller.head_no = 0;
        d.controller.sector_count = 1;
        d.controller.sector_no = 1;
        d.controller.cylinder_no = 0xEB14; // ATAPI signature
        d.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
    }

    /// Determine which channel a port belongs to
    fn port_to_channel(&self, port: u16) -> Option<usize> {
        if (0x1F0..=0x1F7).contains(&port) || port == 0x3F6 {
            Some(0) // Primary
        } else if (0x170..=0x177).contains(&port) || port == 0x376 {
            Some(1) // Secondary
        } else {
            None
        }
    }

    /// Raise interrupt for a channel if interrupts are enabled (nIEN=0).
    ///
    /// Matches Bochs `raise_interrupt()` (harddrv.cc).
    /// Checks the nIEN bit (Device Control register bit 1). If nIEN=0 (interrupts
    /// enabled), sets the per-drive `interrupt_pending` flag and the channel-level
    /// `irqN_pending` flag which the main emulator loop checks to deliver the
    /// hardware interrupt via the PIC (IRQ14 for primary, IRQ15 for secondary).
    ///
    /// If nIEN=1 (interrupts disabled), this is a no-op. The BSY/DRQ/status bits
    /// are still updated by the caller — the host can poll Alternate Status instead.
    fn raise_interrupt(&mut self, channel_num: usize, pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) {
        let drive = self.channels[channel_num].selected_drive_mut();
        // Always record that the drive wants an interrupt (the drive asserts
        // its interrupt line regardless of nIEN). Bochs doesn't have this field
        // but we need it because our commands complete synchronously — before the
        // kernel clears nIEN. When nIEN transitions 1→0 (in the control register
        // write handler), we check interrupt_pending and raise the PIC IRQ then.
        drive.controller.interrupt_pending = true;

        // Only raise PIC IRQ if nIEN bit (bit 1 of control register) is clear.
        // Matches Bochs: raise_interrupt() calls DEV_pic_raise_irq() directly.
        if (drive.controller.control & 0x02) == 0 {
            let irq = match channel_num {
                0 => 14u8,
                _ => 15u8,
            };
            // Bochs harddrv.cc: DEV_ide_bmdma_set_irq(channel)
            pci_ide.bmdma_set_irq(channel_num as u8);
            // Bochs harddrv.cc: DEV_pic_raise_irq(irq)
            // PIC forwards to IOAPIC synchronously (Bochs pic.cc).
            pic.raise_irq(irq);
        }
    }

    /// Get the current IRQ level for a channel (level-based, matching Bochs).
    ///
    /// Returns true if the interrupt line should be HIGH (interrupt pending
    /// and not masked by nIEN). Called every tick by the device manager to
    /// update the PIC via set_irq_level().
    pub fn get_irq_level(&self, channel_num: usize) -> bool {
        let drive = self.channels[channel_num].selected_drive();
        drive.controller.interrupt_pending && (drive.controller.control & 0x02) == 0
    }

    // ─── BM-DMA Callbacks (Bochs harddrv.cc) ──────────────────

    /// Read a sector/block for BM-DMA transfer.
    /// Bochs: bx_hard_drive_c::bmdma_read_sector() (harddrv.cc)
    ///
    /// For ATA READ DMA (0xC8/0x25): reads a 512-byte sector from disk.
    /// For ATAPI PACKET with packet_dma: reads a CD-ROM block.
    /// Returns false if no more data available.
    pub fn bmdma_read_sector(
        &mut self,
        channel: u8,
        buffer: &mut [u8],
        sector_size: &mut u32,
        pic: &mut super::pic::BxPicC,
        pci_ide: &mut super::pci_ide::BxPciIde,
    ) -> bool {
        let ch = channel as usize;
        if ch >= 2 {
            return false;
        }
        let selected = self.channels[ch].drive_select;
        let current_command = self.channels[ch].drives[selected as usize].controller.current_command;

        if current_command == 0xC8 || current_command == 0x25 {
            // ATA READ DMA / READ DMA EXT
            // Bochs harddrv.cc
            let drive = &mut self.channels[ch].drives[selected as usize];
            *sector_size = SECTOR_SIZE as u32;
            if drive.controller.num_sectors == 0 {
                return false;
            }
            if !drive.ide_read_sector() {
                return false;
            }
            let bs = drive.controller.buffer_size.min(buffer.len());
            buffer[..bs].copy_from_slice(&drive.controller.buffer[..bs]);
            return true;
        } else if current_command == ATA_CMD_PACKET {
            // ATAPI PACKET with packet_dma
            // Bochs harddrv.cc
            let drive = &mut self.channels[ch].drives[selected as usize];
            if !drive.controller.packet_dma {
                tracing::warn!("ATAPI: PACKET-DMA not active");
                self.command_aborted(ch, current_command, pic, pci_ide);
                return false;
            }
            let atapi_cmd = drive.atapi.command;
            match atapi_cmd {
                0x28 | 0xA8 | 0xBE => {
                    // READ(10), READ(12), READ CD
                    *sector_size = drive.controller.buffer_size as u32;
                    if !drive.cdrom.ready {
                        tracing::warn!("ATAPI: read with CD-ROM not ready");
                        return false;
                    }
                    let next_lba = drive.cdrom.next_lba;
                    let buf_size = drive.controller.buffer_size;
                    if buf_size > buffer.len() {
                        return false;
                    }
                    if !drive.read_cdrom_block(next_lba, buffer) {
                        tracing::warn!("ATAPI: DMA read block {} failed", next_lba);
                        return false;
                    }
                    drive.cdrom.next_lba += 1;
                    drive.cdrom.remaining_blocks -= 1;
                    if drive.cdrom.remaining_blocks <= 0 {
                        drive.cdrom.curr_lba = drive.cdrom.next_lba;
                    }
                    return true;
                }
                _ => {
                    // Other ATAPI commands: copy from controller buffer
                    // Bochs harddrv.cc
                    let remaining = drive.atapi.total_bytes_remaining as u32;
                    let copy_size = if *sector_size > remaining {
                        remaining as usize
                    } else {
                        *sector_size as usize
                    };
                    let copy_size = copy_size.min(buffer.len()).min(drive.controller.buffer_size);
                    buffer[..copy_size]
                        .copy_from_slice(&drive.controller.buffer[..copy_size]);
                    return true;
                }
            }
        }

        tracing::warn!("BM-DMA read: command {:#04x} not a DMA command", current_command);
        self.command_aborted(channel as usize, current_command, pic, pci_ide);
        false
    }

    /// Write a sector for BM-DMA transfer.
    /// Bochs: bx_hard_drive_c::bmdma_write_sector() (harddrv.cc)
    pub fn bmdma_write_sector(&mut self, channel: u8, buffer: &[u8], pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) -> bool {
        let ch = channel as usize;
        if ch >= 2 {
            return false;
        }
        let selected = self.channels[ch].drive_select;
        let current_command = self.channels[ch].drives[selected as usize].controller.current_command;

        if current_command != 0xCA && current_command != 0x35 {
            tracing::warn!("BM-DMA write: command {:#04x} not a DMA write", current_command);
            self.command_aborted(ch, current_command, pic, pci_ide);
            return false;
        }
        let drive = &mut self.channels[ch].drives[selected as usize];
        if drive.controller.num_sectors == 0 {
            return false;
        }
        // Copy data into controller buffer and write sector
        let copy_len = buffer.len().min(drive.controller.buffer.len());
        drive.controller.buffer[..copy_len].copy_from_slice(&buffer[..copy_len]);
        drive.controller.buffer_size = SECTOR_SIZE;
        if !drive.ide_write_sector() {
            return false;
        }
        true
    }

    /// Complete a BM-DMA transfer — set final status and raise interrupt.
    /// Bochs: bx_hard_drive_c::bmdma_complete() (harddrv.cc)
    pub fn bmdma_complete(&mut self, channel: u8, pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) {
        let ch = channel as usize;
        if ch >= 2 {
            return;
        }
        let selected = self.channels[ch].drive_select;
        let is_cdrom = self.channels[ch].drives[selected as usize].device_type == DeviceType::Cdrom;
        let drive = &mut self.channels[ch].drives[selected as usize];

        // Bochs harddrv.cc
        drive.controller.status.remove(AtaStatus::BSY | AtaStatus::DRQ | AtaStatus::ERR);
        drive.controller.status.insert(AtaStatus::DRDY);
        if is_cdrom {
            // Bochs harddrv.cc: set interrupt_reason I/O=1, C/D=1
            drive.controller.sector_count = (drive.controller.sector_count & 0xF8) | 0x03;
        } else {
            // Bochs harddrv.cc: disk completion
            drive.controller.status.remove(AtaStatus::DWF);
            drive.controller.status.insert(AtaStatus::DSC);
        }

        self.raise_interrupt(ch, pic, pci_ide);
    }

    /// Read from ATA I/O port (Bochs `bx_hard_drive_c::read`, harddrv.cc).
    ///
    /// ## Port Mapping (offsets from channel base address)
    ///
    /// | Offset | Port  | Register                                          |
    /// |--------|-------|---------------------------------------------------|
    /// | 0x00   | 0x1F0 | Data (16/32-bit reads from sector buffer)         |
    /// | 0x01   | 0x1F1 | Error Register (last error code)                  |
    /// | 0x02   | 0x1F2 | Sector Count (remaining sectors to transfer)      |
    /// | 0x03   | 0x1F3 | Sector Number / LBA[7:0]                          |
    /// | 0x04   | 0x1F4 | Cylinder Low / LBA[15:8]                          |
    /// | 0x05   | 0x1F5 | Cylinder High / LBA[23:16]                        |
    /// | 0x06   | 0x1F6 | Drive/Head: `1 LBA 1 DRV HD3 HD2 HD1 HD0`        |
    /// | 0x07   | 0x1F7 | Status (clears pending IRQ on read)               |
    /// | 0x16   | 0x3F6 | Alternate Status (does NOT clear pending IRQ)     |
    ///
    /// ## Data Port Read Protocol (offset 0x00)
    ///
    /// Returns data from the controller's internal buffer. DRQ must be set.
    /// After the last byte of a sector is read:
    /// - If `num_sectors > 0`: reads next sector into buffer, raises IRQ
    /// - If `num_sectors == 0`: clears DRQ, transfer complete
    ///
    /// For READ MULTIPLE (0xC4), multiple sectors are buffered at once.
    /// The buffer_size is set to `min(multiple_sectors, num_sectors) * sect_size`.
    pub fn read(&mut self, port: u16, io_len: u8, pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) -> u32 {
        let channel_num = match self.port_to_channel(port) {
            Some(c) => c,
            None => return 0xFF,
        };
        let channel = &mut self.channels[channel_num];
        let base = channel.ioaddr1;
        let drive_select = channel.drive_select;
        let offset = if port == 0x3F6 || port == 0x376 {
            ATA_ALT_STATUS
        } else {
            port - base
        };

        let drive = channel.selected_drive_mut();

        // Check if drive exists.
        // Bochs harddrv.cc: "Just return zero for these registers" (status/alt-status).
        // Other registers return 0xFF when selected drive absent.
        if drive.device_type == DeviceType::None {
            return if offset == ATA_STATUS || offset == ATA_ALT_STATUS {
                0x00 // Bochs: return zero when selected drive not present
            } else {
                0xFF
            };
        }

        match offset {
            ATA_DATA => {
                // Bochs harddrv.cc — data port read
                // Bochs harddrv.cc: DRQ check
                if !drive.controller.status.contains(AtaStatus::DRQ) {
                    tracing::debug!(
                        "ATA: IO read(0x{:04x}) with drq == 0: last cmd was {:02x}",
                        port,
                        drive.controller.current_command
                    );
                    return 0;
                }
                let current_command = drive.controller.current_command;
                let bytes = io_len as usize;

                // Bochs harddrv.cc — ATAPI lazy-load: when buffer_index >= buffer_size,
                // load the next CD block BEFORE reading data. Loads ONE block at a time,
                // matching Bochs exactly. This ensures remaining_blocks decrements in lockstep
                // with data delivery, so total_bytes_remaining always reaches 0 cleanly.
                if current_command == ATA_CMD_PACKET
                    && drive.controller.buffer_index >= drive.controller.buffer_size
                {
                    let atapi_cmd = drive.atapi.command;
                    match atapi_cmd {
                        0x28 | 0xa8 | 0xbe => {
                            // Bochs harddrv.cc: read ONE block
                            if !drive.cdrom.ready {
                                tracing::warn!("ATAPI: read with CD-ROM not ready");
                                return 0;
                            }
                            let next_lba = drive.cdrom.next_lba;
                            // Use temp buffer to avoid borrow conflict
                            // (read_cdrom_block needs &mut self for file I/O)
                            let mut temp = [0u8; CDROM_SECTOR_SIZE];
                            if !drive.read_cdrom_block(next_lba, &mut temp) {
                                tracing::warn!("ATAPI: read block {} failed", next_lba);
                                return 0;
                            }
                            drive.controller.buffer[..CDROM_SECTOR_SIZE]
                                .copy_from_slice(&temp);
                            drive.cdrom.next_lba += 1;
                            drive.cdrom.remaining_blocks -= 1;
                            if drive.cdrom.remaining_blocks <= 0 {
                                drive.cdrom.curr_lba = drive.cdrom.next_lba;
                            }
                            // Bochs harddrv.cc: index = 0
                            drive.controller.buffer_index = 0;
                        }
                        _ => {} // no need to load a new block
                    }
                }

                let idx = drive.controller.buffer_index;

                if idx + bytes > drive.controller.buffer_size {
                    // This can happen when the BIOS overshoots by one read after
                    // draining the IDENTIFY buffer — harmless, return 0.
                    return 0;
                }

                // Read bytes from buffer (little-endian)
                let mut value: u32 = 0;
                for b in 0..bytes {
                    value |= (drive.controller.buffer[idx + b] as u32) << (b * 8);
                }
                drive.controller.buffer_index += bytes;
                drive.controller.drq_index += bytes as u32;

                // Deferred interrupt flag — set when we need to raise_interrupt
                // after dropping the drive borrow.
                let mut need_raise_irq = false;
                let mut need_abort_cmd: Option<u8> = None;

                // Check if buffer completely read (CD block or ATA sector batch)
                if drive.controller.buffer_index >= drive.controller.buffer_size {
                    match current_command {
                        ATA_CMD_READ_SECTORS
                        | 0x21
                        | ATA_CMD_READ_SECTORS_EXT
                        | ATA_CMD_READ_MULTIPLE => {
                            // Bochs harddrv.cc
                            if current_command == ATA_CMD_READ_MULTIPLE {
                                let ms = drive.controller.multiple_sectors as u32;
                                if drive.controller.num_sectors > ms {
                                    drive.controller.buffer_size = ms as usize * SECTOR_SIZE;
                                } else {
                                    drive.controller.buffer_size =
                                        drive.controller.num_sectors as usize * SECTOR_SIZE;
                                }
                            }

                            drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
                            drive.controller.error = AtaError::empty();

                            if drive.controller.num_sectors == 0 {
                                // All sectors transferred — command complete
                            } else {
                                drive.controller.status.insert(AtaStatus::DRQ);

                                if drive.ide_read_sector() {
                                    drive.controller.buffer_index = 0;
                                    need_raise_irq = true;
                                } else {
                                    need_abort_cmd = Some(drive.controller.current_command);
                                }
                            }
                        }
                        ATA_CMD_IDENTIFY | ATA_CMD_IDENTIFY_PACKET => {
                            drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
                            drive.controller.error = AtaError::empty();
                        }
                        ATA_CMD_PACKET => {
                            // In Bochs, ATAPI block reload happens at the TOP of the
                            // read handler (harddrv.cc), not here. The buffer_index
                            // reset is the only thing needed — the next read will trigger
                            // the lazy-load above.
                            drive.controller.buffer_index = drive.controller.buffer_size;
                        }
                        _ => {
                            drive.controller.status.remove(AtaStatus::DRQ);
                        }
                    }
                }

                // Bochs harddrv.cc: ATAPI DRQ cycle completion check
                // drq_index tracks total bytes read across multiple CD blocks.
                // When drq_index >= drq_bytes, one DRQ cycle is complete.
                if current_command == ATA_CMD_PACKET
                    && drive.controller.drq_index >= drive.atapi.drq_bytes as u32
                {
                    drive.controller.status.remove(AtaStatus::DRQ);
                    drive.controller.drq_index = 0;

                    drive.atapi.total_bytes_remaining -= drive.atapi.drq_bytes;

                    // Bochs harddrv.cc
                    if drive.atapi.total_bytes_remaining > 0 {
                        drive.controller.sector_count =
                            (drive.controller.sector_count & 0xF8) | 0x02;
                        drive.controller.status.insert(AtaStatus::DRDY | AtaStatus::DRQ);
                        drive.controller.status.remove(AtaStatus::BSY | AtaStatus::ERR);
                        if drive.atapi.total_bytes_remaining
                            < drive.controller.cylinder_no as i32
                        {
                            drive.controller.cylinder_no =
                                drive.atapi.total_bytes_remaining as u16;
                        }
                        drive.atapi.drq_bytes = drive.controller.cylinder_no as i32;
                    } else {
                        drive.atapi.total_bytes_remaining = 0;
                        drive.controller.sector_count =
                            (drive.controller.sector_count & 0xF8) | 0x03;
                        drive.controller.status.remove(AtaStatus::BSY | AtaStatus::DRQ | AtaStatus::ERR);
                        drive.controller.status.insert(AtaStatus::DRDY);
                    }
                    need_raise_irq = true;
                }

                // drive borrow ends here (NLL); now we can call &mut self methods
                if let Some(cmd) = need_abort_cmd {
                    self.command_aborted(channel_num, cmd, pic, pci_ide);
                } else if need_raise_irq {
                    self.raise_interrupt(channel_num, pic, pci_ide);
                }

                value
            }
            ATA_ERROR => {
                // Bochs harddrv.cc: HOB read-back for LBA48
                if drive.controller.lba48 && (drive.controller.control & 0x80) != 0 {
                    drive.controller.hob.feature as u32
                } else {
                    drive.controller.error.bits() as u32
                }
            }
            ATA_SECTOR_COUNT => {
                if drive.controller.lba48 && (drive.controller.control & 0x80) != 0 {
                    drive.controller.hob.nsector as u32
                } else {
                    drive.controller.sector_count as u32
                }
            }
            ATA_SECTOR_NUM => {
                if drive.controller.lba48 && (drive.controller.control & 0x80) != 0 {
                    drive.controller.hob.sector as u32
                } else {
                    drive.controller.sector_no as u32
                }
            }
            ATA_CYL_LOW => {
                if drive.controller.lba48 && (drive.controller.control & 0x80) != 0 {
                    drive.controller.hob.lcyl as u32
                } else {
                    (drive.controller.cylinder_no & 0xFF) as u32
                }
            }
            ATA_CYL_HIGH => {
                if drive.controller.lba48 && (drive.controller.control & 0x80) != 0 {
                    drive.controller.hob.hcyl as u32
                } else {
                    (drive.controller.cylinder_no >> 8) as u32
                }
            }
            ATA_DRIVE_HEAD => {
                let lba_bit = if drive.controller.lba_mode { 0x40 } else { 0 };
                let drive_bit = if drive_select != 0 { 0x10 } else { 0 };
                (0xA0 | lba_bit | drive_bit | (drive.controller.head_no & 0x0F)) as u32
            }
            ATA_STATUS | ATA_ALT_STATUS => {
                // Bochs harddrv.cc — build status byte with index pulse
                let mut status = drive.controller.status;

                // Index pulse simulation (Bochs harddrv.cc)
                // INDEX_PULSE_CYCLE = 10: set IDX bit once every 10 status reads
                drive.controller.index_pulse_count += 1;
                if drive.controller.index_pulse_count >= 10 {
                    status |= AtaStatus::IDX;
                    drive.controller.index_pulse_count = 0;
                }

                // Reading primary status clears the ATA-level interrupt flag
                // AND lowers the IRQ line (but not for alternate status port).
                // Bochs harddrv.cc: DEV_pic_lower_irq() on port 0x07.
                if offset == ATA_STATUS {
                    drive.controller.interrupt_pending = false;
                    let irq = match channel_num {
                        0 => 14u8,
                        _ => 15u8,
                    };
                    // Bochs DEV_pic_lower_irq() — PIC forwards to IOAPIC synchronously.
                    pic.lower_irq(irq);
                }
                tracing::debug!(
                    "ATA: Status read {:#04x} (port={:#06x}) cmd={:#04x} drq={}",
                    status.bits(),
                    port,
                    drive.controller.current_command,
                    status.contains(AtaStatus::DRQ),
                );
                status.bits() as u32
            }
            _ => 0xFF,
        }
    }

    /// Bulk-read up to `buf.len()` bytes from the IDE data port.
    ///
    /// Equivalent to calling `read(port, 2)` in a loop but avoids per-word
    /// handler dispatch overhead. Returns the number of bytes actually copied.
    /// Handles ATAPI lazy-load, sector transitions, and DRQ completion.
    pub fn bulk_read_data(&mut self, port: u16, buf: &mut [u8], pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) -> usize {
        let channel_num = match self.port_to_channel(port) {
            Some(c) => c,
            None => return 0,
        };

        let selected = self.channels[channel_num].drive_select;
        let drive = &mut self.channels[channel_num].drives[selected as usize];

        // Bochs BX_SUPPORT_REPEAT_SPEEDUPS (harddrv.cc) bulk path is
        // ONLY for ATA READ SECTORS, NEVER for ATAPI. For ATAPI commands (0xA0),
        // return 0 to force the per-word read() handler path, which has the
        // correct single-block lazy-load and DRQ completion logic matching
        // Bochs harddrv.cc exactly.
        if drive.controller.current_command == ATA_CMD_PACKET {
            return 0;
        }

        if drive.device_type == DeviceType::None {
            return 0;
        }
        if !drive.controller.status.contains(AtaStatus::DRQ) {
            return 0;
        }

        let current_command = drive.controller.current_command;
        let mut total_copied = 0;
        let mut need_raise_irq = false;
        let mut need_abort_cmd: Option<u8> = None;

        while total_copied < buf.len() {
            // Bochs harddrv.cc — ATAPI lazy-load: load ONE block when
            // buffer is exhausted. Single-block loading keeps remaining_blocks
            // in lockstep with data delivery, preventing the hang where
            // remaining_blocks=0 but total_bytes_remaining > 0.
            if current_command == ATA_CMD_PACKET
                && drive.controller.buffer_index >= drive.controller.buffer_size
            {
                let atapi_cmd = drive.atapi.command;
                match atapi_cmd {
                    0x28 | 0xa8 | 0xbe => {
                        if drive.cdrom.remaining_blocks > 0 {
                            let next_lba = drive.cdrom.next_lba;
                            let mut temp = [0u8; CDROM_SECTOR_SIZE];
                            if !drive.read_cdrom_block(next_lba, &mut temp) {
                                break;
                            }
                            drive.controller.buffer[..CDROM_SECTOR_SIZE]
                                .copy_from_slice(&temp);
                            drive.controller.buffer_size = CDROM_SECTOR_SIZE;
                            drive.cdrom.next_lba += 1;
                            drive.cdrom.remaining_blocks -= 1;
                            if drive.cdrom.remaining_blocks <= 0 {
                                drive.cdrom.curr_lba = drive.cdrom.next_lba;
                            }
                            drive.controller.buffer_index = 0;
                        } else {
                            break;
                        }
                    }
                    _ => break,
                }
            }

            // Copy available bytes from buffer
            let available = drive.controller.buffer_size.saturating_sub(drive.controller.buffer_index);
            if available == 0 {
                break;
            }
            let wanted = buf.len() - total_copied;
            let to_copy = available.min(wanted);

            let idx = drive.controller.buffer_index;
            buf[total_copied..total_copied + to_copy]
                .copy_from_slice(&drive.controller.buffer[idx..idx + to_copy]);
            drive.controller.buffer_index += to_copy;
            drive.controller.drq_index += to_copy as u32;
            total_copied += to_copy;

            // Handle buffer drain
            if drive.controller.buffer_index >= drive.controller.buffer_size {
                match current_command {
                    ATA_CMD_READ_SECTORS | 0x21 | ATA_CMD_READ_SECTORS_EXT | ATA_CMD_READ_MULTIPLE => {
                        if current_command == ATA_CMD_READ_MULTIPLE {
                            let ms = drive.controller.multiple_sectors as u32;
                            if drive.controller.num_sectors > ms {
                                drive.controller.buffer_size = ms as usize * SECTOR_SIZE;
                            } else {
                                drive.controller.buffer_size =
                                    drive.controller.num_sectors as usize * SECTOR_SIZE;
                            }
                        }
                        drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
                        drive.controller.error = AtaError::empty();
                        if drive.controller.num_sectors == 0 {
                            // Transfer complete
                            break;
                        } else {
                            drive.controller.status.insert(AtaStatus::DRQ);
                            if drive.ide_read_sector() {
                                drive.controller.buffer_index = 0;
                                need_raise_irq = true;
                            } else {
                                need_abort_cmd = Some(current_command);
                                break;
                            }
                        }
                    }
                    ATA_CMD_IDENTIFY | ATA_CMD_IDENTIFY_PACKET => {
                        drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
                        drive.controller.error = AtaError::empty();
                        break;
                    }
                    ATA_CMD_PACKET => {
                        // Bochs: block reload happens at the lazy-load check at top
                        // of loop. Mark buffer as exhausted so next iteration triggers it.
                        drive.controller.buffer_index = drive.controller.buffer_size;
                    }
                    _ => {
                        drive.controller.status.remove(AtaStatus::DRQ);
                        break;
                    }
                }
            }

            // ATAPI DRQ cycle completion check
            if current_command == ATA_CMD_PACKET
                && drive.controller.drq_index >= drive.atapi.drq_bytes as u32
            {
                drive.controller.status.remove(AtaStatus::DRQ);
                drive.controller.drq_index = 0;
                drive.atapi.total_bytes_remaining -= drive.atapi.drq_bytes;
                // Bochs harddrv.cc
                if drive.atapi.total_bytes_remaining > 0 {
                    drive.controller.sector_count =
                        (drive.controller.sector_count & 0xF8) | 0x02;
                    drive.controller.status.insert(AtaStatus::DRDY | AtaStatus::DRQ);
                    drive.controller.status.remove(AtaStatus::BSY | AtaStatus::ERR);
                    if drive.atapi.total_bytes_remaining < drive.controller.cylinder_no as i32 {
                        drive.controller.cylinder_no =
                            drive.atapi.total_bytes_remaining as u16;
                    }
                    drive.atapi.drq_bytes = drive.controller.cylinder_no as i32;
                } else {
                    drive.controller.sector_count =
                        (drive.controller.sector_count & 0xF8) | 0x03;
                    drive.controller.status.remove(AtaStatus::BSY | AtaStatus::DRQ | AtaStatus::ERR);
                    drive.controller.status.insert(AtaStatus::DRDY);
                }
                need_raise_irq = true;
                // Stop after DRQ completion — caller should check status before continuing
                break;
            }
        }

        // Post-loop DRQ completion check: handles the case where the while loop
        // breaks early but drq_index has reached drq_bytes.
        // Matches Bochs harddrv.cc.
        if !need_raise_irq && need_abort_cmd.is_none()
            && current_command == ATA_CMD_PACKET
        {
            let drive = &mut self.channels[channel_num].drives[selected as usize];
            if drive.controller.drq_index >= drive.atapi.drq_bytes as u32
                && drive.atapi.drq_bytes > 0
            {
                drive.controller.status.remove(AtaStatus::DRQ);
                drive.controller.drq_index = 0;
                drive.atapi.total_bytes_remaining -= drive.atapi.drq_bytes;
                if drive.atapi.total_bytes_remaining > 0 {
                    // Bochs harddrv.cc
                    drive.controller.sector_count =
                        (drive.controller.sector_count & 0xF8) | 0x02;
                    drive.controller.status.insert(AtaStatus::DRDY | AtaStatus::DRQ);
                    drive.controller.status.remove(AtaStatus::BSY | AtaStatus::ERR);
                    if drive.atapi.total_bytes_remaining < drive.controller.cylinder_no as i32 {
                        drive.controller.cylinder_no =
                            drive.atapi.total_bytes_remaining as u16;
                    }
                    drive.atapi.drq_bytes = drive.controller.cylinder_no as i32;
                } else {
                    // Bochs harddrv.cc
                    drive.controller.sector_count =
                        (drive.controller.sector_count & 0xF8) | 0x03;
                    drive.controller.status.remove(AtaStatus::BSY | AtaStatus::DRQ | AtaStatus::ERR);
                    drive.controller.status.insert(AtaStatus::DRDY);
                }
                need_raise_irq = true;
            }
        }

        // Raise interrupt after drive borrow is released
        if let Some(cmd) = need_abort_cmd {
            self.command_aborted(channel_num, cmd, pic, pci_ide);
        } else if need_raise_irq {
            self.raise_interrupt(channel_num, pic, pci_ide);
        }

        total_copied
    }

    /// Write to ATA I/O port (Bochs `bx_hard_drive_c::write`, harddrv.cc+).
    ///
    /// ## Port Mapping (offsets from channel base address)
    ///
    /// | Offset | Port  | Register                                              |
    /// |--------|-------|-------------------------------------------------------|
    /// | 0x00   | 0x1F0 | Data (16/32-bit writes to sector buffer)              |
    /// | 0x01   | 0x1F1 | Features / Write Precompensation                      |
    /// | 0x02   | 0x1F2 | Sector Count                                          |
    /// | 0x03   | 0x1F3 | Sector Number / LBA[7:0]                              |
    /// | 0x04   | 0x1F4 | Cylinder Low / LBA[15:8]                              |
    /// | 0x05   | 0x1F5 | Cylinder High / LBA[23:16]                            |
    /// | 0x06   | 0x1F6 | Drive/Head (selects master/slave + LBA mode)          |
    /// | 0x07   | 0x1F7 | Command (clears pending IRQ, dispatches ATA command)  |
    /// | 0x16   | 0x3F6 | Device Control (nIEN, SRST)                           |
    ///
    /// ## Command Register Write (offset 0x07) — ATA Command Dispatch
    ///
    /// Writing to the command register clears any pending IRQ, then dispatches:
    /// - **0x10**: CALIBRATE DRIVE — moves head to cylinder 0
    /// - **0x20/0x21**: READ SECTORS (with/without retries) — PIO single-sector read
    /// - **0x30**: WRITE SECTORS — PIO single-sector write
    /// - **0x40/0x41**: READ VERIFY — verify sectors without data transfer
    /// - **0x70**: SEEK — move to specified CHS/LBA position
    /// - **0x90**: EXECUTE DEVICE DIAGNOSTIC — sets signature, returns error=0x01
    /// - **0x91**: INITIALIZE DRIVE PARAMETERS — sets logical CHS geometry
    /// - **0xC4**: READ MULTIPLE — PIO multi-sector read (multiple_sectors at a time)
    /// - **0xC5**: WRITE MULTIPLE — PIO multi-sector write
    /// - **0xC6**: SET MULTIPLE MODE — sets sectors-per-interrupt count
    /// - **0xEC**: IDENTIFY DEVICE — returns 512-byte device identification block
    /// - **0xEF**: SET FEATURES — sub-commands for transfer mode, cache control, etc.
    ///
    /// ## Data Port Write Protocol (offset 0x00)
    ///
    /// For WRITE SECTORS: host writes bytes into the controller buffer.
    /// When `buffer_index >= buffer_size`:
    /// - Writes sector(s) to disk image via `ide_write_sector()`
    /// - Decrements `num_sectors` via `increment_address()`
    /// - If `num_sectors > 0`: keeps DRQ=1, raises IRQ for next sector
    /// - If `num_sectors == 0`: clears DRQ, raises final completion IRQ
    pub fn write(&mut self, port: u16, value: u32, io_len: u8, pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) {
        let channel_num = match self.port_to_channel(port) {
            Some(c) => c,
            None => return,
        };

        let channel = &mut self.channels[channel_num];
        let base = channel.ioaddr1;
        let offset = if port == 0x3F6 || port == 0x376 {
            ATA_ALT_STATUS
        } else {
            port - base
        };

        // Bochs harddrv.cc: clear HOB (bit 7 of control) on command block writes
        // (ports 0x01-0x07, i.e., all except ATA_DATA and ATA_ALT_STATUS)
        if (1..=7).contains(&offset) {
            for drive in &mut channel.drives {
                drive.controller.control &= !0x80u8;
            }
        }

        match offset {
            ATA_DATA => {
                // Bochs harddrv.cc — data port write
                let drive = channel.selected_drive_mut();
                if drive.device_type == DeviceType::None {
                    return;
                }

                let bytes = io_len as usize;
                let idx = drive.controller.buffer_index;
                if idx + bytes > drive.controller.buffer.len() {
                    return;
                }

                // Write bytes to buffer (little-endian)
                for b in 0..bytes {
                    drive.controller.buffer[idx + b] = ((value >> (b * 8)) & 0xFF) as u8;
                }
                drive.controller.buffer_index += bytes;

                // Check if buffer completely written
                if drive.controller.buffer_index >= drive.controller.buffer_size {
                    let current_command = drive.controller.current_command;
                    match current_command {
                        ATA_CMD_WRITE_SECTORS
                        | 0x31 // WRITE SECTORS without retries
                        | ATA_CMD_WRITE_SECTORS_EXT
                        | ATA_CMD_WRITE_MULTIPLE => {
                            // Bochs harddrv.cc
                            // Write sector(s) to disk
                            if drive.ide_write_sector() {
                                // Recalculate buffer_size for WRITE MULTIPLE
                                if current_command == ATA_CMD_WRITE_MULTIPLE {
                                    let ms = drive.controller.multiple_sectors as u32;
                                    if drive.controller.num_sectors > ms {
                                        drive.controller.buffer_size = ms as usize * SECTOR_SIZE;
                                    } else {
                                        drive.controller.buffer_size =
                                            drive.controller.num_sectors as usize * SECTOR_SIZE;
                                    }
                                }

                                drive.controller.buffer_index = 0;

                                if drive.controller.num_sectors != 0 {
                                    // More sectors to write — keep DRQ, raise IRQ
                                    drive.controller.status =
                                        AtaStatus::DRDY | AtaStatus::DSC | AtaStatus::DRQ;
                                    drive.controller.error = AtaError::empty();
                                } else {
                                    // All sectors written — clear DRQ
                                    drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
                                    drive.controller.error = AtaError::empty();
                                }

                                // Bochs harddrv.cc: raise_interrupt(channel)
                                // Raises for both "more sectors" and "done"
                                self.raise_interrupt(channel_num, pic, pci_ide);
                            } else {
                                // Write error
                                tracing::error!("ATA: ide_write_sector failed");
                                drive.controller.error = AtaError::ABRT;
                                drive.controller.status = AtaStatus::ERR | AtaStatus::DRDY;
                                drive.controller.status.remove(AtaStatus::DRQ);
                            }
                        }
                        ATA_CMD_PACKET => {
                            // ATAPI: 12-byte CDB completely written — dispatch ATAPI command
                            // Bochs harddrv.cc
                            self.handle_atapi_command(channel_num, pic, pci_ide);
                        }
                        _ => {
                            // Unknown command writing data — shouldn't happen
                            tracing::warn!(
                                "ATA: data write for unknown command {:#04x}",
                                current_command
                            );
                        }
                    }
                }
            }
            ATA_ERROR => {
                // Features register (write) — Bochs WRITE_FEATURES macro
                let val = value as u8;
                for drive in &mut channel.drives {
                    drive.controller.hob.feature = drive.controller.features;
                    drive.controller.features = val;
                }
            }
            ATA_SECTOR_COUNT => {
                // Bochs WRITE_SECTOR_COUNT macro — saves HOB on both drives
                let val = value as u8;
                for drive in &mut channel.drives {
                    drive.controller.hob.nsector = drive.controller.sector_count;
                    drive.controller.sector_count = val;
                }
            }
            ATA_SECTOR_NUM => {
                // Bochs WRITE_SECTOR_NUMBER macro — saves HOB on both drives
                let val = value as u8;
                for drive in &mut channel.drives {
                    drive.controller.hob.sector = drive.controller.sector_no;
                    drive.controller.sector_no = val;
                }
            }
            ATA_CYL_LOW => {
                // Bochs WRITE_CYLINDER_LOW macro — saves HOB on both drives
                let val = value as u8;
                for drive in &mut channel.drives {
                    drive.controller.hob.lcyl = (drive.controller.cylinder_no & 0xFF) as u8;
                    drive.controller.cylinder_no =
                        (drive.controller.cylinder_no & 0xFF00) | (val as u16);
                }
            }
            ATA_CYL_HIGH => {
                // Bochs WRITE_CYLINDER_HIGH macro — saves HOB on both drives
                let val = value as u8;
                for drive in &mut channel.drives {
                    drive.controller.hob.hcyl = (drive.controller.cylinder_no >> 8) as u8;
                    drive.controller.cylinder_no =
                        (drive.controller.cylinder_no & 0x00FF) | ((val as u16) << 8);
                }
            }
            ATA_DRIVE_HEAD => {
                let value = value as u8;
                channel.drive_select = if (value & 0x10) != 0 { 1 } else { 0 };
                for drive in &mut channel.drives {
                    drive.controller.lba_mode = (value & 0x40) != 0;
                    drive.controller.head_no = value & 0x0F;
                }
            }
            ATA_STATUS => {
                // Command register (write) — clears pending IRQ first
                // Bochs harddrv.cc

                // Bochs harddrv.cc: ignore command if slave selected but not present
                if channel.drive_select == 1 && channel.drives[1].device_type == DeviceType::None {
                    tracing::debug!(
                        "ATA ch{}: command {:#04x} ignored, slave not present",
                        channel_num,
                        value
                    );
                    return;
                }

                // Bochs harddrv.cc: DEV_pic_lower_irq() before command dispatch
                // Explicitly lower the IRQ line before dispatching a new command.
                {
                    let drive = channel.selected_drive_mut();
                    drive.controller.interrupt_pending = false;
                }
                {
                    let irq = match channel_num {
                        0 => 14u8,
                        _ => 15u8,
                    };
                    // Bochs DEV_pic_lower_irq() — PIC forwards to IOAPIC synchronously.
                    pic.lower_irq(irq);
                }

                // Bochs harddrv.cc: check BSY before executing command
                let drive = channel.selected_drive();
                if drive.controller.status.contains(AtaStatus::BSY) {
                    tracing::debug!(
                        "ATA ch{}: command {:#04x} sent while BSY, ignoring",
                        channel_num,
                        value
                    );
                    return;
                }

                self.execute_command(channel_num, value as u8, pic, pci_ide);
            }
            ATA_ALT_STATUS => {
                // Device control register (Bochs harddrv.cc)
                // Writes go to BOTH drives on the channel
                let value = value as u8;
                let prev_nien = (channel.drives[0].controller.control & 0x02) != 0;
                let new_nien = (value & 0x02) != 0;
                if new_nien != prev_nien {
                    tracing::debug!(
                        "ATA: nIEN ch{} {} → {} (ctrl={:#04x})",
                        channel_num,
                        if prev_nien { "1" } else { "0" },
                        if new_nien { "1" } else { "0" },
                        value
                    );
                }
                let prev_reset = channel.drives[0].controller.control & 0x04;

                // Bochs harddrv.cc: Store control FIRST, then override
                // during reset transitions (Bochs uses struct fields; we use raw byte).
                for d in 0..2 {
                    channel.drives[d].controller.control = value;
                }

                // nIEN transition 1→0: raise deferred interrupt ONLY if the drive
                // still has an active transfer (DRQ set = data waiting to be read).
                // Without the DRQ check, stale interrupt_pending from PREVIOUS commands
                // (acknowledged via alternate status polling, which doesn't clear
                // interrupt_pending) causes spurious interrupts that confuse ata_piix's
                // HSM and trigger "lost interrupt" errors.
                if prev_nien && !new_nien {
                    let selected = channel.drive_select as usize;
                    let drv = &channel.drives[selected];
                    if drv.controller.interrupt_pending
                        && drv.controller.status.contains(AtaStatus::DRQ)
                    {
                        let irq = if channel_num == 0 { 14u8 } else { 15u8 };
                        pci_ide.bmdma_set_irq(channel_num as u8);
                        pic.raise_irq(irq);
                    }
                }

                // Software reset — affects both drives
                if (value & 0x04) != 0 && prev_reset == 0 {
                    // Transition 0→1: Assert SRST (Bochs harddrv.cc)
                    tracing::debug!("ATA: Software reset asserted ch={}", channel_num);
                    for d in 0..2 {
                        // Bochs: BSY=1, DRDY=0, WF=0, DSC=1, DRQ=0, CORR=0, ERR=0
                        channel.drives[d].controller.status = AtaStatus::BSY | AtaStatus::DSC;
                        channel.drives[d].controller.reset_in_progress = true;
                        channel.drives[d].controller.error = AtaError::from_bits_retain(0x01); // diagnostic: no error
                        channel.drives[d].controller.current_command = 0;
                        channel.drives[d].controller.buffer_index = 0;
                        channel.drives[d].controller.multiple_sectors = 0;
                        channel.drives[d].controller.lba_mode = false;
                        // Bochs harddrv.cc: disable_irq = 0 (clear nIEN)
                        channel.drives[d].controller.control &= !0x02u8;
                        channel.drives[d].controller.interrupt_pending = false;
                    }
                    // Bochs harddrv.cc: DEV_pic_lower_irq()
                    let irq = if channel_num == 0 { 14u8 } else { 15u8 };
                    // Bochs DEV_pic_lower_irq() — PIC forwards to IOAPIC synchronously.
                    pic.lower_irq(irq);
                } else if (value & 0x04) == 0 && channel.drives[0].controller.reset_in_progress {
                    // Transition 1→0: Deassert SRST (Bochs harddrv.cc)
                    tracing::debug!("ATA: Software reset deasserted ch={}", channel_num);
                    for d in 0..2 {
                        channel.drives[d].controller.reset_in_progress = false;
                        channel.drives[d].controller.status = AtaStatus::DRDY | AtaStatus::DSC;
                        // Bochs set_signature(): head_no=0, sector_count=1, sector_no=1
                        channel.drives[d].controller.head_no = 0;
                        channel.drives[d].controller.sector_count = 1;
                        channel.drives[d].controller.sector_no = 1;
                        // Bochs set_signature(): HD → cylinder_no=0, CDROM → 0xEB14, absent → 0xFFFF
                        match channel.drives[d].device_type {
                            DeviceType::Disk => {
                                channel.drives[d].controller.cylinder_no = 0;
                            }
                            DeviceType::Cdrom => {
                                channel.drives[d].controller.cylinder_no = 0xEB14;
                            }
                            DeviceType::None => {
                                channel.drives[d].controller.cylinder_no = 0xFFFF;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Abort the current command (Bochs harddrv.cc).
    ///
    /// Sets error register to ABRT, clears BSY/DRQ/CORR, sets DRDY/ERR.
    /// Preserves DSC (seek_complete) — matches Bochs harddrv.cc.
    fn command_aborted(&mut self, channel_num: usize, _value: u8, pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) {
        {
            let drive = self.channels[channel_num].selected_drive_mut();
            drive.controller.current_command = 0;
            // Bochs: clears busy, drq, corrected_data, write_fault; sets drive_ready, err
            // Does NOT touch seek_complete (DSC) — preserve it
            let dsc = drive.controller.status & AtaStatus::DSC;
            drive.controller.status = AtaStatus::DRDY | AtaStatus::ERR | dsc;
            drive.controller.error = AtaError::ABRT;
            drive.controller.buffer_index = 0;
        }
        // Bochs harddrv.cc: raise_interrupt(channel)
        self.raise_interrupt(channel_num, pic, pci_ide);
    }

    /// Initialize an ATAPI command response (Bochs init_send_atapi_command, harddrv.cc)
    fn init_send_atapi_command(
        &mut self,
        channel_num: usize,
        command: u8,
        req_length: i32,
        alloc_length: i32,
        lazy: bool,
    ) {
        let drive = self.channels[channel_num].selected_drive_mut();
        // Bochs harddrv.cc
        let mut byte_count = drive.controller.cylinder_no as i32;
        if byte_count == 0xffff_i32 {
            byte_count = 0xfffe;
        }
        if (byte_count & 1) != 0 && (alloc_length > byte_count) {
            byte_count -= 1;
        }

        let alloc_length = if alloc_length == 0 {
            byte_count
        } else {
            alloc_length
        };

        // Bochs sets individual fields: busy=1, drive_ready=1, drq=0, err=0
        // preserving other bits like seek_complete (DSC).
        drive.controller.status.remove(AtaStatus::DRQ | AtaStatus::ERR);
        drive.controller.status.insert(AtaStatus::BSY | AtaStatus::DRDY);

        if lazy {
            drive.controller.buffer_index = drive.controller.buffer_size;
        } else {
            drive.controller.buffer_index = 0;
        }
        drive.controller.drq_index = 0;

        if byte_count > req_length {
            byte_count = req_length;
        }
        if byte_count > alloc_length {
            byte_count = alloc_length;
        }

        drive.controller.cylinder_no = byte_count as u16;
        drive.atapi.command = command;
        drive.atapi.drq_bytes = byte_count;
        drive.atapi.total_bytes_remaining = if req_length < alloc_length {
            req_length
        } else {
            alloc_length
        };
    }

    /// Set ATAPI command error (Bochs atapi_cmd_error, harddrv.cc)
    ///
    /// Bochs sets individual fields: busy=0, drive_ready=1, write_fault=0, drq=0, err=1
    /// preserving other bits like seek_complete (DSC).
    fn atapi_cmd_error(&mut self, channel_num: usize, sense_key: SenseKey, asc: Asc) {
        let drive = self.channels[channel_num].selected_drive_mut();
        drive.controller.error = AtaError::from_bits_retain((sense_key as u8) << 4);
        drive.controller.sector_count = (drive.controller.sector_count & 0xF8) | 0x03; // i_o=1, c_d=1
        drive.controller.status.remove(AtaStatus::BSY | AtaStatus::DWF | AtaStatus::DRQ);
        drive.controller.status.insert(AtaStatus::DRDY | AtaStatus::ERR);

        drive.sense.sense_key = sense_key as u8;
        drive.sense.asc = asc as u8;
        drive.sense.ascq = 0;
    }

    /// Set ATAPI command completed (no data) (Bochs atapi_cmd_nop, harddrv.cc)
    ///
    /// Bochs sets individual fields: busy=0, drive_ready=1, drq=0, err=0
    /// preserving other bits like seek_complete (DSC).
    fn atapi_cmd_nop(&mut self, channel_num: usize) {
        let drive = self.channels[channel_num].selected_drive_mut();
        drive.controller.sector_count = (drive.controller.sector_count & 0xF8) | 0x03; // i_o=1, c_d=1
        drive.controller.status.remove(AtaStatus::BSY | AtaStatus::DRQ | AtaStatus::ERR);
        drive.controller.status.insert(AtaStatus::DRDY);
    }

    /// Signal data ready to send (Bochs ready_to_send_atapi, harddrv.cc)
    ///
    /// Bochs sets individual fields: busy=0, drq=1, err=0
    /// preserving other bits like drive_ready (DRDY) and seek_complete (DSC).
    ///
    /// If packet_dma is set (features bit 0 was 1 on PACKET command),
    /// signals DMA engine instead of raising interrupt for PIO.
    /// Bochs harddrv.cc
    pub(crate) fn ready_to_send_atapi(&mut self, channel_num: usize, pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) {
        let drive = self.channels[channel_num].selected_drive_mut();
        drive.controller.sector_count = (drive.controller.sector_count & 0xF8) | 0x02; // i_o=1, c_d=0
        drive.controller.status.remove(AtaStatus::BSY | AtaStatus::ERR);
        drive.controller.status.insert(AtaStatus::DRQ);

        // Bochs harddrv.cc: DMA vs PIO branch
        if drive.controller.packet_dma {
            tracing::debug!("ATAPI: ready_to_send_atapi DMA path ch={}", channel_num);
            pci_ide.bmdma_start_transfer(channel_num as u8);
        } else {
            self.raise_interrupt(channel_num, pic, pci_ide);
        }
    }

    /// Handle an ATAPI command (Bochs harddrv.cc)
    fn handle_atapi_command(&mut self, channel_num: usize, pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) {
        let drive = self.channels[channel_num].selected_drive_mut();
        let atapi_command = drive.controller.buffer[0];
        drive.controller.buffer_size = CDROM_SECTOR_SIZE;


        tracing::debug!("ATAPI: cmd={:#04x} ch={} sc={}", atapi_command, channel_num, drive.status_changed);

        // Clear sense unless REQUEST SENSE
        if atapi_command != 0x03 {
            drive.sense.sense_key = SenseKey::None as u8;
            drive.sense.asc = 0;
            drive.sense.ascq = 0;
        }

        match atapi_command {
            0x00 => {
                // TEST UNIT READY — Bochs harddrv.cc
                // Three-state media change simulation
                let sc = drive.status_changed;
                let ready = drive.cdrom.ready;
                if sc == 1 {
                    // Simulate tray open
                    self.atapi_cmd_error(channel_num, SenseKey::NotReady, Asc::MediumNotPresent);
                    self.channels[channel_num].selected_drive_mut().status_changed = -1;
                } else if sc == -1 {
                    // Simulate tray close — report UNIT_ATTENTION
                    self.atapi_cmd_error(channel_num, SenseKey::UnitAttention, Asc::MediumMayHaveChanged);
                    // Set ascq=1 (Bochs harddrv.cc)
                    let d = self.channels[channel_num].selected_drive_mut();
                    d.sense.ascq = 1;
                    d.status_changed = 0;
                } else if ready {
                    self.atapi_cmd_nop(channel_num);
                } else {
                    self.atapi_cmd_error(channel_num, SenseKey::NotReady, Asc::MediumNotPresent);
                }
                self.raise_interrupt(channel_num, pic, pci_ide);
            }
            0x03 => {
                // REQUEST SENSE
                let drive = self.channels[channel_num].selected_drive_mut();
                let alloc_length = drive.controller.buffer[4] as i32;
                self.init_send_atapi_command(channel_num, atapi_command, 18, alloc_length, false);
                let drive = self.channels[channel_num].selected_drive_mut();
                // Fill sense data
                drive.controller.buffer[0] = 0x70 | (1 << 7);
                drive.controller.buffer[1] = 0;
                drive.controller.buffer[2] = drive.sense.sense_key;
                for i in 3..8 {
                    drive.controller.buffer[i] = 0;
                }
                drive.controller.buffer[7] = 17 - 7;
                for i in 8..12 {
                    drive.controller.buffer[i] = 0;
                }
                drive.controller.buffer[12] = drive.sense.asc;
                drive.controller.buffer[13] = drive.sense.ascq;
                drive.controller.buffer[14] = drive.sense.fruc;
                for i in 15..18 {
                    drive.controller.buffer[i] = 0;
                }

                if drive.sense.sense_key == SenseKey::UnitAttention as u8 {
                    drive.sense.sense_key = SenseKey::None as u8;
                }
                self.ready_to_send_atapi(channel_num, pic, pci_ide);
            }
            0x12 => {
                // INQUIRY
                let drive = self.channels[channel_num].selected_drive_mut();
                let alloc_length = drive.controller.buffer[4] as i32;
                self.init_send_atapi_command(channel_num, atapi_command, 36, alloc_length, false);
                let drive = self.channels[channel_num].selected_drive_mut();

                for i in 0..36 {
                    drive.controller.buffer[i] = 0;
                }
                drive.controller.buffer[0] = 0x05; // CD-ROM
                drive.controller.buffer[1] = 0x80; // Removable
                drive.controller.buffer[2] = 0x00; // Version
                drive.controller.buffer[3] = 0x21; // ATAPI-2
                drive.controller.buffer[4] = 31; // additional length

                // Vendor ID "RUSTYBOX"
                let vendor = b"RUSTYBOX";
                for (i, &b) in vendor.iter().enumerate() {
                    drive.controller.buffer[8 + i] = b;
                }
                // Product ID "Generic CD-ROM  "
                let product = b"Generic CD-ROM  ";
                for (i, &b) in product.iter().enumerate() {
                    drive.controller.buffer[16 + i] = b;
                }
                // Revision "1.0 "
                let rev = b"1.0 ";
                for (i, &b) in rev.iter().enumerate() {
                    drive.controller.buffer[32 + i] = b;
                }

                self.ready_to_send_atapi(channel_num, pic, pci_ide);
            }
            0x1b => {
                // START STOP UNIT — just succeed
                self.atapi_cmd_nop(channel_num);
                self.raise_interrupt(channel_num, pic, pci_ide);
            }
            0x1e => {
                // PREVENT/ALLOW MEDIUM REMOVAL
                let drive = self.channels[channel_num].selected_drive_mut();
                let prevent = (drive.controller.buffer[4] & 1) != 0;
                if drive.cdrom.ready {
                    drive.cdrom.locked = prevent;
                    self.atapi_cmd_nop(channel_num);
                } else {
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::NotReady,
                        Asc::MediumNotPresent,
                    );
                }
                self.raise_interrupt(channel_num, pic, pci_ide);
            }
            0x25 => {
                // READ CAPACITY
                let drive = self.channels[channel_num].selected_drive_mut();
                let ready = drive.cdrom.ready;
                let cap = drive.cdrom.max_lba;
                self.init_send_atapi_command(channel_num, atapi_command, 8, 8, false);
                let drive = self.channels[channel_num].selected_drive_mut();
                if ready {
                    drive.controller.buffer[0] = ((cap >> 24) & 0xff) as u8;
                    drive.controller.buffer[1] = ((cap >> 16) & 0xff) as u8;
                    drive.controller.buffer[2] = ((cap >> 8) & 0xff) as u8;
                    drive.controller.buffer[3] = (cap & 0xff) as u8;
                    drive.controller.buffer[4] = 0; // block size = 2048
                    drive.controller.buffer[5] = 0;
                    drive.controller.buffer[6] = (CDROM_SECTOR_SIZE >> 8) as u8;
                    drive.controller.buffer[7] = (CDROM_SECTOR_SIZE & 0xff) as u8;
                    self.ready_to_send_atapi(channel_num, pic, pci_ide);
                } else {
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::NotReady,
                        Asc::MediumNotPresent,
                    );
                    self.raise_interrupt(channel_num, pic, pci_ide);
                }
            }
            0x28 | 0xa8 => {
                // READ (10) / READ (12)
                let drive = self.channels[channel_num].selected_drive_mut();
                let mut transfer_length: i32;
                if atapi_command == 0x28 {
                    transfer_length = ((drive.controller.buffer[7] as i32) << 8)
                        | drive.controller.buffer[8] as i32;
                } else {
                    transfer_length = ((drive.controller.buffer[6] as i32) << 24)
                        | ((drive.controller.buffer[7] as i32) << 16)
                        | ((drive.controller.buffer[8] as i32) << 8)
                        | drive.controller.buffer[9] as i32;
                }
                let lba = ((drive.controller.buffer[2] as u32) << 24)
                    | ((drive.controller.buffer[3] as u32) << 16)
                    | ((drive.controller.buffer[4] as u32) << 8)
                    | drive.controller.buffer[5] as u32;
                let ready = drive.cdrom.ready;
                let max_lba = drive.cdrom.max_lba;

                if !ready {
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::NotReady,
                        Asc::MediumNotPresent,
                    );
                    self.raise_interrupt(channel_num, pic, pci_ide);
                    return;
                }
                if lba > max_lba {
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::IllegalRequest,
                        Asc::LogicalBlockOor,
                    );
                    self.raise_interrupt(channel_num, pic, pci_ide);
                    return;
                }
                // Bochs harddrv.cc — clip transfer when read extends past end of disc
                if transfer_length > 0 && (lba + transfer_length as u32 - 1) > max_lba {
                    transfer_length = (max_lba - lba + 1) as i32;
                }
                if transfer_length <= 0 {
                    self.atapi_cmd_nop(channel_num);
                    self.raise_interrupt(channel_num, pic, pci_ide);
                    return;
                }

                tracing::debug!(
                    "ATAPI: READ({}) LBA={} len={} sectors",
                    if atapi_command == 0x28 { 10 } else { 12 },
                    lba,
                    transfer_length
                );

                let total_bytes = transfer_length * CDROM_SECTOR_SIZE as i32;
                self.init_send_atapi_command(
                    channel_num,
                    atapi_command,
                    total_bytes,
                    total_bytes,
                    true,
                );
                let drive = self.channels[channel_num].selected_drive_mut();
                drive.cdrom.remaining_blocks = transfer_length;
                drive.cdrom.next_lba = lba;
                // Bochs: start_seek(channel) activates a timer; the timer
                // callback calls ready_to_send_atapi(). We defer it via a flag
                // processed by the emulator's tick loop.
                self.seek_complete_pending[channel_num] = true;
            }
            0x43 => {
                // READ TOC
                let drive = self.channels[channel_num].selected_drive_mut();
                let ready = drive.cdrom.ready;
                let msf = (drive.controller.buffer[1] >> 1) & 1;
                let format = drive.controller.buffer[9] >> 6;
                let alloc_length = ((drive.controller.buffer[7] as i32) << 8)
                    | drive.controller.buffer[8] as i32;
                let max_lba = drive.cdrom.max_lba;

                if !ready {
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::NotReady,
                        Asc::MediumNotPresent,
                    );
                    self.raise_interrupt(channel_num, pic, pci_ide);
                    return;
                }

                match format {
                    0 => {
                        // Standard TOC
                        let toc_length = 20; // 4 header + 8 track1 + 8 leadout
                        self.init_send_atapi_command(
                            channel_num,
                            atapi_command,
                            toc_length,
                            alloc_length,
                            false,
                        );
                        let drive = self.channels[channel_num].selected_drive_mut();
                        // Header
                        drive.controller.buffer[0] = 0; // TOC length MSB
                        drive.controller.buffer[1] = 18; // TOC length LSB
                        drive.controller.buffer[2] = 1; // first track
                        drive.controller.buffer[3] = 1; // last track
                        // Track 1 descriptor (Bochs cdrom.cc)
                        drive.controller.buffer[4] = 0; // reserved
                        drive.controller.buffer[5] = 0x15; // ADR=1, CONTROL=5 (data, incremental)
                        drive.controller.buffer[6] = 1; // track number
                        drive.controller.buffer[7] = 0; // reserved
                        if msf != 0 {
                            drive.controller.buffer[8] = 0; // reserved
                            drive.controller.buffer[9] = 0; // M
                            drive.controller.buffer[10] = 2; // S
                            drive.controller.buffer[11] = 0; // F
                        } else {
                            drive.controller.buffer[8] = 0;
                            drive.controller.buffer[9] = 0;
                            drive.controller.buffer[10] = 0;
                            drive.controller.buffer[11] = 0;
                        }
                        // Lead-out descriptor (Bochs cdrom.cc)
                        drive.controller.buffer[12] = 0;
                        drive.controller.buffer[13] = 0x16; // ADR=1, CONTROL=6
                        drive.controller.buffer[14] = 0xAA; // lead-out track
                        drive.controller.buffer[15] = 0;
                        // Lead-out position = capacity (max_lba + 1), matching Bochs
                        let blocks = max_lba + 1;
                        if msf != 0 {
                            // Bochs cdrom.cc: add 150-frame lead-in offset
                            let adj = blocks + 150;
                            let m = ((adj / 75) / 60) as u8;
                            let s = ((adj / 75) % 60) as u8;
                            let f = (adj % 75) as u8;
                            drive.controller.buffer[16] = 0;
                            drive.controller.buffer[17] = m;
                            drive.controller.buffer[18] = s;
                            drive.controller.buffer[19] = f;
                        } else {
                            drive.controller.buffer[16] = ((blocks >> 24) & 0xff) as u8;
                            drive.controller.buffer[17] = ((blocks >> 16) & 0xff) as u8;
                            drive.controller.buffer[18] = ((blocks >> 8) & 0xff) as u8;
                            drive.controller.buffer[19] = (blocks & 0xff) as u8;
                        }
                        self.ready_to_send_atapi(channel_num, pic, pci_ide);
                    }
                    1 => {
                        // Multi-session info — single session
                        self.init_send_atapi_command(
                            channel_num,
                            atapi_command,
                            12,
                            alloc_length,
                            false,
                        );
                        let drive = self.channels[channel_num].selected_drive_mut();
                        drive.controller.buffer[0] = 0;
                        drive.controller.buffer[1] = 0x0A;
                        drive.controller.buffer[2] = 1;
                        drive.controller.buffer[3] = 1;
                        for i in 4..12 {
                            drive.controller.buffer[i] = 0;
                        }
                        self.ready_to_send_atapi(channel_num, pic, pci_ide);
                    }
                    _ => {
                        self.atapi_cmd_error(
                            channel_num,
                            SenseKey::IllegalRequest,
                            Asc::InvFieldInCmdPacket,
                        );
                        self.raise_interrupt(channel_num, pic, pci_ide);
                    }
                }
            }
            0x46 => {
                // GET CONFIGURATION (MMC-4, mmc4r05a.pdf page 286)
                // Bochs harddrv.cc
                let drive = self.channels[channel_num].selected_drive_mut();
                let start_feature =
                    ((drive.controller.buffer[2] as u16) << 8) | drive.controller.buffer[3] as u16;
                let alloc_length =
                    ((drive.controller.buffer[7] as u16) << 8) | drive.controller.buffer[8] as u16;
                let inserted = drive.cdrom.ready;

                // The controller buffer is guaranteed to be at least 2048 bytes.
                // Build the full response, then truncate to alloc_length.
                if alloc_length >= 8 {
                    // Feature header (page 287)
                    // bytes [0..3] = data length (filled in at end)
                    drive.controller.buffer[4] = 0; // reserved
                    drive.controller.buffer[5] = 0; // reserved
                    drive.controller.buffer[6] = 0; // current profile: 0x0008 (CD-ROM)
                    drive.controller.buffer[7] = 0x08;
                    let mut ptr = 8usize;

                    // Profile 8 requires features: 0x0, 0x1, 0x2, 0x3, 0x10, 0x1E, 0x100, 0x105

                    // Profile List (feature 0x0000) (mmc4r05a.pdf page 174)
                    if start_feature == 0x0000 {
                        let inserted_bit = if inserted { 1u8 } else { 0u8 };
                        drive.controller.buffer[ptr] = 0x00; // Feature Code 0x000
                        drive.controller.buffer[ptr + 1] = 0x00;
                        drive.controller.buffer[ptr + 2] = (1 << 1) | inserted_bit; // persistent=1, current=inserted
                        drive.controller.buffer[ptr + 3] = 4; // additional length = 1*4
                        drive.controller.buffer[ptr + 4] = 0x00; // profile 0x0008
                        drive.controller.buffer[ptr + 5] = 0x08;
                        drive.controller.buffer[ptr + 6] = inserted_bit; // current=inserted
                        drive.controller.buffer[ptr + 7] = 0;
                        ptr += 8;
                    }

                    // Core Feature (feature 0x0001) (mmc4r05a.pdf page 174)
                    if start_feature <= 0x0001 {
                        drive.controller.buffer[ptr] = 0x00; // Feature Code 0x001
                        drive.controller.buffer[ptr + 1] = 0x01;
                        drive.controller.buffer[ptr + 2] = (1 << 2) | (1 << 1) | 1; // version=1, persistent=1, current=1
                        drive.controller.buffer[ptr + 3] = 8; // additional length = 8
                        drive.controller.buffer[ptr + 4] = 0; // physical interface: ATAPI = 2
                        drive.controller.buffer[ptr + 5] = 0;
                        drive.controller.buffer[ptr + 6] = 0;
                        drive.controller.buffer[ptr + 7] = 2;
                        drive.controller.buffer[ptr + 8] = 0; // DBE=0
                        drive.controller.buffer[ptr + 9] = 0;
                        drive.controller.buffer[ptr + 10] = 0;
                        drive.controller.buffer[ptr + 11] = 0;
                        ptr += 12;
                    }

                    // Morphing Feature (feature 0x0002) (mmc4r05a.pdf page 178)
                    if start_feature <= 0x0002 {
                        drive.controller.buffer[ptr] = 0x00; // Feature Code 0x002
                        drive.controller.buffer[ptr + 1] = 0x02;
                        drive.controller.buffer[ptr + 2] = (1 << 2) | (1 << 1) | 1; // version=1, persistent=1, current=1
                        drive.controller.buffer[ptr + 3] = 4; // additional length = 4
                        drive.controller.buffer[ptr + 4] = 0; // OCEvent=0, ASYNC=0
                        drive.controller.buffer[ptr + 5] = 0;
                        drive.controller.buffer[ptr + 6] = 0;
                        drive.controller.buffer[ptr + 7] = 0;
                        ptr += 8;
                    }

                    // Removable Medium Feature (feature 0x0003) (mmc4r05a.pdf page 179)
                    if start_feature <= 0x0003 {
                        drive.controller.buffer[ptr] = 0x00; // Feature Code 0x003
                        drive.controller.buffer[ptr + 1] = 0x03;
                        drive.controller.buffer[ptr + 2] = (1 << 1) | 1; // version=0, persistent=1, current=1
                        drive.controller.buffer[ptr + 3] = 4; // additional length = 4
                        drive.controller.buffer[ptr + 4] = 1 << 2; // Loading Mech: 0, No Eject: 0, No Pvnt Jumper: 1, Lock: 0
                        drive.controller.buffer[ptr + 5] = 0;
                        drive.controller.buffer[ptr + 6] = 0;
                        drive.controller.buffer[ptr + 7] = 0;
                        ptr += 8;
                    }

                    // Random Readable Feature (feature 0x0010) (mmc4r05a.pdf page 182)
                    if start_feature <= 0x0010 {
                        const MAX_MULTIPLE_SECTORS: u16 = 16;
                        drive.controller.buffer[ptr] = 0x00; // Feature Code 0x010
                        drive.controller.buffer[ptr + 1] = 0x10;
                        drive.controller.buffer[ptr + 2] = (1 << 1) | 1; // version=0, persistent=1, current=1
                        drive.controller.buffer[ptr + 3] = 8; // additional length = 8
                        drive.controller.buffer[ptr + 4] = 0x00; // Logical Block Size: 2048 (0x800)
                        drive.controller.buffer[ptr + 5] = 0x00;
                        drive.controller.buffer[ptr + 6] = 0x08;
                        drive.controller.buffer[ptr + 7] = 0x00;
                        drive.controller.buffer[ptr + 8] = (MAX_MULTIPLE_SECTORS >> 8) as u8; // blocking
                        drive.controller.buffer[ptr + 9] = (MAX_MULTIPLE_SECTORS & 0xFF) as u8;
                        drive.controller.buffer[ptr + 10] = 0; // PP = 0
                        drive.controller.buffer[ptr + 11] = 0;
                        ptr += 12;
                    }

                    // CD Read Feature (feature 0x001E) (mmc4r05a.pdf page 185)
                    if start_feature <= 0x001E {
                        drive.controller.buffer[ptr] = 0x00; // Feature Code 0x01E
                        drive.controller.buffer[ptr + 1] = 0x1E;
                        drive.controller.buffer[ptr + 2] = (2 << 2) | (1 << 1) | 1; // version=2, persistent=1, current=1
                        drive.controller.buffer[ptr + 3] = 4; // additional length = 4
                        drive.controller.buffer[ptr + 4] = 0; // DAP=0, C2 Flags=0, CD-Text=0
                        drive.controller.buffer[ptr + 5] = 0;
                        drive.controller.buffer[ptr + 6] = 0;
                        drive.controller.buffer[ptr + 7] = 0;
                        ptr += 8;
                    }

                    // Power Management Feature (feature 0x0100) (mmc4r05a.pdf page 216)
                    if start_feature <= 0x0100 {
                        drive.controller.buffer[ptr] = 0x01; // Feature Code 0x100
                        drive.controller.buffer[ptr + 1] = 0x00;
                        drive.controller.buffer[ptr + 2] = (1 << 1) | 1; // version=0, persistent=1, current=1
                        drive.controller.buffer[ptr + 3] = 0; // additional length = 0
                        ptr += 4;
                    }

                    // Timeout Feature (feature 0x0105) (mmc4r05a.pdf page 222)
                    if start_feature <= 0x0105 {
                        drive.controller.buffer[ptr] = 0x01; // Feature Code 0x105
                        drive.controller.buffer[ptr + 1] = 0x05;
                        drive.controller.buffer[ptr + 2] = (1 << 2) | (1 << 1) | 1; // version=1, persistent=1, current=1
                        drive.controller.buffer[ptr + 3] = 4; // additional length = 4
                        drive.controller.buffer[ptr + 4] = 0; // Group 3 = 0
                        drive.controller.buffer[ptr + 5] = 0;
                        drive.controller.buffer[ptr + 6] = 0;
                        drive.controller.buffer[ptr + 7] = 0;
                        ptr += 8;
                    }

                    // Update the return length
                    // Data Length field = total data following this field (excludes first 4 bytes)
                    let return_length = (ptr - 4) as u16;
                    drive.controller.buffer[0] = 0;
                    drive.controller.buffer[1] = 0;
                    drive.controller.buffer[2] = (return_length >> 8) as u8;
                    drive.controller.buffer[3] = (return_length & 0xFF) as u8;

                    // Bochs comment: "I think the last parameter needs to be 'alloc_length',
                    // but ReactOS won't boot unless it is this:"
                    let total = (return_length + 4) as i32;
                    self.init_send_atapi_command(
                        channel_num,
                        atapi_command,
                        total,
                        total,
                        false,
                    );
                    self.ready_to_send_atapi(channel_num, pic, pci_ide);
                } else {
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::IllegalRequest,
                        Asc::InvFieldInCmdPacket,
                    );
                    self.raise_interrupt(channel_num, pic, pci_ide);
                }
            }
            0x4a => {
                // GET EVENT STATUS NOTIFICATION (Bochs harddrv.cc)
                let drive = self.channels[channel_num].selected_drive_mut();
                let polled = (drive.controller.buffer[1] & 1) != 0;
                let request = drive.controller.buffer[4];
                let alloc_length =
                    ((drive.controller.buffer[7] as i32) << 8) | drive.controller.buffer[8] as i32;
                let inserted = drive.cdrom.ready;

                if polled {
                    let event_length;
                    // We only support the MEDIA event (bit 4)
                    if request == (1 << 4) {
                        let drive = self.channels[channel_num].selected_drive_mut();
                        drive.controller.buffer[0] = 0;
                        drive.controller.buffer[1] = 4; // MEDIA event is 4 bytes long
                        drive.controller.buffer[2] = 4; // 4 = MEDIA event
                        drive.controller.buffer[3] = 1 << 4; // we only support MEDIA event (bit 4)
                        // Event code based on status_changed (Bochs harddrv.cc)
                        drive.controller.buffer[4] = if drive.status_changed == 0 {
                            0 // No change
                        } else if inserted {
                            4 // Media changed (inserted)
                        } else {
                            3 // Media removed
                        };
                        // Clear status_changed after reporting — prevents infinite
                        // media-change loop when kernel polls GESN without issuing TUR
                        drive.status_changed = 0;
                        // Media Status: bit 1 = Media Present
                        drive.controller.buffer[5] = if inserted { 1 << 1 } else { 0 };
                        drive.controller.buffer[6] = 0;
                        drive.controller.buffer[7] = 0;
                        event_length = if alloc_length <= 4 { 4 } else { 8 };
                    } else {
                        let drive = self.channels[channel_num].selected_drive_mut();
                        drive.controller.buffer[0] = 0;
                        drive.controller.buffer[1] = 0;
                        drive.controller.buffer[2] = (1 << 7) | request; // NEA=1 | requested class
                        drive.controller.buffer[3] = 1 << 4; // supported events: MEDIA only
                        event_length = 4;
                    }
                    self.init_send_atapi_command(
                        channel_num,
                        atapi_command,
                        event_length,
                        event_length,
                        false,
                    );
                    self.ready_to_send_atapi(channel_num, pic, pci_ide);
                } else {
                    tracing::debug!("ATAPI: Event Status Notification — polled only supported");
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::IllegalRequest,
                        Asc::InvFieldInCmdPacket,
                    );
                    self.raise_interrupt(channel_num, pic, pci_ide);
                }
            }
            0x51 => {
                // READ DISC INFO — no-op to keep Linux CD-ROM driver happy
                // Bochs harddrv.cc
                self.atapi_cmd_error(
                    channel_num,
                    SenseKey::IllegalRequest,
                    Asc::InvFieldInCmdPacket,
                );
                self.raise_interrupt(channel_num, pic, pci_ide);
            }
            0xbd => {
                // MECHANISM STATUS
                let drive = self.channels[channel_num].selected_drive_mut();
                let alloc_length = ((drive.controller.buffer[8] as i32) << 8)
                    | drive.controller.buffer[9] as i32;
                self.init_send_atapi_command(
                    channel_num,
                    atapi_command,
                    8,
                    alloc_length,
                    false,
                );
                let drive = self.channels[channel_num].selected_drive_mut();
                for i in 0..8 {
                    drive.controller.buffer[i] = 0;
                }
                drive.controller.buffer[5] = 1; // one slot
                self.ready_to_send_atapi(channel_num, pic, pci_ide);
            }
            0x1a => {
                // MODE SENSE (6) — Bochs harddrv.cc
                let drive = self.channels[channel_num].selected_drive_mut();
                let mode_alloc_length = drive.controller.buffer[4] as i32;
                let page_code = drive.controller.buffer[2] & 0x3f;
                let ready = drive.cdrom.ready;
                let locked = drive.cdrom.locked;
                match page_code {
                    0x01 => {
                        // Error recovery page (Bochs harddrv.cc)
                        self.init_send_atapi_command(channel_num, atapi_command, 16, mode_alloc_length, false);
                        let drive = self.channels[channel_num].selected_drive_mut();
                        drive.controller.buffer[0] = 14; // mode data length
                        drive.controller.buffer[1] = if ready { 0x12 } else { 0x70 }; // medium type
                        drive.controller.buffer[2] = 0; // device-specific
                        drive.controller.buffer[3] = 0; // block descriptor length
                        // error recovery page
                        drive.controller.buffer[4] = 0x01;
                        drive.controller.buffer[5] = 0x06;
                        drive.controller.buffer[6] = 0x00; // error recovery params
                        drive.controller.buffer[7] = 0x05; // read retry count
                        drive.controller.buffer[8] = 0x00;
                        drive.controller.buffer[9] = 0x00;
                        drive.controller.buffer[10] = 0x00;
                        drive.controller.buffer[11] = 0x00;
                        self.ready_to_send_atapi(channel_num, pic, pci_ide);
                    }
                    0x2a => {
                        // CD-ROM capabilities page (same as MODE SENSE 10)
                        self.init_send_atapi_command(channel_num, atapi_command, 24, mode_alloc_length, false);
                        let drive = self.channels[channel_num].selected_drive_mut();
                        drive.controller.buffer[0] = 22; // mode data length
                        drive.controller.buffer[1] = if ready { 0x12 } else { 0x70 };
                        drive.controller.buffer[2] = 0;
                        drive.controller.buffer[3] = 0;
                        drive.controller.buffer[4] = 0x2a;
                        drive.controller.buffer[5] = 0x12;
                        drive.controller.buffer[6] = 0x03;
                        drive.controller.buffer[7] = 0x00;
                        drive.controller.buffer[8] = 0x71;
                        drive.controller.buffer[9] = 3 << 5;
                        let locked_bit = if locked { 1 << 1 } else { 0 };
                        drive.controller.buffer[10] = 1 | locked_bit | (1 << 3) | (1 << 5);
                        drive.controller.buffer[11] = 0;
                        drive.controller.buffer[12] = ((16 * 176) >> 8) as u8;
                        drive.controller.buffer[13] = ((16 * 176) & 0xff) as u8;
                        drive.controller.buffer[14] = 0;
                        drive.controller.buffer[15] = 2;
                        drive.controller.buffer[16] = (512 >> 8) as u8;
                        drive.controller.buffer[17] = (512 & 0xff) as u8;
                        drive.controller.buffer[18] = ((16 * 176) >> 8) as u8;
                        drive.controller.buffer[19] = ((16 * 176) & 0xff) as u8;
                        for i in 20..24 { drive.controller.buffer[i] = 0; }
                        self.ready_to_send_atapi(channel_num, pic, pci_ide);
                    }
                    _ => {
                        self.atapi_cmd_error(channel_num, SenseKey::IllegalRequest, Asc::InvFieldInCmdPacket);
                        self.raise_interrupt(channel_num, pic, pci_ide);
                    }
                }
            }
            0x5a => {
                // MODE SENSE (10) — Bochs harddrv.cc
                let drive = self.channels[channel_num].selected_drive_mut();
                let mode_alloc_length = ((drive.controller.buffer[7] as i32) << 8)
                    | drive.controller.buffer[8] as i32;
                let page_code = drive.controller.buffer[2] & 0x3f;
                let ready = drive.cdrom.ready;
                let locked = drive.cdrom.locked;
                match page_code {
                    0x01 => {
                        // Error recovery page (Bochs harddrv.cc)
                        self.init_send_atapi_command(channel_num, atapi_command, 20, mode_alloc_length, false);
                        let drive = self.channels[channel_num].selected_drive_mut();
                        drive.controller.buffer[0] = 0;
                        drive.controller.buffer[1] = 18; // mode data length
                        drive.controller.buffer[2] = if ready { 0x12 } else { 0x70 };
                        for i in 3..8 { drive.controller.buffer[i] = 0; }
                        // error recovery page
                        drive.controller.buffer[8] = 0x01;
                        drive.controller.buffer[9] = 0x06;
                        drive.controller.buffer[10] = 0x00;
                        drive.controller.buffer[11] = 0x05; // read retry count
                        drive.controller.buffer[12] = 0x00;
                        drive.controller.buffer[13] = 0x00;
                        drive.controller.buffer[14] = 0x00;
                        drive.controller.buffer[15] = 0x00;
                        for i in 16..20 { drive.controller.buffer[i] = 0; }
                        self.ready_to_send_atapi(channel_num, pic, pci_ide);
                    }
                    0x2a => {
                        // CD-ROM capabilities
                        self.init_send_atapi_command(
                            channel_num,
                            atapi_command,
                            28,
                            mode_alloc_length,
                            false,
                        );
                        let drive = self.channels[channel_num].selected_drive_mut();
                        let size = 20; // page size
                        drive.controller.buffer[0] = ((size + 6) >> 8) as u8;
                        drive.controller.buffer[1] = ((size + 6) & 0xff) as u8;
                        drive.controller.buffer[2] = if ready { 0x12 } else { 0x70 };
                        for i in 3..8 {
                            drive.controller.buffer[i] = 0;
                        }
                        drive.controller.buffer[8] = 0x2a;
                        drive.controller.buffer[9] = 0x12;
                        drive.controller.buffer[10] = 0x03;
                        drive.controller.buffer[11] = 0x00;
                        drive.controller.buffer[12] = 0x71;
                        drive.controller.buffer[13] = 3 << 5;
                        let locked_bit = if locked { 1 << 1 } else { 0 };
                        drive.controller.buffer[14] = 1 | locked_bit | (1 << 3) | (1 << 5);
                        drive.controller.buffer[15] = 0;
                        drive.controller.buffer[16] = ((16 * 176) >> 8) as u8;
                        drive.controller.buffer[17] = ((16 * 176) & 0xff) as u8;
                        drive.controller.buffer[18] = 0;
                        drive.controller.buffer[19] = 2;
                        drive.controller.buffer[20] = (512 >> 8) as u8;
                        drive.controller.buffer[21] = (512 & 0xff) as u8;
                        drive.controller.buffer[22] = ((16 * 176) >> 8) as u8;
                        drive.controller.buffer[23] = ((16 * 176) & 0xff) as u8;
                        for i in 24..28 {
                            drive.controller.buffer[i] = 0;
                        }
                        self.ready_to_send_atapi(channel_num, pic, pci_ide);
                    }
                    _ => {
                        self.atapi_cmd_error(
                            channel_num,
                            SenseKey::IllegalRequest,
                            Asc::InvFieldInCmdPacket,
                        );
                        self.raise_interrupt(channel_num, pic, pci_ide);
                    }
                }
            }
            0xbe => {
                // READ CD
                let drive = self.channels[channel_num].selected_drive_mut();
                let lba = ((drive.controller.buffer[2] as u32) << 24)
                    | ((drive.controller.buffer[3] as u32) << 16)
                    | ((drive.controller.buffer[4] as u32) << 8)
                    | drive.controller.buffer[5] as u32;
                let transfer_length = ((drive.controller.buffer[6] as i32) << 16)
                    | ((drive.controller.buffer[7] as i32) << 8)
                    | drive.controller.buffer[8] as i32;
                let transfer_req = drive.controller.buffer[9];
                let ready = drive.cdrom.ready;

                if !ready {
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::NotReady,
                        Asc::MediumNotPresent,
                    );
                    self.raise_interrupt(channel_num, pic, pci_ide);
                    return;
                }
                if transfer_length == 0 || (transfer_req & 0xf8) == 0 {
                    self.atapi_cmd_nop(channel_num);
                    self.raise_interrupt(channel_num, pic, pci_ide);
                    return;
                }
                let sector_size = if (transfer_req & 0xf8) == 0xf8 {
                    2352
                } else {
                    2048
                };
                let total_bytes = transfer_length * sector_size;
                self.init_send_atapi_command(
                    channel_num,
                    atapi_command,
                    total_bytes,
                    total_bytes,
                    true,
                );
                let drive = self.channels[channel_num].selected_drive_mut();
                drive.controller.buffer_size = sector_size as usize;
                drive.cdrom.remaining_blocks = transfer_length;
                drive.cdrom.next_lba = lba;
                // Bochs: start_seek(channel) defers via timer
                self.seek_complete_pending[channel_num] = true;
            }
            0x2b => {
                // SEEK (Bochs harddrv.cc)
                let drive = self.channels[channel_num].selected_drive_mut();
                let lba = ((drive.controller.buffer[2] as u32) << 24)
                    | ((drive.controller.buffer[3] as u32) << 16)
                    | ((drive.controller.buffer[4] as u32) << 8)
                    | drive.controller.buffer[5] as u32;
                if !drive.cdrom.ready {
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::NotReady,
                        Asc::MediumNotPresent,
                    );
                    self.raise_interrupt(channel_num, pic, pci_ide);
                } else if lba > drive.cdrom.max_lba {
                    self.atapi_cmd_error(
                        channel_num,
                        SenseKey::IllegalRequest,
                        Asc::LogicalBlockOor,
                    );
                    self.raise_interrupt(channel_num, pic, pci_ide);
                } else {
                    drive.cdrom.curr_lba = lba;
                    self.atapi_cmd_nop(channel_num);
                    self.raise_interrupt(channel_num, pic, pci_ide);
                }
            }
            _ => {
                tracing::warn!("ATAPI: unknown command {:#04x}", atapi_command);
                self.atapi_cmd_error(
                    channel_num,
                    SenseKey::IllegalRequest,
                    Asc::IllegalOpcode,
                );
                self.raise_interrupt(channel_num, pic, pci_ide);
            }
        }
    }

    /// Execute an ATA command.
    ///
    /// READ SECTORS (0x20/0x21) protocol matches Bochs harddrv.cc:
    /// 1. lba48_transform → set num_sectors from sector_count register
    /// 2. buffer_size = 512 (one sector for single-sector reads)
    /// 3. ide_read_sector fills buffer with first sector (decrements num_sectors)
    /// 4. Set DRQ + raise IRQ (we skip seek timer emulation)
    /// 5. Host reads 256 words from data port
    /// 6. When buffer drained: if num_sectors > 0, load next sector + IRQ; else done
    ///
    /// WRITE SECTORS (0x30) protocol matches Bochs harddrv.cc:
    /// 1. lba48_transform → set num_sectors
    /// 2. buffer_size = 512, set DRQ (host will write data)
    /// 3. Host writes 256 words to data port
    /// 4. When buffer full: ide_write_sector writes to disk
    /// 5. If num_sectors > 0: keep DRQ for next sector; else clear DRQ
    fn execute_command(&mut self, channel_num: usize, command: u8, pic: &mut super::pic::BxPicC, pci_ide: &mut super::pci_ide::BxPciIde) {
        // Bochs harddrv.cc: RECALIBRATE range masking
        // Commands 0x10-0x1F all map to RECALIBRATE (only top nibble matters)
        let command = if (command & 0xF0) == 0x10 {
            0x10
        } else {
            command
        };

        let channel = &mut self.channels[channel_num];
        let ds = channel.drive_select;
        let drive = channel.selected_drive_mut();

        // Record command in history (circular buffer — keep last 256 commands so
        // BIOS-phase entries don't crowd out kernel-phase ATA activity).
        let lba = drive.get_lba();
        if self.cmd_history.len() >= 256 {
            self.cmd_history.remove(0);
        }
        self.cmd_history.push((channel_num as u8, command, lba));

        if drive.device_type == DeviceType::None {
            tracing::debug!("[ATA-DIAG] cmd {:#04x} to ch{} (empty) — dropped", command, channel_num);
            return;
        }

        drive.controller.current_command = command;
        // Bochs harddrv.cc: only clears ERR bit, preserves all other status bits
        drive.controller.error = AtaError::empty();
        drive.controller.status.remove(AtaStatus::ERR);

        tracing::debug!(
            "ATA: Command {:#04x} drive={} scount={} sno={} cyl={} head={} lba_mode={}",
            command,
            ds,
            drive.controller.sector_count,
            drive.controller.sector_no,
            drive.controller.cylinder_no,
            drive.controller.head_no,
            drive.controller.lba_mode
        );

        match command {
            ATA_CMD_RECALIBRATE => {
                // Bochs harddrv.cc
                if drive.device_type != DeviceType::Disk {
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                }
                drive.controller.error = AtaError::empty();
                drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
                drive.controller.cylinder_no = 0;
                drive.controller.interrupt_pending = true;
            }
            // 0x20 = READ SECTORS with retries, 0x21 = without retries, 0x24 = READ SECTORS EXT (LBA48)
            ATA_CMD_READ_SECTORS | 0x21 | ATA_CMD_READ_SECTORS_EXT => {
                // Bochs harddrv.cc — READ SECTORS (+ EXT variant)
                let is_lba48 = command == ATA_CMD_READ_SECTORS_EXT;
                drive.lba48_transform(is_lba48);
                // Single-sector reads: one sector per batch
                drive.controller.buffer_size = SECTOR_SIZE;
                drive.controller.buffer_index = 0;

                tracing::debug!(
                    "ATA: READ SECTORS lba={} num_sectors={}",
                    drive.get_lba(),
                    drive.controller.num_sectors
                );

                // Bochs harddrv.cc: validate LBA before reading
                if drive.calculate_logical_address().is_none() {
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                }

                // Read first sector into buffer (decrements num_sectors via increment_address)
                if drive.ide_read_sector() {
                    // Skip seek timer — set DRQ and raise IRQ immediately
                    // Bochs seek_timer (harddrv.cc) does: clear BSY, set DRQ, raise IRQ
                    drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC | AtaStatus::DRQ;
                    drive.controller.buffer_index = 0;
                    drive.controller.interrupt_pending = true;
                } else {
                    // Bochs harddrv.cc: command_aborted on read failure
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                }
            }
            ATA_CMD_READ_MULTIPLE => {
                // Bochs harddrv.cc — READ MULTIPLE (28-bit only; 0x29 EXT not yet)
                drive.lba48_transform(false);
                if drive.controller.multiple_sectors == 0 {
                    drive.controller.error = AtaError::ABRT;
                    drive.controller.status = AtaStatus::ERR | AtaStatus::DRDY;
                } else {
                    let ms = drive.controller.multiple_sectors as u32;
                    if drive.controller.num_sectors > ms {
                        drive.controller.buffer_size = ms as usize * SECTOR_SIZE;
                    } else {
                        drive.controller.buffer_size =
                            drive.controller.num_sectors as usize * SECTOR_SIZE;
                    }
                    drive.controller.buffer_index = 0;

                    tracing::debug!(
                        "ATA: READ MULTIPLE lba={} num_sectors={} batch={}",
                        drive.get_lba(),
                        drive.controller.num_sectors,
                        drive.controller.buffer_size / SECTOR_SIZE
                    );

                    if drive.ide_read_sector() {
                        drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC | AtaStatus::DRQ;
                        drive.controller.buffer_index = 0;
                        drive.controller.interrupt_pending = true;
                    } else {
                        drive.controller.error = AtaError::ABRT;
                        drive.controller.status = AtaStatus::ERR | AtaStatus::DRDY;
                    }
                }
            }
            // 0x30 = WRITE SECTORS with retries, 0x31 = without retries, 0x34 = WRITE SECTORS EXT (LBA48)
            ATA_CMD_WRITE_SECTORS | 0x31 | ATA_CMD_WRITE_SECTORS_EXT => {
                // Bochs harddrv.cc — WRITE SECTORS (+ EXT variant)
                let is_lba48 = command == ATA_CMD_WRITE_SECTORS_EXT;
                drive.lba48_transform(is_lba48);
                // Single-sector writes: one sector per batch
                drive.controller.buffer_size = SECTOR_SIZE;
                drive.controller.buffer_index = 0;

                tracing::debug!(
                    "ATA: WRITE SECTORS lba={} num_sectors={}",
                    drive.get_lba(),
                    drive.controller.num_sectors
                );

                // Set DRQ — host will write sector data
                drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC | AtaStatus::DRQ;
                // No IRQ on initial write command (Bochs doesn't raise here)
            }
            ATA_CMD_WRITE_MULTIPLE => {
                // Bochs harddrv.cc — WRITE MULTIPLE (28-bit only; 0x39 EXT not yet)
                drive.lba48_transform(false);
                if drive.controller.multiple_sectors == 0 {
                    drive.controller.error = AtaError::ABRT;
                    drive.controller.status = AtaStatus::ERR | AtaStatus::DRDY;
                } else {
                    let ms = drive.controller.multiple_sectors as u32;
                    if drive.controller.num_sectors > ms {
                        drive.controller.buffer_size = ms as usize * SECTOR_SIZE;
                    } else {
                        drive.controller.buffer_size =
                            drive.controller.num_sectors as usize * SECTOR_SIZE;
                    }
                    drive.controller.buffer_index = 0;

                    tracing::debug!(
                        "ATA: WRITE MULTIPLE lba={} num_sectors={} batch={}",
                        drive.get_lba(),
                        drive.controller.num_sectors,
                        drive.controller.buffer_size / SECTOR_SIZE
                    );

                    drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC | AtaStatus::DRQ;
                }
            }
            // 0x40 = READ VERIFY with retries, 0x41 = without retries
            ATA_CMD_READ_VERIFY | 0x41 => {
                // Bochs harddrv.cc — verify sectors, no data transfer
                drive.lba48_transform(false);
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_SEEK => {
                // Bochs harddrv.cc — seek to specified CHS/LBA
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_EXECUTE_DIAGNOSTICS => {
                // Bochs harddrv.cc: set_signature + error=0x01 + raise_interrupt
                // Must set signature for BOTH drives on the channel
                drive.controller.head_no = 0;
                drive.controller.sector_count = 1;
                drive.controller.sector_no = 1;
                match drive.device_type {
                    DeviceType::Disk => drive.controller.cylinder_no = 0,
                    DeviceType::Cdrom => drive.controller.cylinder_no = 0xEB14,
                    DeviceType::None => drive.controller.cylinder_no = 0xFFFF,
                }
                drive.controller.status.remove(AtaStatus::DRQ); // Clear DRQ
                drive.controller.error = AtaError::from_bits_retain(0x01); // No error
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_INITIALIZE_PARAMS => {
                // Bochs harddrv.cc — INITIALIZE DRIVE PARAMETERS
                let spt = drive.controller.sector_count;
                let head_no = drive.controller.head_no;
                let disk_spt = drive.geometry.sectors_per_track;
                let disk_heads = drive.geometry.heads;
                tracing::debug!("ATA: Initialize params sec={} head={}", spt, head_no);

                if spt != disk_spt {
                    tracing::error!(
                        "ATA: init drive params: logical sector count {} not supported (expected {})",
                        spt, disk_spt
                    );
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                } else if head_no == 0 {
                    // Linux 2.6.x kernels use head_no=0 — log but don't abort (Bochs behavior)
                    tracing::debug!("ATA: init drive params: max. logical head number 0");
                    drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
                    drive.controller.interrupt_pending = true;
                } else if head_no != (disk_heads - 1) {
                    tracing::error!(
                        "ATA: init drive params: max. logical head number {} not supported (expected {})",
                        head_no, disk_heads - 1
                    );
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                } else {
                    drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC;
                    drive.controller.interrupt_pending = true;
                }
            }
            ATA_CMD_IDENTIFY => {
                if drive.device_type == DeviceType::Cdrom {
                    // Bochs: CDROM drives abort regular IDENTIFY and set signature
                    drive.controller.head_no = 0;
                    drive.controller.sector_count = 1;
                    drive.controller.sector_no = 1;
                    drive.controller.cylinder_no = 0xEB14;
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                }
                tracing::debug!("ATA: IDENTIFY command");
                drive.fill_identify_buffer();
                drive.controller.status.insert(AtaStatus::DRQ);
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_SET_FEATURES => {
                // Bochs harddrv.cc — SET FEATURES sub-commands
                let subcommand = drive.controller.features;
                match subcommand {
                    0x02 => {
                        // Enable write cache — no-op, just succeed
                        tracing::debug!("ATA: SET FEATURES: enable write cache");
                    }
                    0x03 => {
                        // Set transfer mode (PIO/DMA) based on sector_count
                        // Bochs harddrv.cc
                        let xfer_type = drive.controller.sector_count >> 3;
                        let xfer_mode = drive.controller.sector_count & 0x07;
                        match xfer_type {
                            0x00 | 0x01 => {
                                // PIO default / PIO mode
                                tracing::debug!("ATA: SET FEATURES: set transfer mode to PIO");
                                drive.controller.mdma_mode = 0x00;
                                drive.controller.udma_mode = 0x00;
                            }
                            0x04 => {
                                // MDMA mode
                                tracing::debug!("ATA: SET FEATURES: set transfer mode to MDMA{} ch={}", xfer_mode, channel_num);
                                drive.controller.mdma_mode = 1 << xfer_mode;
                                drive.controller.udma_mode = 0x00;
                            }
                            0x08 => {
                                // UDMA mode
                                tracing::debug!("ATA: SET FEATURES: set transfer mode to UDMA{} ch={}", xfer_mode, channel_num);
                                drive.controller.mdma_mode = 0x00;
                                drive.controller.udma_mode = 1 << xfer_mode;
                            }
                            _ => {
                                tracing::debug!(
                                    "ATA: SET FEATURES: unknown transfer mode type {:#04x}",
                                    xfer_type
                                );
                                self.command_aborted(channel_num, command, pic, pci_ide);
                                return;
                            }
                        }
                        // Bochs harddrv.cc — force IDENTIFY re-generation
                        // on next IDENTIFY command to reflect new transfer mode
                        drive.identify_set = false;
                    }
                    0x82 => {
                        // Disable write cache — no-op, just succeed
                        tracing::debug!("ATA: SET FEATURES: disable write cache");
                    }
                    0xAA => {
                        // Enable read look-ahead — no-op
                        tracing::debug!("ATA: SET FEATURES: enable read look-ahead");
                    }
                    0x55 => {
                        // Disable read look-ahead — no-op
                        tracing::debug!("ATA: SET FEATURES: disable read look-ahead");
                    }
                    0xCC => {
                        // Enable reverting to power-on defaults — no-op
                    }
                    0x66 => {
                        // Disable reverting to power-on defaults — no-op
                    }
                    _ => {
                        tracing::debug!(
                            "ATA: SET FEATURES: unknown subcommand {:#04x}",
                            subcommand
                        );
                        self.command_aborted(channel_num, command, pic, pci_ide);
                        return;
                    }
                }
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_SET_MULTIPLE => {
                // Bochs harddrv.cc — SET MULTIPLE MODE
                // Sector count must be a power of 2, 1-128
                let count = drive.controller.sector_count;
                if count == 0 || count > 128 || (count & (count - 1)) != 0 {
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                }
                drive.controller.multiple_sectors = count;
                drive.controller.interrupt_pending = true;
            }
            // Bochs harddrv.cc: CHECK POWER MODE — returns 0xFF in sector count (active/idle)
            0xE5 => {
                drive.controller.sector_count = 0xFF;
                drive.controller.interrupt_pending = true;
            }
            // Bochs harddrv.cc: STANDBY NOW — no-op
            0xE0 => {
                drive.controller.interrupt_pending = true;
            }
            // Bochs harddrv.cc: IDLE IMMEDIATE — no-op
            0xE1 => {
                drive.controller.interrupt_pending = true;
            }
            // Bochs harddrv.cc: FLUSH CACHE — no-op
            0xE7 => {
                drive.controller.interrupt_pending = true;
            }
            // Bochs harddrv.cc: FLUSH CACHE EXT — no-op (LBA48 variant)
            0xEA => {
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_IDENTIFY_PACKET => {
                // Bochs harddrv.cc — IDENTIFY PACKET DEVICE
                if drive.device_type == DeviceType::Cdrom {
                    drive.controller.current_command = command;
                    drive.controller.error = AtaError::empty();
                    drive.controller.status = AtaStatus::DRDY | AtaStatus::DSC | AtaStatus::DRQ;
                    // Set interrupt_reason: i_o=1, c_d=0 (data to host)
                    drive.controller.sector_count = (drive.controller.sector_count & 0xF8) | 0x02;
                    // Restore ATAPI signature in cylinder registers (byte_count).
                    // The Linux ata_piix driver zeroes CYL_LOW/CYL_HIGH before sending
                    // IDENTIFY PACKET, then re-reads them after completion to classify
                    // the device via ata_dev_classify(). ATAPI devices must report
                    // their signature (0xEB14) here so the driver sees ATAPI, not ATA.
                    drive.controller.cylinder_no = 0xEB14;
                    drive.controller.buffer_index = 0;

                    if !drive.identify_set {
                        drive.identify_atapi_drive();
                    }
                    // Convert id_drive[] to controller buffer (byte-swapped)
                    for i in 0..256 {
                        let w = drive.id_drive[i];
                        drive.controller.buffer[i * 2] = (w & 0xFF) as u8;
                        drive.controller.buffer[i * 2 + 1] = (w >> 8) as u8;
                    }
                    drive.controller.buffer_size = 512;
                    drive.controller.interrupt_pending = true;
                } else {
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                }
            }
            ATA_CMD_DEVICE_RESET => {
                // Bochs harddrv.cc — DEVICE RESET
                if drive.device_type == DeviceType::Cdrom {
                    drive.controller.head_no = 0;
                    drive.controller.sector_count = 1;
                    drive.controller.sector_no = 1;
                    drive.controller.cylinder_no = 0xEB14;
                    // Bochs harddrv.cc: clear all status bits
                    drive.controller.status = AtaStatus::empty();
                    drive.controller.error = AtaError::from_bits_retain(drive.controller.error.bits() & !(1 << 7));
                } else {
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                }
            }
            ATA_CMD_PACKET => {
                // Bochs harddrv.cc — SEND PACKET (ATAPI)
                if drive.device_type == DeviceType::Cdrom {
                    // Bochs harddrv.cc
                    let features = drive.controller.features;
                    drive.controller.packet_dma = (features & 1) != 0;
                    if drive.controller.packet_dma {
                        tracing::debug!("ATAPI: PACKET cmd with DMA flag (features={:#04x})", features);
                    }
                    if (features & (1 << 1)) != 0 {
                        tracing::debug!("ATA: PACKET-overlapped not supported");
                        self.command_aborted(channel_num, ATA_CMD_PACKET, pic, pci_ide);
                        return;
                    }
                    drive.controller.sector_count = 1; // c_d=1 (command)
                    // Bochs sets individual fields: busy=0, write_fault=0, drq=1
                    // preserving other bits like drive_ready (DRDY) and seek_complete (DSC).
                    drive.controller.status.remove(AtaStatus::BSY | AtaStatus::DWF);
                    drive.controller.status.insert(AtaStatus::DRQ);
                    drive.controller.current_command = command;
                    drive.controller.buffer_index = 0;
                    drive.controller.buffer_size = PACKET_SIZE;
                    // No interrupt here (Bochs harddrv.cc)
                    return; // Don't raise interrupt
                } else {
                    self.command_aborted(channel_num, command, pic, pci_ide);
                    return;
                }
            }
            _ => {
                tracing::warn!("ATA: Unknown command {:#04x}", command);
                self.command_aborted(channel_num, command, pic, pci_ide);
                return;
            }
        }

        // Bochs harddrv.cc: Commands that want an interrupt set interrupt_pending=true.
        // In Bochs, these commands call raise_interrupt() which does DEV_pic_raise_irq().
        // We use raise_interrupt() here for proper PIC raise + diagnostic counting.
        if self.channels[channel_num]
            .selected_drive()
            .controller
            .interrupt_pending
        {
            self.raise_interrupt(channel_num, pic, pci_ide);
        }
    }

    /// Get diagnostic string for the ATA controller state
    pub fn diag_string(&self) -> String {
        let mut s = String::new();
        for ch in 0..2 {
            for drv in 0..2 {
                let drive = &self.channels[ch].drives[drv];
                s.push_str(&format!(
                    "  ch{} drv{}: type={:?} cmd={:#04x} status={:#04x} ctrl={:#04x} irq_pend={} sec_cnt={} buf_idx={}\n",
                    ch, drv, drive.device_type,
                    drive.controller.current_command,
                    drive.controller.status.bits(),
                    drive.controller.control,
                    drive.controller.interrupt_pending,
                    drive.controller.sector_count,
                    drive.controller.buffer_index,
                ));
            }
        }
        s.push_str(&format!(
            "  irq14_level={} irq15_level={}\n",
            self.get_irq_level(0), self.get_irq_level(1),
        ));
        s.push_str(&format!("  cmd_history ({} cmds):", self.cmd_history.len()));
        for (i, &(ch, cmd, lba)) in self.cmd_history.iter().enumerate() {
            if i % 8 == 0 {
                s.push('\n');
                s.push_str("    ");
            }
            s.push_str(&format!("ch{}:{:#04x}@{} ", ch, cmd, lba));
        }
        s
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_geometry_conversion() {
        let geom = DriveGeometry::from_chs(306, 4, 17);

        // CHS to LBA
        let lba = geom.chs_to_lba(0, 0, 1);
        assert_eq!(lba, 0);

        let lba = geom.chs_to_lba(0, 0, 17);
        assert_eq!(lba, 16);

        let lba = geom.chs_to_lba(0, 1, 1);
        assert_eq!(lba, 17);

        // LBA to CHS
        let (c, h, s) = geom.lba_to_chs(0);
        assert_eq!((c, h, s), (0, 0, 1));
    }

    #[test]
    fn test_controller_creation() {
        let hd = BxHardDriveC::new();
        assert_eq!(hd.channels[0].ioaddr1, 0x1F0);
        assert_eq!(hd.channels[1].ioaddr1, 0x170);
    }

    #[test]
    fn test_lba48_transform() {
        let mut drive = AtaDrive::create_disk(DriveGeometry::from_chs(306, 4, 17));

        // 28-bit mode: sector_count = 5 → num_sectors = 5
        drive.controller.sector_count = 5;
        drive.lba48_transform(false);
        assert_eq!(drive.controller.num_sectors, 5);
        assert!(!drive.controller.lba48);

        // 28-bit mode: sector_count = 0 → num_sectors = 256
        drive.controller.sector_count = 0;
        drive.lba48_transform(false);
        assert_eq!(drive.controller.num_sectors, 256);

        // 48-bit mode: sector_count = 5, hob.nsector = 0 → num_sectors = 5
        drive.controller.sector_count = 5;
        drive.controller.hob.nsector = 0;
        drive.lba48_transform(true);
        assert_eq!(drive.controller.num_sectors, 5);
        assert!(drive.controller.lba48);

        // 48-bit mode: sector_count = 0, hob.nsector = 1 → num_sectors = 256
        drive.controller.sector_count = 0;
        drive.controller.hob.nsector = 1;
        drive.lba48_transform(true);
        assert_eq!(drive.controller.num_sectors, 256);

        // 48-bit mode: both zero → num_sectors = 65536
        drive.controller.sector_count = 0;
        drive.controller.hob.nsector = 0;
        drive.lba48_transform(true);
        assert_eq!(drive.controller.num_sectors, 65536);
    }

    #[test]
    fn test_increment_address_lba() {
        let mut drive = AtaDrive::create_disk(DriveGeometry::from_chs(306, 4, 17));
        drive.controller.lba_mode = true;
        drive.controller.sector_no = 0;
        drive.controller.cylinder_no = 0;
        drive.controller.head_no = 0;
        drive.controller.sector_count = 3;
        drive.controller.num_sectors = 3;

        // First increment: LBA 0 → 1
        drive.increment_address();
        assert_eq!(drive.controller.sector_no, 1);
        assert_eq!(drive.controller.num_sectors, 2);
        assert_eq!(drive.controller.sector_count, 2);

        // Second increment: LBA 1 → 2
        drive.increment_address();
        assert_eq!(drive.controller.sector_no, 2);
        assert_eq!(drive.controller.num_sectors, 1);
    }

    #[test]
    fn test_increment_address_chs() {
        let mut drive = AtaDrive::create_disk(DriveGeometry::from_chs(306, 4, 17));
        drive.controller.lba_mode = false;
        drive.controller.sector_no = 17; // Last sector in track
        drive.controller.cylinder_no = 0;
        drive.controller.head_no = 0;
        drive.controller.sector_count = 2;
        drive.controller.num_sectors = 2;

        // Should wrap to next head
        drive.increment_address();
        assert_eq!(drive.controller.sector_no, 1);
        assert_eq!(drive.controller.head_no, 1);
        assert_eq!(drive.controller.cylinder_no, 0);
        assert_eq!(drive.controller.num_sectors, 1);
    }
}
