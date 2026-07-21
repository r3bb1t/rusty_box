#![allow(private_interfaces, unused_assignments, dead_code)]


//! 8254 PIT (Programmable Interval Timer) Emulation
//!
//! Based on Bochs pit82c54.cc (counter state machines) and pit.cc (port
//! handlers, IRQ0 wiring, speaker state). The 8254 PIT provides three
//! independent 16-bit counters:
//! - Counter 0: System timer (IRQ0) - ~18.2 Hz for DOS tick
//! - Counter 1: DRAM refresh (legacy, not used)
//! - Counter 2: Speaker/beep control
//!
//! Base frequency: 1.193181 MHz (Bochs pit.cc TICKS_PER_SECOND)
//!
//! Port 0x61 (System Control Port B) is owned by the PIT, exactly as in
//! Bochs (pit.cc bx_pit_c::init registers 0x0061): bit 0 = counter 2 GATE,
//! bit 1 = speaker data enable, bit 4 = refresh clock divided by 2 (derived
//! from the microsecond clock), bit 5 = counter 2 OUT.

#[cfg(feature = "std")]
use std::io::{self, ErrorKind, Read, Write};

#[cfg(feature = "std")]
use crate::snapshot::{
    bounds, checked_snapshot_len_add, checked_snapshot_len_mul, SnapshotReader,
    SnapshotWriteExt, SNAPSHOT_SECTION_VERSION,
};

/// PIT I/O port addresses
pub const PIT_COUNTER0: u16 = 0x0040;
pub const PIT_COUNTER1: u16 = 0x0041;
pub const PIT_COUNTER2: u16 = 0x0042;
pub const PIT_CONTROL: u16 = 0x0043;
/// System Control Port B — registered by the PIT (Bochs pit.cc init)
pub const PIT_SYSTEM_CONTROL_B: u16 = 0x0061;

/// PIT input clock ticks per second — Bochs pit.cc TICKS_PER_SECOND
/// ("1.193181MHz Clock"). Note: NOT 1193182; Bochs uses 1193181 and all
/// tick/usec conversions must match it.
pub const TICKS_PER_SECOND: u32 = 1_193_181;

/// PIT base frequency in Hz (alias of the Bochs tick rate)
pub const PIT_FREQUENCY: u32 = TICKS_PER_SECOND;

/// Microseconds per second — Bochs pit.cc USEC_PER_SECOND
pub const USEC_PER_SECOND: u32 = 1_000_000;
/// Data returned to the scheduler after a PIT owner timer callback.
///
/// The PIC is intentionally not borrowed here: the owner dispatcher replays
/// the recorded IRQ0 transitions after releasing the PIT borrow, then uses
/// `rearm_usec` to schedule the next one-shot owner deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PitTimerCallback {
    pub(crate) irq0_transitions: u32,
    pub(crate) irq0_level: bool,
    pub(crate) rearm_usec: Option<u64>,
}

/// Counter-derived state that the machine applies only after all snapshot
/// sections have validated.  In particular, `irq0_level` is a level baseline,
/// never a request to replay saved transitions.
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PitSnapshotRestoreState {
    pub(crate) timer_handle: Option<usize>,
    pub(crate) irq0_level: bool,
}


/// Number of PIT counters
const PIT_NUM_COUNTERS: usize = 3;

// ---- Read/write state machine (Bochs pit82c54.h rw_status) ----
#[derive(Debug, Clone, Copy, PartialEq)]
enum RWState {
    LsByte = 0,
    MsByte = 1,
    LsByteMultiple = 2,
    MsByteMultiple = 3,
}

/// State for a single PIT counter — matches Bochs pit82c54.h counter_type
#[derive(Debug, Clone)]
pub struct PitCounter {
    // ---- Bochs counter_type fields (pit82c54.h) ----
    /// Operating mode (0-5, with 6→2, 7→3 aliasing)
    pub(crate) mode: u8,
    /// Input latch (pending count value written by CPU, loaded into count on clock)
    pub(crate) inlatch: u16,
    /// Current count register
    pub(crate) count: u16,
    /// Binary representation of count (same as count when bcd_mode=false)
    pub(crate) count_binary: u16,
    /// Output latch (for latched reads)
    pub(crate) outlatch: u16,
    /// Read/write mode (1=LSB, 2=MSB, 3=LSB then MSB)
    pub(crate) rw_mode: u8,
    /// Read state machine
    pub(crate) read_state: RWState,
    /// Write state machine
    pub(crate) write_state: RWState,
    /// LSB count latched for reading
    pub(crate) count_lsb_latched: bool,
    /// MSB count latched for reading
    pub(crate) count_msb_latched: bool,
    /// Status latched for reading
    pub(crate) status_latched: bool,
    /// Latched status value (Bochs: status_latch)
    pub(crate) latched_status: u8,
    /// Null count (count not yet loaded into CE from CR)
    pub(crate) null_count: bool,
    /// Gate input pin
    pub(crate) gate: bool,
    /// Output pin state
    pub(crate) output: bool,
    /// GATE rising-edge trigger detected (Bochs: triggerGATE)
    pub(crate) trigger_gate: bool,
    /// BCD mode (false = binary)
    pub(crate) bcd_mode: bool,
    /// Count has been fully written (both bytes for 16-bit mode)
    /// Gates ALL counter behavior in clock() — Bochs: count_written
    pub(crate) count_written: bool,
    /// First pass after count load — distinguishes reload from counting
    /// Bochs: first_pass
    pub(crate) first_pass: bool,
    /// State bits for mode 3 square wave (Bochs: state_bit_1, state_bit_2)
    pub(crate) state_bit_1: bool,
    pub(crate) state_bit_2: bool,
    /// Next change time (for scheduling optimization, 0 = no change expected)
    pub(crate) next_change_time: u32,
    /// Whether an OUT handler is attached (Bochs: out_handler != NULL).
    /// Bochs pit.cc attaches irq_handler to counter 0 and speaker_handler
    /// to counter 2; rusty_box has no host audio backend, so only counter 0
    /// carries a handler (counter 2's Bochs handler only drives
    /// DEV_speaker_set_line, which has no register-visible state).
    pub(crate) out_handler_attached: bool,
    /// Number of OUT pin transitions since the last drain. Bochs
    /// pit82c54.cc set_OUT invokes out_handler synchronously on every
    /// actual transition; rusty_box records them here and the
    /// DeviceManager replays them into the PIC (see
    /// DeviceManager::service_pit_irq0). Transitions strictly alternate,
    /// so (count, current OUT level) reconstructs the exact sequence.
    pub(crate) out_transitions: u32,
}

impl Default for PitCounter {
    /// Default matching Bochs pit82c54::init() (pit82c54.cc).
    fn default() -> Self {
        Self {
            mode: 4, // Bochs: mode=4 (SoftwareStrobe)
            inlatch: 0,
            count: 0,
            count_binary: 0,
            outlatch: 0,
            rw_mode: 1,                   // Bochs: rw_mode=1 (LSByte)
            read_state: RWState::LsByte,  // Bochs: read_state=LSByte
            write_state: RWState::LsByte, // Bochs: write_state=LSByte
            count_lsb_latched: false,
            count_msb_latched: false,
            status_latched: false,
            latched_status: 0,
            null_count: false,   // Bochs: null_count=0
            gate: true,          // Bochs: GATE=1
            output: true,        // Bochs: OUTpin=1
            trigger_gate: false, // Bochs: triggerGATE=0
            bcd_mode: false,
            count_written: true, // Bochs: count_written=1
            first_pass: false,   // Bochs: first_pass=0
            state_bit_1: false,
            state_bit_2: false,
            next_change_time: 0,
            out_handler_attached: false, // Bochs: out_handler=NULL
            out_transitions: 0,
        }
    }
}

impl PitCounter {
    /// Create a new counter. Bochs pit82c54::init() sets GATE=1 for all 3 counters.
    pub fn new(_number: u8) -> Self {
        Self::default()
    }

    /// Bochs pit82c54.cc set_OUT — updates the pin only on an actual
    /// transition and invokes the OUT handler on it. rusty_box records the
    /// transition for a later synchronous replay into the PIC (the CPU never
    /// executes between the transition and the replay, so the observable
    /// ordering matches Bochs's immediate callback).
    fn set_out(&mut self, data: bool) {
        if self.output != data {
            self.output = data;
            if self.out_handler_attached {
                self.out_transitions = self.out_transitions.saturating_add(1);
            }
        }
    }

    /// Take (and clear) the pending OUT transition count.
    fn take_out_transitions(&mut self) -> u32 {
        core::mem::take(&mut self.out_transitions)
    }

    /// Bochs pit82c54.cc set_count
    fn set_count(&mut self, data: u16) {
        self.count = data;
        self.set_binary_to_count();
    }

    /// Bochs pit82c54.cc set_binary_to_count (count → count_binary)
    fn set_binary_to_count(&mut self) {
        if self.bcd_mode {
            self.count_binary = (self.count & 0xF)
                + (10 * ((self.count >> 4) & 0xF))
                + (100 * ((self.count >> 8) & 0xF))
                + (1000 * ((self.count >> 12) & 0xF));
        } else {
            self.count_binary = self.count;
        }
    }

    /// Bochs pit82c54.cc set_count_to_binary (count_binary → count)
    fn set_count_to_binary(&mut self) {
        if self.bcd_mode {
            self.count = (self.count_binary % 10)
                | (((self.count_binary / 10) % 10) << 4)
                | (((self.count_binary / 100) % 10) << 8)
                | (((self.count_binary / 1000) % 10) << 12);
        } else {
            self.count = self.count_binary;
        }
    }

    /// Bochs pit82c54.cc decrement
    fn decrement(&mut self) {
        if self.count == 0 {
            if self.bcd_mode {
                self.count = 0x9999;
                self.count_binary = 9999;
            } else {
                self.count = 0xFFFF;
                self.count_binary = 0xFFFF;
            }
        } else {
            self.count_binary = self.count_binary.wrapping_sub(1);
            self.set_count_to_binary();
        }
    }

    /// Bochs pit82c54.cc decrement_multiple — bulk decrement with wrap
    /// handling; works for both binary and BCD counters.
    fn decrement_multiple(&mut self, mut cycles: u32) {
        while cycles > 0 {
            if cycles <= self.count_binary as u32 {
                self.count_binary -= cycles as u16;
                cycles = 0;
                self.set_count_to_binary();
            } else {
                cycles -= self.count_binary as u32 + 1;
                self.count_binary = 0;
                self.set_count_to_binary();
                self.decrement();
            }
        }
    }

