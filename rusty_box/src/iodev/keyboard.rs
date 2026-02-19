//! 8042 Keyboard Controller Emulation
//!
//! The 8042 PS/2 controller handles:
//! - Keyboard input (IRQ1)
//! - Mouse input (IRQ12)
//! - A20 gate control
//! - System reset
//!
//! I/O Ports:
//! - 0x60: Data port (read/write)
//! - 0x64: Status/command port (read=status, write=command)

use alloc::collections::VecDeque;
use core::ffi::c_void;

/// Keyboard controller I/O ports
pub const KBD_DATA_PORT: u16 = 0x0060;
pub const KBD_STATUS_PORT: u16 = 0x0064;
pub const KBD_COMMAND_PORT: u16 = 0x0064;

/// Port 0x61 - System control port B
pub const SYSTEM_CONTROL_B: u16 = 0x0061;

/// Status register bits
pub const KBD_STATUS_OBF: u8 = 0x01;      // Output buffer full
pub const KBD_STATUS_IBF: u8 = 0x02;      // Input buffer full
pub const KBD_STATUS_SYS: u8 = 0x04;      // System flag
pub const KBD_STATUS_CMD: u8 = 0x08;      // Command/data (0=data, 1=command)
pub const KBD_STATUS_KEYL: u8 = 0x10;     // Keyboard lock
pub const KBD_STATUS_AUXB: u8 = 0x20;     // Aux output buffer full (mouse)
pub const KBD_STATUS_TIMEOUT: u8 = 0x40;  // General timeout
pub const KBD_STATUS_PARITY: u8 = 0x80;   // Parity error

/// Controller commands
pub const KBD_CMD_READ_CCB: u8 = 0x20;        // Read controller configuration byte
pub const KBD_CMD_WRITE_CCB: u8 = 0x60;       // Write controller configuration byte
pub const KBD_CMD_DISABLE_AUX: u8 = 0xA7;     // Disable aux interface
pub const KBD_CMD_ENABLE_AUX: u8 = 0xA8;      // Enable aux interface
pub const KBD_CMD_TEST_AUX: u8 = 0xA9;        // Test aux interface
pub const KBD_CMD_SELF_TEST: u8 = 0xAA;       // Controller self-test
pub const KBD_CMD_KBD_TEST: u8 = 0xAB;        // Keyboard interface test
pub const KBD_CMD_DISABLE_KBD: u8 = 0xAD;     // Disable keyboard interface
pub const KBD_CMD_ENABLE_KBD: u8 = 0xAE;      // Enable keyboard interface
pub const KBD_CMD_READ_INPUT: u8 = 0xC0;      // Read input port
pub const KBD_CMD_READ_OUTPUT: u8 = 0xD0;     // Read output port
pub const KBD_CMD_WRITE_OUTPUT: u8 = 0xD1;    // Write output port
pub const KBD_CMD_WRITE_KBD: u8 = 0xD2;       // Write to keyboard output buffer
pub const KBD_CMD_WRITE_AUX: u8 = 0xD3;       // Write to aux output buffer
pub const KBD_CMD_WRITE_AUX_INPUT: u8 = 0xD4; // Write to aux device
pub const KBD_CMD_PULSE_OUTPUT: u8 = 0xF0;    // Pulse output port (0xFE = reset)

/// Keyboard commands
pub const KBD_KB_CMD_RESET: u8 = 0xFF;
pub const KBD_KB_CMD_RESEND: u8 = 0xFE;
pub const KBD_KB_CMD_SET_DEFAULTS: u8 = 0xF6;
pub const KBD_KB_CMD_DISABLE: u8 = 0xF5;
pub const KBD_KB_CMD_ENABLE: u8 = 0xF4;
pub const KBD_KB_CMD_SET_TYPEMATIC: u8 = 0xF3;
pub const KBD_KB_CMD_ECHO: u8 = 0xEE;
pub const KBD_KB_CMD_SET_LEDS: u8 = 0xED;
pub const KBD_KB_CMD_SET_SCANCODE: u8 = 0xF0;

