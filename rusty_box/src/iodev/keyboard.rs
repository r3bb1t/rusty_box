//! 8042 Keyboard Controller Emulation
//!
//! Faithful port of Bochs keyboard.cc — full PS/2 controller with:
//! - Individual boolean status bits (assembled on port 0x64 read)
//! - Ring-buffered kbd_internal_buffer / mouse_internal_buffer
//! - controller_Q overflow queue (5 entries)
//! - timer_pending / periodic() for deferred buffer transfers
//! - kbd_ctrl_to_kbd / kbd_ctrl_to_mouse command handlers
//! - Keyboard BAT, scancode sets, LED/typematic sub-commands
//! - Mouse PS/2 protocol (reset, sample rate, resolution, stream mode)
//!
//! Reference: cpp_orig/bochs/iodev/keyboard.cc (1477 lines)

use core::ffi::c_void;

// I/O Ports
pub const KBD_DATA_PORT: u16 = 0x0060;
pub const KBD_STATUS_PORT: u16 = 0x0064;
pub const KBD_COMMAND_PORT: u16 = 0x0064;
pub const SYSTEM_CONTROL_B: u16 = 0x0061;

// Buffer sizes (matching Bochs)
const BX_KBD_ELEMENTS: usize = 16;
const BX_MOUSE_BUFF_SIZE: usize = 48;
const BX_KBD_CONTROLLER_QSIZE: usize = 5;

// Keyboard types
const BX_KBD_XT_TYPE: u8 = 0;
const BX_KBD_MF_TYPE: u8 = 2;

// Mouse modes
const MOUSE_MODE_RESET: u8 = 10;
const MOUSE_MODE_STREAM: u8 = 11;
const MOUSE_MODE_REMOTE: u8 = 12;
const MOUSE_MODE_WRAP: u8 = 13;

// Mouse types
const BX_MOUSE_TYPE_PS2: u8 = 2;
const BX_MOUSE_TYPE_IMPS2: u8 = 3;

/// 8042 Controller state — individual booleans matching Bochs kbd_controller struct
#[derive(Debug, Clone)]
pub struct KbdController {
    // Status register bits (assembled on port 0x64 read)
    pub pare: bool, // bit 7: parity error
    pub tim: bool,  // bit 6: timeout (cleared on each status read!)
    pub auxb: bool, // bit 5: mouse data in output buffer
    pub keyl: bool, // bit 4: keyboard lock (init = true)
    pub c_d: bool,  // bit 3: last write was command(1) / data(0)
    pub sysf: bool, // bit 2: system flag (set after self-test)
    pub inpb: bool, // bit 1: input buffer full
    pub outb: bool, // bit 0: output buffer full

    // Internal controller state
    pub kbd_clock_enabled: bool,
    pub aux_clock_enabled: bool,
    pub allow_irq1: bool,
    pub allow_irq12: bool,
    pub kbd_output_buffer: u8,
    pub aux_output_buffer: u8,
    pub last_comm: u8,
    pub expecting_port60h: u8,
    pub expecting_mouse_parameter: u8,
    pub last_mouse_command: u8,
    pub timer_pending: u32,
    pub irq1_requested: bool,
    pub irq12_requested: bool,
    pub scancodes_translate: bool,
    pub expecting_scancodes_set: bool,
    pub current_scancodes_set: u8,
    pub bat_in_progress: bool,
    pub kbd_type: u8,
}

/// Keyboard internal ring buffer
#[derive(Debug, Clone)]
pub struct KbdInternalBuffer {
    pub buffer: [u8; BX_KBD_ELEMENTS],
    pub head: usize,
    pub num_elements: usize,
    pub expecting_typematic: bool,
    pub expecting_led_write: bool,
    pub delay: u8,
    pub repeat_rate: u8,
    pub led_status: u8,
    pub scanning_enabled: bool,
}

/// Mouse internal ring buffer
#[derive(Debug, Clone)]
pub struct MouseInternalBuffer {
    pub buffer: [u8; BX_MOUSE_BUFF_SIZE],
    pub head: usize,
    pub num_elements: usize,
}

/// Mouse device state
#[derive(Debug, Clone)]
pub struct MouseState {
    pub mouse_type: u8,
    pub sample_rate: u8,
    pub resolution_cpmm: u8,
    pub scaling: u8,
    pub mode: u8,
    pub saved_mode: u8,
    pub enable: bool,
    pub button_status: u8,
    pub delayed_dx: i16,
    pub delayed_dy: i16,
    pub delayed_dz: i16,
    pub im_request: u8,
    pub im_mode: bool,
}

/// 8042 Keyboard Controller — full Bochs-compatible implementation
#[derive(Debug)]
pub struct BxKeyboardC {
    pub kbd_controller: KbdController,
    pub kbd_internal_buffer: KbdInternalBuffer,
    pub mouse_internal_buffer: MouseInternalBuffer,
    pub mouse: MouseState,
    pub controller_q: [u8; BX_KBD_CONTROLLER_QSIZE],
    pub controller_q_size: usize,
    pub controller_q_source: u8,
    /// System Control Port B (port 0x61)
    pub system_control_b: u8,
    /// A20 gate state (managed via output port / D1 command)
    pub a20_enabled: bool,
    /// Flag set when A20 state changes, cleared by emulator after propagation
    pub a20_change_pending: bool,
    /// First self-test flag (Bochs static kbd_initialized)
    kbd_initialized: bool,
}