    /// Bochs pit82c54.cc clock_multiple — advance the counter by `cycles`
    /// input clocks, bulk-decrementing between scheduled state changes and
    /// running the full per-clock state machine (clock()) exactly at each
    /// boundary. Mode 3 decrements by 2 per cycle (square-wave halves).
    fn clock_multiple(&mut self, mut cycles: u32) {
        while cycles > 0 {
            if self.next_change_time == 0 {
                if self.count_written {
                    match self.mode {
                        0 => {
                            if self.gate && self.write_state != RWState::MsByteMultiple {
                                self.decrement_multiple(cycles);
                            }
                        }
                        1 => self.decrement_multiple(cycles),
                        2 => {
                            if !self.first_pass && self.gate {
                                self.decrement_multiple(cycles);
                            }
                        }
                        3 => {
                            if !self.first_pass && self.gate {
                                self.decrement_multiple(2 * cycles);
                            }
                        }
                        4 => {
                            if self.gate {
                                self.decrement_multiple(cycles);
                            }
                        }
                        5 => self.decrement_multiple(cycles),
                        _ => {}
                    }
                }
                cycles = 0;
            } else {
                match self.mode {
                    0 | 1 | 2 | 4 | 5 => {
                        if self.next_change_time > cycles {
                            self.decrement_multiple(cycles);
                            self.next_change_time -= cycles;
                            cycles = 0;
                        } else {
                            self.decrement_multiple(self.next_change_time - 1);
                            cycles -= self.next_change_time;
                            self.clock();
                        }
                    }
                    3 => {
                        if self.next_change_time > cycles {
                            self.decrement_multiple(cycles * 2);
                            self.next_change_time -= cycles;
                            cycles = 0;
                        } else {
                            self.decrement_multiple((self.next_change_time - 1) * 2);
                            cycles -= self.next_change_time;
                            self.clock();
                        }
                    }
                    _ => {
                        cycles = 0;
                    }
                }
            }
        }
    }

    /// Latch the current count value — Bochs pit82c54.cc latch_counter
    pub fn latch_count(&mut self) {
        if self.count_lsb_latched || self.count_msb_latched {
            // Previous latch not yet read — do nothing
            return;
        }
        match self.read_state {
            RWState::MsByte => {
                self.outlatch = self.count;
                self.count_msb_latched = true;
            }
            RWState::LsByte => {
                self.outlatch = self.count;
                self.count_lsb_latched = true;
            }
            RWState::LsByteMultiple => {
                self.outlatch = self.count;
                self.count_lsb_latched = true;
                self.count_msb_latched = true;
            }
            RWState::MsByteMultiple => {
                // Latching during 2-part read — reset to LSB first
                // (Bochs pit82c54.cc latch_counter "UNL_2P_READ" guess)
                self.read_state = RWState::LsByteMultiple;
                self.outlatch = self.count;
                self.count_lsb_latched = true;
                self.count_msb_latched = true;
            }
        }
    }

    /// Latch the status register — Bochs pit82c54.cc write (READ_BACK)
    pub fn latch_status(&mut self) {
        if !self.status_latched {
            self.latched_status = ((self.output as u8) << 7)
                | ((self.null_count as u8) << 6)
                | ((self.rw_mode & 0x3) << 4)
                | ((self.mode & 0x7) << 1)
                | (self.bcd_mode as u8);
            self.status_latched = true;
        }
    }

    /// Read counter — Bochs pit82c54.cc read
    pub fn read(&mut self) -> u8 {
        if self.status_latched {
            // Bochs pit82c54.cc read: "Undefined output when status latched
            // and count half read" — Bochs BX_ERRORs and falls through to
            // the trailing `return 0` WITHOUT clearing any latch state, so
            // this configuration keeps returning 0 until the counter is
            // reprogrammed. Reproduced Bochs quirk (iodev parity audit #32).
            if self.count_msb_latched && self.read_state == RWState::MsByteMultiple {
                return 0;
            }
            self.status_latched = false;
            return self.latched_status;
        }

        // Latched count read — Bochs advances the two-part read_state even
        // when reading from the latch.
        if self.count_lsb_latched {
            if self.read_state == RWState::LsByteMultiple {
                self.read_state = RWState::MsByteMultiple;
            }
            self.count_lsb_latched = false;
            return (self.outlatch & 0xFF) as u8;
        }
        if self.count_msb_latched {
            if self.read_state == RWState::MsByteMultiple {
                self.read_state = RWState::LsByteMultiple;
            }
            self.count_msb_latched = false;
            return (self.outlatch >> 8) as u8;
        }

        // Unlatched read — read directly from count register
        match self.read_state {
            RWState::LsByte => (self.count & 0xFF) as u8,
            RWState::MsByte => (self.count >> 8) as u8,
            RWState::LsByteMultiple => {
                self.read_state = RWState::MsByteMultiple;
                (self.count & 0xFF) as u8
            }
            RWState::MsByteMultiple => {
                self.read_state = RWState::LsByteMultiple;
                (self.count >> 8) as u8
            }
        }
    }

    /// Write counter initial value — Bochs pit82c54.cc write
    pub fn write(&mut self, data: u8) {
        match self.write_state {
            RWState::LsByteMultiple => {
                self.inlatch = data as u16;
                self.write_state = RWState::MsByteMultiple;
                self.count_written = false;
            }
            RWState::LsByte => {
                self.inlatch = data as u16;
                self.count_written = true;
            }
            RWState::MsByteMultiple => {
                self.write_state = RWState::LsByteMultiple;
                self.inlatch |= (data as u16) << 8;
                self.count_written = true;
            }
            RWState::MsByte => {
                self.inlatch = (data as u16) << 8;
                self.count_written = true;
            }
        }

        // Bochs pit82c54.cc write
        if self.count_written {
            self.null_count = true;
            self.set_count(self.inlatch);
        }

        // Mode-specific actions after count write (Bochs pit82c54.cc write)
        match self.mode {
            0 => {
                if self.count_written {
                    self.set_out(false);
                }
                self.next_change_time = 1;
            }
            1 if self.trigger_gate => {
                self.next_change_time = 1;
            }
            2 | 6 => {
                self.next_change_time = 1;
            }
            3 | 7 => {
                self.next_change_time = 1;
            }
            4 => {
                self.next_change_time = 1;
            }
            5 if self.trigger_gate => {
                self.next_change_time = 1;
            }
            _ => {}
        }
    }