/// Controller configuration byte bits
pub const CCB_INT_KBD: u8 = 0x01;    // Keyboard interrupt enable
pub const CCB_INT_AUX: u8 = 0x02;    // Aux interrupt enable
pub const CCB_SYS_FLAG: u8 = 0x04;   // System flag
pub const CCB_DIS_KBD: u8 = 0x10;    // Disable keyboard clock
pub const CCB_DIS_AUX: u8 = 0x20;    // Disable aux clock
pub const CCB_XLAT: u8 = 0x40;       // Scancode translation

/// Keyboard state
#[derive(Debug, Clone)]
pub struct KeyboardState {
    /// Keyboard enabled
    pub enabled: bool,
    /// Keyboard output buffer
    pub output_buffer: VecDeque<u8>,
    /// Expecting parameter for command
    pub expecting_param: bool,
    /// Current command waiting for parameter
    pub current_cmd: u8,
    /// LED state
    pub led_state: u8,
    /// Typematic rate
    pub typematic_rate: u8,
    /// Scancode set (1, 2, or 3)
    pub scancode_set: u8,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self {
            enabled: true,
            output_buffer: VecDeque::new(),
            expecting_param: false,
            current_cmd: 0,
            led_state: 0,
            typematic_rate: 0,
            scancode_set: 2, // Default to scancode set 2
        }
    }
}

/// Mouse state
#[derive(Debug, Clone, Default)]
pub struct MouseState {
    /// Mouse enabled
    pub enabled: bool,
    /// Mouse output buffer
    pub output_buffer: VecDeque<u8>,
    /// Sample rate
    pub sample_rate: u8,
    /// Resolution
    pub resolution: u8,
    /// Scaling (1:1 or 2:1)
    pub scaling: bool,
}

/// 8042 Keyboard Controller
#[derive(Debug)]
pub struct BxKeyboardC {
    /// Status register
    pub status: u8,
    /// Controller configuration byte
    pub ccb: u8,
    /// Output port (bits 0-1 control A20 and reset)
    pub output_port: u8,
    /// Input port
    pub input_port: u8,
    /// Controller output buffer
    pub output_buffer: u8,
    /// Is output from aux device?
    pub output_aux: bool,
    /// Command byte waiting for data
    pub pending_command: Option<u8>,
    /// Keyboard state
    pub keyboard: KeyboardState,
    /// Mouse state
    pub mouse: MouseState,
    /// System control port B state
    pub system_control_b: u8,
    /// A20 gate state
    pub a20_enabled: bool,
    /// IRQ1 pending (keyboard)
    pub irq1_pending: bool,
    /// IRQ12 pending (mouse)
    pub irq12_pending: bool,
}

impl Default for BxKeyboardC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxKeyboardC {
    /// Create a new keyboard controller
    pub fn new() -> Self {
        Self {
            status: KBD_STATUS_SYS, // System flag set after POST
            ccb: CCB_INT_KBD | CCB_INT_AUX | CCB_SYS_FLAG | CCB_XLAT,
            output_port: 0xCF, // A20 enabled, reset line high
            input_port: 0x80,
            output_buffer: 0,
            output_aux: false,
            pending_command: None,
            keyboard: KeyboardState::default(),
            mouse: MouseState::default(),
            system_control_b: 0,
            a20_enabled: true,
            irq1_pending: false,
            irq12_pending: false,
        }
    }

    /// Initialize the keyboard controller
    pub fn init(&mut self) {
        tracing::info!("Keyboard: Initializing 8042 PS/2 Controller");
        self.reset();
    }

    /// Reset the keyboard controller
    pub fn reset(&mut self) {
        self.status = KBD_STATUS_SYS;
        self.ccb = CCB_INT_KBD | CCB_INT_AUX | CCB_SYS_FLAG | CCB_XLAT;
        self.output_port = 0xCF;
        self.pending_command = None;
        self.keyboard = KeyboardState::default();
        self.mouse = MouseState::default();
        self.a20_enabled = true;
        self.irq1_pending = false;
        self.irq12_pending = false;
    }