impl Default for BxKeyboardC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxKeyboardC {
    /// Create a new keyboard controller (matching Bochs keyboard.cc init())
    pub fn new() -> Self {
        Self {
            kbd_controller: KbdController {
                pare: false,
                tim: false,
                auxb: false,
                keyl: true,  // keyboard lock = locked initially
                c_d: true,   // last write was command
                sysf: false, // not set until self-test passes
                inpb: false,
                outb: false,
                kbd_clock_enabled: true,
                aux_clock_enabled: false,
                allow_irq1: true,
                allow_irq12: true,
                kbd_output_buffer: 0,
                aux_output_buffer: 0,
                last_comm: 0,
                expecting_port60h: 0,
                expecting_mouse_parameter: 0,
                last_mouse_command: 0,
                timer_pending: 0,
                irq1_requested: false,
                irq12_requested: false,
                scancodes_translate: true,
                expecting_scancodes_set: false,
                current_scancodes_set: 1, // mf2 (0-indexed: 0=set1, 1=set2, 2=set3)
                bat_in_progress: false,
                kbd_type: BX_KBD_MF_TYPE,
            },
            kbd_internal_buffer: KbdInternalBuffer {
                buffer: [0; BX_KBD_ELEMENTS],
                head: 0,
                num_elements: 0,
                expecting_typematic: false,
                expecting_led_write: false,
                delay: 1,          // 500 mS
                repeat_rate: 0x0b, // 10.9 chars/sec
                led_status: 0,
                scanning_enabled: true,
            },
            mouse_internal_buffer: MouseInternalBuffer {
                buffer: [0; BX_MOUSE_BUFF_SIZE],
                head: 0,
                num_elements: 0,
            },
            mouse: MouseState {
                mouse_type: BX_MOUSE_TYPE_PS2,
                sample_rate: 100,
                resolution_cpmm: 4,
                scaling: 1,
                mode: MOUSE_MODE_RESET,
                saved_mode: MOUSE_MODE_RESET,
                enable: false,
                button_status: 0,
                delayed_dx: 0,
                delayed_dy: 0,
                delayed_dz: 0,
                im_request: 0,
                im_mode: false,
            },
            controller_q: [0; BX_KBD_CONTROLLER_QSIZE],
            controller_q_size: 0,
            controller_q_source: 0,
            system_control_b: 0,
            a20_enabled: true,
            a20_change_pending: false,
            kbd_initialized: false,
        }
    }

    /// Initialize the keyboard controller
    pub fn init(&mut self) {
        tracing::info!("Keyboard: Initializing 8042 PS/2 Controller");
        self.resetinternals(true);
    }

    /// Reset the keyboard controller (matches Bochs keyboard.cc reset())
    pub fn reset(&mut self) {
        self.kbd_internal_buffer.led_status = 0;
    }

    /// Flush internal buffer and reset keyboard settings (keyboard.cc:87-105)
    fn resetinternals(&mut self, powerup: bool) {
        self.kbd_internal_buffer.num_elements = 0;
        self.kbd_internal_buffer.buffer = [0; BX_KBD_ELEMENTS];
        self.kbd_internal_buffer.head = 0;
        self.kbd_internal_buffer.expecting_typematic = false;

        // Default scancode set is mf2 (translation controlled by 8042)
        self.kbd_controller.expecting_scancodes_set = false;
        self.kbd_controller.current_scancodes_set = 1;

        if powerup {
            self.kbd_internal_buffer.expecting_led_write = false;
            self.kbd_internal_buffer.delay = 1; // 500 mS
            self.kbd_internal_buffer.repeat_rate = 0x0b; // 10.9 chars/sec
        }
    }