    /// Clock the counter by one tick — Bochs pit82c54.cc clock.
    /// OUT pin transitions are recorded via set_out (Bochs out_handler).
    pub fn clock(&mut self) {
        match self.mode {
            // ---- Mode 0: Interrupt on Terminal Count (Bochs pit82c54.cc clock) ----
            0 => {
                if self.count_written {
                    if self.null_count {
                        self.set_count(self.inlatch);
                        if self.gate {
                            if self.count_binary == 0 {
                                self.next_change_time = 1;
                            } else {
                                self.next_change_time = self.count_binary as u32;
                            }
                        } else {
                            self.next_change_time = 0;
                        }
                        self.null_count = false;
                    } else {
                        // Bochs: GATE && write_state != MSByte_multiple
                        if self.gate && self.write_state != RWState::MsByteMultiple {
                            self.decrement();
                            if !self.output {
                                // OUTpin is LOW — count toward terminal count
                                self.next_change_time = self.count_binary as u32;
                                if self.count == 0 {
                                    self.set_out(true);
                                }
                            } else {
                                // OUTpin already HIGH — nothing to do
                                self.next_change_time = 0;
                            }
                        } else {
                            self.next_change_time = 0; // clock isn't moving
                        }
                    }
                } else {
                    self.next_change_time = 0; // default to 0
                }
                self.trigger_gate = false;
            }

            // ---- Mode 1: Hardware Retriggerable One-Shot (Bochs pit82c54.cc clock) ----
            1 => {
                if self.count_written {
                    if self.trigger_gate {
                        self.set_count(self.inlatch);
                        if self.count_binary == 0 {
                            self.next_change_time = 1;
                        } else {
                            self.next_change_time = self.count_binary as u32;
                        }
                        self.null_count = false;
                        self.set_out(false);
                    } else {
                        self.decrement();
                        if !self.output {
                            if self.count_binary == 0 {
                                self.next_change_time = 1;
                            } else {
                                self.next_change_time = self.count_binary as u32;
                            }
                            if self.count == 0 {
                                self.set_out(true);
                            }
                        } else {
                            self.next_change_time = 0;
                        }
                    }
                } else {
                    self.next_change_time = 0;
                }
                self.trigger_gate = false;
            }

            // ---- Mode 2: Rate Generator (Bochs pit82c54.cc clock) ----
            2 | 6 => {
                if self.count_written {
                    if self.trigger_gate || self.first_pass {
                        // RELOAD phase: load count, set output HIGH
                        self.set_count(self.inlatch);
                        self.next_change_time = self.count_binary.wrapping_sub(1) as u32;
                        self.null_count = false;
                        if !self.output {
                            self.set_out(true);
                        }
                        self.first_pass = false;
                    } else {
                        // COUNTING phase
                        if self.gate {
                            self.decrement();
                            self.next_change_time = self.count_binary.wrapping_sub(1) as u32;
                            if self.count == 1 {
                                // Terminal: pulse LOW, schedule reload
                                self.next_change_time = 1;
                                self.set_out(false);
                                self.first_pass = true;
                            }
                        } else {
                            self.next_change_time = 0;
                        }
                    }
                } else {
                    self.next_change_time = 0;
                }
                self.trigger_gate = false;
            }

            // ---- Mode 3: Square Wave Generator (Bochs pit82c54.cc clock) ----
            3 | 7 => {
                if self.count_written {
                    if (self.trigger_gate || self.first_pass || self.state_bit_2) && self.gate {
                        self.set_count(self.inlatch & 0xFFFE);
                        self.state_bit_1 = (self.inlatch & 0x1) != 0;
                        if !self.output || !self.state_bit_1 {
                            // Bochs pit82c54.cc clock (mode 3):
                            // ((count_binary/2)-1) computed in Bit32u — a
                            // reloaded count of 0 underflows to 0xFFFFFFFF
                            // and masks to 0xFFFF, scheduling the next OUT
                            // toggle 65535 ticks out instead of 32767, so a
                            // count of 0 yields ~9.1 Hz instead of the real
                            // hardware's 18.2 Hz. Reproduced Bochs quirk
                            // (iodev parity audit #32, parity ruling: match
                            // Bochs exactly).
                            let half_minus_1 = (self.count_binary as u32 / 2).wrapping_sub(1);
                            if half_minus_1 == 0 {
                                self.next_change_time = 1;
                            } else {
                                self.next_change_time = half_minus_1 & 0xFFFF;
                            }
                        } else {
                            let half = self.count_binary as u32 / 2;
                            if half == 0 {
                                self.next_change_time = 1;
                            } else {
                                self.next_change_time = half & 0xFFFF;
                            }
                        }
                        self.null_count = false;
                        if !self.output {
                            self.set_out(true);
                        } else if self.output && !self.first_pass {
                            self.set_out(false);
                        }
                        self.state_bit_2 = false;
                        self.first_pass = false;
                    } else {
                        if self.gate {
                            self.decrement();
                            self.decrement();
                            if !self.output || !self.state_bit_1 {
                                self.next_change_time =
                                    (self.count_binary as u32 / 2).wrapping_sub(1) & 0xFFFF;
                            } else {
                                self.next_change_time = (self.count_binary as u32 / 2) & 0xFFFF;
                            }
                            if self.count == 0 {
                                self.state_bit_2 = true;
                                self.next_change_time = 1;
                            }
                            if self.count == 2 && (!self.output || !self.state_bit_1) {
                                self.state_bit_2 = true;
                                self.next_change_time = 1;
                            }
                        } else {
                            self.next_change_time = 0;
                        }
                    }
                } else {
                    self.next_change_time = 0;
                }
                self.trigger_gate = false;
            }

            // ---- Mode 4: Software Triggered Strobe (Bochs pit82c54.cc clock) ----
            4 => {
                if self.count_written {
                    if !self.output {
                        self.set_out(true);
                    }
                    if self.null_count {
                        self.set_count(self.inlatch);
                        if self.gate {
                            if self.count_binary == 0 {
                                self.next_change_time = 1;
                            } else {
                                self.next_change_time = self.count_binary as u32;
                            }
                        } else {
                            self.next_change_time = 0;
                        }
                        self.null_count = false;
                        self.first_pass = true;
                    } else {
                        if self.gate {
                            self.decrement();
                            if self.first_pass {
                                self.next_change_time = self.count_binary as u32;
                                if self.count == 0 {
                                    self.set_out(false);
                                    self.next_change_time = 1;
                                    self.first_pass = false;
                                }
                            } else {
                                self.next_change_time = 0;
                            }
                        } else {
                            self.next_change_time = 0;
                        }
                    }
                } else {
                    self.next_change_time = 0;
                }
                self.trigger_gate = false;
            }

            // ---- Mode 5: Hardware Triggered Strobe (Bochs pit82c54.cc clock) ----
            5 => {
                if self.count_written {
                    if !self.output {
                        self.set_out(true);
                    }
                    if self.trigger_gate {
                        self.set_count(self.inlatch);
                        if self.count_binary == 0 {
                            self.next_change_time = 1;
                        } else {
                            self.next_change_time = self.count_binary as u32;
                        }
                        self.null_count = false;
                        self.first_pass = true;
                    } else {
                        self.decrement();
                        if self.first_pass {
                            self.next_change_time = self.count_binary as u32;
                            if self.count == 0 {
                                self.set_out(false);
                                self.next_change_time = 1;
                                self.first_pass = false;
                            }
                        } else {
                            self.next_change_time = 0;
                        }
                    }
                } else {
                    self.next_change_time = 0;
                }
                self.trigger_gate = false;
            }

            _ => {
                // Bochs pit82c54.cc clock default: "Mode not implemented."
                self.next_change_time = 0;
                self.trigger_gate = false;
            }
        }
    }

    /// Set GATE input — Bochs pit82c54.cc set_GATE
    /// Detects rising edge and sets triggerGATE; mode-specific behavior
    pub fn set_gate(&mut self, data: bool) {
        let old_gate = self.gate;
        // Only process on actual change (Bochs pit82c54.cc set_GATE)
        if old_gate == data {
            return;
        }

        self.gate = data;
        if data {
            self.trigger_gate = true; // Rising edge detected
        }

        match self.mode {
            0 => {
                if data && self.count_written {
                    if self.null_count {
                        self.next_change_time = 1;
                    } else if !self.output && self.write_state != RWState::MsByteMultiple {
                        if self.count_binary == 0 {
                            self.next_change_time = 1;
                        } else {
                            self.next_change_time = self.count_binary as u32;
                        }
                    } else {
                        self.next_change_time = 0;
                    }
                } else if self.null_count {
                    self.next_change_time = 1;
                } else {
                    self.next_change_time = 0;
                }
            }
            1 if data && self.count_written => {
                self.next_change_time = 1;
            }
            2 | 6 => {
                if !data {
                    // GATE dropped LOW: force output HIGH, stop counting
                    self.set_out(true);
                    self.next_change_time = 0;
                } else if self.count_written {
                    self.next_change_time = 1;
                } else {
                    self.next_change_time = 0;
                }
            }
            3 | 7 => {
                if !data {
                    self.set_out(true);
                    self.first_pass = true;
                    self.next_change_time = 0;
                } else if self.count_written {
                    self.next_change_time = 1;
                } else {
                    self.next_change_time = 0;
                }
            }
            4 => {
                if !self.output || self.null_count {
                    self.next_change_time = 1;
                } else if data && self.count_written {
                    if self.first_pass {
                        if self.count_binary == 0 {
                            self.next_change_time = 1;
                        } else {
                            self.next_change_time = self.count_binary as u32;
                        }
                    } else {
                        self.next_change_time = 0;
                    }
                } else {
                    self.next_change_time = 0;
                }
            }
            5 if data && self.count_written => {
                self.next_change_time = 1;
            }
            _ => {}
        }
    }
}

/// 8254 PIT Controller
#[derive(Debug)]
pub struct BxPitC {
    /// Three counters
    pub(crate) counters: [PitCounter; 3],
    /// Total ticks elapsed
    pub(crate) total_ticks: u64,
    /// One-shot PC-system owner timer handle for all PIT counter events.
    pub(crate) timer_handle: Option<usize>,
    /// Bochs pit.cc s.speaker_data_on — port 0x61 bit 1 (write) / bit 1 (read)
    pub(crate) speaker_data_on: bool,
    /// Bochs pit.cc s.speaker_active — tracks (port 0x61 & 3) == 3 while
    /// counter 2 is in mode 3 (beep on/off state)
    pub(crate) speaker_active: bool,
    /// Bochs pit.cc s.speaker_level — speaker line level for non-mode-3
    /// counter 2 operation (updated on port 0x61 writes)
    pub(crate) speaker_level: bool,
    /// IPS (instructions per second) for converting icount to PIT ticks.
    ips: u64,
    /// icount value at last PIT synchronization point.
    icount_at_last_sync: u64,
    /// Monotonic emulated-microsecond clock — the Bochs bx_virt_timer
    /// time_usec() equivalent. All counter movement is quantized to whole
    /// microseconds of this clock, exactly like Bochs pit.cc periodic();
    /// the port 0x61 refresh bit reads it directly.
    total_usec: u64,
    /// Sub-microsecond remainder of the icount→usec conversion
    /// (units: instruction_count * USEC_PER_SECOND, modulo ips). Carries
    /// fractions across sync points so no emulated time is lost.
    usec_remainder: u128,
    /// Sub-tick remainder of the usec→tick conversion — the integer-exact
    /// equivalent of Bochs pit.cc periodic()'s
    /// `USEC_TO_TICKS(total_usec) - total_ticks` derivation.
    pit_usec_accumulator: u128,
    /// Last host timestamp for Bochs-style realtime PIT synchronization.
    #[cfg(feature = "std")]
    realtime_last: Option<std::time::Instant>,
}

impl Default for BxPitC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPitC {
    /// Create a new PIT controller in power-on state (Bochs bx_pit_c
    /// constructor + init()).
    pub fn new() -> Self {
        let mut pit = Self {
            counters: [PitCounter::new(0), PitCounter::new(1), PitCounter::new(2)],
            total_ticks: 0,
            timer_handle: None,
            speaker_data_on: false,
            speaker_active: false,
            speaker_level: false,
            ips: 0,
            icount_at_last_sync: 0,
            total_usec: 0,
            usec_remainder: 0,
            pit_usec_accumulator: 0,
            #[cfg(feature = "std")]
            realtime_last: None,
        };
        pit.init();
        pit
    }

    /// Power-on initialization — Bochs pit.cc bx_pit_c::init. This is the
    /// ONLY place the counters are programmed to their defaults; a guest
    /// reset (reset()) deliberately leaves all PIT state alone.
    pub fn init(&mut self) {
        tracing::debug!("PIT: Initializing 8254 Programmable Interval Timer");
        // Bochs pit.cc init: speaker state cleared
        self.speaker_data_on = false;
        self.speaker_active = false;
        self.speaker_level = false;
        // Bochs pit.cc init → s.timer.init() (pit82c54.cc init)
        self.counters = [PitCounter::new(0), PitCounter::new(1), PitCounter::new(2)];
        // Bochs pit.cc init: s.timer.set_OUT_handler(0, irq_handler).
        // Counter 2's Bochs handler (speaker_handler) only drives the host
        // audio line (DEV_speaker_set_line) with no register-visible state;
        // rusty_box has no audio backend, so no handler is attached there.
        self.counters[0].out_handler_attached = true;
        self.total_ticks = 0;
        self.total_usec = 0;
        self.icount_at_last_sync = 0;
        self.usec_remainder = 0;
        self.pit_usec_accumulator = 0;
        #[cfg(feature = "std")]
        if self.realtime_last.is_some() {
            self.realtime_last = Some(std::time::Instant::now());
        }
    }

    /// Guest reset — Bochs pit.cc bx_pit_c::reset delegates to pit82c54.cc
    /// reset, which is intentionally EMPTY: the counters, speaker state and
    /// time baselines all survive a guest-initiated reset. Only power-on
    /// init() programs the counters.
    pub fn reset(&mut self) {}

