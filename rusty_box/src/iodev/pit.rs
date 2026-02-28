//! 8254 PIT (Programmable Interval Timer) Emulation
//!
//! The 8254 PIT provides three independent 16-bit counters:
//! - Counter 0: System timer (IRQ0) - ~18.2 Hz for DOS tick
//! - Counter 1: DRAM refresh (legacy, not used)
//! - Counter 2: Speaker/beep control
//!
//! Base frequency: 1.193182 MHz

use core::ffi::c_void;

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

// ---- Control register bit fields (port 0x43 write) ----
// Bits 7-6: Counter select (0-2, or 3 = read-back command)
const CONTROL_COUNTER_SHIFT: u32 = 6;
const CONTROL_COUNTER_MASK: u8 = 0x03;
const CONTROL_READBACK_SELECT: u8 = 3;
// Bits 5-4: Access mode
const CONTROL_ACCESS_MODE_SHIFT: u32 = 4;
const CONTROL_ACCESS_MODE_MASK: u8 = 0x03;
// Bits 3-1: Operating mode (0-5)
const CONTROL_MODE_SHIFT: u32 = 1;
const CONTROL_MODE_MASK: u8 = 0x07;
// Bit 0: BCD mode
const CONTROL_BCD_BIT: u8 = 0x01;

// ---- Status register bit positions (latch_status) ----
const STATUS_OUTPUT_SHIFT: u32 = 7;
const STATUS_NULL_COUNT_SHIFT: u32 = 6;
const STATUS_ACCESS_MODE_SHIFT: u32 = 4;
const STATUS_MODE_SHIFT: u32 = 1;

// ---- Read-back command bit fields (D7-D6 = 11) ----
// Bit 5: COUNT — 0 = latch count, 1 = don't latch count
const READBACK_LATCH_COUNT_BIT: u8 = 0x20;
// Bit 4: STATUS — 0 = latch status, 1 = don't latch status
const READBACK_LATCH_STATUS_BIT: u8 = 0x10;
// Bits 3-1: Counter select (bit 1 = counter 0, bit 2 = counter 1, bit 3 = counter 2)
const READBACK_COUNTER0_BIT: u8 = 0x02;

/// Counter operating modes
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PitMode {
    /// Mode 0: Interrupt on terminal count
    InterruptOnTerminalCount = 0,
    /// Mode 1: Hardware retriggerable one-shot
    HardwareOneShot = 1,
    /// Mode 2: Rate generator
    RateGenerator = 2,
    /// Mode 3: Square wave generator
    SquareWave = 3,
    /// Mode 4: Software triggered strobe
    SoftwareStrobe = 4,
    /// Mode 5: Hardware triggered strobe
    HardwareStrobe = 5,
}

impl From<u8> for PitMode {
    fn from(value: u8) -> Self {
        match value & 0x07 {
            0 => PitMode::InterruptOnTerminalCount,
            1 => PitMode::HardwareOneShot,
            2 | 6 => PitMode::RateGenerator,
            3 | 7 => PitMode::SquareWave,
            4 => PitMode::SoftwareStrobe,
            5 => PitMode::HardwareStrobe,
            _ => PitMode::InterruptOnTerminalCount,
        }
    }
}

/// Read/Write access mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PitAccessMode {
    /// Latch count value
    Latch = 0,
    /// Low byte only
    LowByte = 1,
    /// High byte only  
    HighByte = 2,
    /// Low byte then high byte
    LowHighByte = 3,
}

impl From<u8> for PitAccessMode {
    fn from(value: u8) -> Self {
        match value & 0x03 {
            0 => PitAccessMode::Latch,
            1 => PitAccessMode::LowByte,
            2 => PitAccessMode::HighByte,
            3 => PitAccessMode::LowHighByte,
            _ => PitAccessMode::Latch,
        }
    }
}

