//! CMOS RAM and Real Time Clock (RTC) Emulation
//!
//! Ported from Bochs `iodev/cmos.cc`.
//!
//! The CMOS/RTC provides:
//! - 64 or 128 bytes of battery-backed CMOS RAM
//! - Real-time clock with date/time (one-second timer)
//! - Periodic interrupt generation (programmable rate from REG_STAT_A)
//! - Update-In-Progress (UIP) 244μs one-shot timer
//! - Alarm functionality
//!
//! ## Timer Architecture (matching Bochs cmos.cc)
//!
//! Three timers drive the RTC:
//!
//! 1. **Periodic timer**: Fires at a programmable rate derived from REG_STAT_A[3:0].
//!    When REG_STAT_B bit 6 (PIE) is set, each fire sets REG_STAT_C bits 7+6
//!    and raises IRQ8.
//!
//! 2. **One-second timer**: Fires every 1,000,000 μs. Increments internal `timeval`
//!    (Unix timestamp). If REG_STAT_B bit 7 (SET) is clear, sets UIP bit in
//!    REG_STAT_A and triggers the 244μs UIP timer.
//!
//! 3. **UIP timer**: 244μs one-shot. When it fires, clears UIP bit, calls
//!    `update_clock()` to copy `timeval` into CMOS date/time registers, and
//!    checks for alarm match (setting REG_STAT_C bits 7+5 and raising IRQ8
//!    if REG_STAT_B bit 5 is set).
//!
//! I/O Ports:
//! - 0x70: CMOS address register (write-only on most machines; reads return 0xFF)
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

/// CMOS RAM size (256 bytes: standard 128 + extended 128 via ports 0x72/0x73)
pub const CMOS_SIZE: usize = 256;

/// Days per month (non-leap year)
const DAYS_IN_MONTH: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

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

/// Check if year is a leap year
fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// CMOS/RTC Controller (matching Bochs cmos.cc structure)
#[derive(Debug)]
pub struct BxCmosC {
    /// CMOS RAM contents
    pub(crate) ram: [u8; CMOS_SIZE],
    /// Current address register (set by port 0x70 write)
    pub(crate) address: u8,
    /// NMI mask (bit 7 of address port)
    pub(crate) nmi_mask: bool,

    // --- Timer state (Bochs cmos.cc timer architecture) ---
    /// Internal Unix timestamp (seconds since epoch). Incremented by one_second_timer.
    timeval: u64,
    /// Periodic interrupt interval in microseconds (from CRA_change).
    /// u32::MAX means disabled.
    periodic_interval_usec: u32,
    /// Microseconds remaining until next periodic timer fire.
    /// 0 means timer is not active.
    periodic_timer_remaining: u32,
    /// Microseconds remaining until next one-second timer fire.
    one_second_remaining: u32,
    /// Microseconds remaining until UIP timer fires (244μs one-shot).
    /// 0 means not active.
    uip_timer_remaining: u32,
    /// Whether timeval was changed while in SET mode (REG_STAT_B bit 7).
    /// When SET mode is exited, update_timeval() is called.
    timeval_change: bool,

