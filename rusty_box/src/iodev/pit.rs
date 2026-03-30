//! 8254 PIT (Programmable Interval Timer) Emulation
//!
//! Based on Bochs pit82c54.cc — faithful port of the Bochs state machine.
//! The 8254 PIT provides three independent 16-bit counters:
//! - Counter 0: System timer (IRQ0) - ~18.2 Hz for DOS tick
//! - Counter 1: DRAM refresh (legacy, not used)
//! - Counter 2: Speaker/beep control
//!
//! Base frequency: 1.193182 MHz


/// PIT I/O port addresses
pub const PIT_COUNTER0: u16 = 0x0040;
pub const PIT_COUNTER1: u16 = 0x0041;
pub const PIT_COUNTER2: u16 = 0x0042;
pub const PIT_CONTROL: u16 = 0x0043;

/// PIT base frequency in Hz
pub const PIT_FREQUENCY: u32 = 1193182;

/// Ticks per second
pub const TICKS_PER_SECOND: u32 = PIT_FREQUENCY;

/// Microseconds per second
pub const USEC_PER_SECOND: u32 = 1_000_000;

/// Number of PIT counters
const PIT_NUM_COUNTERS: usize = 3;

// ---- Read/write state machine (Bochs pit82c54.h:60-66) ----
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
    // ---- Bochs counter_type fields (pit82c54.h:70-98) ----
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
    /// Latched status value
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
}

impl Default for PitCounter {
    /// Default matching Bochs pit82c54::init() (pit82c54.cc:174-200).
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
        }
    }
}

impl PitCounter {
    /// Create a new counter. Bochs pit82c54::init() sets GATE=1 for all 3 counters.
    pub fn new(_number: u8) -> Self {
        Self::default()
    }

    /// Bochs pit82c54.cc:116-124 set_OUT — only calls handler on transition
    fn set_out(&mut self, data: bool) {
        self.output = data;
        // Note: Bochs calls out_handler callback here; we detect transitions in tick()
    }

    /// Bochs pit82c54.cc:126-130 set_count
    fn set_count(&mut self, data: u16) {
        self.count = data & 0xFFFF;
        self.set_binary_to_count();
    }

    /// Bochs pit82c54.cc:145-156 set_binary_to_count (count → count_binary)
    fn set_binary_to_count(&mut self) {
        if self.bcd_mode {
            self.count_binary = (1 * ((self.count >> 0) & 0xF))
                + (10 * ((self.count >> 4) & 0xF))
                + (100 * ((self.count >> 8) & 0xF))
                + (1000 * ((self.count >> 12) & 0xF));
        } else {
            self.count_binary = self.count;
        }
    }

    /// Bochs pit82c54.cc:132-143 set_count_to_binary (count_binary → count)
    fn set_count_to_binary(&mut self) {
        if self.bcd_mode {
            self.count = (((self.count_binary / 1) % 10) << 0)
                | (((self.count_binary / 10) % 10) << 4)
                | (((self.count_binary / 100) % 10) << 8)
                | (((self.count_binary / 1000) % 10) << 12);
        } else {
            self.count = self.count_binary;
        }
    }

    /// Bochs pit82c54.cc:158-172 decrement
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

    /// Fast-path: advance counter by N ticks without calling clock() for each.
    /// Bochs pit82c54.cc:259-335 clock_multiple().
    /// Returns true if counter 0 output transitioned (IRQ0 should fire).
    /// When next_change_time >= ticks, we can just decrement count_binary by ticks.
    fn clock_multiple(&mut self, ticks: u32) -> bool {
        if ticks == 0 {
            return false;
        }
        // If next_change_time is 0, state machine needs per-tick evaluation
        if self.next_change_time == 0 {
            return false; // Caller must use per-tick clock()
        }
        if self.next_change_time > ticks {
            // No state change within these ticks — just decrement
            self.next_change_time -= ticks;
            if !self.bcd_mode {
                self.count_binary = self.count_binary.wrapping_sub(ticks as u16);
                self.count = self.count_binary;
            } else {
                // BCD: decrement one at a time (rare, keep simple)
                return false;
            }
            return false;
        }
        // next_change_time <= ticks: a state change will occur
        // Fall back to per-tick for the remaining portion
        false
    }