/// State for a single PIT counter
#[derive(Debug, Clone)]
pub struct PitCounter {
    /// Counter number (0-2)
    pub(crate) number: u8,
    /// Current count value
    pub(crate) count: u16,
    /// Initial count (reload value)
    pub(crate) initial_count: u16,
    /// Count latched for reading
    pub(crate) latched_count: u16,
    /// Is count latched?
    pub(crate) count_latched: bool,
    /// Status latched for reading
    pub(crate) status_latched: bool,
    /// Latched status value
    pub(crate) latched_status: u8,
    /// Operating mode
    pub(crate) mode: PitMode,
    /// Access mode (low/high byte)
    pub(crate) access_mode: PitAccessMode,
    /// BCD mode (false = binary)
    pub(crate) bcd_mode: bool,
    /// Next byte is low (for low-high access)
    pub(crate) read_lsb_next: bool,
    /// Next write is low (for low-high access)
    pub(crate) write_lsb_next: bool,
    /// Output pin state
    pub(crate) output: bool,
    /// Gate input state
    pub(crate) gate: bool,
    /// Counter is enabled
    pub(crate) enabled: bool,
    /// Null count (count not yet loaded)
    pub(crate) null_count: bool,
    /// Countdown in progress
    pub(crate) counting: bool,
}

impl Default for PitCounter {
    /// Default matching Bochs pit82c54::init() (pit82c54.cc:174-200).
    ///
    /// Key values from Bochs:
    /// - read_state = LSByte, write_state = LSByte  (we model as access_mode + lsb_next flags)
    /// - GATE = 1, OUTpin = 1, mode = 4 (SoftwareStrobe)
    /// - count = 0, null_count = 0, count_written = 1
    fn default() -> Self {
        Self {
            number: 0,
            count: 0,
            initial_count: 0,
            latched_count: 0,
            count_latched: false,
            status_latched: false,
            latched_status: 0,
            mode: PitMode::SoftwareStrobe,       // Bochs: mode=4
            access_mode: PitAccessMode::LowByte, // Bochs: rw_mode=1 (LSB_real), read_state=LSByte
            bcd_mode: false,
            read_lsb_next: true,
            write_lsb_next: true,
            output: true, // Bochs: OUTpin=1
            gate: true,   // Bochs: GATE=1 for all counters
            enabled: false,
            null_count: false, // Bochs: null_count=0
            counting: false,
        }
    }
}

impl PitCounter {
    /// Create a new counter with specified number.
    ///
    /// Bochs pit82c54::init() sets GATE=1 for all 3 counters.
    /// Counter 2's gate is later controlled by port 0x61, but starts high.
    pub fn new(number: u8) -> Self {
        let mut counter = Self::default();
        counter.number = number;
        // Bochs: GATE=1 for ALL counters in init().
        // Counter 2's gate is later controlled by port 0x61 writes.
        counter.gate = true;
        counter
    }

    /// Read the current count value
    pub fn read(&mut self) -> u8 {
        if self.status_latched {
            self.status_latched = false;
            return self.latched_status;
        }

        let value = if self.count_latched {
            self.latched_count
        } else {
            self.count
        };

        match self.access_mode {
            PitAccessMode::LowByte => {
                self.count_latched = false;
                (value & 0xFF) as u8
            }
            PitAccessMode::HighByte => {
                self.count_latched = false;
                (value >> 8) as u8
            }
            PitAccessMode::LowHighByte => {
                if self.read_lsb_next {
                    self.read_lsb_next = false;
                    (value & 0xFF) as u8
                } else {
                    self.read_lsb_next = true;
                    self.count_latched = false;
                    (value >> 8) as u8
                }
            }
            PitAccessMode::Latch => {
                self.count_latched = false;
                (value & 0xFF) as u8
            }
        }
    }

    /// Write to the counter
    pub fn write(&mut self, value: u8) {
        match self.access_mode {
            PitAccessMode::LowByte => {
                self.initial_count = (self.initial_count & 0xFF00) | (value as u16);
                self.load_count();
            }
            PitAccessMode::HighByte => {
                self.initial_count = (self.initial_count & 0x00FF) | ((value as u16) << 8);
                self.load_count();
            }
            PitAccessMode::LowHighByte => {
                if self.write_lsb_next {
                    self.initial_count = (self.initial_count & 0xFF00) | (value as u16);
                    self.write_lsb_next = false;
                } else {
                    self.initial_count = (self.initial_count & 0x00FF) | ((value as u16) << 8);
                    self.write_lsb_next = true;
                    self.load_count();
                }
            }
            PitAccessMode::Latch => {
                // Latch mode doesn't accept writes
            }
        }
    }

