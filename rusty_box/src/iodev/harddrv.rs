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

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;

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

/// Status register bits
pub const ATA_STATUS_ERR: u8 = 0x01; // Error
pub const ATA_STATUS_IDX: u8 = 0x02; // Index (always 0)
pub const ATA_STATUS_CORR: u8 = 0x04; // Corrected data (always 0)
pub const ATA_STATUS_DRQ: u8 = 0x08; // Data request
pub const ATA_STATUS_DSC: u8 = 0x10; // Drive seek complete
pub const ATA_STATUS_DWF: u8 = 0x20; // Drive write fault
pub const ATA_STATUS_DRDY: u8 = 0x40; // Drive ready
pub const ATA_STATUS_BSY: u8 = 0x80; // Busy

/// Error register bits
pub const ATA_ERROR_AMNF: u8 = 0x01; // Address mark not found
pub const ATA_ERROR_TK0NF: u8 = 0x02; // Track 0 not found
pub const ATA_ERROR_ABRT: u8 = 0x04; // Command aborted
pub const ATA_ERROR_IDNF: u8 = 0x10; // ID not found
pub const ATA_ERROR_UNC: u8 = 0x40; // Uncorrectable data error

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

        (cylinder as u32 * heads * spt) + (head as u32 * spt) + (sector as u32 - 1)
    }
}

/// Controller state for one ATA drive (Bochs `controller_t`, harddrv.h:45-107).
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
    /// Status register (port+7 read). See ATA_STATUS_* constants for bit definitions.
    pub(crate) status: u8,
    /// Error register (port+1 read). Set when status ERR bit is set.
    /// After EXECUTE DEVICE DIAGNOSTIC: 0x01 = no error (diagnostic passed).
    pub(crate) error: u8,
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
    /// Number of valid bytes in the buffer for the current transfer batch.
    /// For single-sector commands: 512 bytes.
    /// For IDENTIFY DEVICE: 512 bytes (256 words of device info).
    /// For multi-sector: `min(multiple_sectors, num_sectors) * sect_size`.
    pub(crate) buffer_size: usize,
    /// Internal remaining-sector counter (Bochs `controller_t::num_sectors`).
    /// Set at command start by `lba48_transform()` from `sector_count` register
    /// (0 means 256). Decremented by `increment_address()` after each sector.
    /// When it reaches 0, the transfer is complete and DRQ is cleared.
    pub(crate) num_sectors: u32,
    /// Reset in progress — set when SRST bit is written to Device Control register.
    /// Cleared when SRST is deasserted, at which point the drive signature is set.
    pub(crate) reset_in_progress: bool,
}

impl Default for AtaController {
    fn default() -> Self {
        Self {
            status: ATA_STATUS_DRDY | ATA_STATUS_DSC,
            error: 0x01, // Diagnostic passed
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
            buffer_size: 0,
            num_sectors: 0,
            reset_in_progress: false,
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
}

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
        }
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

    /// Initialize `num_sectors` from the `sector_count` register.
    ///
    /// Matches Bochs `lba48_transform()` (harddrv.cc:3787-3802).
    /// Called at the start of every READ/WRITE/VERIFY command to set up the
    /// internal transfer counter.
    ///
    /// In the ATA spec, a sector count of 0 means 256 sectors (for 28-bit commands)
    /// or 65536 sectors (for 48-bit LBA48 commands). We only support 28-bit LBA,
    /// so `num_sectors = sector_count` or 256 if `sector_count == 0`.
    fn lba48_transform(&mut self) {
        if self.controller.sector_count == 0 {
            self.controller.num_sectors = 256;
        } else {
            self.controller.num_sectors = self.controller.sector_count as u32;
        }
    }

    /// Advance CHS/LBA registers to the next sector and decrement counters.
    ///
    /// Called after each sector is successfully read from or written to disk.
    /// Matches Bochs `increment_address()` (harddrv.cc:2908-2944).
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
            // LBA mode: increment the 28-bit LBA value stored across registers
            let logical_sector = self.get_lba() as u64 + 1;
            self.controller.head_no = ((logical_sector >> 24) & 0xf) as u8;
            self.controller.cylinder_no = ((logical_sector >> 8) & 0xffff) as u16;
            self.controller.sector_no = (logical_sector & 0xff) as u8;
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
    /// Matches Bochs `ide_read_sector()` (harddrv.cc:3713-3748).
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

