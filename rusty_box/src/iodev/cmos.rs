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
/// Century register (aka REG_IBM_CENTURY_BYTE, Bochs cmos.cc)
pub const REG_CENTURY: u8 = 0x32;
/// PS/2-style alternative century register — mirrors REG_CENTURY on writes
/// (Bochs cmos.cc REG_IBM_PS2_CENTURY_BYTE; needed by WinXP per Bochs comment)
pub const REG_IBM_PS2_CENTURY_BYTE: u8 = 0x37;

/// CMOS RAM size (256 bytes: standard 128 + extended 128 via ports 0x72/0x73)
pub const CMOS_SIZE: usize = 256;

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

// =============================================================================
// Portable UTC time conversion (Bochs iodev/utctime.h)
//
// Ported verbatim from `utctime_ext`/`timeutc`: pure integer math, no
// per-year/per-day iteration, so it is O(1) regardless of how far `timeval`
// or the broken-down fields are from a sane range. The previous hand-rolled
// loops in `update_clock`/`update_timeval` were O(days) and O(months) and
// could be driven by the guest into a multi-hour host stall (finding #1) or
// an out-of-bounds panic (month >= 14) / underflow panic (day == 0).
// =============================================================================

/// Days elapsed between the start of a month and the start of the year,
/// indexed `[is_leap][month]` (Bochs utctime.h `utctime_ext`/`timeutc`
/// `monthlydays`).
const MONTHLYDAYS: [[i64; 13]; 2] = [
    [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365],
    [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335, 366],
];

/// Broken-down UTC time, mirror of Bochs `struct utctm` (utctime.h).
/// `mon` is 0-based (0 = January); `year` is years since 1900. Bochs uses
/// `Bit16s` for these fields; we use `i64` throughout (per design) since the
/// values here are only ever a handful of register-byte-sized inputs, and
/// the overflow check in `utctime_ext` explicitly reproduces the effect of
/// the narrower Bochs type instead of relying on it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct BrokenTime {
    sec: i64,
    min: i64,
    hour: i64,
    mday: i64,
    mon: i64,
    year: i64,
    wday: i64,
    yday: i64,
}

/// Bochs utctime.h `utctime_ext`: epoch seconds (since 1970-01-01) ->
/// normalized broken-down time. Returns `false` if the resulting year does
/// not fit in the range Bochs's `Bit16s tm_year` could represent (Bochs
/// signals this by returning `NULL`; we reproduce the equivalent bound
/// explicitly since `BrokenTime::year` is `i64`, not a narrow type that
/// would actually truncate).
fn utctime_ext(epoch: i64, t: &mut BrokenTime) -> bool {
    let mut etmp: i64 = epoch;
    let mut eyear: i64 = 2001;

    // Get time of day, then days number based at 2001-01-01 (nearest
    // non-leap start of a 400yr cycle).
    let mut tsec = etmp % 86400;
    etmp /= 86400;
    etmp -= 11323;
    if tsec < 0 {
        etmp -= 1;
        tsec += 86400;
    }

    let sec = tsec % 60;
    tsec /= 60;
    let min = tsec % 60;
    tsec /= 60;
    let hour = tsec;

    let mut wday = (etmp - 6) % 7;
    if wday < 0 {
        wday += 7;
    }

    if etmp < 0 {
        eyear += 400 * (etmp / 146097 - 1);
        etmp %= 146097;
        etmp += 146097;
    }
    eyear += 400 * (etmp / 146097);
    etmp %= 146097;
    eyear += 100 * (etmp / 36524);
    etmp %= 36524;
    eyear += 4 * (etmp / 1461);
    etmp %= 1461;
    while (eyear % 4 != 0) && (etmp >= 365) {
        eyear += 1;
        etmp -= 365;
    }

    // Find out if the year is leap (Bochs does this with Bit8u bitwise
    // tricks on isleap; reproduced with the same boolean result).
    let mut isleap: u8 = 0;
    isleap |= if eyear % 400 == 0 { 2 } else { 0 };
    isleap |= if eyear % 4 == 0 { 1 } else { 0 };
    isleap &= if eyear % 100 == 0 { !1u8 } else { !0u8 };
    let isleap = usize::from(isleap != 0);

    eyear -= 1900;
    // Bochs: `bdt.tm_year = (Bit16s)eyear; if (eyear != bdt.tm_year) return
    // NULL;` — a cast-truncation to Bit16s that fails whenever `eyear` does
    // not fit in that range. Reproduce the same bound directly.
    if !(i64::from(i16::MIN)..=i64::from(i16::MAX)).contains(&eyear) {
        return false;
    }

    let yday = etmp;
    let mut mon: i64 = 0;
    while etmp >= MONTHLYDAYS[isleap][(mon + 1) as usize] {
        mon += 1;
    }
    etmp -= MONTHLYDAYS[isleap][mon as usize];
    let mday = etmp + 1;

    t.sec = sec;
    t.min = min;
    t.hour = hour;
    t.wday = wday;
    t.yday = yday;
    t.mday = mday;
    t.mon = mon;
    t.year = eyear;
    true
}

