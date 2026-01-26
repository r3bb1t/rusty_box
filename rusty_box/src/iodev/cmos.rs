//! CMOS RAM and Real Time Clock (RTC) Emulation
//!
//! The CMOS/RTC provides:
//! - 64 or 128 bytes of battery-backed CMOS RAM
//! - Real-time clock with date/time
//! - Alarm functionality
//! - Periodic interrupt generation
//!
//! I/O Ports:
//! - 0x70: CMOS address register (also controls NMI mask)
//! - 0x71: CMOS data register

use core::ffi::c_void;

/// CMOS I/O port addresses
pub const CMOS_ADDR: u16 = 0x0070;
pub const CMOS_DATA: u16 = 0x0071;

/// CMOS register definitions
pub const REG_SEC: u8 = 0x00;
pub const REG_SEC_ALARM: u8 = 0x01;
pub const REG_MIN: u8 = 0x02;
pub const REG_MIN_ALARM: u8 = 0x03;
pub const REG_HOUR: u8 = 0x04;
pub const REG_HOUR_ALARM: u8 = 0x05;
pub const REG_WEEK_DAY: u8 = 0x06;
pub const REG_MONTH_DAY: u8 = 0x07;
pub const REG_MONTH: u8 = 0x08;
pub const REG_YEAR: u8 = 0x09;
pub const REG_STAT_A: u8 = 0x0A;
pub const REG_STAT_B: u8 = 0x0B;
pub const REG_STAT_C: u8 = 0x0C;
pub const REG_STAT_D: u8 = 0x0D;
pub const REG_DIAGNOSTIC: u8 = 0x0E;
pub const REG_SHUTDOWN: u8 = 0x0F;
pub const REG_EQUIPMENT: u8 = 0x14;
pub const REG_CSUM_HIGH: u8 = 0x2E;
pub const REG_CSUM_LOW: u8 = 0x2F;
pub const REG_CENTURY: u8 = 0x32;

/// CMOS RAM size (standard is 64 or 128 bytes)
pub const CMOS_SIZE: usize = 128;

/// Convert BCD to binary
fn bcd_to_bin(value: u8, is_binary: bool) -> u8 {
    if is_binary {
        value
    } else {
        ((value >> 4) * 10) + (value & 0x0F)
    }
}

/// Convert binary to BCD
fn bin_to_bcd(value: u8, is_binary: bool) -> u8 {
    if is_binary {
        value
    } else {
        ((value / 10) << 4) | (value % 10)
    }
}

/// CMOS/RTC Controller
#[derive(Debug)]
pub struct BxCmosC {
    /// CMOS RAM contents
    pub ram: [u8; CMOS_SIZE],
    /// Current address register
    pub address: u8,
    /// NMI mask (bit 7 of address port)
    pub nmi_mask: bool,
    /// Periodic interrupt rate
    pub periodic_rate: u8,
    /// IRQ8 pending
    pub irq8_pending: bool,
    /// Time of last update
    last_update_time: u64,
}

impl Default for BxCmosC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxCmosC {
    /// Create a new CMOS/RTC controller
    pub fn new() -> Self {
        let mut cmos = Self {
            ram: [0; CMOS_SIZE],
            address: 0,
            nmi_mask: false,
            periodic_rate: 0,
            irq8_pending: false,
            last_update_time: 0,
        };
        cmos.init_defaults();
        cmos
    }

    /// Initialize default CMOS values
    fn init_defaults(&mut self) {
        // Status Register A: 32.768kHz timebase, no periodic interrupt
        self.ram[REG_STAT_A as usize] = 0x26;
        
        // Status Register B: 24-hour mode, binary mode, DST disabled
        self.ram[REG_STAT_B as usize] = 0x02;
        
        // Status Register C: Clear all interrupt flags
        self.ram[REG_STAT_C as usize] = 0x00;
        
        // Status Register D: RTC valid, battery good
        self.ram[REG_STAT_D as usize] = 0x80;
        
        // Equipment byte: 2 floppy drives, VGA display
        self.ram[REG_EQUIPMENT as usize] = 0x41;
        
        // Set a default time (2025-01-01 12:00:00)
        self.set_time(0, 0, 12, 1, 1, 25);
        self.ram[REG_CENTURY as usize] = 0x20; // 20xx
        self.ram[REG_WEEK_DAY as usize] = 3; // Wednesday
        
        // Base memory: 640KB
        self.ram[0x15] = 0x80;
        self.ram[0x16] = 0x02;
        
        // Extended memory above 1MB: 31MB (32MB total - 1MB)
        self.ram[0x17] = 0x00;
        self.ram[0x18] = 0x7C; // 31*1024 = 31744 = 0x7C00
        self.ram[0x30] = 0x00;
        self.ram[0x31] = 0x7C;
        
        // Update CMOS checksum
        self.update_checksum();
    }