    /// Store the scheduler's single PIT owner timer handle.
    pub(crate) fn set_timer_handle(&mut self, handle: usize) {
        self.timer_handle = Some(handle);
    }

    /// Return the scheduler's PIT owner timer handle, if registered.
    pub(crate) fn timer_handle(&self) -> Option<usize> {
        self.timer_handle
    }

    /// Initialize icount synchronization for fine-grained PIT timing.
    /// Stores IPS and the initial icount baseline for port-access syncs and
    /// exact owner callbacks.
    pub fn init_icount_sync(&mut self, icount: u64, ips: u64) {
        self.ips = ips;
        self.icount_at_last_sync = icount;
    }

    /// Enable Bochs-style realtime synchronization for wall-clock PIT users.
    #[cfg(feature = "std")]
    pub fn enable_realtime_sync(&mut self) {
        self.realtime_last = Some(std::time::Instant::now());
        self.usec_remainder = 0;
        self.pit_usec_accumulator = 0;
    }

    /// Bochs pit82c54.cc get_next_event_time — including the Bit32u quirk:
    /// when counter 0's next_change_time is 0, the `time1 < out` / `time2 <
    /// out` comparisons against 0 are never true, so the result is 0
    /// regardless of the other counters.
    fn get_next_event_time(&self) -> u32 {
        let time0 = self.counters[0].next_change_time;
        let time1 = self.counters[1].next_change_time;
        let time2 = self.counters[2].next_change_time;
        let mut out = time0;
        if time1 != 0 && time1 < out {
            out = time1;
        }
        if time2 != 0 && time2 < out {
            out = time2;
        }
        out
    }

    /// Relative microsecond delay for the next PIT state change.
    ///
    /// This is Bochs `TICKS_TO_USEC(get_next_event_time())`, with Bochs's
    /// `BX_MAX(1, ...)` scheduling rule. A zero event time deactivates the
    /// one-shot owner instead of scheduling a spurious poll.
    pub(crate) fn next_event_usec(&self) -> Option<u64> {
        let event_ticks = self.get_next_event_time();
        if event_ticks == 0 {
            return None;
        }

        Some(
            ((u64::from(event_ticks) * u64::from(USEC_PER_SECOND))
                / u64::from(TICKS_PER_SECOND))
            .max(1),
        )
    }

    /// Advance all three counters by `ticks_delta` PIT input clocks —
    /// Bochs pit.cc bx_pit_c::periodic (the ticks loop) + pit82c54.cc
    /// clock_all. Chunks by the next scheduled counter event; Bochs works
    /// in Bit32u, so u64 deltas are additionally chunked to keep mode 3's
    /// `2*cycles` in clock_multiple from overflowing.
    pub(crate) fn clock_pit_ticks(&mut self, mut ticks_delta: u64) {
        const MAX_CHUNK: u64 = 0x3FFF_FFFF;
        while ticks_delta > 0 {
            let maxchange = self.get_next_event_time() as u64;
            let timedelta = if maxchange == 0 || maxchange > ticks_delta {
                ticks_delta.min(MAX_CHUNK)
            } else {
                maxchange
            };
            let cycles = timedelta as u32;
            // Bochs pit82c54.cc clock_all
            self.counters[0].clock_multiple(cycles);
            self.counters[1].clock_multiple(cycles);
            self.counters[2].clock_multiple(cycles);
            self.total_ticks = self.total_ticks.wrapping_add(timedelta);
            ticks_delta -= timedelta;
        }
    }

    /// Advance the PIT by whole emulated microseconds — Bochs pit.cc
    /// bx_pit_c::periodic: `total_usec += delta; ticks_delta =
    /// USEC_TO_TICKS(total_usec) - total_ticks;` (the remainder-carrying
    /// accumulator below yields the identical cumulative integer floors).
    fn advance_by_usec(&mut self, usec: u64) {
        self.total_usec += usec;
        self.pit_usec_accumulator += usec as u128 * TICKS_PER_SECOND as u128;
        let pit_ticks = (self.pit_usec_accumulator / USEC_PER_SECOND as u128) as u64;
        self.pit_usec_accumulator %= USEC_PER_SECOND as u128;
        self.clock_pit_ticks(pit_ticks);
    }

    #[cfg(feature = "std")]
    fn sync_to_realtime(&mut self) {
        let Some(last) = self.realtime_last else {
            return;
        };
        let now = std::time::Instant::now();
        let elapsed = now.saturating_duration_since(last).as_micros() as u64;
        if elapsed == 0 {
            return;
        }
        self.realtime_last = Some(now);
        self.advance_by_usec(elapsed);
    }

    /// Remaining host-time prediction for a realtime PIT owner deadline.
    ///
    /// A PC-system callback may arrive at its virtual prediction before the
    /// wall clock reached the corresponding PIT event. In that case the
    /// dispatcher rearms this delay and does not mutate PIT state.
    pub(crate) fn host_remaining_usec(&self) -> Option<u64> {
        #[cfg(feature = "std")]
        {
            let last = self.realtime_last?;
            let elapsed = std::time::Instant::now()
                .saturating_duration_since(last)
                .as_micros()
                .min(u128::from(u64::MAX)) as u64;
            return self
                .next_event_usec()
                .map(|deadline| deadline.saturating_sub(elapsed));
        }

        #[cfg(not(feature = "std"))]
        {
            None
        }
    }

    /// Monotonic emulated-microsecond clock — the icount/realtime-sync
    /// equivalent of Bochs bx_virt_timer.time_usec() as consumed by pit.cc.
    /// Single conversion from the time source (no tick round-trip), so the
    /// port 0x61 refresh bit `(usec/15)&1` matches Bochs exactly.
    pub(crate) fn time_usec(&self) -> u64 {
        self.total_usec
    }

    /// Synchronize PIT counters to match elapsed CPU time.
    /// Called before counter reads AND writes (Bochs pit.cc bx_pit_c::read
    /// runs handle_timer() and bx_pit_c::write runs periodic(time_passed32)
    /// before touching the counters). Uses a fractional accumulator to
    /// avoid losing ticks when only a few instructions have elapsed between
    /// accesses (~13 instructions per PIT tick at 15M IPS).
    pub fn sync_to_icount(&mut self, icount: u64) {
        #[cfg(feature = "std")]
        if self.realtime_last.is_some() {
            self.sync_to_realtime();
            self.icount_at_last_sync = icount;
            self.usec_remainder = 0;
            return;
        }

        if self.ips == 0 {
            return;
        }

        // Bochs pit.cc keeps a SINGLE time cursor (`s.last_usec`): every sync —
        // the timer callback and every port access — advances the counters to
        // the absolute emulated microsecond, from the same baseline. `total_usec`
        // is that one cursor here. Deriving elapsed from a *separate* icount
        // baseline let the timer-callback path (`sync_to_system_ticks`, which
        // advances `total_usec` absolutely) leave this baseline stale, so the
        // next port access re-applied the whole span — doubling `total_usec` and
        // freezing the PIT until wall time caught up. That freeze is what made
        // Linux `check_timer()` see zero IRQ0 ticks and panic
        // "IO-APIC + timer doesn't work!".
        self.icount_at_last_sync = icount;
        let scaled = u128::from(icount) * u128::from(USEC_PER_SECOND);
        let target_usec = (scaled / u128::from(self.ips)) as u64;
        self.usec_remainder = scaled % u128::from(self.ips);
        if target_usec > self.total_usec {
            self.advance_by_usec(target_usec - self.total_usec);
        }
    }

    /// Synchronize the PIT to an absolute PC-system tick epoch.
    ///
    /// The conversion is cumulative (`floor(system_ticks * 1_000_000 / IPS)`)
    /// like `BxPcSystemC::time_usec_at_ticks`, rather than a per-callback
    /// relative conversion that could lose fractional microseconds.
    pub(crate) fn sync_to_system_ticks(&mut self, system_ticks: u64, ips: u64) {
        #[cfg(feature = "std")]
        if self.realtime_last.is_some() {
            self.sync_to_realtime();
            return;
        }

        if ips == 0 {
            return;
        }

        let target_usec = ((u128::from(system_ticks) * u128::from(USEC_PER_SECOND))
            / u128::from(ips))
        .min(u128::from(u64::MAX)) as u64;
        if target_usec > self.total_usec {
            self.advance_by_usec(target_usec - self.total_usec);
        }
    }

    /// Service the PIT's one-shot owner callback without borrowing the PC
    /// system or PIC. The dispatcher consumes the IRQ replay data and arms
    /// `rearm_usec` as the next absolute owner deadline.
    pub(crate) fn timer_callback(
        &mut self,
        system_ticks: u64,
        ips: u64,
    ) -> PitTimerCallback {
        #[cfg(feature = "std")]
        if self.realtime_last.is_some() {
            if let Some(remaining) = self.host_remaining_usec() {
                if remaining > 0 {
                    return PitTimerCallback {
                        irq0_transitions: 0,
                        irq0_level: self.counters[0].output,
                        rearm_usec: Some(remaining),
                    };
                }
            }
        }

        self.sync_to_system_ticks(system_ticks, ips);
        let (irq0_transitions, irq0_level) = self.drain_irq0_events();
        PitTimerCallback {
            irq0_transitions,
            irq0_level,
            rearm_usec: self.next_event_usec(),
        }
    }

    /// Read from PIT I/O port — Bochs pit.cc bx_pit_c::read
    pub fn read(&mut self, port: u16, _io_len: u8, icount: u64) -> u32 {
        // Bochs pit.cc bx_pit_c::read runs handle_timer() (periodic to
        // "now") before reading any register, so the guest observes the
        // counter state as of the current instruction.
        self.sync_to_icount(icount);
        match port {
            PIT_COUNTER0 => self.counters[0].read() as u32,
            PIT_COUNTER1 => self.counters[1].read() as u32,
            PIT_COUNTER2 => self.counters[2].read() as u32,
            PIT_CONTROL => {
                // Bochs pit.cc read case 0x43 → pit82c54.cc read
                // (CONTROL_ADDRESS): "Read from control word register not
                // defined." — returns 0.
                0
            }
            PIT_SYSTEM_CONTROL_B => {
                // Bochs pit.cc bx_pit_c::read case 0x61 — the value is
                // composed FRESH on every read; bits 2/3/6/7 always read 0:
                //   bit 5 = timer.read_OUT(2)
                //   bit 4 = refresh_clock_div2 = (time_usec / 15) & 1
                //   bit 1 = speaker_data_on
                //   bit 0 = timer.read_GATE(2)
                let refresh_clock_div2 = (self.time_usec() / 15) & 1 != 0;
                ((self.counters[2].output as u32) << 5)
                    | ((refresh_clock_div2 as u32) << 4)
                    | ((self.speaker_data_on as u32) << 1)
                    | (self.counters[2].gate as u32)
            }
            _ => {
                tracing::warn!("PIT: Unknown read port {:#06x}", port);
                0xFF
            }
        }
    }