/// Bochs utctime.h `timeutc`: broken-down (possibly out-of-range) time ->
/// epoch seconds. Normalizes `t` in place via `utctime_ext`. Returns `-1` on
/// failure (resulting year out of the representable range), mirroring
/// `timegm()`/`mktime()` semantics. This is what lets `update_timeval`
/// accept malformed register contents (e.g. month=19, day=0) safely: the
/// out-of-range fields fall out of the integer math below without ever
/// indexing an array by them.
fn timeutc(t: &mut BrokenTime) -> i64 {
    let mut isleap: u8 = 3;

    let mut etmp = t.year;
    let mut tmon = t.mon;

    etmp += tmon / 12;
    tmon %= 12;
    if tmon < 0 {
        etmp -= 1;
        tmon += 12;
    }

    etmp -= 101; // years passed since 2001
    let mut epoch: i64 = 0;
    if etmp < 0 {
        epoch += 146097 * (etmp / 400 - 1);
        etmp %= 400;
        etmp += 400;
    }

    epoch += (etmp / 400) * 146097;
    etmp %= 400;
    isleap &= if etmp == 399 { !0u8 } else { !2u8 };
    epoch += (etmp / 100) * 36524;
    etmp %= 100;
    isleap &= if etmp == 99 { !1u8 } else { !0u8 };
    epoch += (etmp / 4) * 1461;
    etmp %= 4;
    isleap &= if etmp == 3 { !0u8 } else { !1u8 };
    let isleap = usize::from(isleap != 0);
    epoch += etmp * 365;

    // Number of entire days between the current date and 2001-01-01.
    epoch += MONTHLYDAYS[isleap][tmon as usize];
    epoch += t.mday - 1;
    epoch *= 24;
    epoch += t.hour;
    epoch *= 60;
    epoch += t.min;
    epoch *= 60;
    epoch += t.sec;
    epoch += 978_307_200; // seconds between 2001-01-01 and 1970-01-01

    if utctime_ext(epoch, t) {
        epoch
    } else {
        -1
    }
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
    /// Signed (Bochs cmos.cc uses `Bit64s`) — the CMOS clamp range extends
    /// below 1970 (year 0 minimum), and normalization math in
    /// `utctime_ext`/`timeutc` requires signed arithmetic throughout.
    timeval: i64,
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
                .map(|d| d.as_secs() as i64)
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
        tracing::debug!("CMOS: Initializing CMOS/RTC");
        self.init_defaults();
    }

    /// Reset the CMOS/RTC
    pub fn reset(&mut self) {
        self.address = 0;
        self.nmi_mask = false;
        self.irq8_pending = false;
        self.irq8_lower_pending = false;

        // Bochs cmos.cc reset: RESET affects the following registers:
        //  CRA: no effects
        //  CRB: bits 4,5,6 forced to 0 (UIE/AIE/PIE)
        //  CRC: bits 4,5,6,7 forced to 0
        //  CRD: no effects
        self.ram[REG_STAT_B as usize] &= 0x8F;
        self.ram[REG_STAT_C as usize] = 0x00;

        // Bochs cmos.cc reset: handle periodic interrupt rate select —
        // CRA_change() re-evaluates periodic_interval_usec/timer state,
        // which also picks up PIE having just been forced off above.
        self.cra_change();
    }

    // =========================================================================
    // Timer handlers (matching Bochs cmos.cc)
    // =========================================================================

    /// Recalculate periodic interval from REG_STAT_A (Bochs cmos.cc CRA_change)
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

            // If Periodic Interrupt Enable bit set, activate timer.
            // Bochs cmos.cc CRA_change: `activate_timer()` unconditionally
            // restarts the countdown from periodic_interval_usec — it is
            // not gated on the timer having been idle (finding #19).
            if self.ram[REG_STAT_B as usize] & 0x40 != 0 {
                self.periodic_timer_remaining = self.periodic_interval_usec;
            } else {
                self.periodic_timer_remaining = 0;
            }
        }
    }

    /// Periodic timer handler (Bochs cmos.cc periodic_timer)
    fn periodic_timer(&mut self) {
        // If periodic interrupts are enabled, trip IRQ8 and update status C
        if self.ram[REG_STAT_B as usize] & 0x40 != 0 {
            self.ram[REG_STAT_C as usize] |= 0xC0; // IRQF + PF (bits 7,6)
            if self.irq_enabled {
                self.irq8_pending = true;
            }
        }
    }

    /// One-second timer handler (Bochs cmos.cc one_second_timer)
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

    /// UIP timer handler (Bochs cmos.cc uip_timer)
    fn uip_timer(&mut self) {
        // Clear UIP bit
        self.ram[REG_STAT_A as usize] &= !0x80;

        // Update CMOS registers from timeval
        self.update_clock();

        // Bochs cmos.cc uip_timer: the Update-Ended flag (UF, bit 4 of
        // Status C) is only set together with IRQF when Update-Ended
        // Interrupt Enable (UIE, bit 4 of Status B) is set — NOT
        // unconditionally on every update cycle (finding #33).
        if self.ram[REG_STAT_B as usize] & 0x10 != 0 {
            self.ram[REG_STAT_C as usize] |= 0x90; // IRQF + UF
            if self.irq_enabled {
                self.irq8_pending = true;
            }
        }

        // Check alarm match
        self.check_alarm();
    }

    /// Check if current time matches alarm registers (Bochs cmos.cc)
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
            // Bochs cmos.cc uip_timer: the Alarm Flag (AF, bit 5 of Status
            // C) is only set together with IRQF when Alarm Interrupt
            // Enable (AIE, bit 5 of Status B) is set (finding #33).
            if self.ram[REG_STAT_B as usize] & 0x20 != 0 {
                self.ram[REG_STAT_C as usize] |= 0xA0; // IRQF + AF
                if self.irq_enabled {
                    self.irq8_pending = true;
                }
            }
        }
    }

    /// Update CMOS date/time registers from internal timeval
    /// (Bochs cmos.cc update_clock)
    fn update_clock(&mut self) {
        let is_binary = (self.ram[REG_STAT_B as usize] & 0x04) != 0;
        let is_24hour = (self.ram[REG_STAT_B as usize] & 0x02) != 0;

        // Bochs cmos.cc update_clock: clamp timeval into the representable
        // range before decoding it. This is the host-DoS fix (finding #1):
        // previously an unbounded `timeval` fed an O(days) year-search loop
        // that a guest could drive into a multi-hour host stall by writing
        // an extreme date and exiting SET mode. The clamp wraps like a
        // simple overflow, exactly mirroring Bochs.
        const MINTVALSET: i64 = -62_167_219_200; // year 0000-01-01
        const MAXTVALSET_BCD: i64 = 253_402_300_799; // year 9999-12-31 23:59:59
        const MAXTVALSET_BIN: i64 = 745_690_751_999; // year 25599-12-31 23:59:59
        let maxtvalset = if is_binary {
            MAXTVALSET_BIN
        } else {
            MAXTVALSET_BCD
        };
        while self.timeval > maxtvalset {
            self.timeval -= maxtvalset - MINTVALSET + 1;
        }
        while self.timeval < MINTVALSET {
            self.timeval += maxtvalset - MINTVALSET + 1;
        }

        // Bochs cmos.cc update_clock: `time_calendar = utctime(&s.timeval);`
        let mut bt = BrokenTime::default();
        // timeval is clamped into a range whose years fit comfortably
        // within utctime_ext's overflow bound, so this cannot fail.
        let decoded = utctime_ext(self.timeval, &mut bt);
        debug_assert!(
            decoded,
            "cmos update_clock: clamped timeval failed to decode"
        );
        if !decoded {
            return;
        }

        // update seconds / minutes
        self.ram[REG_SEC as usize] = bin_to_bcd(bt.sec as u8, is_binary);
        self.ram[REG_MIN as usize] = bin_to_bcd(bt.min as u8, is_binary);

        // update hours
        if is_24hour {
            self.ram[REG_HOUR as usize] = bin_to_bcd(bt.hour as u8, is_binary);
        } else {
            let mut hour = bt.hour;
            let pm = if hour > 11 { 0x80u8 } else { 0x00u8 };
            if hour > 11 {
                hour -= 12;
            }
            if hour == 0 {
                hour = 12;
            }
            self.ram[REG_HOUR as usize] = bin_to_bcd(hour as u8, is_binary) | pm;
        }

        // update day of the week (0..6 -> 1..7)
        self.ram[REG_WEEK_DAY as usize] = bin_to_bcd((bt.wday + 1) as u8, is_binary);

        // update day of the month
        self.ram[REG_MONTH_DAY as usize] = bin_to_bcd(bt.mday as u8, is_binary);

        // update month (0..11 -> 1..12)
        self.ram[REG_MONTH as usize] = bin_to_bcd((bt.mon + 1) as u8, is_binary);

        // update year
        self.ram[REG_YEAR as usize] = bin_to_bcd((bt.year % 100) as u8, is_binary);

        // update century
        self.ram[REG_CENTURY as usize] = bin_to_bcd((bt.year / 100 + 19) as u8, is_binary);

        // Bochs cmos.cc update_clock: some BIOSes also use reg 0x37 for the
        // century byte (critical for WinXP per Bochs comment) — mirror it.
        self.ram[REG_IBM_PS2_CENTURY_BYTE as usize] = self.ram[REG_CENTURY as usize];
    }

    /// Convert CMOS date/time registers back to timeval
    /// Called when exiting SET mode, or immediately on a non-SET-mode write
    /// (Bochs cmos.cc update_timeval)
    fn update_timeval(&mut self) {
        let is_binary = (self.ram[REG_STAT_B as usize] & 0x04) != 0;
        let is_24hour = (self.ram[REG_STAT_B as usize] & 0x02) != 0;

        let mut bt = BrokenTime::default();

        // update seconds / minutes
        bt.sec = bcd_to_bin(self.ram[REG_SEC as usize], is_binary) as i64;
        bt.min = bcd_to_bin(self.ram[REG_MIN as usize], is_binary) as i64;

        // update hours
        if is_24hour {
            bt.hour = bcd_to_bin(self.ram[REG_HOUR as usize], is_binary) as i64;
        } else {
            let pm_flag = self.ram[REG_HOUR as usize] & 0x80;
            let mut val_bin = bcd_to_bin(self.ram[REG_HOUR as usize] & 0x7F, is_binary) as i64;
            if val_bin < 12 && pm_flag > 0 {
                val_bin += 12;
            } else if val_bin == 12 && pm_flag == 0 {
                val_bin = 0;
            }
            bt.hour = val_bin;
        }

        // update day of the month
        bt.mday = bcd_to_bin(self.ram[REG_MONTH_DAY as usize], is_binary) as i64;

        // update month (register is 1..12 -> BrokenTime is 0..11; may be
        // out of range for a malformed guest write, e.g. 0 or >=14 — that
        // is fine, timeutc()/utctime_ext() normalize it without indexing
        // anything by the raw value, unlike the old DAYS_IN_MONTH loop)
        bt.mon = bcd_to_bin(self.ram[REG_MONTH as usize], is_binary) as i64 - 1;

        // update year
        let mut val_yr = bcd_to_bin(self.ram[REG_CENTURY as usize], is_binary) as i64;
        val_yr = (val_yr - 19) * 100;
        val_yr += bcd_to_bin(self.ram[REG_YEAR as usize], is_binary) as i64;
        bt.year = val_yr;

        // Bochs cmos.cc update_timeval: `s.timeval = timeutc(&time_calendar);`
        self.timeval = timeutc(&mut bt);
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

    /// Read from CMOS I/O port (Bochs cmos.cc)
    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        match port {
            CMOS_ADDR | 0x0072 => {
                // Port 0x70/0x72 is write-only on most machines (Bochs cmos.cc)
                0xFF
            }
            0x0073 => {
                // Bochs cmos.cc — extended CMOS data port
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
                        // (Bochs cmos.cc)
                        let val = self.ram[addr];
                        self.ram[addr] = 0x00;
                        if self.irq_enabled {
                            self.irq8_lower_pending = true;
                        }
                        val
                    }
                    REG_SHUTDOWN => {
                        let val = self.ram[addr];
                        tracing::debug!(
                            "CMOS: Read shutdown status [{:#04x}] = {:#04x}",
                            addr,
                            val
                        );
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

    /// Write to CMOS I/O port (Bochs cmos.cc)
    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let value = value as u8;
        match port {
            CMOS_ADDR => {
                // Bochs cmos.cc — standard CMOS address port
                self.nmi_mask = (value & 0x80) != 0;
                self.address = value & 0x7F;
            }
            0x0072 => {
                // Bochs cmos.cc — extended CMOS address port
                self.cmos_ext_mem_addr = value | 0x80;
            }
            0x0073 => {
                // Bochs cmos.cc — extended CMOS data port
                self.ram[self.cmos_ext_mem_addr as usize] = value;
            }
            CMOS_DATA => {
                let addr = (self.address & 0x7F) as usize;

                match addr as u8 {
                    REG_STAT_A => {
                        // Bits 0-6 are writable, bit 7 (UIP) is read-only
                        self.ram[addr] = (self.ram[addr] & 0x80) | (value & 0x7F);

                        // Bochs cmos.cc write REG_STAT_A: CRA_change() is
                        // called unconditionally on every write, not only
                        // when the rate/divider bits changed — it always
                        // re-activates (restarts) the periodic timer when
                        // PIE is set (finding #19).
                        self.cra_change();
                    }
                    REG_STAT_B => {
                        let old_val = self.ram[addr];

                        // Bochs cmos.cc: bit 3 always forced to 0
                        // (square wave output not supported)
                        let new_val = value & !0x08;

                        // Bochs cmos.cc: setting bit 7 clears bit 4
                        // (entering SET mode clears update-ended interrupt)
                        let new_val = if new_val & 0x80 != 0 {
                            new_val & !0x10
                        } else {
                            new_val
                        };

                        self.ram[addr] = new_val;

                        // Bochs cmos.cc: If 12/24-hour or binary/BCD mode changed,
                        // update clock registers
                        if (old_val ^ new_val) & 0x06 != 0 {
                            self.update_clock();
                        }

                        // Bochs cmos.cc: Periodic Interrupt Enable (bit 6) changes
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

                        // Bochs cmos.cc: Exiting SET mode (bit 7: 1→0)
                        if (old_val & 0x80) != 0 && (new_val & 0x80) == 0 && self.timeval_change {
                            self.update_timeval();
                            self.timeval_change = false;
                        }
                    }
                    REG_STAT_C | REG_STAT_D => {
                        // Read-only registers — writes ignored
                    }
                    // Time registers: in SET mode, defer the timeval sync
                    // (mark timeval_change) until SET mode is exited;
                    // otherwise apply it immediately (Bochs cmos.cc write:
                    // `if (reg[STAT_B] & 0x80) timeval_change=1; else
                    // update_timeval();` — finding #9. Previously only the
                    // SET-mode branch existed, so a guest writing time
                    // registers outside SET mode had no effect at all.
                    REG_SEC | REG_MIN | REG_HOUR | REG_WEEK_DAY | REG_MONTH_DAY | REG_MONTH
                    | REG_YEAR | REG_CENTURY | REG_IBM_PS2_CENTURY_BYTE => {
                        if addr < CMOS_SIZE {
                            self.ram[addr] = value;

                            // Bochs cmos.cc write: PS/2 BIOSes also use
                            // 0x37 for the century byte; mirror writes to
                            // 0x37 into the canonical 0x32 register.
                            if addr as u8 == REG_IBM_PS2_CENTURY_BYTE {
                                self.ram[REG_CENTURY as usize] = value;
                            }

                            if self.ram[REG_STAT_B as usize] & 0x80 != 0 {
                                self.timeval_change = true;
                            } else {
                                self.update_timeval();
                            }
                        }
                    }
                    _ => {
                        if addr < CMOS_SIZE {
                            self.ram[addr] = value;
                            // Bochs cmos.cc write: the checksum is never
                            // recomputed from an I/O write — only by
                            // explicit calls like set_memory_size() /
                            // configure_disk_geometry() at setup time
                            // (finding #33).
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
                // Reload for next second (continuous timer). Saturating:
                // a tick spanning more than 1s (usec32 - remaining >
                // 1_000_000) must not underflow this u32 subtraction
                // (finding #33) — it only fires the timer once per call
                // regardless of overshoot, same as before, just safely.
                let elapsed_over = usec32 - self.one_second_remaining;
                self.one_second_remaining = 1_000_000u32.saturating_sub(elapsed_over);
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
    #[inline]
    pub fn check_irq8(&mut self) -> bool {
        let pending = self.irq8_pending;
        self.irq8_pending = false;
        pending
    }

    /// Check and clear IRQ8 lower pending flag (set on REG_STAT_C read)
    #[inline]
    pub fn check_irq8_lower(&mut self) -> bool {
        let pending = self.irq8_lower_pending;
        self.irq8_lower_pending = false;
        pending
    }

    // =========================================================================
    // Configuration helpers
    // =========================================================================

    /// Configure memory size in CMOS from total RAM bytes.
    /// Matches Bochs devices.cc exactly.
    ///
    /// Sets CMOS registers:
    /// - 0x15-0x16: Base memory (640 KB)
    /// - 0x17-0x18, 0x30-0x31: Extended memory 1MB-65MB (KB, capped at 0xFC00)
    /// - 0x34-0x35: Extended memory above 16MB (64KB blocks, capped at 0xBF00)
    /// - 0x5b-0x5d: Memory above 4GB in 64KB units (QEMU-compatible extension)
    pub fn set_memory_size_from_bytes(&mut self, total_bytes: u64) {
        const BASE_MEMORY_IN_K: u16 = 640;

        // Base memory: always 640 KB
        self.ram[0x15] = (BASE_MEMORY_IN_K & 0xFF) as u8;
        self.ram[0x16] = ((BASE_MEMORY_IN_K >> 8) & 0xFF) as u8;

        // Extended memory above 1MB (in KB), capped at 0xFC00 (63 MB)
        // Bochs devices.cc
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
        // Bochs devices.cc
        let extended_memory_in_64k = if memory_in_k > 16384 {
            ((memory_in_k - 16384) / 64).min(0xBF00)
        } else {
            0
        };
        self.ram[0x34] = (extended_memory_in_64k & 0xFF) as u8;
        self.ram[0x35] = ((extended_memory_in_64k >> 8) & 0xFF) as u8;

        // Memory above 4GB via QEMU-compatible CMOS extension (registers 0x5b-0x5d).
        // For configurations with a 3GB-4GB PCI MMIO hole, RAM above 3GB is remapped
        // above 4GB, so the threshold is 3GB (0xC000_0000), not 4GB.
        let memory_above_4gb = if total_bytes > 0xC000_0000 {
            total_bytes - 0xC000_0000
        } else {
            0
        };
        let memory_above_4gb_in_64k = memory_above_4gb >> 16;
        self.ram[0x5b] = (memory_above_4gb_in_64k & 0xFF) as u8;
        self.ram[0x5c] = ((memory_above_4gb_in_64k >> 8) & 0xFF) as u8;
        self.ram[0x5d] = ((memory_above_4gb_in_64k >> 16) & 0xFF) as u8;

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

    /// Configure full hard drive geometry in CMOS (matching Bochs harddrv.cc)
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

    /// Configure floppy drive types in CMOS (matching Bochs floppy.cc / cmos.cc init)
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

    // =========================================================================
    // Low-level utctime.h port sanity checks
    // =========================================================================

    #[test]
    fn utctime_ext_epoch_zero_is_1970_01_01_thursday() {
        let mut bt = BrokenTime::default();
        assert!(utctime_ext(0, &mut bt));
        assert_eq!(bt.year, 70); // 1970 - 1900
        assert_eq!(bt.mon, 0); // January (0-based)
        assert_eq!(bt.mday, 1);
        assert_eq!(bt.hour, 0);
        assert_eq!(bt.min, 0);
        assert_eq!(bt.sec, 0);
        assert_eq!(bt.wday, 4); // Thursday (0=Sunday) — the real epoch weekday
    }

    #[test]
    fn timeutc_is_inverse_of_utctime_ext() {
        // A handful of representative epochs, including negative (pre-1970)
        // and one that lands past a 400-year cycle boundary.
        for &epoch in &[0i64, 1_735_732_800, -1, -62_167_219_200, 253_402_300_799] {
            let mut bt = BrokenTime::default();
            assert!(utctime_ext(epoch, &mut bt), "utctime_ext({epoch}) failed");
            assert_eq!(timeutc(&mut bt), epoch, "round-trip failed for {epoch}");
        }
    }

    // =========================================================================
    // Finding #1 — host-DoS in date conversion (Bochs utctime.h port)
    // =========================================================================

    #[test]
    fn cmos_malformed_date_does_not_panic() {
        let mut cmos = BxCmosC::new();

        // Enter SET mode (Bochs cmos.cc: CRB bit7 = 1 freezes the user
        // copy of time so registers can be written without a mid-write
        // update racing in). Default mode bits: BCD, 12-hour.
        cmos.write(CMOS_ADDR, REG_STAT_B as u32, 1);
        cmos.write(CMOS_DATA, 0x80, 1);

        // Out-of-range month (13, BCD 0x13) and day 0. The old code did
        // `for m in 1..month { DAYS_IN_MONTH[(m-1)] }` (OOB panic once
        // month >= 14) and `days += mday - 1` on a u64 (underflow panic
        // when mday == 0). Neither can happen anymore: timeutc()
        // normalizes both without ever indexing by the raw value.
        cmos.write(CMOS_ADDR, REG_MONTH as u32, 1);
        cmos.write(CMOS_DATA, 0x13, 1);
        cmos.write(CMOS_ADDR, REG_MONTH_DAY as u32, 1);
        cmos.write(CMOS_DATA, 0x00, 1);

        // Exit SET mode -> update_timeval() runs on the malformed registers.
        cmos.write(CMOS_ADDR, REG_STAT_B as u32, 1);
        cmos.write(CMOS_DATA, 0x00, 1);

        // Refresh registers from the now-normalized timeval and check the
        // result landed in-range (reaching this point at all proves no
        // panic occurred).
        cmos.update_clock();
        let month = bcd_to_bin(cmos.ram[REG_MONTH as usize], false);
        let mday = bcd_to_bin(cmos.ram[REG_MONTH_DAY as usize], false);
        assert!((1..=12).contains(&month), "month {month} out of range");
        assert!((1..=31).contains(&mday), "mday {mday} out of range");
    }

    #[test]
    fn cmos_update_clock_clamps_extreme_timeval() {
        let mut cmos = BxCmosC::new();
        // BCD mode is the default (STAT_B bit2 = 0) -> the smaller of the
        // two Bochs clamp ranges applies (year 0000..=9999).
        const MINTVALSET: i64 = -62_167_219_200;
        const MAXTVALSET_BCD: i64 = 253_402_300_799;

        // A value the old `loop { days -= days_in_year; year += 1; }`
        // would have iterated through year-by-year (finding #1's host-DoS
        // surface) — the new clamp+utctime_ext path is O(1) date math, so
        // this returns promptly regardless of magnitude.
        cmos.timeval = 10_000_000_000_000; // ~317,000 years past epoch
        cmos.update_clock();
        assert!((MINTVALSET..=MAXTVALSET_BCD).contains(&cmos.timeval));

        cmos.timeval = -10_000_000_000_000;
        cmos.update_clock();
        assert!((MINTVALSET..=MAXTVALSET_BCD).contains(&cmos.timeval));
    }

    // =========================================================================
    // Finding #9 — century register 0x37 mirror + non-SET write branch
    // =========================================================================

    #[test]
    fn cmos_century_0x37_mirrors_0x32() {
        let mut cmos = BxCmosC::new();

        // A non-SET-mode write to 0x37 must mirror into 0x32 immediately
        // (Bochs cmos.cc write: `if (addr == REG_IBM_PS2_CENTURY_BYTE)
        // reg[REG_IBM_CENTURY_BYTE] = value;`).
        cmos.write(CMOS_ADDR, REG_IBM_PS2_CENTURY_BYTE as u32, 1);
        cmos.write(CMOS_DATA, 0x20, 1); // BCD 20 (21st century)
        assert_eq!(cmos.ram[REG_CENTURY as usize], 0x20);
        assert_eq!(cmos.ram[REG_IBM_PS2_CENTURY_BYTE as usize], 0x20);

        // update_clock() must also keep both registers mirrored (Bochs
        // cmos.cc update_clock: `reg[REG_IBM_PS2_CENTURY_BYTE] =
        // reg[REG_IBM_CENTURY_BYTE];`).
        cmos.ram[REG_IBM_PS2_CENTURY_BYTE as usize] = 0x00; // desync it
        cmos.update_clock();
        assert_eq!(
            cmos.ram[REG_IBM_PS2_CENTURY_BYTE as usize],
            cmos.ram[REG_CENTURY as usize]
        );
    }

    #[test]
    fn cmos_time_write_outside_set_mode_applies() {
        let mut cmos = BxCmosC::new();
        // SET mode is off by default (STAT_B bit 7 = 0).
        assert_eq!(cmos.ram[REG_STAT_B as usize] & 0x80, 0);

        let before = cmos.timeval;

        // BCD mode (default): 0x30 = 30 seconds.
        cmos.write(CMOS_ADDR, REG_SEC as u32, 1);
        cmos.write(CMOS_DATA, 0x30, 1);

        // Finding #9: previously only the SET-mode branch existed, so a
        // write outside SET mode had no effect on timeval at all.
        assert_ne!(cmos.timeval, before, "update_timeval() did not run");
        let mut bt = BrokenTime::default();
        assert!(utctime_ext(cmos.timeval, &mut bt));
        assert_eq!(bt.sec, 30);
    }

    // =========================================================================
    // Finding #1 / #9 combined — BCD + binary, 12h + 24h round trip
    // =========================================================================

    #[test]
    fn cmos_date_roundtrip() {
        for &(is_binary, is_24hour) in &[
            (false, false),
            (false, true),
            (true, false),
            (true, true),
        ] {
            let mut cmos = BxCmosC::new();
            let stat_b = (if is_24hour { 0x02 } else { 0 }) | (if is_binary { 0x04 } else { 0 });

            // Program mode bits first (no SET mode yet).
            cmos.write(CMOS_ADDR, REG_STAT_B as u32, 1);
            cmos.write(CMOS_DATA, stat_b as u32, 1);

            // Enter SET mode.
            cmos.write(CMOS_ADDR, REG_STAT_B as u32, 1);
            cmos.write(CMOS_DATA, (stat_b | 0x80) as u32, 1);

            let (sec, min, hour24, mday, month, year2, century) =
                (45u8, 30u8, 15u8, 4u8, 7u8, 25u8, 20u8);
            let enc = |v: u8| bin_to_bcd(v, is_binary);

            cmos.write(CMOS_ADDR, REG_SEC as u32, 1);
            cmos.write(CMOS_DATA, enc(sec) as u32, 1);
            cmos.write(CMOS_ADDR, REG_MIN as u32, 1);
            cmos.write(CMOS_DATA, enc(min) as u32, 1);

            let hour_reg = if is_24hour {
                enc(hour24)
            } else {
                enc(hour24 - 12) | 0x80 // 15:00 -> 3 PM
            };
            cmos.write(CMOS_ADDR, REG_HOUR as u32, 1);
            cmos.write(CMOS_DATA, hour_reg as u32, 1);

            cmos.write(CMOS_ADDR, REG_MONTH_DAY as u32, 1);
            cmos.write(CMOS_DATA, enc(mday) as u32, 1);
            cmos.write(CMOS_ADDR, REG_MONTH as u32, 1);
            cmos.write(CMOS_DATA, enc(month) as u32, 1);
            cmos.write(CMOS_ADDR, REG_YEAR as u32, 1);
            cmos.write(CMOS_DATA, enc(year2) as u32, 1);
            cmos.write(CMOS_ADDR, REG_CENTURY as u32, 1);
            cmos.write(CMOS_DATA, enc(century) as u32, 1);

            // Exit SET mode -> update_timeval() applies the registers.
            cmos.write(CMOS_ADDR, REG_STAT_B as u32, 1);
            cmos.write(CMOS_DATA, stat_b as u32, 1);

            // Round trip: registers -> timeval -> registers.
            cmos.update_clock();

            assert_eq!(
                bcd_to_bin(cmos.ram[REG_SEC as usize], is_binary),
                sec,
                "sec (binary={is_binary} 24h={is_24hour})"
            );
            assert_eq!(
                bcd_to_bin(cmos.ram[REG_MIN as usize], is_binary),
                min,
                "min (binary={is_binary} 24h={is_24hour})"
            );
            assert_eq!(
                bcd_to_bin(cmos.ram[REG_MONTH_DAY as usize], is_binary),
                mday,
                "mday (binary={is_binary} 24h={is_24hour})"
            );
            assert_eq!(
                bcd_to_bin(cmos.ram[REG_MONTH as usize], is_binary),
                month,
                "month (binary={is_binary} 24h={is_24hour})"
            );
            assert_eq!(
                bcd_to_bin(cmos.ram[REG_YEAR as usize], is_binary),
                year2,
                "year2 (binary={is_binary} 24h={is_24hour})"
            );
            assert_eq!(
                bcd_to_bin(cmos.ram[REG_CENTURY as usize], is_binary),
                century,
                "century (binary={is_binary} 24h={is_24hour})"
            );

            if is_24hour {
                assert_eq!(
                    bcd_to_bin(cmos.ram[REG_HOUR as usize], is_binary),
                    hour24,
                    "hour24 (binary={is_binary})"
                );
            } else {
                let raw = cmos.ram[REG_HOUR as usize];
                assert_ne!(raw & 0x80, 0, "expected PM flag (binary={is_binary})");
                assert_eq!(
                    bcd_to_bin(raw & 0x7F, is_binary),
                    hour24 - 12,
                    "hour12 (binary={is_binary})"
                );
            }
        }
    }

    // =========================================================================
    // Finding #19 — reset() masks CRB + restarts periodic; STAT_A rewrite
    // restarts timer
    // =========================================================================

    #[test]
    fn cmos_reset_masks_stat_b_and_clears_stat_c() {
        let mut cmos = BxCmosC::new();
        // Set UIE(0x10)/AIE(0x20)/PIE(0x40) plus a bit outside that mask,
        // and dirty Status C.
        cmos.ram[REG_STAT_B as usize] = 0xFF & !0x08; // all bits except sq-wave
        cmos.ram[REG_STAT_C as usize] = 0xF0;

        cmos.reset();

        // Bochs cmos.cc reset: `reg[REG_STAT_B] &= 0x8f;` clears bits 4-6.
        assert_eq!(cmos.ram[REG_STAT_B as usize] & 0x70, 0);
        // Bits 7,3,2,1,0 must be preserved.
        assert_eq!(cmos.ram[REG_STAT_B as usize] & 0x8F, 0xFF & !0x08 & 0x8F);
        assert_eq!(cmos.ram[REG_STAT_C as usize], 0x00);
    }

    #[test]
    fn cmos_stat_a_write_restarts_periodic_timer() {
        let mut cmos = BxCmosC::new();
        // Enable PIE with a periodic rate already active.
        cmos.write(CMOS_ADDR, REG_STAT_B as u32, 1);
        cmos.write(CMOS_DATA, 0x42, 1); // 24-hour + PIE

        let interval = cmos.periodic_interval_usec;
        assert!(cmos.periodic_timer_remaining > 0);

        // Burn most of the countdown down.
        cmos.periodic_timer_remaining = 1;

        // Bochs cmos.cc write REG_STAT_A: CRA_change() always re-activates
        // (restarts) the timer, even though the rate nibble is unchanged.
        cmos.write(CMOS_ADDR, REG_STAT_A as u32, 1);
        let stat_a = cmos.ram[REG_STAT_A as usize];
        cmos.write(CMOS_DATA, stat_a as u32, 1);

        assert_eq!(cmos.periodic_timer_remaining, interval);
    }

    // =========================================================================
    // Finding #33 (cmos.rs-local parts) — UF/AF gated on enables, no
    // auto-checksum on I/O writes, saturating one-second reload
    // =========================================================================

    #[test]
    fn cmos_update_ended_flag_gated_on_uie() {
        let mut cmos = BxCmosC::new();
        // UIE (Status B bit 4) left clear.
        assert_eq!(cmos.ram[REG_STAT_B as usize] & 0x10, 0);

        cmos.tick(1_000_001); // fires one_second_timer -> schedules UIP
        cmos.tick(300); // fires the 244us UIP timer -> uip_timer()

        // UF (Status C bit 4) must NOT be set when UIE is disabled.
        assert_eq!(cmos.ram[REG_STAT_C as usize] & 0x10, 0);
    }

    #[test]
    fn cmos_update_ended_flag_set_when_uie_enabled() {
        let mut cmos = BxCmosC::new();
        cmos.write(CMOS_ADDR, REG_STAT_B as u32, 1);
        cmos.write(CMOS_DATA, 0x12, 1); // 24-hour + UIE

        cmos.tick(1_000_001);
        cmos.tick(300);

        assert_ne!(cmos.ram[REG_STAT_C as usize] & 0x10, 0);
    }

    #[test]
    fn cmos_write_does_not_auto_recompute_checksum() {
        let mut cmos = BxCmosC::new();
        let good_high = cmos.ram[REG_CSUM_HIGH as usize];
        let good_low = cmos.ram[REG_CSUM_LOW as usize];

        // Write into the checksummed region (0x10..0x2E) via the I/O port.
        cmos.write(CMOS_ADDR, 0x15, 1);
        cmos.write(CMOS_DATA, 0xAB, 1);

        // Bochs cmos.cc never recomputes the checksum from an I/O write.
        assert_eq!(cmos.ram[REG_CSUM_HIGH as usize], good_high);
        assert_eq!(cmos.ram[REG_CSUM_LOW as usize], good_low);
    }

    #[test]
    fn cmos_one_second_tick_overshoot_does_not_underflow() {
        let mut cmos = BxCmosC::new();
        let before = cmos.timeval;

        // A single tick spanning several seconds must not panic (u32
        // underflow in the old unguarded `1_000_000 - overshoot`).
        cmos.tick(5_000_000);

        assert_eq!(cmos.timeval, before + 1); // fires once per tick() call
        assert!(cmos.one_second_remaining > 0);
        assert!(cmos.one_second_remaining <= 1_000_000);
    }
}
