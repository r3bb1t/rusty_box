//! ATA/IDE Hard Drive Controller Emulation
//!
//! Implements the ATA (AT Attachment) interface for hard drives.
//! Supports basic PIO mode operations for reading/writing sectors.
//!
//! Primary ATA: ports 0x1F0-0x1F7, 0x3F6 (IRQ14)
//! Secondary ATA: ports 0x170-0x177, 0x376 (IRQ15)

use alloc::vec;
use alloc::vec::Vec;
use alloc::string::String;
#[cfg(feature = "std")]
use alloc::format;
use core::ffi::c_void;

#[cfg(feature = "std")]
use std::fs::File;
#[cfg(feature = "std")]
use std::io::{Read, Seek, SeekFrom, Write};

/// Sector size in bytes
pub const SECTOR_SIZE: usize = 512;

/// ATA I/O port offsets (from base address)
pub const ATA_DATA: u16 = 0;          // Data register (R/W)
pub const ATA_ERROR: u16 = 1;         // Error register (R) / Features (W)
pub const ATA_SECTOR_COUNT: u16 = 2;  // Sector count
pub const ATA_SECTOR_NUM: u16 = 3;    // Sector number / LBA low
pub const ATA_CYL_LOW: u16 = 4;       // Cylinder low / LBA mid
pub const ATA_CYL_HIGH: u16 = 5;      // Cylinder high / LBA high
pub const ATA_DRIVE_HEAD: u16 = 6;    // Drive/Head / LBA top 4 bits
pub const ATA_STATUS: u16 = 7;        // Status (R) / Command (W)
pub const ATA_ALT_STATUS: u16 = 0x206; // Alternate status / Device control

/// Status register bits
pub const ATA_STATUS_ERR: u8 = 0x01;  // Error
pub const ATA_STATUS_IDX: u8 = 0x02;  // Index (always 0)
pub const ATA_STATUS_CORR: u8 = 0x04; // Corrected data (always 0)
pub const ATA_STATUS_DRQ: u8 = 0x08;  // Data request
pub const ATA_STATUS_DSC: u8 = 0x10;  // Drive seek complete
pub const ATA_STATUS_DWF: u8 = 0x20;  // Drive write fault
pub const ATA_STATUS_DRDY: u8 = 0x40; // Drive ready
pub const ATA_STATUS_BSY: u8 = 0x80;  // Busy

/// Error register bits
pub const ATA_ERROR_AMNF: u8 = 0x01;  // Address mark not found
pub const ATA_ERROR_TK0NF: u8 = 0x02; // Track 0 not found
pub const ATA_ERROR_ABRT: u8 = 0x04;  // Command aborted
pub const ATA_ERROR_IDNF: u8 = 0x10;  // ID not found
pub const ATA_ERROR_UNC: u8 = 0x40;   // Uncorrectable data error

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
    pub cylinders: u16,
    pub heads: u8,
    pub sectors_per_track: u8,
    pub total_sectors: u32,
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

/// Controller state for one drive
#[derive(Debug)]
pub struct AtaController {
    /// Status register
    pub status: u8,
    /// Error register
    pub error: u8,
    /// Features register
    pub features: u8,
    /// Sector count
    pub sector_count: u8,
    /// Sector number (LBA bits 0-7)
    pub sector_no: u8,
    /// Cylinder number
    pub cylinder_no: u16,
    /// Head number
    pub head_no: u8,
    /// LBA mode enabled
    pub lba_mode: bool,
    /// Device control register
    pub control: u8,
    /// Interrupt pending
    pub interrupt_pending: bool,
    /// Current command
    pub current_command: u8,
    /// Multiple sector count
    pub multiple_sectors: u8,
    /// Data buffer
    pub buffer: Vec<u8>,
    /// Buffer index
    pub buffer_index: usize,
    /// Buffer size (bytes to transfer)
    pub buffer_size: usize,
    /// Reset in progress
    pub reset_in_progress: bool,
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
            reset_in_progress: false,
        }
    }
}

/// ATA Drive
#[derive(Debug)]
pub struct AtaDrive {
    /// Device type
    pub device_type: DeviceType,
    /// Drive geometry
    pub geometry: DriveGeometry,
    /// Model name
    pub model: String,
    /// Serial number
    pub serial: String,
    /// Firmware revision
    pub firmware: String,
    /// Controller state
    pub controller: AtaController,
    /// Image file path
    pub image_path: Option<String>,
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
        let file = File::options()
            .read(true)
            .write(true)
            .open(path)?;
        
        let size = file.metadata()?.len() as u32;
        self.geometry.total_sectors = size / SECTOR_SIZE as u32;
        