    // --- IRQ state ---
    /// IRQ8 enabled (controls whether PIC is signaled)
    pub(crate) irq_enabled: bool,
    /// IRQ8 raise pending — set by periodic/alarm timer, consumed by tick_devices
    pub(crate) irq8_pending: bool,
    /// IRQ8 lower pending — set by REG_STAT_C read, consumed by tick_devices
    pub(crate) irq8_lower_pending: bool,
    /// Extended CMOS address register (port 0x0072 write).
    /// Bit 7 is forced on so addresses 0x80-0xFF are accessible.
    /// Matches Bochs cmos.cc `cmos_ext_mem_addr`.
    cmos_ext_mem_addr: u8,
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
            timeval: 0,
            periodic_interval_usec: u32::MAX,
            periodic_timer_remaining: 0,
            one_second_remaining: 1_000_000,
            uip_timer_remaining: 0,
            timeval_change: false,
            irq_enabled: true,
            irq8_pending: false,
            irq8_lower_pending: false,
            cmos_ext_mem_addr: 0x80,
        };
        cmos.init_defaults();
        cmos
    }

    /// Initialize default CMOS values
    fn init_defaults(&mut self) {
        // Status Register A: 32.768kHz timebase, default periodic rate
        // 0x26 = divider=010 (32.768kHz), rate=0110 (1024 Hz = ~976μs)
        self.ram[REG_STAT_A as usize] = 0x26;

        // Status Register B: 24-hour mode, binary mode, DST disabled
        // Bit 1 = 24-hour mode, bit 2 = binary (not BCD)
        self.ram[REG_STAT_B as usize] = 0x02;

        // Status Register C: Clear all interrupt flags
        self.ram[REG_STAT_C as usize] = 0x00;

        // Status Register D: RTC valid, battery good
        self.ram[REG_STAT_D as usize] = 0x80;

        // Equipment byte — built up the same way Bochs does it:
        //   cmos.cc init():   |= 0x02 (FPU present)
        //   keyboard.cc init(): |= 0x04 (mouse port on system board)
        //   vgacore.cc init_standard_vga(): &= 0xcf | 0x00 (EGA/VGA display)
        //   No floppy controller: bits 0,6-7 stay 0
        // Final: 0x06 = FPU + mouse port, no floppy, EGA/VGA
        self.ram[REG_EQUIPMENT as usize] = 0x06;

        // Use current system time when available, else fall back to 2025-01-01 12:00:00
        #[cfg(feature = "std")]
        {
            use std::time::{SystemTime, UNIX_EPOCH};
            self.timeval = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(1_735_732_800);
        }
        #[cfg(not(feature = "std"))]
        {
            self.timeval = 1_735_732_800;
        }
        self.update_clock();
        // Century and weekday are computed by update_clock() from timeval

        // Base memory: 640KB
        self.ram[0x15] = 0x80;
        self.ram[0x16] = 0x02;

        // Extended memory above 1MB: 31MB (32MB total - 1MB)
        self.ram[0x17] = 0x00;
        self.ram[0x18] = 0x7C; // 31*1024 = 31744 = 0x7C00
        self.ram[0x30] = 0x00;
        self.ram[0x31] = 0x7C;

        // Calculate initial periodic interval
        self.cra_change();

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
        self.irq8_lower_pending = false;

        // Clear interrupt flags
        self.ram[REG_STAT_C as usize] = 0x00;
    }

    // =========================================================================
    // Timer handlers (matching Bochs cmos.cc)
    // =========================================================================

    /// Recalculate periodic interval from REG_STAT_A (Bochs cmos.cc:342-367 CRA_change)
    fn cra_change(&mut self) {
        let nibble = self.ram[REG_STAT_A as usize] & 0x0F;
        let dcc = (self.ram[REG_STAT_A as usize] >> 4) & 0x07;

        if nibble == 0 || (dcc & 0x06) == 0 {
            // No periodic interrupt rate — deactivate timer
            self.periodic_timer_remaining = 0;
            self.periodic_interval_usec = u32::MAX;
        } else {
            // Values 0001b and 0010b are the same as 1000b and 1001b
            let effective_nibble = if nibble <= 2 { nibble + 7 } else { nibble };
            // Formula: 1_000_000 / (32768 / 2^(nibble-1))
            self.periodic_interval_usec =
                (1_000_000.0f64 / (32768.0f64 / ((1u32 << (effective_nibble - 1)) as f64))) as u32;

            // If Periodic Interrupt Enable bit set, activate timer
            if self.ram[REG_STAT_B as usize] & 0x40 != 0 {
                if self.periodic_timer_remaining == 0 {
                    self.periodic_timer_remaining = self.periodic_interval_usec;
                }
            } else {
                self.periodic_timer_remaining = 0;
            }
        }
    }

    /// Periodic timer handler (Bochs cmos.cc:696-712 periodic_timer)
    fn periodic_timer(&mut self) {
        // If periodic interrupts are enabled, trip IRQ8 and update status C
        if self.ram[REG_STAT_B as usize] & 0x40 != 0 {
            self.ram[REG_STAT_C as usize] |= 0xC0; // IRQF + PF (bits 7,6)
            if self.irq_enabled {
                self.irq8_pending = true;
            }
        }
    }

    /// One-second timer handler (Bochs cmos.cc:714-738 one_second_timer)
    fn one_second_timer(&mut self) {
        // Divider chain reset — RTC stopped
        if (self.ram[REG_STAT_A as usize] & 0x60) == 0x60 {
            return;
        }

        // Update internal time/date buffer
        self.timeval += 1;

        // Don't update CMOS user copy of time/date if CRB bit 7 (SET) is 1
        if self.ram[REG_STAT_B as usize] & 0x80 != 0 {
            return;
        }

        // Set UIP (Update In Progress) bit
        self.ram[REG_STAT_A as usize] |= 0x80;

        // Schedule UIP timer for 244μs
        self.uip_timer_remaining = 244;
    }

    /// UIP timer handler (Bochs cmos.cc:740-786 uip_timer)
    fn uip_timer(&mut self) {
        // Clear UIP bit
        self.ram[REG_STAT_A as usize] &= !0x80;

        // Update CMOS registers from timeval
        self.update_clock();

        // Set Update-Ended flag (UF, bit 4 of Status C)
        self.ram[REG_STAT_C as usize] |= 0x10;

        // If Update-Ended Interrupt Enable (UIE, bit 4 of Status B) is set
        if self.ram[REG_STAT_B as usize] & 0x10 != 0 {
            self.ram[REG_STAT_C as usize] |= 0x80; // Set IRQF
            if self.irq_enabled {
                self.irq8_pending = true;
            }
        }

        // Check alarm match
        self.check_alarm();
    }

    /// Check if current time matches alarm registers (Bochs cmos.cc:770-786)
    fn check_alarm(&mut self) {
        let _is_binary = (self.ram[REG_STAT_B as usize] & 0x04) != 0;

        // Alarm registers: "don't care" values (0xC0-0xFF in BCD, or >= 0xC0 in binary)
        let sec_match = self.ram[REG_SEC_ALARM as usize] >= 0xC0
            || self.ram[REG_SEC_ALARM as usize] == self.ram[REG_SEC as usize];
        let min_match = self.ram[REG_MIN_ALARM as usize] >= 0xC0
            || self.ram[REG_MIN_ALARM as usize] == self.ram[REG_MIN as usize];
        let hour_match = self.ram[REG_HOUR_ALARM as usize] >= 0xC0
            || self.ram[REG_HOUR_ALARM as usize] == self.ram[REG_HOUR as usize];

        if sec_match && min_match && hour_match {
            // Set Alarm Flag (AF, bit 5 of Status C)
            self.ram[REG_STAT_C as usize] |= 0x20;

            // If Alarm Interrupt Enable (AIE, bit 5 of Status B) is set
            if self.ram[REG_STAT_B as usize] & 0x20 != 0 {
                self.ram[REG_STAT_C as usize] |= 0x80; // Set IRQF
                if self.irq_enabled {
                    self.irq8_pending = true;
                }
            }
        }
    }

    /// Update CMOS date/time registers from internal timeval
    /// (Bochs cmos.cc:788-846 update_clock)
    fn update_clock(&mut self) {
        let is_binary = (self.ram[REG_STAT_B as usize] & 0x04) != 0;
        let is_24hour = (self.ram[REG_STAT_B as usize] & 0x02) != 0;

        // Convert Unix timestamp to date components
        let mut remaining = self.timeval;

        // Seconds of day
        let total_seconds_today = (remaining % 86400) as u32;
        remaining /= 86400;

        let sec = (total_seconds_today % 60) as u8;
        let min = ((total_seconds_today / 60) % 60) as u8;
        let mut hour = (total_seconds_today / 3600) as u8;

        // Days since Unix epoch (1970-01-01)
        let mut days = remaining as u32;

        // Day of week (1970-01-01 was Thursday=5 in 1-based Sun=1 convention)
        let wday = ((days + 4) % 7) + 1; // 1=Sunday

        // Calculate year
        let mut year: u32 = 1970;
        loop {
            let days_in_year = if is_leap_year(year) { 366 } else { 365 };
            if days < days_in_year {
                break;
            }
            days -= days_in_year;
            year += 1;
        }

        // Calculate month and day
        let mut month: u8 = 1;
        for m in 0..12 {
            let mut dim = DAYS_IN_MONTH[m] as u32;
            if m == 1 && is_leap_year(year) {
                dim += 1;
            }
            if days < dim {
                break;
            }
            days -= dim;
            month += 1;
        }
        let mday = days as u8 + 1;

        // Handle 12-hour format
        if !is_24hour {
            if hour == 0 {
                hour = 12; // 12 AM
            } else if hour > 12 {
                hour -= 12;
                // In BCD 12-hour mode, bit 7 of hour = PM flag
                // We'll set it after BCD conversion
            }
        }

        // Store in CMOS registers
        self.ram[REG_SEC as usize] = bin_to_bcd(sec, is_binary);
        self.ram[REG_MIN as usize] = bin_to_bcd(min, is_binary);

        let pm = if !is_24hour && (total_seconds_today / 3600) >= 12 {
            0x80
        } else {
            0
        };
        let raw_hour = if !is_24hour {
            let h = (total_seconds_today / 3600) as u8;
            if h == 0 {
                12
            } else if h > 12 {
                h - 12
            } else {
                h
            }
        } else {
            hour
        };
        self.ram[REG_HOUR as usize] = bin_to_bcd(raw_hour, is_binary) | pm;

        self.ram[REG_WEEK_DAY as usize] = bin_to_bcd(wday as u8, is_binary);
        self.ram[REG_MONTH_DAY as usize] = bin_to_bcd(mday, is_binary);
        self.ram[REG_MONTH as usize] = bin_to_bcd(month, is_binary);
        self.ram[REG_YEAR as usize] = bin_to_bcd((year % 100) as u8, is_binary);
        self.ram[REG_CENTURY as usize] = bin_to_bcd((year / 100) as u8, is_binary);
    }

    /// Convert CMOS date/time registers back to timeval
    /// Called when exiting SET mode (Bochs cmos.cc:848-895 update_timeval)
    fn update_timeval(&mut self) {
        let is_binary = (self.ram[REG_STAT_B as usize] & 0x04) != 0;
        let is_24hour = (self.ram[REG_STAT_B as usize] & 0x02) != 0;

        let sec = bcd_to_bin(self.ram[REG_SEC as usize], is_binary) as u64;
        let min = bcd_to_bin(self.ram[REG_MIN as usize], is_binary) as u64;

        let hour_raw = self.ram[REG_HOUR as usize];
        let pm = (hour_raw & 0x80) != 0;
        let mut hour = bcd_to_bin(hour_raw & 0x7F, is_binary) as u64;
        if !is_24hour {
            if pm && hour < 12 {
                hour += 12;
            } else if !pm && hour == 12 {
                hour = 0;
            }
        }

        let mday = bcd_to_bin(self.ram[REG_MONTH_DAY as usize], is_binary) as u64;
        let month = bcd_to_bin(self.ram[REG_MONTH as usize], is_binary) as u64;
        let year_2digit = bcd_to_bin(self.ram[REG_YEAR as usize], is_binary) as u64;
        let century = bcd_to_bin(self.ram[REG_CENTURY as usize], is_binary) as u64;
        let year = century * 100 + year_2digit;

        // Convert to days since epoch
        let mut days: u64 = 0;
        for y in 1970..year {
            days += if is_leap_year(y as u32) { 366 } else { 365 };
        }
        for m in 1..month {
            let mut dim = DAYS_IN_MONTH[(m - 1) as usize] as u64;
            if m == 2 && is_leap_year(year as u32) {
                dim += 1;
            }
            days += dim;
        }
        days += mday - 1;

        self.timeval = days * 86400 + hour * 3600 + min * 60 + sec;
    }

    // =========================================================================
    // I/O port handlers
    // =========================================================================

    /// Set the current time (convenience for configuration)
    pub fn set_time(&mut self, sec: u8, min: u8, hour: u8, day: u8, month: u8, year: u8) {
        let is_binary = (self.ram[REG_STAT_B as usize] & 0x04) != 0;

        self.ram[REG_SEC as usize] = bin_to_bcd(sec, is_binary);
        self.ram[REG_MIN as usize] = bin_to_bcd(min, is_binary);
        self.ram[REG_HOUR as usize] = bin_to_bcd(hour, is_binary);
        self.ram[REG_MONTH_DAY as usize] = bin_to_bcd(day, is_binary);
        self.ram[REG_MONTH as usize] = bin_to_bcd(month, is_binary);
        self.ram[REG_YEAR as usize] = bin_to_bcd(year, is_binary);

        // Sync timeval from registers
        self.update_timeval();
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

    /// Read from CMOS I/O port (Bochs cmos.cc:383-408)
    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        match port {
            CMOS_ADDR | 0x0072 => {
                // Port 0x70/0x72 is write-only on most machines (Bochs cmos.cc:389-394)
                0xFF
            }
            0x0073 => {
                // Bochs cmos.cc:407-408 — extended CMOS data port
                self.ram[self.cmos_ext_mem_addr as usize] as u32
            }
            CMOS_DATA => {
                let addr = (self.address & 0x7F) as usize;
                let value = match addr as u8 {
                    REG_STAT_A => {
                        // UIP bit is dynamically maintained by timers
                        self.ram[addr]
                    }
                    REG_STAT_C => {
                        // Reading Status C clears all interrupt flags and lowers IRQ8
                        // (Bochs cmos.cc:396-405)
                        let val = self.ram[addr];
                        self.ram[addr] = 0x00;
                        if self.irq_enabled {
                            self.irq8_lower_pending = true;
                        }
                        val
                    }
                    REG_SHUTDOWN => {
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
                value as u32
            }
            _ => {
                tracing::warn!("CMOS: Unknown read port {:#06x}", port);
                0xFF
            }
        }
    }

    /// Write to CMOS I/O port (Bochs cmos.cc:407-685)
    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let value = value as u8;
        match port {
            CMOS_ADDR => {
                // Bochs cmos.cc:436 — standard CMOS address port
                self.nmi_mask = (value & 0x80) != 0;
                self.address = value & 0x7F;
            }
            0x0072 => {
                // Bochs cmos.cc:439-440 — extended CMOS address port
                self.cmos_ext_mem_addr = value | 0x80;
            }
            0x0073 => {
                // Bochs cmos.cc:681-682 — extended CMOS data port
                self.ram[self.cmos_ext_mem_addr as usize] = value;
            }
            CMOS_DATA => {
                let addr = (self.address & 0x7F) as usize;

                match addr as u8 {
                    REG_STAT_A => {
                        // Bits 0-6 are writable, bit 7 (UIP) is read-only
                        let old_val = self.ram[addr];
                        self.ram[addr] = (self.ram[addr] & 0x80) | (value & 0x7F);

                        // If rate or divider changed, recalculate periodic timer
                        if (old_val & 0x7F) != (value & 0x7F) {
                            self.cra_change();
                        }
                    }
                    REG_STAT_B => {
                        let old_val = self.ram[addr];

                        // Bochs cmos.cc:563: bit 3 always forced to 0
                        // (square wave output not supported)
                        let new_val = value & !0x08;

                        // Bochs cmos.cc:565-566: setting bit 7 clears bit 4
                        // (entering SET mode clears update-ended interrupt)
                        let new_val = if new_val & 0x80 != 0 {
                            new_val & !0x10
                        } else {
                            new_val
                        };

                        self.ram[addr] = new_val;

                        // Bochs cmos.cc:571-577: If 12/24-hour or binary/BCD mode changed,
                        // update clock registers
                        if (old_val ^ new_val) & 0x06 != 0 {
                            self.update_clock();
                        }

                        // Bochs cmos.cc:579-593: Periodic Interrupt Enable (bit 6) changes
                        if (old_val ^ new_val) & 0x40 != 0 {
                            if new_val & 0x40 != 0 {
                                // PIE set — activate periodic timer
                                if self.periodic_interval_usec != u32::MAX {
                                    self.periodic_timer_remaining = self.periodic_interval_usec;
                                }
                            } else {
                                // PIE cleared — deactivate periodic timer
                                self.periodic_timer_remaining = 0;
                            }
                        }

                        // Bochs cmos.cc:594-597: Exiting SET mode (bit 7: 1→0)
                        if (old_val & 0x80) != 0 && (new_val & 0x80) == 0 {
                            if self.timeval_change {
                                self.update_timeval();
                                self.timeval_change = false;
                            }
                        }
                    }
                    REG_STAT_C | REG_STAT_D => {
                        // Read-only registers — writes ignored
                    }
                    // Time registers: if in SET mode, mark timeval_change
                    REG_SEC | REG_MIN | REG_HOUR | REG_WEEK_DAY | REG_MONTH_DAY | REG_MONTH
                    | REG_YEAR | REG_CENTURY => {
                        if addr < CMOS_SIZE {
                            self.ram[addr] = value;
                            if self.ram[REG_STAT_B as usize] & 0x80 != 0 {
                                self.timeval_change = true;
                            }
                        }
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
                tracing::warn!(
                    "CMOS: Unknown write port {:#06x} value={:#04x}",
                    port,
                    value
                );
            }
        }
    }

    // =========================================================================
    // Tick / timer advance
    // =========================================================================

    /// Advance all timers by `usec` microseconds.
    /// Returns true if IRQ8 should be raised (periodic or alarm fired).
    pub fn tick(&mut self, usec: u64) -> bool {
        let usec32 = usec as u32;
        let mut irq_fired = false;

        // Advance periodic timer
        if self.periodic_timer_remaining > 0 {
            if usec32 >= self.periodic_timer_remaining {
                let mut elapsed = usec32;
                // Fire as many periodic ticks as elapsed time covers
                while elapsed >= self.periodic_timer_remaining {
                    elapsed -= self.periodic_timer_remaining;
                    self.periodic_timer();
                    irq_fired = true;
                    // Reload for next period (continuous timer)
                    if self.periodic_interval_usec == u32::MAX
                        || self.ram[REG_STAT_B as usize] & 0x40 == 0
                    {
                        self.periodic_timer_remaining = 0;
                        break;
                    }
                    self.periodic_timer_remaining = self.periodic_interval_usec;
                }
                if self.periodic_timer_remaining > 0 && elapsed > 0 {
                    self.periodic_timer_remaining -= elapsed;
                }
            } else {
                self.periodic_timer_remaining -= usec32;
            }
        }

        // Advance one-second timer
        if self.one_second_remaining > 0 {
            if usec32 >= self.one_second_remaining {
                self.one_second_timer();
                // Reload for next second (continuous timer)
                self.one_second_remaining = 1_000_000 - (usec32 - self.one_second_remaining);
                if self.one_second_remaining == 0 {
                    self.one_second_remaining = 1_000_000;
                }
            } else {
                self.one_second_remaining -= usec32;
            }
        }

        // Advance UIP timer (one-shot)
        if self.uip_timer_remaining > 0 {
            if usec32 >= self.uip_timer_remaining {
                self.uip_timer_remaining = 0;
                self.uip_timer();
                if self.irq8_pending {
                    irq_fired = true;
                }
            } else {
                self.uip_timer_remaining -= usec32;
            }
        }

        irq_fired
    }

    /// Check and clear IRQ8 raise pending flag
    pub fn check_irq8(&mut self) -> bool {
        let pending = self.irq8_pending;
        self.irq8_pending = false;
        pending
    }

    /// Check and clear IRQ8 lower pending flag (set on REG_STAT_C read)
    pub fn check_irq8_lower(&mut self) -> bool {
        let pending = self.irq8_lower_pending;
        self.irq8_lower_pending = false;
        pending
    }

    // =========================================================================
    // Configuration helpers
    // =========================================================================

    /// Configure memory size in CMOS from total RAM bytes.
    /// Matches Bochs devices.cc:320-345 exactly.
    ///
    /// Sets CMOS registers:
    /// - 0x15-0x16: Base memory (640 KB)
    /// - 0x17-0x18, 0x30-0x31: Extended memory 1MB-65MB (KB, capped at 0xFC00)
    /// - 0x34-0x35: Extended memory above 16MB (64KB blocks, capped at 0xBF00)
    pub fn set_memory_size_from_bytes(&mut self, total_bytes: u64) {
        const BASE_MEMORY_IN_K: u16 = 640;

        // Base memory: always 640 KB
        self.ram[0x15] = (BASE_MEMORY_IN_K & 0xFF) as u8;
        self.ram[0x16] = ((BASE_MEMORY_IN_K >> 8) & 0xFF) as u8;

        // Extended memory above 1MB (in KB), capped at 0xFC00 (63 MB)
        // Bochs devices.cc:324-326
        let memory_in_k = total_bytes / 1024;
        let extended_memory_in_k = if memory_in_k > 1024 {
            (memory_in_k - 1024).min(0xFC00)
        } else {
            0
        };
        self.ram[0x17] = (extended_memory_in_k & 0xFF) as u8;
        self.ram[0x18] = ((extended_memory_in_k >> 8) & 0xFF) as u8;
        self.ram[0x30] = (extended_memory_in_k & 0xFF) as u8;
        self.ram[0x31] = ((extended_memory_in_k >> 8) & 0xFF) as u8;

        // Extended memory above 16MB (in 64KB blocks), capped at 0xBF00
        // Bochs devices.cc:332-337
        let extended_memory_in_64k = if memory_in_k > 16384 {
            ((memory_in_k - 16384) / 64).min(0xBF00)
        } else {
            0
        };
        self.ram[0x34] = (extended_memory_in_64k & 0xFF) as u8;
        self.ram[0x35] = ((extended_memory_in_64k >> 8) & 0xFF) as u8;

        self.update_checksum();
    }

    /// Configure memory size in CMOS (legacy interface, kept for compatibility)
    ///
    /// `base_kb`: conventional memory (typically 640 KB, within the first 1 MB)
    /// `extended_kb`: extended memory above 1 MB
    ///
    /// Total physical = 1 MB + extended_kb (base_kb is within the first 1 MB,
    /// not added separately — it was previously double-counted causing the kernel
    /// to allocate pages beyond physical RAM).
    pub fn set_memory_size(&mut self, base_kb: u16, extended_kb: u16) {
        let _ = base_kb; // base_kb is within first 1 MB, always reported as 640k
        let total_bytes = (1024u64 + extended_kb as u64) * 1024;
        self.set_memory_size_from_bytes(total_bytes);
    }

    /// Configure hard drive type byte only (legacy — prefer configure_disk_geometry)
    pub fn set_hard_drive(&mut self, drive_num: u8, drive_type: u8) {
        if drive_num == 0 {
            self.ram[0x12] = (self.ram[0x12] & 0x0F) | (drive_type << 4);
        } else if drive_num == 1 {
            self.ram[0x12] = (self.ram[0x12] & 0xF0) | (drive_type & 0x0F);
        }
        self.update_checksum();
    }

    /// Configure full hard drive geometry in CMOS (matching Bochs harddrv.cc:448-474)
    ///
    /// Sets drive type byte (0x12) plus extended geometry registers:
    /// - Drive 0: registers 0x19, 0x1B-0x23
    /// - Drive 1: registers 0x1A, 0x24-0x2C
    pub fn configure_disk_geometry(&mut self, drive: u8, cylinders: u16, heads: u8, spt: u8) {
        if drive == 0 {
            // Flag drive type as 0xF (extended), upper nibble of 0x12
            self.ram[0x12] = (self.ram[0x12] & 0x0F) | 0xF0;
            // User-definable type
            self.ram[0x19] = 47;
            // Cylinders (low, high)
            self.ram[0x1B] = (cylinders & 0xFF) as u8;
            self.ram[0x1C] = (cylinders >> 8) as u8;
            // Heads
            self.ram[0x1D] = heads;
            // Write precompensation cylinder (0xFFFF = -1 = none)
            self.ram[0x1E] = 0xFF;
            self.ram[0x1F] = 0xFF;
            // Control byte: bit 7,6 always set; bit 3 = heads > 8
            self.ram[0x20] = 0xC0 | if heads > 8 { 0x08 } else { 0 };
            // Landing zone = cylinders
            self.ram[0x21] = self.ram[0x1B];
            self.ram[0x22] = self.ram[0x1C];
            // Sectors per track
            self.ram[0x23] = spt;
        } else if drive == 1 {
            // Flag drive type as 0xF (extended), lower nibble of 0x12
            self.ram[0x12] = (self.ram[0x12] & 0xF0) | 0x0F;
            self.ram[0x1A] = 47;
            self.ram[0x24] = (cylinders & 0xFF) as u8;
            self.ram[0x25] = (cylinders >> 8) as u8;
            self.ram[0x26] = heads;
            self.ram[0x27] = 0xFF;
            self.ram[0x28] = 0xFF;
            self.ram[0x29] = 0xC0 | if heads > 8 { 0x08 } else { 0 };
            self.ram[0x2A] = self.ram[0x24];
            self.ram[0x2B] = self.ram[0x25];
            self.ram[0x2C] = spt;
        }
        self.update_checksum();
    }

    /// Configure floppy drive types in CMOS (matching Bochs floppy.cc:332-337 / cmos.cc init)
    ///
    /// drive_type: 0=none, 1=360K, 2=1.2M, 3=720K, 4=1.44M, 5=2.88M
    /// Sets CMOS 0x10 (floppy types) and updates equipment byte (0x14).
    pub fn set_floppy_config(&mut self, drive_a_type: u8, drive_b_type: u8) {
        // CMOS 0x10: high nibble = drive A type, low nibble = drive B type
        self.ram[0x10] = (drive_a_type << 4) | (drive_b_type & 0x0F);

        // CMOS 0x14 equipment byte floppy bits:
        //   bit 0: floppy controller present
        //   bits 7-6: number of floppy drives - 1 (0=1 drive, 1=2 drives)
        let num_drives = match (drive_a_type > 0, drive_b_type > 0) {
            (false, _) => 0u8,
            (true, false) => 1,
            (true, true) => 2,
        };
        if num_drives == 0 {
            // No drives: clear floppy present bit and drive count
            self.ram[REG_EQUIPMENT as usize] &= 0x3E; // clear bits 0, 7-6
        } else {
            // Floppy installed (bit 0) + (num_drives-1) in bits 7-6
            let drive_bits = ((num_drives - 1) & 0x03) << 6;
            self.ram[REG_EQUIPMENT as usize] =
                (self.ram[REG_EQUIPMENT as usize] & 0x3E) | drive_bits | 0x01;
        }

        self.update_checksum();
    }

    /// Configure boot sequence in CMOS
    ///
    /// Sets both the legacy (0x2D) and ELTORITO (0x3D, 0x38) boot sequence registers.
    /// Boot device codes for ELTORITO: 0=none, 1=floppy, 2=hard disk, 3=cdrom
    pub fn set_boot_sequence(&mut self, first: u8, second: u8, third: u8) {
        // Legacy register 0x2D bit 5: 0=boot C: then A:, 1=boot A: then C:
        if first == 1 {
            // First boot is floppy → set bit 5
            self.ram[0x2D] |= 0x20;
        } else {
            // First boot is hard disk or other → clear bit 5
            self.ram[0x2D] &= !0x20;
        }

        // ELTORITO boot sequence registers (used by BIOS-bochs-latest)
        // 0x3D: low nibble = 1st boot device, high nibble = 2nd boot device
        self.ram[0x3D] = first | (second << 4);
        // 0x38: high nibble = 3rd boot device, low nibble = signature check flag
        self.ram[0x38] = (self.ram[0x38] & 0x0F) | (third << 4);

        self.update_checksum();
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

    #[test]
    fn test_port_70_read_returns_ff() {
        let mut cmos = BxCmosC::new();
        assert_eq!(cmos.read(CMOS_ADDR, 1), 0xFF);
    }

    #[test]
    fn test_stat_c_read_clears_and_lowers_irq() {
        let mut cmos = BxCmosC::new();
        cmos.ram[REG_STAT_C as usize] = 0xC0; // IRQF + PF set

        // Select Status C register
        cmos.write(CMOS_ADDR, REG_STAT_C as u32, 1);
        let val = cmos.read(CMOS_DATA, 1);

        assert_eq!(val, 0xC0); // Should return old value
        assert_eq!(cmos.ram[REG_STAT_C as usize], 0x00); // Should be cleared
        assert!(cmos.irq8_lower_pending); // Should request IRQ8 lower
    }

    #[test]
    fn test_periodic_timer() {
        let mut cmos = BxCmosC::new();
        // Enable periodic interrupt (bit 6 of Status B)
        cmos.write(CMOS_ADDR, REG_STAT_B as u32, 1);
        cmos.write(CMOS_DATA, 0x42, 1); // 24-hour + PIE

        // Verify periodic timer is active
        assert!(cmos.periodic_timer_remaining > 0);
        assert_ne!(cmos.periodic_interval_usec, u32::MAX);

        // Tick enough to fire periodic timer
        let interval = cmos.periodic_interval_usec;
        cmos.tick(interval as u64 + 1);

        // Check that IRQ8 was raised
        assert!(cmos.check_irq8());
        // Check Status C has periodic flag
        // (Note: check_irq8 doesn't clear Status C — that happens on read)
    }

    #[test]
    fn test_one_second_timer() {
        let mut cmos = BxCmosC::new();
        let initial_timeval = cmos.timeval;

        // Tick one second
        cmos.tick(1_000_001);

        // Timeval should have incremented
        assert_eq!(cmos.timeval, initial_timeval + 1);
    }
}