    /// Initialize the CMOS/RTC
    pub fn init(&mut self) {
        tracing::info!("CMOS: Initializing CMOS/RTC");
        self.init_defaults();
    }

    /// Reset the CMOS/RTC
    pub fn reset(&mut self) {
        self.address = 0;
        self.nmi_mask = false;
        self.irq8_pending = false;
        
        // Clear interrupt flags
        self.ram[REG_STAT_C as usize] = 0x00;
    }

    /// Set the current time
    pub fn set_time(&mut self, sec: u8, min: u8, hour: u8, day: u8, month: u8, year: u8) {
        let is_binary = (self.ram[REG_STAT_B as usize] & 0x04) != 0;
        
        self.ram[REG_SEC as usize] = bin_to_bcd(sec, is_binary);
        self.ram[REG_MIN as usize] = bin_to_bcd(min, is_binary);
        self.ram[REG_HOUR as usize] = bin_to_bcd(hour, is_binary);
        self.ram[REG_MONTH_DAY as usize] = bin_to_bcd(day, is_binary);
        self.ram[REG_MONTH as usize] = bin_to_bcd(month, is_binary);
        self.ram[REG_YEAR as usize] = bin_to_bcd(year, is_binary);
    }

    /// Update the CMOS checksum
    fn update_checksum(&mut self) {
        let mut sum: u16 = 0;
        for i in 0x10..0x2E {
            sum = sum.wrapping_add(self.ram[i] as u16);
        }
        self.ram[REG_CSUM_HIGH as usize] = (sum >> 8) as u8;
        self.ram[REG_CSUM_LOW as usize] = (sum & 0xFF) as u8;
    }