    /// Load the count from initial_count
    fn load_count(&mut self) {
        // Handle 0 meaning 65536
        self.count = if self.initial_count == 0 {
            0xFFFF
        } else {
            self.initial_count
        };
        self.null_count = false;
        self.counting = true;
        self.enabled = true;

        // Set initial output based on mode
        match self.mode {
            PitMode::InterruptOnTerminalCount | PitMode::SoftwareStrobe => {
                self.output = false;
            }
            PitMode::RateGenerator
            | PitMode::SquareWave
            | PitMode::HardwareOneShot
            | PitMode::HardwareStrobe => {
                self.output = true;
            }
        }

        tracing::debug!(
            "PIT: Counter {} loaded with {} (mode {:?})",
            self.number,
            self.count,
            self.mode
        );
    }

    /// Latch the current count value
    pub fn latch_count(&mut self) {
        if !self.count_latched {
            self.latched_count = self.count;
            self.count_latched = true;
            self.read_lsb_next = true;
        }
    }

    /// Latch the status register
    pub fn latch_status(&mut self) {
        if !self.status_latched {
            self.latched_status = ((self.output as u8) << STATUS_OUTPUT_SHIFT)
                | ((self.null_count as u8) << STATUS_NULL_COUNT_SHIFT)
                | ((self.access_mode as u8) << STATUS_ACCESS_MODE_SHIFT)
                | ((self.mode as u8) << STATUS_MODE_SHIFT)
                | (self.bcd_mode as u8);
            self.status_latched = true;
        }
    }