    /// Write to PIT I/O port — Bochs pit.cc bx_pit_c::write
    pub fn write(&mut self, port: u16, value: u32, _io_len: u8, icount: u64) {
        // Bochs pit.cc bx_pit_c::write: periodic(time_passed32) runs BEFORE
        // s.timer.write(...) — the counters advance to "now" under the OLD
        // programming, then the write is applied. This holds for all of
        // 0x40-0x43 and 0x61.
        self.sync_to_icount(icount);
        let value = value as u8;
        match port {
            PIT_COUNTER0 => self.counters[0].write(value),
            PIT_COUNTER1 => self.counters[1].write(value),
            PIT_COUNTER2 => {
                self.counters[2].write(value);
                // Bochs pit.cc write case 0x42: if speaker_active and
                // counter 2 is in mode 3 with a complete new count,
                // DEV_speaker_beep_on(1193180.0/count) retunes the beep.
                // rusty_box has no host audio backend (iodev parity audit
                // #32), and the retune changes no register-visible state,
                // so nothing further happens here.
            }
            PIT_CONTROL => self.write_control(value),
            PIT_SYSTEM_CONTROL_B => self.write_port61(value),
            _ => {
                tracing::warn!("PIT: Unknown write port {:#06x} value={:#04x}", port, value);
            }
        }
    }

    /// Port 0x61 write — Bochs pit.cc bx_pit_c::write case 0x61
    fn write_port61(&mut self, value: u8) {
        self.counters[2].set_gate(value & 0x01 != 0);
        self.speaker_data_on = (value >> 1) & 0x01 != 0;
        let new_speaker_active = (value & 3) == 3;
        if self.counters[2].mode == 3 {
            if self.speaker_active != new_speaker_active {
                // Bochs pit.cc: DEV_speaker_beep_on(1193180.0/count) /
                // DEV_speaker_beep_off() — the host audio backend is absent
                // in rusty_box, so only the speaker_active state tracking
                // remains (it is what Bochs saves/restores and gates the
                // beep retune on the 0x42 write path).
                self.speaker_active = new_speaker_active;
            }
        } else {
            let new_speaker_level = self.speaker_data_on && self.counters[2].output;
            if self.speaker_level != new_speaker_level {
                // Bochs pit.cc: DEV_speaker_set_line(new_speaker_level) —
                // audio backend absent; state tracking only.
                self.speaker_level = new_speaker_level;
            }
        }
    }

    /// Write to the control register — Bochs pit82c54.cc write (address 3)
    fn write_control(&mut self, value: u8) {
        let sc = (value >> 6) & 0x03;

        if sc == 3 {
            // Read-back command (D7-D6 = 11)
            self.read_back(value);
            return;
        }

        let rw = (value >> 4) & 0x03;

        if rw == 0 {
            // Counter Latch command
            self.counters[sc as usize].latch_count();
            return;
        }

        // Counter Program Command — Bochs pit82c54.cc write
        let m = (value >> 1) & 0x07;
        let bcd = (value & 0x01) != 0;

        let ctr = &mut self.counters[sc as usize];
        ctr.null_count = true;
        ctr.count_lsb_latched = false;
        ctr.count_msb_latched = false;
        ctr.status_latched = false;
        ctr.inlatch = 0;
        ctr.count_written = false;
        ctr.first_pass = true;
        ctr.rw_mode = rw;
        ctr.bcd_mode = bcd;
        ctr.mode = m;
        // Mode aliasing: 6→2, 7→3 (Bochs pit82c54.cc write)
        if ctr.mode > 5 {
            ctr.mode &= 0x3;
        }

        match rw {
            1 => {
                ctr.read_state = RWState::LsByte;
                ctr.write_state = RWState::LsByte;
            }
            2 => {
                ctr.read_state = RWState::MsByte;
                ctr.write_state = RWState::MsByte;
            }
            3 => {
                ctr.read_state = RWState::LsByteMultiple;
                ctr.write_state = RWState::LsByteMultiple;
            }
            _ => {}
        }

        // All modes except mode 0 have initial output of 1 (Bochs
        // pit82c54.cc write). set_out records the transition, so a
        // control-word write that moves counter 0's OUT pin produces an
        // IRQ0 edge exactly like Bochs's set_OUT → out_handler path.
        if m != 0 {
            ctr.set_out(true);
        } else {
            ctr.set_out(false);
        }
        ctr.next_change_time = 0;

        tracing::trace!(
            "PIT: Counter {} configured: mode={}, rw={}, bcd={}",
            sc,
            ctr.mode,
            rw,
            bcd
        );
    }

    /// Handle read-back command — Bochs pit82c54.cc write (READ_BACK)
    fn read_back(&mut self, value: u8) {
        let latch_count = (value & 0x20) == 0; // Bit 5: 0 = latch count
        let latch_status = (value & 0x10) == 0; // Bit 4: 0 = latch status

        for i in 0..PIT_NUM_COUNTERS {
            if (value & (0x02 << i)) != 0 {
                // Bochs order: latch count first, then status
                if latch_count {
                    self.counters[i].latch_count();
                }
                if latch_status {
                    self.counters[i].latch_status();
                }
            }
        }
    }

    /// Set gate input for counter 2 (speaker control) — uses edge-detecting set_gate
    pub fn set_gate2(&mut self, gate: bool) {
        self.counters[2].set_gate(gate);
    }

    /// Get output state of counter 2 (speaker)
    pub fn get_output2(&self) -> bool {
        self.counters[2].output
    }

    /// Take pending counter-0 OUT transitions and the current OUT level.
    ///
    /// Bochs pit.cc bx_pit_c::irq_handler is invoked synchronously on every
    /// counter-0 set_OUT transition (raise_irq(0) on 0→1, lower_irq(0) on
    /// 1→0). rusty_box records the transitions on the counter and the
    /// DeviceManager replays them into the PIC after every PIT-mutating
    /// operation (DeviceManager::service_pit_irq0). Transitions strictly
    /// alternate (set_OUT fires only on a change), so (count, final level)
    /// reconstructs the exact sequence.
    pub fn drain_irq0_events(&mut self) -> (u32, bool) {
        let transitions = self.counters[0].take_out_transitions();
        (transitions, self.counters[0].output)
    }

}

#[cfg(feature = "std")]
impl BxPitC {
    /// Exact length of the versioned PIT v3 section payload.
    pub(crate) fn snapshot_v3_len(&self) -> io::Result<u64> {
        const COUNTER_LEN: u64 = 33;
        if PIT_NUM_COUNTERS > bounds::MAX_SNAPSHOT_COUNT {
            return Err(pit_snapshot_invalid("PIT counter count exceeds implementation bound"));
        }
        let counters = checked_snapshot_len_mul(
            u64::try_from(PIT_NUM_COUNTERS)
                .map_err(|_| pit_snapshot_invalid("PIT counter count does not fit u64"))?,
            COUNTER_LEN,
        )?;
        let len = checked_snapshot_len_add(4, counters)?;
        let len = checked_snapshot_len_add(len, 8)?; // total ticks
        let len = checked_snapshot_len_add(len, 1)?; // timer-handle presence
        let len = if self.timer_handle.is_some() {
            checked_snapshot_len_add(len, 8)?
        } else {
            len
        };
        let len = checked_snapshot_len_add(len, 3)?; // speaker state
        let len = checked_snapshot_len_add(len, 8)?; // icount phase
        let len = checked_snapshot_len_add(len, 8)?; // usec phase
        let len = checked_snapshot_len_add(len, 16)?; // icount remainder
        let len = checked_snapshot_len_add(len, 16)?; // PIT-tick remainder
        checked_snapshot_len_add(len, 1) // realtime topology
    }

    /// Stream all mutable PIT counter and phase state.  Registered handlers,
    /// host-time anchors, and the configured instruction rate remain live.
    pub(crate) fn save_snapshot_v3<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        validate_pit_phase(
            self.ips,
            self.usec_remainder,
            self.pit_usec_accumulator,
        )?;
        writer.write_u32(SNAPSHOT_SECTION_VERSION)?;
        for counter in &self.counters {
            validate_pit_counter(counter)?;
            write_pit_counter(writer, counter)?;
        }
        writer.write_u64(self.total_ticks)?;
        write_snapshot_handle(writer, self.timer_handle)?;
        writer.write_bool(self.speaker_data_on)?;
        writer.write_bool(self.speaker_active)?;
        writer.write_bool(self.speaker_level)?;
        writer.write_u64(self.icount_at_last_sync)?;
        writer.write_u64(self.total_usec)?;
        writer.write_bytes(&self.usec_remainder.to_le_bytes())?;
        writer.write_bytes(&self.pit_usec_accumulator.to_le_bytes())?;
        writer.write_bool(self.realtime_last.is_some())
    }

    /// Restore mutable PIT state without touching its callback topology or
    /// generating IRQ0 edges.  PC-system owner validation is intentionally
    /// deferred until its full timer table has been restored.
    pub(crate) fn restore_snapshot_v3<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<()> {
        if reader.read_u32()? != SNAPSHOT_SECTION_VERSION {
            return Err(pit_snapshot_invalid("unsupported PIT section version"));
        }

        let mut counters = self.counters.clone();
        for counter in &mut counters {
            read_pit_counter(reader, counter)?;
            validate_pit_counter(counter)?;
        }
        let total_ticks = reader.read_u64()?;
        let timer_handle = read_snapshot_handle(reader)?;
        let speaker_data_on = reader.read_bool()?;
        let speaker_active = reader.read_bool()?;
        let speaker_level = reader.read_bool()?;
        let icount_at_last_sync = reader.read_u64()?;
        let total_usec = reader.read_u64()?;
        let mut usec_remainder = [0u8; 16];
        reader.read_bytes(&mut usec_remainder)?;
        let usec_remainder = u128::from_le_bytes(usec_remainder);
        let mut pit_usec_accumulator = [0u8; 16];
        reader.read_bytes(&mut pit_usec_accumulator)?;
        let pit_usec_accumulator = u128::from_le_bytes(pit_usec_accumulator);
        let realtime_sync = reader.read_bool()?;
        if realtime_sync != self.realtime_last.is_some() {
            return Err(pit_snapshot_invalid("PIT realtime configuration does not match"));
        }
        validate_pit_phase(self.ips, usec_remainder, pit_usec_accumulator)?;
        reader.finish_exact()?;

        self.counters = counters;
        self.total_ticks = total_ticks;
        self.timer_handle = timer_handle;
        self.speaker_data_on = speaker_data_on;
        self.speaker_active = speaker_active;
        self.speaker_level = speaker_level;
        self.icount_at_last_sync = icount_at_last_sync;
        self.total_usec = total_usec;
        self.usec_remainder = usec_remainder;
        self.pit_usec_accumulator = pit_usec_accumulator;
        Ok(())
    }

    /// Re-anchor host-only realtime state and report the final IRQ0 level
    /// after every section is restored.  The PC-system deadline already holds
    /// the exact PIT phase, so this deliberately neither arms nor replays it.
    pub(crate) fn post_restore_snapshot_v3(&mut self) -> PitSnapshotRestoreState {
        if self.realtime_last.is_some() {
            self.realtime_last = Some(std::time::Instant::now());
        }
        let [counter0, _, _] = &self.counters;
        PitSnapshotRestoreState {
            timer_handle: self.timer_handle,
            irq0_level: counter0.output,
        }
    }
}