        tracing::trace!(
            "ATA: ide_read_sector: read {} sector(s), num_sectors remaining={}",
            sector_count,
            self.controller.num_sectors
        );
        true
    }

    /// Write buffer_size/512 sectors from controller buffer to disk at current register position.
    /// Matches Bochs ide_write_sector() (harddrv.cc:3750-3785).
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
            let _ = f.flush();
        }

        tracing::trace!(
            "ATA: ide_write_sector: wrote {} sector(s), num_sectors remaining={}",
            sector_count,
            self.controller.num_sectors
        );
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

        tracing::trace!(
            "ATA: ide_write_sector: wrote {} sector(s), num_sectors remaining={}",
            sector_count,
            self.controller.num_sectors
        );
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
        buf[95] = 0x80;

        // Word 48: PIO32 support (1 = 32-bit PIO supported)
        buf[96] = 0x01;
        buf[97] = 0x00;

        // Word 49: Capabilities
        buf[98] = 0x00;
        buf[99] = 0x02; // LBA supported

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

        // Word 60-61: Total addressable sectors (LBA)
        buf[120] = (total & 0xFF) as u8;
        buf[121] = ((total >> 8) & 0xFF) as u8;
        buf[122] = ((total >> 16) & 0xFF) as u8;
        buf[123] = ((total >> 24) & 0xFF) as u8;

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
    /// IRQ14 pending (primary)
    pub(crate) irq14_pending: bool,
    /// IRQ15 pending (secondary)
    pub(crate) irq15_pending: bool,
    /// Diagnostic: total read() calls
    pub(crate) read_count: u64,
    /// Diagnostic: total write() calls
    pub(crate) write_count: u64,
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
            irq14_pending: false,
            irq15_pending: false,
            read_count: 0,
            write_count: 0,
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
            }
            channel.drive_select = 0;
        }
        self.irq14_pending = false;
        self.irq15_pending = false;
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
    /// Matches Bochs `raise_interrupt()` (harddrv.cc:2900-2906).
    /// Checks the nIEN bit (Device Control register bit 1). If nIEN=0 (interrupts
    /// enabled), sets the per-drive `interrupt_pending` flag and the channel-level
    /// `irqN_pending` flag which the main emulator loop checks to deliver the
    /// hardware interrupt via the PIC (IRQ14 for primary, IRQ15 for secondary).
    ///
    /// If nIEN=1 (interrupts disabled), this is a no-op. The BSY/DRQ/status bits
    /// are still updated by the caller — the host can poll Alternate Status instead.
    fn raise_interrupt(&mut self, channel_num: usize) {
        let drive = self.channels[channel_num].selected_drive_mut();
        // Only raise if nIEN bit (bit 1 of control register) is clear
        if (drive.controller.control & 0x02) == 0 {
            drive.controller.interrupt_pending = true;
            if channel_num == 0 {
                self.irq14_pending = true;
            } else {
                self.irq15_pending = true;
            }
        }
    }

    /// Read from ATA I/O port (Bochs `bx_hard_drive_c::read`, harddrv.cc:770-1152).
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
    pub fn read(&mut self, port: u16, io_len: u8) -> u32 {
        self.read_count += 1;
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

        // Check if drive exists
        if drive.device_type == DeviceType::None {
            return if offset == ATA_STATUS || offset == ATA_ALT_STATUS {
                0x00 // No drive
            } else {
                0xFF
            };
        }

        match offset {
            ATA_DATA => {
                // Bochs harddrv.cc:806-894 — data port read
                let current_command = drive.controller.current_command;
                let idx = drive.controller.buffer_index;
                let bytes = io_len as usize;

                if idx + bytes > drive.controller.buffer_size {
                    // This can happen when the BIOS overshoots by one read after
                    // draining the IDENTIFY buffer — harmless, return 0.
                    tracing::trace!(
                        "ATA: data read past buffer end: index={} io_len={} buffer_size={}",
                        idx,
                        bytes,
                        drive.controller.buffer_size
                    );
                    return 0;
                }

                // Read bytes from buffer (little-endian)
                let mut value: u32 = 0;
                for b in 0..bytes {
                    value |= (drive.controller.buffer[idx + b] as u32) << (b * 8);
                }
                drive.controller.buffer_index += bytes;

                // Check if buffer completely read
                if drive.controller.buffer_index >= drive.controller.buffer_size {
                    match current_command {
                        ATA_CMD_READ_SECTORS | ATA_CMD_READ_SECTORS_EXT | ATA_CMD_READ_MULTIPLE => {
                            // Bochs harddrv.cc:860-893
                            // Recalculate buffer_size for READ MULTIPLE
                            if current_command == ATA_CMD_READ_MULTIPLE {
                                let ms = drive.controller.multiple_sectors as u32;
                                if drive.controller.num_sectors > ms {
                                    drive.controller.buffer_size = ms as usize * SECTOR_SIZE;
                                } else {
                                    drive.controller.buffer_size =
                                        drive.controller.num_sectors as usize * SECTOR_SIZE;
                                }
                            }

                            drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC;
                            drive.controller.error = 0;

                            if drive.controller.num_sectors == 0 {
                                // All sectors transferred — command complete
                                // DRQ already cleared by status assignment above
                            } else {
                                // More sectors to read — load next batch into buffer
                                drive.controller.status |= ATA_STATUS_DRQ;

                                if drive.ide_read_sector() {
                                    drive.controller.buffer_index = 0;
                                    // Raise interrupt for next sector
                                    if (drive.controller.control & 0x02) == 0 {
                                        drive.controller.interrupt_pending = true;
                                        if channel_num == 0 {
                                            self.irq14_pending = true;
                                        } else {
                                            self.irq15_pending = true;
                                        }
                                    }
                                } else {
                                    // Read error — abort command
                                    drive.controller.error = ATA_ERROR_ABRT;
                                    drive.controller.status = ATA_STATUS_ERR | ATA_STATUS_DRDY;
                                }
                            }
                        }
                        ATA_CMD_IDENTIFY => {
                            // IDENTIFY buffer drained — clear DRQ
                            drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC;
                            drive.controller.error = 0;
                        }
                        _ => {
                            // Generic: clear DRQ when buffer drained
                            drive.controller.status &= !ATA_STATUS_DRQ;
                        }
                    }
                }

                return value;
            }
            ATA_ERROR => drive.controller.error as u32,
            ATA_SECTOR_COUNT => drive.controller.sector_count as u32,
            ATA_SECTOR_NUM => drive.controller.sector_no as u32,
            ATA_CYL_LOW => (drive.controller.cylinder_no & 0xFF) as u32,
            ATA_CYL_HIGH => (drive.controller.cylinder_no >> 8) as u32,
            ATA_DRIVE_HEAD => {
                let lba_bit = if drive.controller.lba_mode { 0x40 } else { 0 };
                let drive_bit = if drive_select != 0 { 0x10 } else { 0 };
                (0xA0 | lba_bit | drive_bit | (drive.controller.head_no & 0x0F)) as u32
            }
            ATA_STATUS | ATA_ALT_STATUS => {
                // Reading primary status clears the ATA-level interrupt flag
                // (but not for alternate status port).
                // NOTE: We do NOT clear irq14/15_pending here.  In Bochs,
                // raise_interrupt() calls DEV_pic_raise_irq() directly so
                // the PIC latches the IRQ immediately.  Our architecture
                // defers IRQ delivery to the next tick().  If we cleared
                // the pending flag on status read, the IRQ would be lost
                // whenever the kernel polls status in the same CPU batch
                // as the command that raised the interrupt.
                if offset == ATA_STATUS {
                    drive.controller.interrupt_pending = false;
                }
                tracing::debug!(
                    "ATA: Status read #{} = {:#04x} (port={:#06x}) cmd={:#04x} drq={}",
                    self.read_count,
                    drive.controller.status,
                    port,
                    drive.controller.current_command,
                    (drive.controller.status & 0x08) != 0,
                );
                drive.controller.status as u32
            }
            _ => 0xFF,
        }
    }

    /// Write to ATA I/O port (Bochs `bx_hard_drive_c::write`, harddrv.cc:1157-2500+).
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
    pub fn write(&mut self, port: u16, value: u32, io_len: u8) {
        self.write_count += 1;
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

        match offset {
            ATA_DATA => {
                // Bochs harddrv.cc:1229-1302 — data port write
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
                        | ATA_CMD_WRITE_SECTORS_EXT
                        | ATA_CMD_WRITE_MULTIPLE => {
                            // Bochs harddrv.cc:1266-1301
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
                                        ATA_STATUS_DRDY | ATA_STATUS_DSC | ATA_STATUS_DRQ;
                                    drive.controller.error = 0;
                                } else {
                                    // All sectors written — clear DRQ
                                    drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC;
                                    drive.controller.error = 0;
                                }

                                // Raise interrupt after each sector write
                                // (Bochs raises for both "more sectors" and "done")
                                if (drive.controller.control & 0x02) == 0 {
                                    drive.controller.interrupt_pending = true;
                                    if channel_num == 0 {
                                        self.irq14_pending = true;
                                    } else {
                                        self.irq15_pending = true;
                                    }
                                }
                            } else {
                                // Write error
                                tracing::error!("ATA: ide_write_sector failed");
                                drive.controller.error = ATA_ERROR_ABRT;
                                drive.controller.status = ATA_STATUS_ERR | ATA_STATUS_DRDY;
                                drive.controller.status &= !ATA_STATUS_DRQ;
                            }
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
                // Features register (write)
                let drive = channel.selected_drive_mut();
                drive.controller.features = value as u8;
            }
            ATA_SECTOR_COUNT => {
                for drive in &mut channel.drives {
                    drive.controller.sector_count = value as u8;
                }
            }
            ATA_SECTOR_NUM => {
                for drive in &mut channel.drives {
                    drive.controller.sector_no = value as u8;
                }
            }
            ATA_CYL_LOW => {
                for drive in &mut channel.drives {
                    drive.controller.cylinder_no =
                        (drive.controller.cylinder_no & 0xFF00) | (value as u16 & 0xFF);
                }
            }
            ATA_CYL_HIGH => {
                for drive in &mut channel.drives {
                    drive.controller.cylinder_no =
                        (drive.controller.cylinder_no & 0x00FF) | ((value as u16 & 0xFF) << 8);
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
                // Command register (write)
                self.execute_command(channel_num, value as u8);
            }
            ATA_ALT_STATUS => {
                // Device control register
                let value = value as u8;
                let drive = channel.selected_drive_mut();

                // Software reset
                if (value & 0x04) != 0 && (drive.controller.control & 0x04) == 0 {
                    tracing::debug!("ATA: Software reset");
                    drive.controller.reset_in_progress = true;
                    drive.controller.status = ATA_STATUS_BSY;
                } else if (value & 0x04) == 0 && drive.controller.reset_in_progress {
                    drive.controller.reset_in_progress = false;
                    drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC;
                    drive.controller.error = 0x01; // Diagnostic passed
                    drive.controller.sector_count = 1;
                    drive.controller.sector_no = 1;
                    drive.controller.cylinder_no = 0;
                    drive.controller.head_no = 0;
                }

                drive.controller.control = value;
            }
            _ => {}
        }
    }

    /// Execute an ATA command.
    ///
    /// READ SECTORS (0x20) protocol matches Bochs harddrv.cc:2220-2283:
    /// 1. lba48_transform → set num_sectors from sector_count register
    /// 2. buffer_size = 512 (one sector for single-sector reads)
    /// 3. ide_read_sector fills buffer with first sector (decrements num_sectors)
    /// 4. Set DRQ + raise IRQ (we skip seek timer emulation)
    /// 5. Host reads 256 words from data port
    /// 6. When buffer drained: if num_sectors > 0, load next sector + IRQ; else done
    ///
    /// WRITE SECTORS (0x30) protocol matches Bochs harddrv.cc:2288-2345:
    /// 1. lba48_transform → set num_sectors
    /// 2. buffer_size = 512, set DRQ (host will write data)
    /// 3. Host writes 256 words to data port
    /// 4. When buffer full: ide_write_sector writes to disk
    /// 5. If num_sectors > 0: keep DRQ for next sector; else clear DRQ
    fn execute_command(&mut self, channel_num: usize, command: u8) {
        let channel = &mut self.channels[channel_num];
        let ds = channel.drive_select;
        let drive = channel.selected_drive_mut();

        if drive.device_type == DeviceType::None {
            return;
        }

        drive.controller.current_command = command;
        drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC;
        drive.controller.error = 0;

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
                drive.controller.cylinder_no = 0;
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_READ_SECTORS | ATA_CMD_READ_SECTORS_EXT => {
                // Bochs harddrv.cc:2220-2283 — READ SECTORS
                drive.lba48_transform();
                // Single-sector reads: one sector per batch
                drive.controller.buffer_size = SECTOR_SIZE;
                drive.controller.buffer_index = 0;

                tracing::debug!(
                    "ATA: READ SECTORS lba={} num_sectors={}",
                    drive.get_lba(),
                    drive.controller.num_sectors
                );

                // Read first sector into buffer (decrements num_sectors via increment_address)
                if drive.ide_read_sector() {
                    // Skip seek timer — set DRQ and raise IRQ immediately
                    // Bochs seek_timer (harddrv.cc:655-718) does: clear BSY, set DRQ, raise IRQ
                    drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC | ATA_STATUS_DRQ;
                    drive.controller.buffer_index = 0;
                    drive.controller.interrupt_pending = true;
                } else {
                    drive.controller.error = ATA_ERROR_ABRT;
                    drive.controller.status = ATA_STATUS_ERR | ATA_STATUS_DRDY;
                }
            }
            ATA_CMD_READ_MULTIPLE => {
                // Bochs harddrv.cc:2250-2262 — READ MULTIPLE
                drive.lba48_transform();
                if drive.controller.multiple_sectors == 0 {
                    drive.controller.error = ATA_ERROR_ABRT;
                    drive.controller.status = ATA_STATUS_ERR | ATA_STATUS_DRDY;
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
                        drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC | ATA_STATUS_DRQ;
                        drive.controller.buffer_index = 0;
                        drive.controller.interrupt_pending = true;
                    } else {
                        drive.controller.error = ATA_ERROR_ABRT;
                        drive.controller.status = ATA_STATUS_ERR | ATA_STATUS_DRDY;
                    }
                }
            }
            ATA_CMD_WRITE_SECTORS | ATA_CMD_WRITE_SECTORS_EXT => {
                // Bochs harddrv.cc:2288-2345 — WRITE SECTORS
                drive.lba48_transform();
                // Single-sector writes: one sector per batch
                drive.controller.buffer_size = SECTOR_SIZE;
                drive.controller.buffer_index = 0;

                tracing::debug!(
                    "ATA: WRITE SECTORS lba={} num_sectors={}",
                    drive.get_lba(),
                    drive.controller.num_sectors
                );

                // Set DRQ — host will write sector data
                drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC | ATA_STATUS_DRQ;
                // No IRQ on initial write command (Bochs doesn't raise here)
            }
            ATA_CMD_WRITE_MULTIPLE => {
                // Bochs harddrv.cc:2304-2345 — WRITE MULTIPLE
                drive.lba48_transform();
                if drive.controller.multiple_sectors == 0 {
                    drive.controller.error = ATA_ERROR_ABRT;
                    drive.controller.status = ATA_STATUS_ERR | ATA_STATUS_DRDY;
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

                    drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC | ATA_STATUS_DRQ;
                }
            }
            ATA_CMD_READ_VERIFY => {
                // Just verify, no data transfer
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_SEEK => {
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_EXECUTE_DIAGNOSTICS => {
                drive.controller.error = 0x01; // No error
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_INITIALIZE_PARAMS => {
                // Initialize drive parameters
                let heads = (drive.controller.head_no & 0x0F) + 1;
                let spt = drive.controller.sector_count;
                tracing::debug!("ATA: Initialize params heads={} spt={}", heads, spt);
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_IDENTIFY => {
                tracing::debug!("ATA: IDENTIFY command");
                drive.fill_identify_buffer();
                drive.controller.status |= ATA_STATUS_DRQ;
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_SET_FEATURES => {
                // Accept but don't do anything special
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_SET_MULTIPLE => {
                drive.controller.multiple_sectors = drive.controller.sector_count;
                drive.controller.interrupt_pending = true;
            }
            _ => {
                tracing::warn!("ATA: Unknown command {:#04x}", command);
                drive.controller.error = ATA_ERROR_ABRT;
                drive.controller.status = ATA_STATUS_ERR | ATA_STATUS_DRDY;
            }
        }

        // Generate interrupt if pending and enabled
        if drive.controller.interrupt_pending && (drive.controller.control & 0x02) == 0 {
            if channel_num == 0 {
                self.irq14_pending = true;
            } else {
                self.irq15_pending = true;
            }
        }
    }

    /// Check and clear IRQ14 pending
    pub fn check_irq14(&mut self) -> bool {
        let pending = self.irq14_pending;
        self.irq14_pending = false;
        pending
    }

    /// Check and clear IRQ15 pending
    pub fn check_irq15(&mut self) -> bool {
        let pending = self.irq15_pending;
        self.irq15_pending = false;
        pending
    }
}

/// Hard drive read handler for I/O port infrastructure
pub fn harddrv_read_handler(this_ptr: *mut c_void, port: u16, io_len: u8) -> u32 {
    let hd = unsafe { &mut *(this_ptr as *mut BxHardDriveC) };
    hd.read(port, io_len)
}

/// Hard drive write handler for I/O port infrastructure
pub fn harddrv_write_handler(this_ptr: *mut c_void, port: u16, value: u32, io_len: u8) {
    let hd = unsafe { &mut *(this_ptr as *mut BxHardDriveC) };
    hd.write(port, value, io_len);
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

        // sector_count = 5 → num_sectors = 5
        drive.controller.sector_count = 5;
        drive.lba48_transform();
        assert_eq!(drive.controller.num_sectors, 5);

        // sector_count = 0 → num_sectors = 256
        drive.controller.sector_count = 0;
        drive.lba48_transform();
        assert_eq!(drive.controller.num_sectors, 256);
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