    // =========================================================================
    // Port read handler
    // =========================================================================

    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        match port {
            KBD_DATA_PORT => self.read_port_60(),
            KBD_STATUS_PORT => self.read_port_64(),
            SYSTEM_CONTROL_B => {
                // Toggle bit 4 (PIT channel 2 output) on each read.
                // The BIOS delay_ms() polls this bit waiting for transitions.
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

    /// Port 0x60 read — keyboard.cc:292-348
    fn read_port_60(&mut self) -> u32 {
        if self.kbd_controller.auxb {
            // Mouse byte available
            let val = self.kbd_controller.aux_output_buffer;
            self.kbd_controller.aux_output_buffer = 0;
            self.kbd_controller.outb = false;
            self.kbd_controller.auxb = false;
            self.kbd_controller.irq12_requested = false;

            if self.controller_q_size > 0 {
                self.kbd_controller.aux_output_buffer = self.controller_q[0];
                self.kbd_controller.outb = true;
                self.kbd_controller.auxb = true;
                if self.kbd_controller.allow_irq12 {
                    self.kbd_controller.irq12_requested = true;
                }
                for i in 0..self.controller_q_size - 1 {
                    self.controller_q[i] = self.controller_q[i + 1];
                }
                self.controller_q_size -= 1;
            }

            self.activate_timer();
            tracing::debug!(
                "Keyboard: Read port 0x60 [mouse] = {:#04x}",
                val
            );
            val as u32
        } else if self.kbd_controller.outb {
            // Keyboard byte available
            let val = self.kbd_controller.kbd_output_buffer;
            self.kbd_controller.outb = false;
            self.kbd_controller.auxb = false;
            self.kbd_controller.irq1_requested = false;
            self.kbd_controller.bat_in_progress = false;

            if self.controller_q_size > 0 {
                // Drain controller_Q — matching Bochs line 325-338
                self.kbd_controller.aux_output_buffer = self.controller_q[0];
                self.kbd_controller.outb = true;
                self.kbd_controller.auxb = true;
                if self.kbd_controller.allow_irq1 {
                    self.kbd_controller.irq1_requested = true;
                }
                for i in 0..self.controller_q_size - 1 {
                    self.controller_q[i] = self.controller_q[i + 1];
                }
                self.controller_q_size -= 1;
            }

            self.activate_timer();
            tracing::debug!("Keyboard: Read port 0x60 [kbd] = {:#04x}", val);
            val as u32
        } else {
            // Nothing ready — return last value
            tracing::debug!(
                "Keyboard: Read port 0x60 (outb empty) = {:#04x}",
                self.kbd_controller.kbd_output_buffer
            );
            self.kbd_controller.kbd_output_buffer as u32
        }
    }

    /// Port 0x64 read — assemble status byte from individual booleans (keyboard.cc:349-360)
    fn read_port_64(&mut self) -> u32 {
        let val = ((self.kbd_controller.pare as u8) << 7)
            | ((self.kbd_controller.tim as u8) << 6)
            | ((self.kbd_controller.auxb as u8) << 5)
            | ((self.kbd_controller.keyl as u8) << 4)
            | ((self.kbd_controller.c_d as u8) << 3)
            | ((self.kbd_controller.sysf as u8) << 2)
            | ((self.kbd_controller.inpb as u8) << 1)
            | (self.kbd_controller.outb as u8);
        self.kbd_controller.tim = false; // cleared on each status read
        tracing::trace!("Keyboard: Read status 0x64 = {:#04x}", val);
        val as u32
    }

    // =========================================================================
    // Port write handler
    // =========================================================================

    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let value_u8 = value as u8;
        match port {
            KBD_DATA_PORT => self.write_port_60(value_u8),
            KBD_COMMAND_PORT => self.write_port_64(value_u8),
            SYSTEM_CONTROL_B => {
                tracing::trace!("Keyboard: Write system control B = {:#04x}", value_u8);
                self.system_control_b = value_u8;
            }
            _ => {
                tracing::warn!(
                    "Keyboard: Unknown write port {:#06x} value={:#04x}",
                    port,
                    value_u8
                );
            }
        }
    }

    /// Port 0x60 write — keyboard.cc:387-467
    fn write_port_60(&mut self, value: u8) {
        tracing::debug!("Keyboard: Write port 0x60 = {:#04x}", value);

        if self.kbd_controller.expecting_port60h != 0 {
            self.kbd_controller.expecting_port60h = 0;
            // data byte written to port 60h
            self.kbd_controller.c_d = false;

            match self.kbd_controller.last_comm {
                0x60 => {
                    // Write command byte (CCB) — keyboard.cc:397-421
                    let scan_convert = (value >> 6) & 0x01 != 0;
                    let disable_aux = (value >> 5) & 0x01 != 0;
                    let disable_keyboard = (value >> 4) & 0x01 != 0;
                    self.kbd_controller.sysf = (value >> 2) & 0x01 != 0;
                    self.kbd_controller.allow_irq1 = (value >> 0) & 0x01 != 0;
                    self.kbd_controller.allow_irq12 = (value >> 1) & 0x01 != 0;
                    self.set_kbd_clock_enable(!disable_keyboard);
                    self.set_aux_clock_enable(!disable_aux);
                    if self.kbd_controller.allow_irq12 && self.kbd_controller.auxb {
                        self.kbd_controller.irq12_requested = true;
                    } else if self.kbd_controller.allow_irq1 && self.kbd_controller.outb {
                        self.kbd_controller.irq1_requested = true;
                    }
                    self.kbd_controller.scancodes_translate = scan_convert;
                    tracing::debug!(
                        "Keyboard: CCB written: irq1={}, irq12={}, xlat={}, sysf={}, kbd_clk={}, aux_clk={}",
                        self.kbd_controller.allow_irq1,
                        self.kbd_controller.allow_irq12,
                        scan_convert,
                        self.kbd_controller.sysf,
                        self.kbd_controller.kbd_clock_enabled,
                        self.kbd_controller.aux_clock_enabled
                    );
                }
                0xCB => {
                    // Write keyboard controller mode
                    tracing::debug!(
                        "Keyboard: Write controller mode {:#04x}",
                        value
                    );
                }
                0xD1 => {
                    // Write output port — keyboard.cc:427-433
                    tracing::debug!(
                        "Keyboard: Write output port {:#04x}",
                        value
                    );
                    let new_a20 = (value & 0x02) != 0;
                    if self.a20_enabled != new_a20 {
                        self.a20_enabled = new_a20;
                        self.a20_change_pending = true;
                        tracing::debug!(
                            "Keyboard: A20 gate = {} via output port",
                            new_a20
                        );
                    }
                    if (value & 0x01) == 0 {
                        tracing::warn!(
                            "Keyboard: Processor reset requested via output port!"
                        );
                    }
                }
                0xD4 => {
                    // Write to mouse — keyboard.cc:435-439
                    self.kbd_ctrl_to_mouse(value);
                }
                0xD3 => {
                    // Write mouse output buffer — keyboard.cc:442-445
                    self.controller_enq(value, 1);
                }
                0xD2 => {
                    // Write keyboard output buffer — keyboard.cc:447-449
                    self.controller_enq(value, 0);
                }
                _ => {
                    tracing::warn!(
                        "Keyboard: Unsupported port 0x60 write (last_comm={:#04x}): {:#04x}",
                        self.kbd_controller.last_comm,
                        value
                    );
                }
            }
        } else {
            // Data byte to keyboard — keyboard.cc:456-466
            self.kbd_controller.c_d = false;
            self.kbd_controller.expecting_port60h = 0;
            if !self.kbd_controller.kbd_clock_enabled {
                self.set_kbd_clock_enable(true);
            }
            self.kbd_ctrl_to_kbd(value);
        }
    }

    /// Port 0x64 write — keyboard.cc:469-640
    fn write_port_64(&mut self, value: u8) {
        tracing::debug!("Keyboard: Write command 0x64 = {:#04x}", value);

        // Command byte written to port 64h
        self.kbd_controller.c_d = true;
        self.kbd_controller.last_comm = value;
        // Most commands do NOT expect port60h write next
        self.kbd_controller.expecting_port60h = 0;

        match value {
            0x20 => {
                // Get keyboard command byte (CCB) — keyboard.cc:477-493
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for cmd 0x20");
                    return;
                }
                let command_byte =
                    ((self.kbd_controller.scancodes_translate as u8) << 6)
                        | ((!self.kbd_controller.aux_clock_enabled as u8) << 5)
                        | ((!self.kbd_controller.kbd_clock_enabled as u8) << 4)
                        | (0 << 3)
                        | ((self.kbd_controller.sysf as u8) << 2)
                        | ((self.kbd_controller.allow_irq12 as u8) << 1)
                        | (self.kbd_controller.allow_irq1 as u8);
                self.controller_enq(command_byte, 0);
            }
            0x60 => {
                // Write command byte — next byte to port 60h
                self.kbd_controller.expecting_port60h = 1;
            }
            0xA0 | 0xA1 => {
                // BIOS name / version — not supported
                tracing::trace!("Keyboard: BIOS name/version cmd {:#04x} (unsupported)", value);
            }
            0xA7 => {
                // Disable aux device — keyboard.cc:508-510
                self.set_aux_clock_enable(false);
                tracing::debug!("Keyboard: Aux device disabled");
            }
            0xA8 => {
                // Enable aux device — keyboard.cc:512-514
                self.set_aux_clock_enable(true);
                tracing::debug!("Keyboard: Aux device enabled");
            }
            0xA9 => {
                // Test mouse port — keyboard.cc:516-523
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for cmd 0xA9");
                    return;
                }
                self.controller_enq(0x00, 0); // no errors
            }
            0xAA => {
                // Motherboard controller self test — keyboard.cc:524-539
                if !self.kbd_initialized {
                    self.controller_q_size = 0;
                    self.kbd_controller.outb = false;
                    self.kbd_initialized = true;
                }
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for cmd 0xAA");
                    return;
                }
                self.kbd_controller.sysf = true; // self test complete
                self.controller_enq(0x55, 0); // controller OK
                tracing::debug!("Keyboard: Self-test passed");
            }
            0xAB => {
                // Interface test — keyboard.cc:540-547
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for cmd 0xAB");
                    return;
                }
                self.controller_enq(0x00, 0);
            }
            0xAD => {
                // Disable keyboard — keyboard.cc:548-550
                self.set_kbd_clock_enable(false);
                tracing::debug!("Keyboard: Keyboard disabled");
            }
            0xAE => {
                // Enable keyboard — keyboard.cc:552-554
                self.set_kbd_clock_enable(true);
                tracing::debug!("Keyboard: Keyboard enabled");
            }
            0xAF => {
                // Get controller version — not supported
                tracing::trace!("Keyboard: Get controller version (unsupported)");
            }
            0xC0 => {
                // Read input port — keyboard.cc:559-567
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for cmd 0xC0");
                    return;
                }
                self.controller_enq(0x80, 0); // keyboard not inhibited
            }
            0xCA => {
                // Read keyboard controller mode
                self.controller_enq(0x01, 0); // PS/2 (MCA) interface
            }
            0xCB => {
                // Write keyboard controller mode
                self.kbd_controller.expecting_port60h = 1;
            }
            0xD0 => {
                // Read output port — keyboard.cc:576-588
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for cmd 0xD0");
                    return;
                }
                let output_port_val =
                    ((self.kbd_controller.irq12_requested as u8) << 5)
                        | ((self.kbd_controller.irq1_requested as u8) << 4)
                        | ((self.a20_enabled as u8) << 1)
                        | 0x01;
                self.controller_enq(output_port_val, 0);
            }
            0xD1 => {
                // Write output port — next byte to port 60h
                self.kbd_controller.expecting_port60h = 1;
            }
            0xD2 => {
                // Write keyboard output buffer — keyboard.cc:609-611
                self.kbd_controller.expecting_port60h = 1;
            }
            0xD3 => {
                // Write mouse output buffer — keyboard.cc:596-601
                self.kbd_controller.expecting_port60h = 1;
            }
            0xD4 => {
                // Write to mouse — keyboard.cc:603-607
                self.kbd_controller.expecting_port60h = 1;
            }
            0xDD => {
                // Disable A20 Address Line — keyboard.cc:613-614
                self.a20_enabled = false;
                self.a20_change_pending = true;
                tracing::debug!("Keyboard: A20 disabled via 0xDD");
            }
            0xDF => {
                // Enable A20 Address Line — keyboard.cc:616-617
                self.a20_enabled = true;
                self.a20_change_pending = true;
                tracing::debug!("Keyboard: A20 enabled via 0xDF");
            }
            0xFE => {
                // System reset — keyboard.cc:625-627
                tracing::warn!("Keyboard: System reset via 0xFE");
            }
            _ => {
                if value == 0xFF || (value >= 0xF0 && value <= 0xFD) {
                    // Useless pulse output bit commands
                    tracing::trace!(
                        "Keyboard: Pulse command {:#04x}",
                        value
                    );
                } else {
                    tracing::warn!(
                        "Keyboard: Unknown command {:#04x}",
                        value
                    );
                }
            }
        }
    }

    // =========================================================================
    // Controller queue and buffer management
    // =========================================================================

    /// Queue data from controller to output buffer (keyboard.cc:752-784)
    ///
    /// If output buffer is already full, pushes to controller_Q overflow queue.
    /// Otherwise, puts directly in kbd_output_buffer or aux_output_buffer.
    fn controller_enq(&mut self, data: u8, source: u8) {
        tracing::debug!(
            "Keyboard: controller_enQ({:#04x}) source={}",
            data,
            source
        );

        if self.kbd_controller.outb {
            // Output buffer full — queue for later
            if self.controller_q_size >= BX_KBD_CONTROLLER_QSIZE {
                tracing::error!("Keyboard: controller_Q full!");
                return;
            }
            self.controller_q[self.controller_q_size] = data;
            self.controller_q_size += 1;
            self.controller_q_source = source;
            return;
        }

        // Q is empty, put directly in output buffer
        if source == 0 {
            // Keyboard
            self.kbd_controller.kbd_output_buffer = data;
            self.kbd_controller.outb = true;
            self.kbd_controller.auxb = false;
            self.kbd_controller.inpb = false;
            if self.kbd_controller.allow_irq1 {
                self.kbd_controller.irq1_requested = true;
            }
        } else {
            // Mouse
            self.kbd_controller.aux_output_buffer = data;
            self.kbd_controller.outb = true;
            self.kbd_controller.auxb = true;
            self.kbd_controller.inpb = false;
            if self.kbd_controller.allow_irq12 {
                self.kbd_controller.irq12_requested = true;
            }
        }
    }

    /// Immediate enqueue to keyboard output buffer (keyboard.cc:786-797)
    ///
    /// Bypasses internal buffer — used for LED-write ACK (0xFA).
    fn kbd_enq_imm(&mut self, val: u8) {
        self.kbd_controller.kbd_output_buffer = val;
        self.kbd_controller.outb = true;
        if self.kbd_controller.allow_irq1 {
            self.kbd_controller.irq1_requested = true;
        }
    }

    /// Queue scancode in internal keyboard ring buffer (keyboard.cc:799-822)
    ///
    /// The byte is NOT immediately visible in the output buffer. It must be
    /// transferred by `periodic()` (called from the device tick path).
    fn kbd_enq(&mut self, scancode: u8) {
        if self.kbd_internal_buffer.num_elements >= BX_KBD_ELEMENTS {
            tracing::warn!(
                "Keyboard: Internal buffer full, ignoring {:#04x}",
                scancode
            );
            return;
        }

        let tail = (self.kbd_internal_buffer.head
            + self.kbd_internal_buffer.num_elements)
            % BX_KBD_ELEMENTS;
        self.kbd_internal_buffer.buffer[tail] = scancode;
        self.kbd_internal_buffer.num_elements += 1;

        if !self.kbd_controller.outb && self.kbd_controller.kbd_clock_enabled {
            self.activate_timer();
        }
    }

    /// Queue byte in internal mouse ring buffer (keyboard.cc:841-863)
    #[allow(dead_code)]
    fn mouse_enq(&mut self, data: u8) {
        if self.mouse_internal_buffer.num_elements >= BX_MOUSE_BUFF_SIZE {
            tracing::warn!(
                "Keyboard: Mouse buffer full, ignoring {:#04x}",
                data
            );
            return;
        }

        let tail = (self.mouse_internal_buffer.head
            + self.mouse_internal_buffer.num_elements)
            % BX_MOUSE_BUFF_SIZE;
        self.mouse_internal_buffer.buffer[tail] = data;
        self.mouse_internal_buffer.num_elements += 1;

        if !self.kbd_controller.outb && self.kbd_controller.aux_clock_enabled {
            self.activate_timer();
        }
    }

    /// Set timer_pending flag (keyboard.cc:1097-1102)
    fn activate_timer(&mut self) {
        if self.kbd_controller.timer_pending == 0 {
            self.kbd_controller.timer_pending = 1;
        }
    }

    // =========================================================================
    // Keyboard command handler (keyboard.cc:865-1022)
    // =========================================================================

    fn kbd_ctrl_to_kbd(&mut self, value: u8) {
        tracing::debug!("Keyboard: kbd_ctrl_to_kbd({:#04x})", value);

        if self.kbd_internal_buffer.expecting_typematic {
            self.kbd_internal_buffer.expecting_typematic = false;
            self.kbd_internal_buffer.delay = (value >> 5) & 0x03;
            self.kbd_internal_buffer.repeat_rate = value & 0x1f;
            self.kbd_enq(0xFA); // ACK
            return;
        }

        if self.kbd_internal_buffer.expecting_led_write {
            self.kbd_internal_buffer.led_status = value;
            self.kbd_internal_buffer.expecting_led_write = false;
            self.kbd_enq(0xFA); // ACK
            return;
        }

        if self.kbd_controller.expecting_scancodes_set {
            self.kbd_controller.expecting_scancodes_set = false;
            if value != 0 {
                if value < 4 {
                    self.kbd_controller.current_scancodes_set = value - 1;
                    self.kbd_enq(0xFA);
                } else {
                    self.kbd_enq(0xFF); // ERROR
                }
            } else {
                // Query current set: send ACK then set number
                self.kbd_enq(0xFA);
                self.kbd_enq(1 + self.kbd_controller.current_scancodes_set);
            }
            return;
        }

        match value {
            0x00 => {
                self.kbd_enq(0xFA); // ACK
            }
            0x05 => {
                // (mch) trying to get this to work...
                self.kbd_controller.sysf = true;
                self.kbd_enq_imm(0xFE);
            }
            0xED => {
                // LED Write
                self.kbd_internal_buffer.expecting_led_write = true;
                self.kbd_enq_imm(0xFA); // ACK (immediate)
            }
            0xEE => {
                // Echo
                self.kbd_enq(0xEE);
            }
            0xF0 => {
                // Select alternate scan code set
                self.kbd_controller.expecting_scancodes_set = true;
                self.kbd_enq(0xFA); // ACK
            }
            0xF2 => {
                // Identify keyboard — keyboard.cc:950-967
                if self.kbd_controller.kbd_type != BX_KBD_XT_TYPE {
                    self.kbd_enq(0xFA); // ACK
                    if self.kbd_controller.kbd_type == BX_KBD_MF_TYPE {
                        self.kbd_enq(0xAB);
                        if self.kbd_controller.scancodes_translate {
                            self.kbd_enq(0x41);
                        } else {
                            self.kbd_enq(0x83);
                        }
                    }
                }
            }
            0xF3 => {
                // Set typematic rate
                self.kbd_internal_buffer.expecting_typematic = true;
                self.kbd_enq(0xFA); // ACK
            }
            0xF4 => {
                // Enable scanning
                self.kbd_internal_buffer.scanning_enabled = true;
                self.kbd_enq(0xFA); // ACK
            }
            0xF5 => {
                // Reset keyboard and disable scanning
                self.resetinternals(true);
                self.kbd_enq(0xFA); // ACK
                self.kbd_internal_buffer.scanning_enabled = false;
            }
            0xF6 => {
                // Reset keyboard and enable scanning
                self.resetinternals(true);
                self.kbd_enq(0xFA); // ACK
                self.kbd_internal_buffer.scanning_enabled = true;
            }
            0xFE => {
                // Resend — not supported
                tracing::warn!("Keyboard: Resend command (0xFE) received");
            }
            0xFF => {
                // Reset keyboard + BAT — keyboard.cc:998-1004
                tracing::debug!("Keyboard: Reset command received");
                self.resetinternals(true);
                self.kbd_enq(0xFA); // ACK
                self.kbd_controller.bat_in_progress = true;
                self.kbd_enq(0xAA); // BAT passed
            }
            0xD3 => {
                self.kbd_enq(0xFA); // ACK
            }
            0xF7..=0xFD => {
                // PS/2 extensions — silently ignored with NACK
                self.kbd_enq(0xFE);
            }
            _ => {
                tracing::warn!(
                    "Keyboard: Unknown kbd command {:#04x}",
                    value
                );
                self.kbd_enq(0xFE); // NACK
            }
        }
    }

    // =========================================================================
    // Mouse command handler (keyboard.cc:1104-1339)
    // =========================================================================

    fn kbd_ctrl_to_mouse(&mut self, value: u8) {
        let is_ps2 = self.mouse.mouse_type == BX_MOUSE_TYPE_PS2
            || self.mouse.mouse_type == BX_MOUSE_TYPE_IMPS2;

        tracing::debug!("Keyboard: kbd_ctrl_to_mouse({:#04x})", value);

        if self.kbd_controller.expecting_mouse_parameter != 0 {
            self.kbd_controller.expecting_mouse_parameter = 0;
            match self.kbd_controller.last_mouse_command {
                0xF3 => {
                    // Set sample rate
                    self.mouse.sample_rate = value;
                    // Wheel mouse detection sequence
                    match (value, self.mouse.im_request) {
                        (200, 0) => self.mouse.im_request = 1,
                        (100, 1) => self.mouse.im_request = 2,
                        (80, 2) => {
                            if self.mouse.mouse_type == BX_MOUSE_TYPE_IMPS2 {
                                self.mouse.im_mode = true;
                            }
                            self.mouse.im_request = 0;
                        }
                        _ => self.mouse.im_request = 0,
                    }
                    self.controller_enq(0xFA, 1); // ACK
                }
                0xE8 => {
                    // Set resolution
                    self.mouse.resolution_cpmm = match value {
                        0 => 1,
                        1 => 2,
                        2 => 4,
                        3 => 8,
                        _ => 4,
                    };
                    self.controller_enq(0xFA, 1); // ACK
                }
                _ => {
                    tracing::warn!(
                        "Keyboard: Unknown mouse param for cmd {:#04x}",
                        self.kbd_controller.last_mouse_command
                    );
                }
            }
            return;
        }

        self.kbd_controller.expecting_mouse_parameter = 0;
        self.kbd_controller.last_mouse_command = value;

        // Wrap mode handling
        if self.mouse.mode == MOUSE_MODE_WRAP {
            if value != 0xFF && value != 0xEC {
                self.controller_enq(value, 1);
                return;
            }
        }

        match value {
            0xE6 => {
                // Scaling 1:1
                self.controller_enq(0xFA, 1);
                self.mouse.scaling = 1;
            }
            0xE7 => {
                // Scaling 2:1
                self.controller_enq(0xFA, 1);
                self.mouse.scaling = 2;
            }
            0xE8 => {
                // Set resolution (next byte)
                self.controller_enq(0xFA, 1);
                self.kbd_controller.expecting_mouse_parameter = 1;
            }
            0xE9 => {
                // Get mouse information
                self.controller_enq(0xFA, 1);
                let status = self.get_mouse_status_byte();
                self.controller_enq(status, 1);
                let resolution = self.get_mouse_resolution_byte();
                self.controller_enq(resolution, 1);
                self.controller_enq(self.mouse.sample_rate, 1);
            }
            0xEA => {
                // Set stream mode
                self.mouse.mode = MOUSE_MODE_STREAM;
                self.controller_enq(0xFA, 1);
            }
            0xEC => {
                // Reset wrap mode
                if self.mouse.mode == MOUSE_MODE_WRAP {
                    self.mouse.mode = self.mouse.saved_mode;
                    self.controller_enq(0xFA, 1);
                }
            }
            0xEE => {
                // Set wrap mode
                self.mouse.saved_mode = self.mouse.mode;
                self.mouse.mode = MOUSE_MODE_WRAP;
                self.controller_enq(0xFA, 1);
            }
            0xF0 => {
                // Set remote mode
                self.mouse.mode = MOUSE_MODE_REMOTE;
                self.controller_enq(0xFA, 1);
            }
            0xF2 => {
                // Read device type
                self.controller_enq(0xFA, 1);
                if self.mouse.im_mode {
                    self.controller_enq(0x03, 1); // Wheel mouse
                } else {
                    self.controller_enq(0x00, 1); // Standard
                }
            }
            0xF3 => {
                // Set sample rate (next byte)
                self.controller_enq(0xFA, 1);
                self.kbd_controller.expecting_mouse_parameter = 1;
            }
            0xF4 => {
                // Enable (stream mode)
                if is_ps2 {
                    self.mouse.enable = true;
                    self.controller_enq(0xFA, 1);
                } else {
                    self.controller_enq(0xFE, 1); // RESEND
                    self.kbd_controller.tim = true;
                }
            }
            0xF5 => {
                // Disable
                self.mouse.enable = false;
                self.controller_enq(0xFA, 1);
            }
            0xF6 => {
                // Set defaults
                self.mouse.sample_rate = 100;
                self.mouse.resolution_cpmm = 4;
                self.mouse.scaling = 1;
                self.mouse.enable = false;
                self.mouse.mode = MOUSE_MODE_STREAM;
                self.controller_enq(0xFA, 1);
            }
            0xFF => {
                // Reset mouse
                if is_ps2 {
                    self.mouse.sample_rate = 100;
                    self.mouse.resolution_cpmm = 4;
                    self.mouse.scaling = 1;
                    self.mouse.mode = MOUSE_MODE_RESET;
                    self.mouse.enable = false;
                    self.mouse.im_mode = false;
                    self.controller_enq(0xFA, 1); // ACK
                    self.controller_enq(0xAA, 1); // Completion
                    self.controller_enq(0x00, 1); // ID
                } else {
                    self.controller_enq(0xFE, 1); // RESEND
                    self.kbd_controller.tim = true;
                }
            }
            0xE1 => {
                // Read secondary ID
                self.controller_enq(0xFA, 1);
                self.controller_enq(0x00, 1);
            }
            0xEB => {
                // Read data (remote mode)
                self.controller_enq(0xFA, 1);
                // Send empty packet
                self.controller_enq(
                    0x08 | (self.mouse.button_status & 0x0F),
                    1,
                );
                self.controller_enq(0x00, 1);
                self.controller_enq(0x00, 1);
            }
            _ => {
                if is_ps2 {
                    tracing::warn!(
                        "Keyboard: Unknown mouse command {:#04x}",
                        value
                    );
                    self.controller_enq(0xFE, 1); // NACK
                }
            }
        }
    }

    fn get_mouse_status_byte(&self) -> u8 {
        let mut ret: u8 =
            if self.mouse.mode == MOUSE_MODE_REMOTE { 0x40 } else { 0 };
        ret |= (self.mouse.enable as u8) << 5;
        if self.mouse.scaling != 1 {
            ret |= 1 << 4;
        }
        ret |= (self.mouse.button_status & 0x01) << 2;
        ret |= self.mouse.button_status & 0x02;
        ret
    }

    fn get_mouse_resolution_byte(&self) -> u8 {
        match self.mouse.resolution_cpmm {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => 2,
        }
    }

    // =========================================================================
    // Clock enable (keyboard.cc:710-742)
    // =========================================================================

    fn set_kbd_clock_enable(&mut self, enable: bool) {
        if !enable {
            self.kbd_controller.kbd_clock_enabled = false;
        } else {
            let prev = self.kbd_controller.kbd_clock_enabled;
            self.kbd_controller.kbd_clock_enabled = true;
            if !prev && !self.kbd_controller.outb {
                self.activate_timer();
            }
        }
    }

    fn set_aux_clock_enable(&mut self, enable: bool) {
        if !enable {
            self.kbd_controller.aux_clock_enabled = false;
        } else {
            let prev = self.kbd_controller.aux_clock_enabled;
            self.kbd_controller.aux_clock_enabled = true;
            if !prev && !self.kbd_controller.outb {
                self.activate_timer();
            }
        }
    }

    // =========================================================================
    // Periodic timer (keyboard.cc:1037-1095)
    // =========================================================================

    /// Timer-driven transfer from internal buffers to output buffer.
    ///
    /// Returns IRQ bitmask: bit 0 = IRQ1 (keyboard), bit 1 = IRQ12 (mouse).
    /// Called from DeviceManager::tick().
    pub fn periodic(&mut self, usec_delta: u32) -> u8 {
        // Collect pending IRQ requests
        let mut retval: u8 = 0;
        if self.kbd_controller.irq1_requested {
            retval |= 0x01;
        }
        if self.kbd_controller.irq12_requested {
            retval |= 0x02;
        }
        self.kbd_controller.irq1_requested = false;
        self.kbd_controller.irq12_requested = false;

        if self.kbd_controller.timer_pending == 0 {
            return retval;
        }

        if usec_delta >= self.kbd_controller.timer_pending {
            self.kbd_controller.timer_pending = 0;
        } else {
            self.kbd_controller.timer_pending -= usec_delta;
            return retval;
        }

        // Timer expired — try to transfer a byte to output buffer
        if self.kbd_controller.outb {
            return retval;
        }

        // Transfer from keyboard internal buffer
        if self.kbd_internal_buffer.num_elements > 0
            && (self.kbd_controller.kbd_clock_enabled
                || self.kbd_controller.bat_in_progress)
        {
            self.kbd_controller.kbd_output_buffer =
                self.kbd_internal_buffer.buffer[self.kbd_internal_buffer.head];
            self.kbd_controller.outb = true;
            self.kbd_internal_buffer.head =
                (self.kbd_internal_buffer.head + 1) % BX_KBD_ELEMENTS;
            self.kbd_internal_buffer.num_elements -= 1;
            if self.kbd_controller.allow_irq1 {
                self.kbd_controller.irq1_requested = true;
                retval |= 0x01;
            }
        } else {
            // Try mouse internal buffer
            if self.kbd_controller.aux_clock_enabled
                && self.mouse_internal_buffer.num_elements > 0
            {
                self.kbd_controller.aux_output_buffer = self
                    .mouse_internal_buffer.buffer
                    [self.mouse_internal_buffer.head];
                self.kbd_controller.outb = true;
                self.kbd_controller.auxb = true;
                self.mouse_internal_buffer.head =
                    (self.mouse_internal_buffer.head + 1)
                        % BX_MOUSE_BUFF_SIZE;
                self.mouse_internal_buffer.num_elements -= 1;
                if self.kbd_controller.allow_irq12 {
                    self.kbd_controller.irq12_requested = true;
                    retval |= 0x02;
                }
            }
        }

        retval
    }

    // =========================================================================
    // External API
    // =========================================================================

    /// Send a scancode from external input (GUI)
    pub fn send_scancode(&mut self, scancode: u8) {
        if self.kbd_controller.kbd_clock_enabled
            && self.kbd_internal_buffer.scanning_enabled
        {
            self.kbd_enq(scancode);
        }
    }

    /// Get A20 gate state
    pub fn get_a20_enabled(&self) -> bool {
        self.a20_enabled
    }

    /// Check and clear IRQ1 pending (compatibility — prefer periodic() return value)
    pub fn check_irq1(&mut self) -> bool {
        let pending = self.kbd_controller.irq1_requested;
        self.kbd_controller.irq1_requested = false;
        pending
    }

    /// Check and clear IRQ12 pending (compatibility — prefer periodic() return value)
    pub fn check_irq12(&mut self) -> bool {
        let pending = self.kbd_controller.irq12_requested;
        self.kbd_controller.irq12_requested = false;
        pending
    }
}