    /// Read from keyboard I/O port
    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        match port {
            KBD_DATA_PORT => {
                let data = self.output_buffer;
                self.status &= !(KBD_STATUS_OBF | KBD_STATUS_AUXB);
                self.irq1_pending = false;
                self.irq12_pending = false;

                // Load next byte from keyboard/mouse buffer
                self.update_output_buffer();

                tracing::debug!("Keyboard: Read data port 0x60 = {:#04x}, status now = {:#04x}", data, self.status);
                data as u32
            }
            KBD_STATUS_PORT => {
                tracing::trace!("Keyboard: Read status port 0x64 = {:#04x}", self.status);
                self.status as u32
            }
            SYSTEM_CONTROL_B => {
                // Toggle bit 4 (PIT channel 2 output) on each read to simulate timing.
                // The BIOS delay_ms() polls this bit waiting for transitions; if it never
                // changes, delay_ms() hangs forever. Real hardware toggles at ~18Hz.
                self.system_control_b ^= 0x10;
                let value = self.system_control_b;
                tracing::trace!("Keyboard: Read system control B = {:#04x}", value);
                value as u32
            }
            _ => {
                tracing::warn!("Keyboard: Unknown read port {:#06x}", port);
                0xFF
            }
        }
    }

    /// Write to keyboard I/O port
    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let value = value as u8;
        match port {
            KBD_DATA_PORT => {
                tracing::trace!("Keyboard: Write data = {:#04x}", value);
                self.write_data(value);
            }
            KBD_COMMAND_PORT => {
                tracing::debug!("Keyboard: Write command port 0x64 = {:#04x}", value);
                self.write_command(value);
            }
            SYSTEM_CONTROL_B => {
                tracing::trace!("Keyboard: Write system control B = {:#04x}", value);
                self.system_control_b = value;
                // Bit 0: PIT timer 2 gate
                // Bit 1: Speaker enable
            }
            _ => {
                tracing::warn!("Keyboard: Unknown write port {:#06x} value={:#04x}", port, value);
            }
        }
    }

    /// Write to data port
    fn write_data(&mut self, value: u8) {
        if let Some(cmd) = self.pending_command {
            self.pending_command = None;
            match cmd {
                KBD_CMD_WRITE_CCB => {
                    self.ccb = value;
                    tracing::debug!("Keyboard: CCB set to {:#04x}", value);
                }
                KBD_CMD_WRITE_OUTPUT => {
                    self.write_output_port(value);
                }
                KBD_CMD_WRITE_KBD => {
                    self.queue_keyboard_byte(value);
                }
                KBD_CMD_WRITE_AUX => {
                    self.queue_mouse_byte(value);
                }
                KBD_CMD_WRITE_AUX_INPUT => {
                    // Send to mouse device
                    self.handle_mouse_command(value);
                }
                _ => {}
            }
        } else {
            // Data to keyboard
            self.handle_keyboard_command(value);
        }
    }

    /// Write to command port
    fn write_command(&mut self, value: u8) {
        match value {
            KBD_CMD_READ_CCB => {
                self.queue_controller_byte(self.ccb);
            }
            KBD_CMD_WRITE_CCB => {
                self.pending_command = Some(value);
            }
            KBD_CMD_DISABLE_AUX => {
                self.ccb |= CCB_DIS_AUX;
                self.mouse.enabled = false;
            }
            KBD_CMD_ENABLE_AUX => {
                self.ccb &= !CCB_DIS_AUX;
                self.mouse.enabled = true;
            }
            KBD_CMD_TEST_AUX => {
                self.queue_controller_byte(0x00); // Test passed
            }
            KBD_CMD_SELF_TEST => {
                self.status |= KBD_STATUS_SYS;
                self.queue_controller_byte(0x55); // Self-test passed
                tracing::debug!("Keyboard: Self-test passed");
            }
            KBD_CMD_KBD_TEST => {
                self.queue_controller_byte(0x00); // Test passed
            }
            KBD_CMD_DISABLE_KBD => {
                self.ccb |= CCB_DIS_KBD;
                self.keyboard.enabled = false;
                tracing::debug!("Keyboard: Keyboard disabled");
            }
            KBD_CMD_ENABLE_KBD => {
                self.ccb &= !CCB_DIS_KBD;
                self.keyboard.enabled = true;
                tracing::debug!("Keyboard: Keyboard enabled");
            }
            KBD_CMD_READ_INPUT => {
                self.queue_controller_byte(self.input_port);
            }
            KBD_CMD_READ_OUTPUT => {
                self.queue_controller_byte(self.output_port);
            }
            KBD_CMD_WRITE_OUTPUT | KBD_CMD_WRITE_KBD | 
            KBD_CMD_WRITE_AUX | KBD_CMD_WRITE_AUX_INPUT => {
                self.pending_command = Some(value);
            }
            0xF0..=0xFF => {
                // Pulse output port lines
                let pulse = !(value & 0x0F);
                if (pulse & 0x01) != 0 {
                    // Bit 0 low = system reset
                    tracing::warn!("Keyboard: System reset requested via pulse");
                }
            }
            _ => {
                if (0x20..0x40).contains(&value) {
                    // Read internal RAM
                    let offset = (value - 0x20) as usize;
                    if offset == 0 {
                        self.queue_controller_byte(self.ccb);
                    } else {
                        self.queue_controller_byte(0);
                    }
                } else if (0x60..0x80).contains(&value) {
                    // Write internal RAM
                    self.pending_command = Some(value);
                } else {
                    tracing::trace!("Keyboard: Unknown command {:#04x}", value);
                }
            }
        }
    }

    /// Write to output port
    fn write_output_port(&mut self, value: u8) {
        self.output_port = value;
        
        // Bit 0: System reset (0 = reset)
        if (value & 0x01) == 0 {
            tracing::warn!("Keyboard: System reset via output port");
        }
        
        // Bit 1: A20 gate
        let new_a20 = (value & 0x02) != 0;
        if self.a20_enabled != new_a20 {
            self.a20_enabled = new_a20;
            tracing::debug!("Keyboard: A20 gate = {}", new_a20);
        }
    }

    /// Handle keyboard command
    fn handle_keyboard_command(&mut self, value: u8) {
        if self.keyboard.expecting_param {
            self.keyboard.expecting_param = false;
            match self.keyboard.current_cmd {
                KBD_KB_CMD_SET_LEDS => {
                    self.keyboard.led_state = value;
                    self.queue_keyboard_byte(0xFA); // ACK
                }
                KBD_KB_CMD_SET_TYPEMATIC => {
                    self.keyboard.typematic_rate = value;
                    self.queue_keyboard_byte(0xFA); // ACK
                }
                KBD_KB_CMD_SET_SCANCODE => {
                    if value == 0 {
                        // Query current scancode set
                        self.queue_keyboard_byte(0xFA); // ACK
                        self.queue_keyboard_byte(self.keyboard.scancode_set);
                    } else if value <= 3 {
                        self.keyboard.scancode_set = value;
                        self.queue_keyboard_byte(0xFA); // ACK
                    }
                }
                _ => {}
            }
            return;
        }

        match value {
            KBD_KB_CMD_RESET => {
                self.queue_keyboard_byte(0xFA); // ACK
                self.queue_keyboard_byte(0xAA); // BAT passed
                tracing::debug!("Keyboard: Reset");
            }
            KBD_KB_CMD_RESEND => {
                // Resend last byte (not implemented)
                self.queue_keyboard_byte(0xFE);
            }
            KBD_KB_CMD_SET_DEFAULTS => {
                self.keyboard.typematic_rate = 0;
                self.keyboard.led_state = 0;
                self.queue_keyboard_byte(0xFA); // ACK
            }
            KBD_KB_CMD_DISABLE => {
                self.keyboard.enabled = false;
                self.queue_keyboard_byte(0xFA); // ACK
            }
            KBD_KB_CMD_ENABLE => {
                self.keyboard.enabled = true;
                self.queue_keyboard_byte(0xFA); // ACK
            }
            KBD_KB_CMD_SET_TYPEMATIC | KBD_KB_CMD_SET_LEDS | KBD_KB_CMD_SET_SCANCODE => {
                self.keyboard.expecting_param = true;
                self.keyboard.current_cmd = value;
                self.queue_keyboard_byte(0xFA); // ACK
            }
            KBD_KB_CMD_ECHO => {
                self.queue_keyboard_byte(0xEE); // Echo
            }
            0xF2 => {
                // Read keyboard ID
                self.queue_keyboard_byte(0xFA); // ACK
                self.queue_keyboard_byte(0xAB); // Keyboard ID byte 1
                self.queue_keyboard_byte(0x83); // Keyboard ID byte 2
            }
            _ => {
                self.queue_keyboard_byte(0xFA); // ACK
            }
        }
    }

    /// Handle mouse command
    fn handle_mouse_command(&mut self, value: u8) {
        match value {
            0xFF => {
                // Reset
                self.queue_mouse_byte(0xFA); // ACK
                self.queue_mouse_byte(0xAA); // BAT passed
                self.queue_mouse_byte(0x00); // Device ID
            }
            0xF4 => {
                // Enable
                self.mouse.enabled = true;
                self.queue_mouse_byte(0xFA); // ACK
            }
            0xF5 => {
                // Disable
                self.mouse.enabled = false;
                self.queue_mouse_byte(0xFA); // ACK
            }
            0xF2 => {
                // Read device ID
                self.queue_mouse_byte(0xFA); // ACK
                self.queue_mouse_byte(0x00); // Standard mouse
            }
            _ => {
                self.queue_mouse_byte(0xFA); // ACK
            }
        }
    }

    /// Queue a byte from controller to output buffer
    fn queue_controller_byte(&mut self, byte: u8) {
        self.output_buffer = byte;
        self.output_aux = false;
        self.status |= KBD_STATUS_OBF;
    }

    /// Queue a byte from keyboard
    fn queue_keyboard_byte(&mut self, byte: u8) {
        self.keyboard.output_buffer.push_back(byte);
        self.update_output_buffer();
    }

    /// Queue a byte from mouse
    fn queue_mouse_byte(&mut self, byte: u8) {
        self.mouse.output_buffer.push_back(byte);
        self.update_output_buffer();
    }

    /// Update the output buffer from keyboard/mouse queues
    fn update_output_buffer(&mut self) {
        if (self.status & KBD_STATUS_OBF) == 0 {
            // Try keyboard first, then mouse
            if let Some(byte) = self.keyboard.output_buffer.pop_front() {
                self.output_buffer = byte;
                self.output_aux = false;
                self.status |= KBD_STATUS_OBF;
                if (self.ccb & CCB_INT_KBD) != 0 {
                    self.irq1_pending = true;
                }
            } else if let Some(byte) = self.mouse.output_buffer.pop_front() {
                self.output_buffer = byte;
                self.output_aux = true;
                self.status |= KBD_STATUS_OBF | KBD_STATUS_AUXB;
                if (self.ccb & CCB_INT_AUX) != 0 {
                    self.irq12_pending = true;
                }
            }
        }
    }

    /// Send a scancode to the keyboard
    pub fn send_scancode(&mut self, scancode: u8) {
        if self.keyboard.enabled && (self.ccb & CCB_DIS_KBD) == 0 {
            self.queue_keyboard_byte(scancode);
        }
    }

    /// Check and clear IRQ1 pending
    pub fn check_irq1(&mut self) -> bool {
        let pending = self.irq1_pending;
        self.irq1_pending = false;
        pending
    }

    /// Check and clear IRQ12 pending
    pub fn check_irq12(&mut self) -> bool {
        let pending = self.irq12_pending;
        self.irq12_pending = false;
        pending
    }

    /// Get A20 gate state
    pub fn get_a20_enabled(&self) -> bool {
        self.a20_enabled
    }
}

/// Keyboard read handler for I/O port infrastructure
pub fn keyboard_read_handler(this_ptr: *mut c_void, port: u16, io_len: u8) -> u32 {
    let kbd = unsafe { &mut *(this_ptr as *mut BxKeyboardC) };
    kbd.read(port, io_len)
}

/// Keyboard write handler for I/O port infrastructure
pub fn keyboard_write_handler(this_ptr: *mut c_void, port: u16, value: u32, io_len: u8) {
    let kbd = unsafe { &mut *(this_ptr as *mut BxKeyboardC) };
    kbd.write(port, value, io_len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyboard_creation() {
        let kbd = BxKeyboardC::new();
        assert!(kbd.a20_enabled);
        assert!((kbd.status & KBD_STATUS_SYS) != 0);
    }

    #[test]
    fn test_keyboard_self_test() {
        let mut kbd = BxKeyboardC::new();
        
        // Send self-test command
        kbd.write(KBD_COMMAND_PORT, KBD_CMD_SELF_TEST as u32, 1);
        
        // Should have response in output buffer
        assert!((kbd.status & KBD_STATUS_OBF) != 0);
        
        let response = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(response, 0x55); // Self-test passed
    }
}