    /// Clock the counter (decrement by 1)
    pub fn clock(&mut self) -> bool {
        if !self.enabled || !self.gate || !self.counting {
            return false;
        }

        let old_output = self.output;

        match self.mode {
            PitMode::InterruptOnTerminalCount => {
                if self.count > 0 {
                    self.count -= 1;
                }
                if self.count == 0 {
                    self.output = true;
                }
            }
            PitMode::RateGenerator => {
                // Mode 2: output is normally HIGH.  When count decrements to 1,
                // output goes LOW for exactly one clock period, then reloads and
                // goes back HIGH (generating the IRQ on the rising edge).
                //
                // We must NOT set output back to HIGH in the same clock() call as
                // the LOW pulse, otherwise the transition check at the end will
                // never see a change.  Instead, leave it LOW for one tick; on the
                // *next* clock() call the "count > 1" path raises it back HIGH.
                if self.count > 1 {
                    self.count -= 1;
                    // Rising edge after the one-clock LOW pulse
                    if !self.output {
                        self.output = true;
                    }
                } else {
                    // Terminal count: reload and pulse output LOW
                    self.count = if self.initial_count == 0 {
                        0xFFFF
                    } else {
                        self.initial_count
                    };
                    self.output = false;
                }
            }
            PitMode::SquareWave => {
                if self.count > 1 {
                    self.count -= 2;
                } else {
                    self.count = if self.initial_count == 0 {
                        0xFFFF
                    } else {
                        self.initial_count
                    };
                    self.output = !self.output;
                }
            }
            PitMode::SoftwareStrobe => {
                if self.count > 0 {
                    self.count -= 1;
                }
                if self.count == 0 && !self.output {
                    self.output = true;
                    self.counting = false;
                }
            }
            _ => {
                if self.count > 0 {
                    self.count -= 1;
                }
            }
        }

        // Return true if output transitioned (for IRQ generation)
        old_output != self.output && self.output
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
        }
    }

    /// Initialize the PIT
    pub fn init(&mut self) {
        tracing::info!("PIT: Initializing 8254 Programmable Interval Timer");
        self.reset();
    }

    /// Reset the PIT
    pub fn reset(&mut self) {
        for counter in &mut self.counters {
            *counter = PitCounter::new(counter.number);
        }
        self.total_ticks = 0;
        self.irq0_pending = false;
    }

    /// Read from PIT I/O port
    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
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

    /// Write to the control register
    fn write_control(&mut self, value: u8) {
        let counter_num = (value >> CONTROL_COUNTER_SHIFT) & CONTROL_COUNTER_MASK;

        if counter_num == CONTROL_READBACK_SELECT {
            // Read-back command
            self.read_back(value);
            return;
        }

        let counter = &mut self.counters[counter_num as usize];
        let access_mode =
            PitAccessMode::from((value >> CONTROL_ACCESS_MODE_SHIFT) & CONTROL_ACCESS_MODE_MASK);

        if access_mode == PitAccessMode::Latch {
            // Counter latch command
            counter.latch_count();
            tracing::trace!(
                "PIT: Latched counter {} = {}",
                counter_num,
                counter.latched_count
            );
        } else {
            // Set counter mode
            counter.access_mode = access_mode;
            counter.mode = PitMode::from((value >> CONTROL_MODE_SHIFT) & CONTROL_MODE_MASK);
            counter.bcd_mode = (value & CONTROL_BCD_BIT) != 0;
            counter.write_lsb_next = true;
            counter.read_lsb_next = true;
            counter.null_count = true;
            counter.counting = false;

            tracing::debug!(
                "PIT: Counter {} configured: mode={:?}, access={:?}, bcd={}",
                counter_num,
                counter.mode,
                counter.access_mode,
                counter.bcd_mode
            );
        }
    }

    /// Handle read-back command
    fn read_back(&mut self, value: u8) {
        let latch_count = (value & READBACK_LATCH_COUNT_BIT) == 0;
        let latch_status = (value & READBACK_LATCH_STATUS_BIT) == 0;

        for i in 0..PIT_NUM_COUNTERS {
            if (value & (READBACK_COUNTER0_BIT << i)) != 0 {
                if latch_status {
                    self.counters[i].latch_status();
                }
                if latch_count {
                    self.counters[i].latch_count();
                }
            }
        }
    }

    /// Set gate input for counter 2 (speaker control)
    pub fn set_gate2(&mut self, gate: bool) {
        self.counters[2].gate = gate;
    }

    /// Get output state of counter 2 (speaker)
    pub fn get_output2(&self) -> bool {
        self.counters[2].output
    }

    /// Simulate time passing (in microseconds)
    pub fn tick(&mut self, usec: u64) -> bool {
        // Convert microseconds to PIT ticks
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

        irq0
    }

    /// Check and clear IRQ0 pending flag
    pub fn check_irq0(&mut self) -> bool {
        let pending = self.irq0_pending;
        self.irq0_pending = false;
        pending
    }
}

/// PIT read handler for I/O port infrastructure
pub fn pit_read_handler(this_ptr: *mut c_void, port: u16, io_len: u8) -> u32 {
    let pit = unsafe { &mut *(this_ptr as *mut BxPitC) };
    pit.read(port, io_len)
}

/// PIT write handler for I/O port infrastructure
pub fn pit_write_handler(this_ptr: *mut c_void, port: u16, value: u32, io_len: u8) {
    let pit = unsafe { &mut *(this_ptr as *mut BxPitC) };
    pit.write(port, value, io_len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pit_creation() {
        let pit = BxPitC::new();
        assert_eq!(pit.counters[0].number, 0);
        assert_eq!(pit.counters[1].number, 1);
        assert_eq!(pit.counters[2].number, 2);
    }

    #[test]
    fn test_pit_counter_write() {
        let mut pit = BxPitC::new();

        // Configure counter 0 for mode 2 (rate generator), low-high access
        pit.write(PIT_CONTROL, 0x34, 1); // Counter 0, low-high, mode 2

        // Write count value 0x1234
        pit.write(PIT_COUNTER0, 0x34, 1); // Low byte
        pit.write(PIT_COUNTER0, 0x12, 1); // High byte

        assert_eq!(pit.counters[0].initial_count, 0x1234);
        assert_eq!(pit.counters[0].count, 0x1234);
    }
}
