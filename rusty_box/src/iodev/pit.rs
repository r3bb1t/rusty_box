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
    pub number: u8,
    /// Current count value
    pub count: u16,
    /// Initial count (reload value)
    pub initial_count: u16,
    /// Count latched for reading
    pub latched_count: u16,
    /// Is count latched?
    pub count_latched: bool,
    /// Status latched for reading
    pub status_latched: bool,
    /// Latched status value
    pub latched_status: u8,
    /// Operating mode
    pub mode: PitMode,
    /// Access mode (low/high byte)
    pub access_mode: PitAccessMode,
    /// BCD mode (false = binary)
    pub bcd_mode: bool,
    /// Next byte is low (for low-high access)
    pub read_lsb_next: bool,
    /// Next write is low (for low-high access)
    pub write_lsb_next: bool,
    /// Output pin state
    pub output: bool,
    /// Gate input state
    pub gate: bool,
    /// Counter is enabled
    pub enabled: bool,
    /// Null count (count not yet loaded)
    pub null_count: bool,
    /// Countdown in progress
    pub counting: bool,
}

impl Default for PitCounter {
    fn default() -> Self {
        Self {
            number: 0,
            count: 0,
            initial_count: 0,
            latched_count: 0,
            count_latched: false,
            status_latched: false,
            latched_status: 0,
            mode: PitMode::InterruptOnTerminalCount,
            access_mode: PitAccessMode::LowHighByte,
            bcd_mode: false,
            read_lsb_next: true,
            write_lsb_next: true,
            output: false,
            gate: true, // Gate is typically high
            enabled: false,
            null_count: true,
            counting: false,
        }
    }
}

impl PitCounter {
    /// Create a new counter with specified number
    pub fn new(number: u8) -> Self {
        let mut counter = Self::default();
        counter.number = number;
        // Counter 2 gate is controlled by port 0x61
        counter.gate = number != 2;
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
        self.count = if self.initial_count == 0 { 0xFFFF } else { self.initial_count };
        self.null_count = false;
        self.counting = true;
        self.enabled = true;
        
        // Set initial output based on mode
        match self.mode {
            PitMode::InterruptOnTerminalCount | PitMode::SoftwareStrobe => {
                self.output = false;
            }
            PitMode::RateGenerator | PitMode::SquareWave | 
            PitMode::HardwareOneShot | PitMode::HardwareStrobe => {
                self.output = true;
            }
        }
        
        tracing::debug!(
            "PIT: Counter {} loaded with {} (mode {:?})",
            self.number, self.count, self.mode
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
            self.latched_status = 
                ((self.output as u8) << 7) |
                ((self.null_count as u8) << 6) |
                ((self.access_mode as u8) << 4) |
                ((self.mode as u8) << 1) |
                (self.bcd_mode as u8);
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
                if self.count > 1 {
                    self.count -= 1;
                } else {
                    // Reload and pulse output low
                    self.count = if self.initial_count == 0 { 0xFFFF } else { self.initial_count };
                    self.output = false;
                }
                if !self.output {
                    self.output = true;
                }
            }
            PitMode::SquareWave => {
                if self.count > 1 {
                    self.count -= 2;
                } else {
                    self.count = if self.initial_count == 0 { 0xFFFF } else { self.initial_count };
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
    pub counters: [PitCounter; 3],
    /// Total ticks elapsed
    pub total_ticks: u64,
    /// Timer handles for scheduling
    pub timer_handles: [Option<usize>; 3],
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
            counters: [
                PitCounter::new(0),
                PitCounter::new(1),
                PitCounter::new(2),
            ],
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
        let counter_num = (value >> 6) & 0x03;
        
        if counter_num == 3 {
            // Read-back command
            self.read_back(value);
            return;
        }

        let counter = &mut self.counters[counter_num as usize];
        let access_mode = PitAccessMode::from((value >> 4) & 0x03);

        if access_mode == PitAccessMode::Latch {
            // Counter latch command
            counter.latch_count();
            tracing::trace!("PIT: Latched counter {} = {}", counter_num, counter.latched_count);
        } else {
            // Set counter mode
            counter.access_mode = access_mode;
            counter.mode = PitMode::from((value >> 1) & 0x07);
            counter.bcd_mode = (value & 0x01) != 0;
            counter.write_lsb_next = true;
            counter.read_lsb_next = true;
            counter.null_count = true;
            counter.counting = false;
            
            tracing::debug!(
                "PIT: Counter {} configured: mode={:?}, access={:?}, bcd={}",
                counter_num, counter.mode, counter.access_mode, counter.bcd_mode
            );
        }
    }

    /// Handle read-back command
    fn read_back(&mut self, value: u8) {
        let latch_count = (value & 0x02) == 0;
        let latch_status = (value & 0x10) == 0;

        for i in 0..3 {
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