        tracing::info!(
            "ATA: Attached image '{}' ({} sectors, {} MB)",
            path, self.geometry.total_sectors,
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

    /// Read sectors from disk (std version)
    #[cfg(feature = "std")]
    fn read_sectors(&mut self, lba: u32, count: u8) -> Result<(), String> {
        if self.device_type != DeviceType::Disk {
            return Err(String::from("Not a disk device"));
        }

        let file = self.image_file.as_mut()
            .ok_or_else(|| String::from("No image attached"))?;
        
        let offset = lba as u64 * SECTOR_SIZE as u64;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("Seek failed: {}", e))?;
        
        let bytes_to_read = count as usize * SECTOR_SIZE;
        self.controller.buffer_size = bytes_to_read;
        self.controller.buffer_index = 0;
        
        file.read_exact(&mut self.controller.buffer[..bytes_to_read])
            .map_err(|e| format!("Read failed: {}", e))?;
        
        tracing::debug!("ATA: Read {} sectors from LBA {} ({} bytes), first8=[{:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}]",
            count, lba, bytes_to_read,
            self.controller.buffer[0], self.controller.buffer[1],
            self.controller.buffer[2], self.controller.buffer[3],
            self.controller.buffer[4], self.controller.buffer[5],
            self.controller.buffer[6], self.controller.buffer[7]);
        Ok(())
    }

    /// Read sectors from disk (no_std version)
    #[cfg(not(feature = "std"))]
    fn read_sectors(&mut self, lba: u32, count: u8) -> Result<(), String> {
        if self.device_type != DeviceType::Disk {
            return Err(String::from("Not a disk device"));
        }

        let data = self.disk_data.as_ref()
            .ok_or_else(|| String::from("No disk data attached"))?;
        
        let offset = lba as usize * SECTOR_SIZE;
        let bytes_to_read = count as usize * SECTOR_SIZE;
        
        if offset + bytes_to_read > data.len() {
            return Err(String::from("Read beyond disk size"));
        }
        
        self.controller.buffer_size = bytes_to_read;
        self.controller.buffer_index = 0;
        self.controller.buffer[..bytes_to_read].copy_from_slice(&data[offset..offset + bytes_to_read]);
        
        tracing::trace!("ATA: Read {} sectors from LBA {}", count, lba);
        Ok(())
    }

    /// Write sectors to disk (std version)
    #[cfg(feature = "std")]
    fn write_sectors(&mut self, lba: u32, count: u8) -> Result<(), String> {
        if self.device_type != DeviceType::Disk {
            return Err(String::from("Not a disk device"));
        }

        let file = self.image_file.as_mut()
            .ok_or_else(|| String::from("No image attached"))?;
        
        let offset = lba as u64 * SECTOR_SIZE as u64;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| format!("Seek failed: {}", e))?;
        
        let bytes_to_write = count as usize * SECTOR_SIZE;
        file.write_all(&self.controller.buffer[..bytes_to_write])
            .map_err(|e| format!("Write failed: {}", e))?;
        
        file.flush()
            .map_err(|e| format!("Flush failed: {}", e))?;
        