/// Keyboard read handler for I/O port infrastructure
pub fn keyboard_read_handler(
    this_ptr: *mut c_void,
    port: u16,
    io_len: u8,
) -> u32 {
    let kbd = unsafe { &mut *(this_ptr as *mut BxKeyboardC) };
    kbd.read(port, io_len)
}

/// Keyboard write handler for I/O port infrastructure
pub fn keyboard_write_handler(
    this_ptr: *mut c_void,
    port: u16,
    value: u32,
    io_len: u8,
) {
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
        assert!(!kbd.kbd_controller.sysf); // Not set until self-test
        assert!(kbd.kbd_controller.keyl);
        assert!(kbd.kbd_controller.kbd_clock_enabled);
        assert!(!kbd.kbd_controller.aux_clock_enabled);
    }

    #[test]
    fn test_keyboard_self_test() {
        let mut kbd = BxKeyboardC::new();

        // Send self-test command (0xAA to port 0x64)
        kbd.write(KBD_COMMAND_PORT, 0xAA, 1);

        // Should have 0x55 in output buffer immediately (via controller_enQ)
        assert!(kbd.kbd_controller.outb);
        assert!(kbd.kbd_controller.sysf);

        let response = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(response, 0x55);
    }

    #[test]
    fn test_keyboard_interface_test() {
        let mut kbd = BxKeyboardC::new();

        // Self test first
        kbd.write(KBD_COMMAND_PORT, 0xAA, 1);
        let _ = kbd.read(KBD_DATA_PORT, 1);

        // Interface test
        kbd.write(KBD_COMMAND_PORT, 0xAB, 1);
        assert!(kbd.kbd_controller.outb);

        let response = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(response, 0x00); // Test passed
    }

    #[test]
    fn test_keyboard_reset_bat() {
        let mut kbd = BxKeyboardC::new();

        // Self-test first
        kbd.write(KBD_COMMAND_PORT, 0xAA, 1);
        let _ = kbd.read(KBD_DATA_PORT, 1);

        // Enable keyboard
        kbd.write(KBD_COMMAND_PORT, 0xAE, 1);

        // Send reset (0xFF to port 0x60)
        kbd.write(KBD_DATA_PORT, 0xFF, 1);

        // ACK and BAT are in internal buffer, need periodic() to transfer
        assert_eq!(kbd.kbd_internal_buffer.num_elements, 2);
        assert!(kbd.kbd_controller.bat_in_progress);

        // Transfer ACK
        let irq = kbd.periodic(10);
        assert!(kbd.kbd_controller.outb);
        let ack = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(ack, 0xFA);

        // Activate timer for next transfer
        // (read_port_60 calls activate_timer internally)
        let irq = kbd.periodic(10);
        assert!(kbd.kbd_controller.outb);
        let bat = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(bat, 0xAA);
        let _ = irq; // suppress unused warning
    }

    #[test]
    fn test_keyboard_disable_enable() {
        let mut kbd = BxKeyboardC::new();

        // Disable keyboard
        kbd.write(KBD_COMMAND_PORT, 0xAD, 1);
        assert!(!kbd.kbd_controller.kbd_clock_enabled);

        // Enable keyboard
        kbd.write(KBD_COMMAND_PORT, 0xAE, 1);
        assert!(kbd.kbd_controller.kbd_clock_enabled);
    }

    #[test]
    fn test_status_register_assembly() {
        let mut kbd = BxKeyboardC::new();

        // After init: keyl=1, c_d=1, everything else 0
        let status = kbd.read(KBD_STATUS_PORT, 1);
        // keyl(bit4)=1, c_d(bit3)=1 => 0x18
        assert_eq!(status, 0x18);

        // After self-test: sysf=1, outb=1 (0x55 in buffer)
        kbd.write(KBD_COMMAND_PORT, 0xAA, 1);
        let status = kbd.read(KBD_STATUS_PORT, 1);
        // keyl=1, c_d=1, sysf=1, outb=1 => 0x1D
        assert_eq!(status, 0x1D);
    }

    #[test]
    fn test_ccb_write_read() {
        let mut kbd = BxKeyboardC::new();

        // Self-test first
        kbd.write(KBD_COMMAND_PORT, 0xAA, 1);
        let _ = kbd.read(KBD_DATA_PORT, 1);

        // Write CCB: translate=1, disable_aux=1, sysf=1, irq1=1
        // = 0b01100101 = 0x65
        kbd.write(KBD_COMMAND_PORT, 0x60, 1);
        kbd.write(KBD_DATA_PORT, 0x65, 1);

        assert!(kbd.kbd_controller.scancodes_translate);
        assert!(!kbd.kbd_controller.aux_clock_enabled);
        assert!(kbd.kbd_controller.kbd_clock_enabled);
        assert!(kbd.kbd_controller.sysf);
        assert!(kbd.kbd_controller.allow_irq1);
        assert!(!kbd.kbd_controller.allow_irq12);

        // Read CCB back
        kbd.write(KBD_COMMAND_PORT, 0x20, 1);
        let ccb = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(ccb, 0x65);
    }
}