#[cfg(feature = "std")]
fn pit_snapshot_invalid(message: &'static str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
fn write_snapshot_handle<W: Write>(
    writer: &mut W,
    handle: Option<usize>,
) -> io::Result<()> {
    writer.write_bool(handle.is_some())?;
    if let Some(handle) = handle {
        writer.write_u64(
            u64::try_from(handle)
                .map_err(|_| pit_snapshot_invalid("PIT timer handle does not fit u64"))?,
        )?;
    }
    Ok(())
}

#[cfg(feature = "std")]
fn read_snapshot_handle<R: Read>(
    reader: &mut SnapshotReader<R>,
) -> io::Result<Option<usize>> {
    if !reader.read_bool()? {
        return Ok(None);
    }
    usize::try_from(reader.read_u64()?)
        .map(Some)
        .map_err(|_| pit_snapshot_invalid("PIT timer handle does not fit usize"))
}

#[cfg(feature = "std")]
fn write_pit_counter<W: Write>(writer: &mut W, counter: &PitCounter) -> io::Result<()> {
    writer.write_u8(counter.mode)?;
    writer.write_u16(counter.inlatch)?;
    writer.write_u16(counter.count)?;
    writer.write_u16(counter.count_binary)?;
    writer.write_u16(counter.outlatch)?;
    writer.write_u8(counter.rw_mode)?;
    writer.write_u8(rw_state_wire(counter.read_state))?;
    writer.write_u8(rw_state_wire(counter.write_state))?;
    writer.write_bool(counter.count_lsb_latched)?;
    writer.write_bool(counter.count_msb_latched)?;
    writer.write_bool(counter.status_latched)?;
    writer.write_u8(counter.latched_status)?;
    writer.write_bool(counter.null_count)?;
    writer.write_bool(counter.gate)?;
    writer.write_bool(counter.output)?;
    writer.write_bool(counter.trigger_gate)?;
    writer.write_bool(counter.bcd_mode)?;
    writer.write_bool(counter.count_written)?;
    writer.write_bool(counter.first_pass)?;
    writer.write_bool(counter.state_bit_1)?;
    writer.write_bool(counter.state_bit_2)?;
    writer.write_u32(counter.next_change_time)?;
    writer.write_u32(counter.out_transitions)
}

#[cfg(feature = "std")]
fn read_pit_counter<R: Read>(
    reader: &mut SnapshotReader<R>,
    counter: &mut PitCounter,
) -> io::Result<()> {
    counter.mode = reader.read_u8()?;
    counter.inlatch = reader.read_u16()?;
    counter.count = reader.read_u16()?;
    counter.count_binary = reader.read_u16()?;
    counter.outlatch = reader.read_u16()?;
    counter.rw_mode = reader.read_u8()?;
    counter.read_state = rw_state_from_wire(reader.read_u8()?)?;
    counter.write_state = rw_state_from_wire(reader.read_u8()?)?;
    counter.count_lsb_latched = reader.read_bool()?;
    counter.count_msb_latched = reader.read_bool()?;
    counter.status_latched = reader.read_bool()?;
    counter.latched_status = reader.read_u8()?;
    counter.null_count = reader.read_bool()?;
    counter.gate = reader.read_bool()?;
    counter.output = reader.read_bool()?;
    counter.trigger_gate = reader.read_bool()?;
    counter.bcd_mode = reader.read_bool()?;
    counter.count_written = reader.read_bool()?;
    counter.first_pass = reader.read_bool()?;
    counter.state_bit_1 = reader.read_bool()?;
    counter.state_bit_2 = reader.read_bool()?;
    counter.next_change_time = reader.read_u32()?;
    counter.out_transitions = reader.read_u32()?;
    Ok(())
}

#[cfg(feature = "std")]
fn rw_state_wire(state: RWState) -> u8 {
    match state {
        RWState::LsByte => 0,
        RWState::MsByte => 1,
        RWState::LsByteMultiple => 2,
        RWState::MsByteMultiple => 3,
    }
}

#[cfg(feature = "std")]
fn rw_state_from_wire(value: u8) -> io::Result<RWState> {
    match value {
        0 => Ok(RWState::LsByte),
        1 => Ok(RWState::MsByte),
        2 => Ok(RWState::LsByteMultiple),
        3 => Ok(RWState::MsByteMultiple),
        _ => Err(pit_snapshot_invalid("PIT read/write state is out of range")),
    }
}

#[cfg(feature = "std")]
fn validate_pit_counter(counter: &PitCounter) -> io::Result<()> {
    if counter.mode > 5 {
        return Err(pit_snapshot_invalid("PIT mode is out of range"));
    }
    if !(1..=3).contains(&counter.rw_mode) {
        return Err(pit_snapshot_invalid("PIT read/write mode is out of range"));
    }
    if counter.status_latched {
        let status_rw_mode = (counter.latched_status >> 4) & 0x03;
        let status_mode = (counter.latched_status >> 1) & 0x07;
        if counter.latched_status & 0x08 != 0 || status_rw_mode == 0 || status_mode > 5 {
            return Err(pit_snapshot_invalid("PIT latched status is invalid"));
        }
    }
    if counter.bcd_mode && counter.count_binary > 16_665 {
        return Err(pit_snapshot_invalid("PIT BCD count is out of range"));
    }
    Ok(())
}

#[cfg(feature = "std")]
fn validate_pit_phase(
    ips: u64,
    usec_remainder: u128,
    pit_usec_accumulator: u128,
) -> io::Result<()> {
    if (ips == 0 && usec_remainder != 0) || (ips != 0 && usec_remainder >= u128::from(ips)) {
        return Err(pit_snapshot_invalid("PIT icount remainder is invalid"));
    }
    if pit_usec_accumulator >= u128::from(USEC_PER_SECOND) {
        return Err(pit_snapshot_invalid("PIT tick remainder is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the counters by an exact number of PIT input clocks — the
    /// pit82c54.cc clock_all tick domain (after the usec→tick conversion).
    fn advance_ticks(pit: &mut BxPitC, ticks: u64) {
        pit.clock_pit_ticks(ticks);
    }

    /// PIT with icount sync at ips = USEC_PER_SECOND: one instruction is
    /// exactly one emulated microsecond, so cumulative PIT ticks after
    /// icount N are floor(N * TICKS_PER_SECOND / USEC_PER_SECOND) — the
    /// exact Bochs usec→tick derivation (pit.cc USEC_TO_TICKS), quantized
    /// to whole microseconds just like pit.cc periodic().
    fn usec_locked_pit() -> BxPitC {
        let mut pit = BxPitC::new();
        pit.init_icount_sync(0, USEC_PER_SECOND as u64);
        pit
    }

    #[test]
    fn test_pit_creation() {
        let pit = BxPitC::new();
        // Bochs init: mode=4, GATE=1, OUTpin=1, count_written=1
        assert_eq!(pit.counters[0].mode, 4);
        assert!(pit.counters[0].gate);
        assert!(pit.counters[0].output);
        assert!(pit.counters[0].count_written);
        assert!(!pit.counters[0].first_pass);
        // Bochs pit.cc init: only counter 0 has an OUT handler (irq_handler)
        assert!(pit.counters[0].out_handler_attached);
        assert!(!pit.counters[1].out_handler_attached);
        assert!(!pit.counters[2].out_handler_attached);
        // Bochs pit.cc init: speaker state cleared
        assert!(!pit.speaker_data_on);
        assert!(!pit.speaker_active);
        assert!(!pit.speaker_level);
    }

    #[test]
    fn frequency_constant_matches_bochs() {
        // Bochs pit.cc: #define TICKS_PER_SECOND (1193181)
        assert_eq!(TICKS_PER_SECOND, 1_193_181);
        assert_eq!(PIT_FREQUENCY, 1_193_181);
        // Derived math: exactly one second of usec yields exactly one
        // second of PIT ticks (Bochs USEC_TO_TICKS).
        let mut pit = BxPitC::new();
        pit.advance_by_usec(1_000_000);
        assert_eq!(pit.total_ticks, 1_193_181);
    }

    #[test]
    fn test_pit_mode2_rate_generator() {
        let mut pit = BxPitC::new();

        // Configure counter 0 for mode 2 (rate generator), low-high access
        pit.write(PIT_CONTROL, 0x34, 1, 0); // Counter 0, low-high, mode 2

        // After control word: count_written=false, first_pass=true
        assert!(!pit.counters[0].count_written);
        assert!(pit.counters[0].first_pass);
        // Control word for mode != 0 sets OUT high — already high, so no
        // transition is recorded.
        assert_eq!(pit.drain_irq0_events(), (0, true));

        // Write count value 10
        pit.write(PIT_COUNTER0, 10, 1, 0); // Low byte
        pit.write(PIT_COUNTER0, 0, 1, 0); // High byte

        // After full write: count_written=true
        assert!(pit.counters[0].count_written);
        assert_eq!(pit.counters[0].inlatch, 10);

        // Tick 1: reload from inlatch, output stays HIGH
        advance_ticks(&mut pit, 1);
        assert!(pit.counters[0].output);
        assert!(!pit.counters[0].first_pass);
        assert_eq!(pit.counters[0].count, 10);

        // Ticks 2..=9: count 10 → 2, output stays HIGH
        advance_ticks(&mut pit, 8);
        assert!(pit.counters[0].output);
        assert_eq!(pit.counters[0].count, 2);
        assert_eq!(pit.drain_irq0_events(), (0, true));

        // Tick 10: count reaches 1 → OUT pulses LOW (Bochs irq_handler
        // would call lower_irq(0))
        advance_ticks(&mut pit, 1);
        assert!(!pit.counters[0].output);
        assert_eq!(pit.drain_irq0_events(), (1, false));

        // Tick 11: reload → OUT back HIGH (raise_irq(0))
        advance_ticks(&mut pit, 1);
        assert!(pit.counters[0].output);
        assert_eq!(pit.drain_irq0_events(), (1, true));

        // A full period processed in one bulk advance yields the LOW+HIGH
        // transition pair (lower then raise, in order).
        advance_ticks(&mut pit, 10);
        assert!(pit.counters[0].output);
        assert_eq!(pit.drain_irq0_events(), (2, true));
    }

    #[test]
    fn write_syncs_counter_to_now_before_applying() {
        // Finding #16: Bochs pit.cc bx_pit_c::write runs periodic() BEFORE
        // s.timer.write(...), so elapsed ticks replay under the OLD program.
        let mut pit = usec_locked_pit();

        // Program counter 0: mode 2, LSB/MSB, count 100 (at icount 0)
        pit.write(PIT_CONTROL, 0x34, 1, 0);
        pit.write(PIT_COUNTER0, 100, 1, 0);
        pit.write(PIT_COUNTER0, 0, 1, 0);

        // Write to port 0x61 (GATE2) at icount 51 (= 51 usec) — the write
        // handler must first advance the counters:
        // ticks = floor(51 * 1193181 / 1e6) = 60 → 1 reload + 59 decrements.
        pit.write(PIT_SYSTEM_CONTROL_B, 0x01, 1, 51);
        assert_eq!(
            pit.counters[0].count, 41,
            "port 0x61 write must sync counters to now first"
        );

        // Write a new LSB to counter 0 itself at icount 76 — cumulative
        // ticks = floor(76 * 1193181 / 1e6) = 90, so 30 more decrements
        // must elapse under the old count before the (partial) write applies.
        pit.write(PIT_COUNTER0, 200, 1, 76);
        assert_eq!(
            pit.counters[0].count, 11,
            "count-register write must sync counters to now first"
        );
        // The partial (LSB-only) write latched the new value but did not
        // load it yet (write_state = MSByte_multiple pauses the load).
        assert_eq!(pit.counters[0].inlatch, 200);
        assert!(!pit.counters[0].count_written);
    }

    #[test]
    fn mode3_bulk_decrement_is_two_per_tick() {
        // Finding #17: Bochs pit82c54.cc clock_multiple decrements mode 3
        // by 2*cycles in the bulk path.
        let mut pit = BxPitC::new();

        pit.write(PIT_CONTROL, 0x36, 1, 0); // Counter 0, low-high, mode 3
        pit.write(PIT_COUNTER0, 0xE8, 1, 0); // 1000 & 0xFF
        pit.write(PIT_COUNTER0, 0x03, 1, 0); // 1000 >> 8

        // Tick 1: reload → count = 1000, next change at half-period
        advance_ticks(&mut pit, 1);
        assert_eq!(pit.counters[0].count, 1000);
        assert_eq!(pit.counters[0].next_change_time, 499);

        // 100 more ticks in bulk → count decrements by 2 per tick = 200
        advance_ticks(&mut pit, 100);
        assert_eq!(
            pit.counters[0].count, 800,
            "mode 3 bulk path must decrement 2 per tick"
        );
        // count stays even in binary mode 3
        assert_eq!(pit.counters[0].count % 2, 0);
    }

    #[test]
    fn bcd_bulk_decrement_consumes_ticks() {
        // Finding #17: the old bulk path returned without decrementing BCD
        // counters while the caller still consumed the ticks.
        let mut pit = BxPitC::new();

        pit.write(PIT_CONTROL, 0x35, 1, 0); // Counter 0, low-high, mode 2, BCD
        pit.write(PIT_COUNTER0, 0x00, 1, 0); // BCD 100 LSB
        pit.write(PIT_COUNTER0, 0x01, 1, 0); // BCD 100 MSB

        // Tick 1: reload → count = 0x0100 (BCD 100)
        advance_ticks(&mut pit, 1);
        assert_eq!(pit.counters[0].count, 0x0100);
        assert_eq!(pit.counters[0].count_binary, 100);

        // 50 more ticks in bulk → BCD count must actually decrement
        advance_ticks(&mut pit, 50);
        assert_eq!(pit.counters[0].count_binary, 50);
        assert_eq!(
            pit.counters[0].count, 0x0050,
            "BCD bulk decrement must consume the ticks"
        );
    }

    #[test]
    fn port61_read_composition_is_fresh_every_read() {
        // Finding #18: Bochs pit.cc read case 0x61 composes the value fresh:
        // bit5=OUT2, bit4=(usec/15)&1, bit1=speaker_data_on, bit0=GATE2;
        // bits 2/3/6/7 read 0.
        let mut pit = usec_locked_pit();

        // Power-on: GATE2=1, OUT2=1, speaker off, usec=0 → 0b0010_0001
        assert_eq!(pit.read(PIT_SYSTEM_CONTROL_B, 1, 0), 0x21);
        // Repeated read without time passing: identical (no per-read toggle)
        assert_eq!(pit.read(PIT_SYSTEM_CONTROL_B, 1, 0), 0x21);

        // Write bits 1/2/3 set, bit 0 clear: GATE2 drops, speaker data on;
        // bits 2/3 must NOT be echoed back.
        pit.write(PIT_SYSTEM_CONTROL_B, 0x0E, 1, 0);
        assert!(pit.speaker_data_on);
        assert!(!pit.counters[2].gate);
        // Counter 2 is mode 4 (power-on) → speaker_level = data_on && OUT2
        assert!(pit.speaker_level);
        assert_eq!(pit.read(PIT_SYSTEM_CONTROL_B, 1, 0), 0x22);

        // Advance virtual time past 15 usec (18 ticks ≈ 15.09 usec) → the
        // refresh bit (bit 4) flips because it derives from the usec clock.
        assert_eq!(pit.read(PIT_SYSTEM_CONTROL_B, 1, 18) & 0x10, 0x10);
        // ... and stays put when read again with no time elapsed.
        assert_eq!(pit.read(PIT_SYSTEM_CONTROL_B, 1, 18) & 0x10, 0x10);
    }

    #[test]
    fn port43_read_returns_zero() {
        // Finding #32b: Bochs pit82c54.cc read(CONTROL_ADDRESS) returns 0.
        let mut pit = BxPitC::new();
        assert_eq!(pit.read(PIT_CONTROL, 1, 0), 0);
    }

    #[test]
    fn guest_reset_preserves_counter_state() {
        // Finding #32a: Bochs pit82c54.cc reset is empty — counters keep
        // their programming across a guest reset.
        let mut pit = BxPitC::new();
        pit.write(PIT_CONTROL, 0x34, 1, 0);
        pit.write(PIT_COUNTER0, 100, 1, 0);
        pit.write(PIT_COUNTER0, 0, 1, 0);
        pit.write(PIT_SYSTEM_CONTROL_B, 0x02, 1, 0); // speaker_data_on
        advance_ticks(&mut pit, 11);
        let count_before = pit.counters[0].count;

        pit.reset();

        assert_eq!(pit.counters[0].mode, 2);
        assert_eq!(pit.counters[0].inlatch, 100);
        assert_eq!(pit.counters[0].count, count_before);
        assert!(pit.counters[0].count_written);
        assert!(pit.speaker_data_on);
        assert_eq!(pit.total_ticks, 11);
    }

    #[test]
    fn control_word_out_transition_is_recorded() {
        // Finding #32d: Bochs pit82c54.cc write (control word) calls
        // set_OUT, which invokes the counter-0 out_handler on a transition
        // → IRQ0 edge from a control-word write alone.
        let mut pit = BxPitC::new();
        assert!(pit.counters[0].output); // power-on OUT=1

        pit.write(PIT_CONTROL, 0x30, 1, 0); // Counter 0, low-high, mode 0 → OUT low
        assert!(!pit.counters[0].output);
        assert_eq!(
            pit.drain_irq0_events(),
            (1, false),
            "control-word OUT 1→0 must be recorded as an IRQ0 lower"
        );

        // Reprogramming to a mode with initial OUT high transitions back.
        pit.write(PIT_CONTROL, 0x34, 1, 0);
        assert_eq!(pit.drain_irq0_events(), (1, true));
    }

    #[test]
    fn mode0_terminal_count_raises_out_level() {
        let mut pit = BxPitC::new();
        pit.write(PIT_CONTROL, 0x30, 1, 0); // mode 0 → OUT low
        assert_eq!(pit.drain_irq0_events(), (1, false));
        pit.write(PIT_COUNTER0, 5, 1, 0);
        pit.write(PIT_COUNTER0, 0, 1, 0);

        // Tick 1 loads the count; ticks 2..=6 count 5→0; OUT goes HIGH at
        // terminal count and STAYS high (level, not pulse).
        advance_ticks(&mut pit, 6);
        assert!(pit.counters[0].output);
        assert_eq!(pit.drain_irq0_events(), (1, true));

        // Further ticks: count wraps but OUT stays high — no transitions.
        advance_ticks(&mut pit, 94);
        assert_eq!(pit.drain_irq0_events(), (0, true));
    }

    #[test]
    fn mode3_count0_reproduces_bochs_91hz_quirk() {
        // Finding #32f (parity ruling): Bochs pit82c54.cc clock (mode 3
        // reload) computes ((count_binary/2)-1) in Bit32u; count 0
        // underflows and masks to 0xFFFF, so each half-period is 65536
        // ticks (~9.1 Hz square wave) instead of real hardware's 32768
        // (~18.2 Hz). Reproduced Bochs quirk.
        let mut pit = BxPitC::new();
        pit.write(PIT_CONTROL, 0x36, 1, 0); // Counter 0, low-high, mode 3
        pit.write(PIT_COUNTER0, 0, 1, 0);
        pit.write(PIT_COUNTER0, 0, 1, 0); // count 0 (= 0x10000 on real HW)

        // Tick 1: reload — the quirk schedules the next OUT change 0xFFFF
        // ticks out (18.2 Hz behavior would be 0x7FFF/0x8000).
        advance_ticks(&mut pit, 1);
        assert_eq!(pit.counters[0].next_change_time, 0xFFFF);
        assert!(pit.counters[0].output);

        // OUT must still be high after 32768 ticks (where real hardware
        // would have toggled) ...
        advance_ticks(&mut pit, 32_768);
        assert!(pit.counters[0].output);

        // ... and toggles only at tick 65537 (reload boundary + 1).
        advance_ticks(&mut pit, 32_767);
        assert!(pit.counters[0].output);
        advance_ticks(&mut pit, 1);
        assert!(!pit.counters[0].output);
        assert_eq!(pit.drain_irq0_events(), (1, false));
    }

    #[test]
    fn latched_status_with_half_read_count_returns_zero_forever() {
        // Finding #32e: Bochs pit82c54.cc read — status latched while a
        // latched count is half-read (MSB pending in MSByte_multiple) hits
        // the "Undefined output" error path and returns 0 WITHOUT clearing
        // any latch, so every subsequent read also returns 0.
        let mut pit = usec_locked_pit();
        pit.write(PIT_CONTROL, 0x34, 1, 0); // Counter 0, low-high, mode 2
        pit.write(PIT_COUNTER0, 0x34, 1, 0);
        pit.write(PIT_COUNTER0, 0x12, 1, 0);
        pit.sync_to_icount(5);

        // Latch the count, read only the LSB (read_state → MSByte_multiple)
        pit.write(PIT_CONTROL, 0x00, 1, 5);
        let lsb = pit.read(PIT_COUNTER0, 1, 5);
        assert_eq!(lsb, (pit.counters[0].outlatch & 0xFF) as u32);
        assert!(pit.counters[0].count_msb_latched);

        // READ_BACK latch status for counter 0 (bit5=1: no count latch,
        // bit4=0: latch status, bit1: select counter 0)
        pit.write(PIT_CONTROL, 0xE2, 1, 5);
        assert!(pit.counters[0].status_latched);

        // Bochs error path: returns 0 and clears nothing — forever.
        assert_eq!(pit.read(PIT_COUNTER0, 1, 5), 0);
        assert_eq!(pit.read(PIT_COUNTER0, 1, 5), 0);
        assert!(pit.counters[0].status_latched);
        assert!(pit.counters[0].count_msb_latched);
    }

    #[test]
    fn latched_two_part_read_advances_read_state() {
        // Bochs pit82c54.cc read: reading a latched LSB in LSByte_multiple
        // advances read_state to MSByte_multiple (and back on the MSB).
        let mut pit = usec_locked_pit();
        pit.write(PIT_CONTROL, 0x34, 1, 0);
        pit.write(PIT_COUNTER0, 0x34, 1, 0);
        pit.write(PIT_COUNTER0, 0x12, 1, 0);
        pit.sync_to_icount(3);

        pit.write(PIT_CONTROL, 0x00, 1, 3); // latch counter 0
        let outlatch = pit.counters[0].outlatch;
        assert_eq!(pit.read(PIT_COUNTER0, 1, 3), (outlatch & 0xFF) as u32);
        assert_eq!(pit.counters[0].read_state, RWState::MsByteMultiple);
        assert_eq!(pit.read(PIT_COUNTER0, 1, 3), (outlatch >> 8) as u32);
        assert_eq!(pit.counters[0].read_state, RWState::LsByteMultiple);
    }

    #[test]
    fn test_pit_gate_edge_detection() {
        let mut ctr = PitCounter::new(0);
        ctr.mode = 2;
        ctr.count_written = true;
        ctr.inlatch = 100;
        ctr.gate = true;

        // Gate is already true, setting again should NOT trigger
        ctr.set_gate(true);
        assert!(!ctr.trigger_gate);

        // Drop gate LOW
        ctr.set_gate(false);
        assert!(!ctr.trigger_gate); // Falling edge doesn't set trigger
        assert!(ctr.output); // Mode 2: gate LOW forces output HIGH

        // Raise gate HIGH — rising edge
        ctr.set_gate(true);
        assert!(ctr.trigger_gate); // Rising edge detected!
    }

    #[test]
    fn test_pit_count_written_gates_behavior() {
        let mut ctr = PitCounter::new(0);
        // After init: count_written=true but mode=4, no interesting behavior
        // Program mode 2 via control word simulation:
        ctr.null_count = true;
        ctr.count_written = false; // Control word clears this
        ctr.first_pass = true;
        ctr.mode = 2;

        // Clock with count_written=false → should be no-op
        let old_count = ctr.count;
        ctr.clock();
        assert_eq!(ctr.count, old_count); // Count unchanged
    }

    #[cfg(feature = "std")]
    #[test]
    fn realtime_sync_advances_pit_without_icount_progress() {
        let mut pit = BxPitC::new();
        pit.init_icount_sync(1_000, 300_000_000);
        pit.enable_realtime_sync();

        let before = pit.total_ticks;
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = pit.read(PIT_COUNTER0, 1, 1_000);

        assert!(
            pit.total_ticks > before,
            "PIT should advance from host realtime even when icount is unchanged"
        );
    }

    #[cfg(feature = "std")]
    #[test]
    fn realtime_sync_preserves_baseline_when_elapsed_truncates_to_zero() {
        let mut pit = BxPitC::new();
        pit.enable_realtime_sync();

        let baseline = std::time::Instant::now() + std::time::Duration::from_millis(1);
        pit.realtime_last = Some(baseline);

        pit.sync_to_realtime();
        assert_eq!(
            pit.realtime_last,
            Some(baseline),
            "zero-elapsed realtime polling must not discard the sub-microsecond baseline"
        );
    }
    #[test]
    fn pit_port_access_after_timer_span_does_not_freeze_the_clock() {
        // Bochs pit.cc keeps ONE time cursor (s.last_usec): the timer callback
        // and every port access advance the counters from the same baseline.
        // A port access after a timer-driven span must not re-apply that span
        // and stall the counter — the exact regression behind Linux
        // check_timer()'s "IO-APIC + timer doesn't work!" panic.
        const IPS: u64 = 1_000_000; // 1 tick == 1 microsecond
        let mut pit = BxPitC::new();
        pit.counters[0].out_handler_attached = true;
        pit.init_icount_sync(0, IPS);

        // Counter 0, mode 2 (rate generator), divisor 100.
        pit.write(PIT_CONTROL, 0x34, 1, 0);
        pit.write(PIT_COUNTER0, 100, 1, 0);
        pit.write(PIT_COUNTER0, 0, 1, 0);
        let _ = pit.drain_irq0_events();

        // Advance ~1000 us purely through the timer-callback (tick) path.
        let early = pit.timer_callback(1_000, IPS).irq0_transitions
            + pit.drain_irq0_events().0;
        assert!(early > 0, "PIT must tick via the timer path");

        // A guest reads a PIT port at the same emulated time. This must NOT
        // re-apply the 1000 us already consumed by the timer path.
        let _ = pit.read(PIT_COUNTER0, 1, 1_000);
        let _ = pit.drain_irq0_events();

        // Continue the tick path; the PIT must keep generating IRQ0 edges.
        let mut later = 0u32;
        for tick in 1_001..=2_000 {
            later += pit.timer_callback(tick, IPS).irq0_transitions;
            later += pit.drain_irq0_events().0;
        }
        assert!(
            later > 0,
            "PIT froze after a port access following a timer span (dual-cursor double-count)"
        );
    }

    #[test]
    fn pit_irq0_fires_at_submillisecond_owner_deadline() {
        let mut pit = BxPitC::new();
        assert_eq!(pit.next_event_usec(), None);
        pit.set_timer_handle(7);
        assert_eq!(pit.timer_handle(), Some(7));

        // Counter 0, LSB/MSB, mode 0. Programming drives OUT low. Its load
        // and terminal-count phases both remain exact owner deadlines.
        pit.write(PIT_CONTROL, 0x30, 1, 0);
        pit.write(PIT_COUNTER0, 1, 1, 0);
        pit.write(PIT_COUNTER0, 0, 1, 0);
        let _ = pit.drain_irq0_events();
        assert_eq!(pit.next_event_usec(), Some(1));

        let mut transitions = 0;
        let mut final_level = false;
        let mut fired_at_usec = None;
        for current_usec in 1..=4 {
            let callback =
                pit.timer_callback(current_usec, u64::from(USEC_PER_SECOND));
            transitions += callback.irq0_transitions;
            final_level = callback.irq0_level;
            if callback.irq0_transitions != 0 {
                fired_at_usec = Some(current_usec);
                break;
            }
            assert!(callback.rearm_usec.is_some());
        }
        assert_eq!(transitions, 1);
        assert!(final_level);
        assert!(fired_at_usec.is_some_and(|deadline| deadline < 1_000));
    }

    #[cfg(feature = "std")]
    #[test]
    fn realtime_owner_callback_rearms_early_without_mutating_pit() {
        let mut pit = BxPitC::new();
        pit.enable_realtime_sync();
        pit.counters[0].next_change_time = u32::MAX;

        let before_usec = pit.total_usec;
        let before_ticks = pit.total_ticks;
        let before_event = pit.counters[0].next_change_time;
        let callback = pit.timer_callback(1_000_000, u64::from(USEC_PER_SECOND));

        assert!(
            callback.rearm_usec.is_some_and(|delay| delay > 0),
            "a host-early callback must rearm for the predicted remaining time"
        );
        assert_eq!(pit.total_usec, before_usec);
        assert_eq!(pit.total_ticks, before_ticks);
        assert_eq!(pit.counters[0].next_change_time, before_event);
    }

}