        tracing::trace!("ATA: Wrote {} sectors to LBA {}", count, lba);
        Ok(())
    }

    /// Write sectors to disk (no_std version)
    #[cfg(not(feature = "std"))]
    fn write_sectors(&mut self, lba: u32, count: u8) -> Result<(), String> {
        if self.device_type != DeviceType::Disk {
            return Err(String::from("Not a disk device"));
        }

        let data = self.disk_data.as_mut()
            .ok_or_else(|| String::from("No disk data attached"))?;
        
        let offset = lba as usize * SECTOR_SIZE;
        let bytes_to_write = count as usize * SECTOR_SIZE;
        
        if offset + bytes_to_write > data.len() {
            return Err(String::from("Write beyond disk size"));
        }
        
        data[offset..offset + bytes_to_write].copy_from_slice(&self.controller.buffer[..bytes_to_write]);
        
        tracing::trace!("ATA: Wrote {} sectors to LBA {}", count, lba);
        Ok(())
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
    pub ioaddr1: u16,
    /// Control I/O address  
    pub ioaddr2: u16,
    /// IRQ number
    pub irq: u8,
    /// Master and slave drives
    pub drives: [AtaDrive; 2],
    /// Currently selected drive (0=master, 1=slave)
    pub drive_select: u8,
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
    pub channels: [AtaChannel; 2],
    /// IRQ14 pending (primary)
    pub irq14_pending: bool,
    /// IRQ15 pending (secondary)
    pub irq15_pending: bool,
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
    pub fn attach_disk(&mut self, channel: usize, drive: usize, path: &str, 
                       cylinders: u16, heads: u8, spt: u8) -> std::io::Result<()> {
        let geometry = DriveGeometry::from_chs(cylinders, heads, spt);
        self.channels[channel].drives[drive] = AtaDrive::create_disk(geometry);
        self.channels[channel].drives[drive].attach_image(path)?;
        Ok(())
    }

    /// Attach disk data to a drive (for no_std environments)
    #[cfg(not(feature = "std"))]
    pub fn attach_disk_data(&mut self, channel: usize, drive: usize, data: Vec<u8>,
                            cylinders: u16, heads: u8, spt: u8) {
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

    /// Read from ATA I/O port
    pub fn read(&mut self, port: u16, io_len: u8) -> u32 {
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
                let idx = drive.controller.buffer_index;
                let bytes = io_len as usize; // 1, 2, or 4
                if idx + bytes <= drive.controller.buffer_size {
                    let mut value: u32 = 0;
                    for b in 0..bytes {
                        value |= (drive.controller.buffer[idx + b] as u32) << (b * 8);
                    }
                    drive.controller.buffer_index += bytes;

                    // Check if transfer complete
                    if drive.controller.buffer_index >= drive.controller.buffer_size {
                        drive.controller.status &= !ATA_STATUS_DRQ;
                    }

                    return value;
                }
                0
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
                // Reading status clears interrupt
                if offset == ATA_STATUS {
                    drive.controller.interrupt_pending = false;
                    if channel_num == 0 {
                        self.irq14_pending = false;
                    } else {
                        self.irq15_pending = false;
                    }
                }
                tracing::trace!("ATA: Status read = {:#04x} (port={:#06x})", drive.controller.status, port);
                drive.controller.status as u32
            }
            _ => 0xFF,
        }
    }

    /// Write to ATA I/O port
    pub fn write(&mut self, port: u16, value: u32, io_len: u8) {
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
                let drive = channel.selected_drive_mut();
                if drive.device_type == DeviceType::None {
                    return;
                }

                let bytes = io_len as usize; // 1, 2, or 4
                let idx = drive.controller.buffer_index;
                if idx + bytes <= drive.controller.buffer.len() {
                    for b in 0..bytes {
                        drive.controller.buffer[idx + b] = ((value >> (b * 8)) & 0xFF) as u8;
                    }
                    drive.controller.buffer_index += bytes;

                    // Check if transfer complete
                    if drive.controller.buffer_index >= drive.controller.buffer_size {
                        // Execute write command
                        let lba = drive.get_lba();
                        let count = if drive.controller.sector_count == 0 { 256u16 } else { drive.controller.sector_count as u16 };

                        if let Err(e) = drive.write_sectors(lba, count as u8) {
                            tracing::error!("ATA: Write failed: {}", e);
                            drive.controller.error = ATA_ERROR_ABRT;
                            drive.controller.status = ATA_STATUS_ERR | ATA_STATUS_DRDY;
                        } else {
                            drive.controller.status = ATA_STATUS_DRDY | ATA_STATUS_DSC;
                        }

                        drive.controller.status &= !ATA_STATUS_DRQ;
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

    /// Execute an ATA command
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

        tracing::debug!("ATA: Command {:#04x} drive={} scount={} sno={} cyl={} head={} lba_mode={}",
            command, ds,
            drive.controller.sector_count, drive.controller.sector_no,
            drive.controller.cylinder_no, drive.controller.head_no,
            drive.controller.lba_mode);

        match command {
            ATA_CMD_RECALIBRATE => {
                drive.controller.cylinder_no = 0;
                drive.controller.interrupt_pending = true;
            }
            ATA_CMD_READ_SECTORS | ATA_CMD_READ_SECTORS_EXT => {
                let lba = drive.get_lba();
                let count = if drive.controller.sector_count == 0 { 256u16 } else { drive.controller.sector_count as u16 };

                tracing::debug!("ATA: READ LBA={} count={}", lba, count);

                if let Err(e) = drive.read_sectors(lba, count as u8) {
                    tracing::error!("ATA: Read failed: {}", e);
                    drive.controller.error = ATA_ERROR_ABRT;
                    drive.controller.status = ATA_STATUS_ERR | ATA_STATUS_DRDY;
                } else {
                    drive.controller.status |= ATA_STATUS_DRQ;
                    drive.controller.interrupt_pending = true;
                }
            }
            ATA_CMD_WRITE_SECTORS | ATA_CMD_WRITE_SECTORS_EXT => {
                let count = if drive.controller.sector_count == 0 { 256u16 } else { drive.controller.sector_count as u16 };
                
                drive.controller.buffer_size = count as usize * SECTOR_SIZE;
                drive.controller.buffer_index = 0;
                drive.controller.status |= ATA_STATUS_DRQ;
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
        
        // Generate interrupt if enabled
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
}