    /// Read from CMOS I/O port
    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        match port {
            CMOS_ADDR => self.address as u32,
            CMOS_DATA => {
                let addr = (self.address & 0x7F) as usize;
                let value = match addr as u8 {
                    REG_STAT_C => {
                        // Reading Status C clears interrupt flags
                        let val = self.ram[addr];
                        self.ram[addr] = 0;
                        self.irq8_pending = false;
                        val
                    }
                    REG_SHUTDOWN => {
                        // Debug: log shutdown status read
                        let val = self.ram[addr];
                        tracing::info!("CMOS: Read shutdown status [{:#04x}] = {:#04x}", addr, val);
                        val
                    }
                    _ => {
                        if addr < CMOS_SIZE {
                            self.ram[addr]
                        } else {
                            0xFF
                        }
                    }
                };
                tracing::trace!("CMOS: Read [{:#04x}] = {:#04x}", addr, value);
                value as u32
            }
            _ => {
                tracing::warn!("CMOS: Unknown read port {:#06x}", port);
                0xFF
            }
        }
    }

    /// Write to CMOS I/O port
    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let value = value as u8;
        match port {
            CMOS_ADDR => {
                self.nmi_mask = (value & 0x80) != 0;
                self.address = value & 0x7F;
            }
            CMOS_DATA => {
                let addr = (self.address & 0x7F) as usize;
                tracing::trace!("CMOS: Write [{:#04x}] = {:#04x}", addr, value);
                
                match addr as u8 {
                    REG_STAT_A => {
                        // Bits 0-6 are writable, bit 7 (UIP) is read-only
                        self.ram[addr] = (self.ram[addr] & 0x80) | (value & 0x7F);
                        self.periodic_rate = value & 0x0F;
                    }
                    REG_STAT_B => {
                        self.ram[addr] = value;
                    }
                    REG_STAT_C | REG_STAT_D => {
                        // Read-only registers
                    }
                    _ => {
                        if addr < CMOS_SIZE {
                            self.ram[addr] = value;
                            // Update checksum if we wrote to checksum-covered area
                            if (0x10..0x2E).contains(&addr) {
                                self.update_checksum();
                            }
                        }
                    }
                }
            }
            _ => {
                tracing::warn!("CMOS: Unknown write port {:#06x} value={:#04x}", port, value);
            }
        }
    }

    /// Configure memory size in CMOS
    pub fn set_memory_size(&mut self, base_kb: u16, extended_kb: u16) {
        // Base memory (in KB) - typically 640
        self.ram[0x15] = (base_kb & 0xFF) as u8;
        self.ram[0x16] = ((base_kb >> 8) & 0xFF) as u8;
        
        // Extended memory above 1MB (in KB)
        self.ram[0x17] = (extended_kb & 0xFF) as u8;
        self.ram[0x18] = ((extended_kb >> 8) & 0xFF) as u8;
        self.ram[0x30] = (extended_kb & 0xFF) as u8;
        self.ram[0x31] = ((extended_kb >> 8) & 0xFF) as u8;
        
        self.update_checksum();
    }

    /// Configure hard drive in CMOS
    pub fn set_hard_drive(&mut self, drive_num: u8, drive_type: u8) {
        if drive_num == 0 {
            // Drive 0 in high nibble
            self.ram[0x12] = (self.ram[0x12] & 0x0F) | (drive_type << 4);
        } else if drive_num == 1 {
            // Drive 1 in low nibble
            self.ram[0x12] = (self.ram[0x12] & 0xF0) | (drive_type & 0x0F);
        }
        self.update_checksum();
    }

    /// Configure boot device
    pub fn set_boot_device(&mut self, first_boot: u8) {
        // Boot sequence: 0=floppy, 1=hard disk, 2=CD-ROM
        self.ram[0x2D] = first_boot;
        self.update_checksum();
    }

    /// Simulate time passing (in microseconds)
    pub fn tick(&mut self, _usec: u64) -> bool {
        // In a full implementation, we would update the RTC time
        // and potentially trigger periodic/alarm interrupts
        false
    }

    /// Check and clear IRQ8 pending flag
    pub fn check_irq8(&mut self) -> bool {
        let pending = self.irq8_pending;
        self.irq8_pending = false;
        pending
    }
}

/// CMOS read handler for I/O port infrastructure
pub fn cmos_read_handler(this_ptr: *mut c_void, port: u16, io_len: u8) -> u32 {
    let cmos = unsafe { &mut *(this_ptr as *mut BxCmosC) };
    cmos.read(port, io_len)
}

/// CMOS write handler for I/O port infrastructure
pub fn cmos_write_handler(this_ptr: *mut c_void, port: u16, value: u32, io_len: u8) {
    let cmos = unsafe { &mut *(this_ptr as *mut BxCmosC) };
    cmos.write(port, value, io_len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmos_creation() {
        let cmos = BxCmosC::new();
        // Status D should indicate battery OK
        assert_eq!(cmos.ram[REG_STAT_D as usize] & 0x80, 0x80);
    }

    #[test]
    fn test_cmos_address() {
        let mut cmos = BxCmosC::new();
        
        // Write address with NMI mask
        cmos.write(CMOS_ADDR, 0x8A, 1); // Address 0x0A with NMI mask
        assert!(cmos.nmi_mask);
        assert_eq!(cmos.address, 0x0A);
        
        // Read Status A
        let value = cmos.read(CMOS_DATA, 1);
        assert_eq!(value, cmos.ram[REG_STAT_A as usize] as u32);
    }

    #[test]
    fn test_cmos_memory_config() {
        let mut cmos = BxCmosC::new();
        
        // Set 32MB total memory
        cmos.set_memory_size(640, 31744); // 640KB base + 31MB extended
        
        assert_eq!(cmos.ram[0x15], 0x80); // 640 low byte
        assert_eq!(cmos.ram[0x16], 0x02); // 640 high byte
    }
}