    /// Latch the current count value — Bochs pit82c54.cc:77-114
    pub fn latch_count(&mut self) {
        if self.count_lsb_latched || self.count_msb_latched {
            // Previous latch not yet read — do nothing
            return;
        }
        match self.read_state {
            RWState::MsByte => {
                self.outlatch = self.count & 0xFFFF;
                self.count_msb_latched = true;
            }
            RWState::LsByte => {
                self.outlatch = self.count & 0xFFFF;
                self.count_lsb_latched = true;
            }
            RWState::LsByteMultiple => {
                self.outlatch = self.count & 0xFFFF;
                self.count_lsb_latched = true;
                self.count_msb_latched = true;
            }
            RWState::MsByteMultiple => {
                // Latching during 2-part read — reset to LSB first
                self.read_state = RWState::LsByteMultiple;
                self.outlatch = self.count & 0xFFFF;
                self.count_lsb_latched = true;
                self.count_msb_latched = true;
            }
        }
    }

    /// Latch the status register — Bochs pit82c54.cc:695-706
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

    /// Read counter — Bochs pit82c54.cc:602-672
    pub fn read(&mut self) -> u8 {
        if self.status_latched {
            self.status_latched = false;
            return self.latched_status;
        }

        // Latched count read
        if self.count_lsb_latched {
            // Read LSB of latched value
            self.count_lsb_latched = false;
            return (self.outlatch & 0xFF) as u8;
        }
        if self.count_msb_latched {
            // Read MSB of latched value
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

    /// Write counter — Bochs pit82c54.cc:762-821
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

        // Bochs pit82c54.cc:788-791
        if self.count_written {
            self.null_count = true;
            self.set_count(self.inlatch);
        }

        // Mode-specific actions after count write (Bochs pit82c54.cc:792-820)
        match self.mode {
            0 => {
                if self.count_written {
                    self.set_out(false);
                }
                self.next_change_time = 1;
            }
            1 => {
                if self.trigger_gate {
                    self.next_change_time = 1;
                }
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
            5 => {
                if self.trigger_gate {
                    self.next_change_time = 1;
                }
            }
            _ => {}
        }
    }

    /// Clock the counter by one tick — Bochs pit82c54.cc:259-591
    /// Returns true if output transitioned LOW→HIGH (for IRQ generation)
    pub fn clock(&mut self) -> bool {
        let old_output = self.output;

        match self.mode {
            // ---- Mode 0: Interrupt on Terminal Count (Bochs pit82c54.cc:344-377) ----
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

            // ---- Mode 1: Hardware Retriggerable One-Shot (Bochs pit82c54.cc:378-411) ----
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

            // ---- Mode 2: Rate Generator (Bochs pit82c54.cc:412-444) ----
            2 | 6 => {
                if self.count_written {
                    if self.trigger_gate || self.first_pass {
                        // RELOAD phase: load count, set output HIGH
                        self.set_count(self.inlatch);
                        self.next_change_time = (self.count_binary.wrapping_sub(1) & 0xFFFF) as u32;
                        self.null_count = false;
                        if !self.output {
                            self.set_out(true);
                        }
                        self.first_pass = false;
                    } else {
                        // COUNTING phase
                        if self.gate {
                            self.decrement();
                            self.next_change_time =
                                (self.count_binary.wrapping_sub(1) & 0xFFFF) as u32;
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

            // ---- Mode 3: Square Wave Generator (Bochs pit82c54.cc:446-506) ----
            3 | 7 => {
                if self.count_written {
                    if (self.trigger_gate || self.first_pass || self.state_bit_2) && self.gate {
                        self.set_count(self.inlatch & 0xFFFE);
                        self.state_bit_1 = (self.inlatch & 0x1) != 0;
                        if !self.output || !self.state_bit_1 {
                            let half = self.count_binary / 2;
                            if half <= 1 {
                                self.next_change_time = 1;
                            } else {
                                self.next_change_time = (half - 1) as u32;
                            }
                        } else {
                            let half = self.count_binary / 2;
                            if half == 0 {
                                self.next_change_time = 1;
                            } else {
                                self.next_change_time = half as u32;
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
                                    ((self.count_binary / 2).wrapping_sub(1) & 0xFFFF) as u32;
                            } else {
                                self.next_change_time = (self.count_binary / 2) as u32;
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

            // ---- Mode 4: Software Triggered Strobe (Bochs pit82c54.cc:507-549) ----
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

            // ---- Mode 5: Hardware Triggered Strobe (Bochs pit82c54.cc:550-591) ----
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
                self.trigger_gate = false;
            }
        }

        // Return true if output transitioned LOW→HIGH (rising edge for IRQ)
        !old_output && self.output
    }

    /// Set GATE input — Bochs pit82c54.cc:824-921
    /// Detects rising edge and sets triggerGATE; mode-specific behavior
    pub fn set_gate(&mut self, data: bool) {
        let old_gate = self.gate;
        // Only process on actual change (Bochs line 830)
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
            1 => {
                if data && self.count_written {
                    self.next_change_time = 1;
                }
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
            5 => {
                if data && self.count_written {
                    self.next_change_time = 1;
                }
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
    /// Timer handles for scheduling
    pub(crate) timer_handles: [Option<usize>; 3],
    /// IRQ0 callback (for system timer)
    irq0_pending: bool,
    /// Pointer to CPU's icount for fine-grained PIT synchronization.
    /// When set, the PIT advances counters on port reads (not just batch boundaries),
    /// allowing the kernel's PIT-polling calibration loops to see the counter decrement.
    icount_ptr: Option<*const u64>,
    /// IPS (instructions per second) for converting icount to PIT ticks.
    ips: u64,
    /// icount value at last PIT synchronization point.
    icount_at_last_sync: u64,
    /// Fractional PIT tick accumulator (units: instruction_count * PIT_FREQUENCY).
    /// Preserves fractions across calls so that even a few instructions
    /// between reads can accumulate enough for a PIT tick (~13 instr at 15M IPS).
    pit_tick_accumulator: u128,
}

impl Default for BxPitC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPitC {
    /// Create a new PIT controller
    pub fn new() -> Self {
        Self {
            counters: [PitCounter::new(0), PitCounter::new(1), PitCounter::new(2)],
            total_ticks: 0,
            timer_handles: [None; 3],
            irq0_pending: false,
            icount_ptr: None,
            ips: 0,
            icount_at_last_sync: 0,
            pit_tick_accumulator: 0,
        }
    }

    /// Initialize the PIT
    pub fn init(&mut self) {
        tracing::info!("PIT: Initializing 8254 Programmable Interval Timer");
        self.reset();
    }

    /// Reset the PIT
    pub fn reset(&mut self) {
        self.counters = [PitCounter::new(0), PitCounter::new(1), PitCounter::new(2)];
        self.total_ticks = 0;
        self.irq0_pending = false;
        self.icount_at_last_sync = 0;
        self.pit_tick_accumulator = 0;
    }

    /// Set the icount pointer for fine-grained PIT synchronization.
    /// When set, PIT counter reads will advance counters to match elapsed CPU time.
    /// SAFETY: The pointer must remain valid for the lifetime of the PIT.
    pub unsafe fn set_icount_sync(&mut self, icount_ptr: *const u64, ips: u64) {
        self.icount_ptr = Some(icount_ptr);
        self.ips = ips;
        self.icount_at_last_sync = *icount_ptr;
    }

    /// Synchronize PIT counters to match elapsed CPU time.
    /// Called before counter reads to ensure counters are up-to-date.
    /// Uses a fractional accumulator to avoid losing ticks when only a few
    /// instructions have elapsed between reads (~13 instructions per PIT tick
    /// at 15M IPS).
    pub fn sync_to_icount(&mut self) {
        if let Some(ptr) = self.icount_ptr {
            let current_icount = unsafe { *ptr };
            let elapsed_instr = current_icount.saturating_sub(self.icount_at_last_sync);
            if elapsed_instr > 0 && self.ips > 0 {
                // Accumulate fractional PIT ticks
                self.pit_tick_accumulator += elapsed_instr as u128 * TICKS_PER_SECOND as u128;
                let pit_ticks = (self.pit_tick_accumulator / self.ips as u128) as u64;
                self.pit_tick_accumulator %= self.ips as u128;

                // Fast path: skip ticks in bulk when no state change is imminent
                // (Bochs pit82c54.cc:259-335 clock_multiple). Then per-tick for remainder.
                // With clock_multiple bulk skip, we can process more ticks safely.
                // 5M PIT ticks ≈ 4.2 seconds at 1.193182 MHz.
                let mut remaining = pit_ticks.min(5_000_000) as u32;
                while remaining > 0 {
                    // Try bulk skip on all 3 counters
                    let skip = remaining
                        .min(self.counters[0].next_change_time.saturating_sub(1).max(1))
                        .min(self.counters[1].next_change_time.saturating_sub(1).max(1))
                        .min(self.counters[2].next_change_time.saturating_sub(1).max(1));
                    if skip > 1 {
                        let c0_fired = self.counters[0].clock_multiple(skip);
                        self.counters[1].clock_multiple(skip);
                        self.counters[2].clock_multiple(skip);
                        self.total_ticks += skip as u64;
                        remaining -= skip;
                        if c0_fired {
                            self.irq0_pending = true;
                        }
                    } else {
                        // Per-tick fallback
                        self.total_ticks += 1;
                        if self.counters[0].clock() {
                            self.irq0_pending = true;
                        }
                        self.counters[1].clock();
                        self.counters[2].clock();
                        remaining -= 1;
                    }
                }
                self.icount_at_last_sync = current_icount;
            }
        }
    }

    /// Read from PIT I/O port
    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        // Synchronize counters to current CPU time before reading.
        // This ensures the kernel's PIT-polling calibration loops see the counter decrement.
        self.sync_to_icount();
        match port {
            PIT_COUNTER0 => self.counters[0].read() as u32,
            PIT_COUNTER1 => self.counters[1].read() as u32,
            PIT_COUNTER2 => self.counters[2].read() as u32,
            PIT_CONTROL => 0xFF, // Control port is write-only
            _ => {
                tracing::warn!("PIT: Unknown read port {:#06x}", port);
                0xFF
            }
        }
    }

    /// Write to PIT I/O port
    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let value = value as u8;
        match port {
            PIT_COUNTER0 => self.counters[0].write(value),
            PIT_COUNTER1 => self.counters[1].write(value),
            PIT_COUNTER2 => self.counters[2].write(value),
            PIT_CONTROL => self.write_control(value),
            _ => {
                tracing::warn!("PIT: Unknown write port {:#06x} value={:#04x}", port, value);
            }
        }
    }

    /// Write to the control register — Bochs pit82c54.cc:674-759
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

        // Counter Program Command — Bochs pit82c54.cc:717-759
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
        // Mode aliasing: 6→2, 7→3 (Bochs pit82c54.cc:729-731)
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

        // All modes except mode 0 have initial output of 1 (Bochs line 752-757)
        if m != 0 {
            ctr.set_out(true);
        } else {
            ctr.set_out(false);
        }
        ctr.next_change_time = 0;

        tracing::debug!(
            "PIT: Counter {} configured: mode={}, rw={}, bcd={}",
            sc,
            ctr.mode,
            rw,
            bcd
        );
    }

    /// Handle read-back command — Bochs pit82c54.cc:674-709
    fn read_back(&mut self, value: u8) {
        let latch_count = (value & 0x20) == 0; // Bit 5: 0 = latch count
        let latch_status = (value & 0x10) == 0; // Bit 4: 0 = latch status

        for i in 0..PIT_NUM_COUNTERS {
            if (value & (0x02 << i)) != 0 {
                if latch_status {
                    self.counters[i].latch_status();
                }
                if latch_count {
                    self.counters[i].latch_count();
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

    /// Simulate time passing (in microseconds)
    pub fn tick(&mut self, usec: u64) -> bool {
        let pit_ticks = (usec * TICKS_PER_SECOND as u64) / USEC_PER_SECOND as u64;

        let mut irq0 = false;
        for _ in 0..pit_ticks {
            self.total_ticks += 1;

            // Clock counter 0 (system timer)
            if self.counters[0].clock() {
                irq0 = true;
            }

            // Clock counter 1 (DRAM refresh - legacy)
            self.counters[1].clock();

            // Clock counter 2 (speaker)
            self.counters[2].clock();
        }

        if irq0 {
            self.irq0_pending = true;
        }

        // Reset icount baseline and accumulator so that read()-based sync
        // doesn't double-count the ticks we just advanced via usec.
        if let Some(ptr) = self.icount_ptr {
            self.icount_at_last_sync = unsafe { *ptr };
            self.pit_tick_accumulator = 0;
        }

        irq0
    }

    /// Check and clear IRQ0 pending flag
    pub fn check_irq0(&mut self) -> bool {
        let pending = self.irq0_pending;
        self.irq0_pending = false;
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pit_creation() {
        let pit = BxPitC::new();
        // Bochs init: mode=4, GATE=1, OUTpin=1, count_written=1
        assert_eq!(pit.counters[0].mode, 4);
        assert!(pit.counters[0].gate);
        assert!(pit.counters[0].output);
        assert!(pit.counters[0].count_written);
        assert!(!pit.counters[0].first_pass);
    }

    #[test]
    fn test_pit_mode2_rate_generator() {
        let mut pit = BxPitC::new();

        // Configure counter 0 for mode 2 (rate generator), low-high access
        pit.write(PIT_CONTROL, 0x34, 1); // Counter 0, low-high, mode 2

        // After control word: count_written=false, first_pass=true
        assert!(!pit.counters[0].count_written);
        assert!(pit.counters[0].first_pass);

        // Write count value 10
        pit.write(PIT_COUNTER0, 10, 1); // Low byte
        pit.write(PIT_COUNTER0, 0, 1); // High byte

        // After full write: count_written=true
        assert!(pit.counters[0].count_written);
        assert_eq!(pit.counters[0].inlatch, 10);

        // Clock: first_pass=true → reload from inlatch, set output HIGH
        pit.counters[0].clock();
        assert!(pit.counters[0].output); // HIGH after reload
        assert!(!pit.counters[0].first_pass); // first_pass cleared

        // Clock 8 more times (count goes 10→9→...→2)
        for _ in 0..8 {
            let irq = pit.counters[0].clock();
            assert!(!irq); // No IRQ yet
            assert!(pit.counters[0].output); // Still HIGH
        }

        // Clock once more: count reaches 1, output goes LOW, first_pass=true
        let irq = pit.counters[0].clock();
        assert!(!irq); // LOW transition, not rising edge
        assert!(!pit.counters[0].output); // LOW pulse
        assert!(pit.counters[0].first_pass);

        // Next clock: first_pass → reload and output HIGH (rising edge = IRQ)
        let irq = pit.counters[0].clock();
        assert!(irq); // Rising edge! IRQ fires
        assert!(pit.counters[0].output); // Back to HIGH
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
}
