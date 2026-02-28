//! 8042 Keyboard Controller (PS/2 Controller) Emulation
//!
//! Ported from Bochs `iodev/keyboard.cc` (1477 lines).
//!
//! # Architecture Overview
//!
//! The 8042 (or compatible) is the PS/2 controller chip that sits between the
//! CPU's I/O bus and two serial devices: the keyboard and the PS/2 mouse.
//! It has its own internal processor and firmware, but from the host CPU's
//! perspective it exposes just 4 I/O ports:
//!
//! ```text
//! Port 0x60 (Data Port):
//!   READ:  Returns data from the output buffer.
//!          - If AUXB=0: keyboard data (scancode or command response)
//!          - If AUXB=1: mouse data (movement packet or command response)
//!          Reading clears the OBF (Output Buffer Full) status bit.
//!   WRITE: Sends data to the keyboard device.
//!          Unless a controller command is pending (e.g., 0x60, 0xD4),
//!          the byte is forwarded to the keyboard via kbd_ctrl_to_kbd().
//!
//! Port 0x61 (System Control Port B):
//!   READ:  System status byte. Bit 4 = PIT channel 2 output (toggled on
//!          each read for delay_ms() compatibility). Bit 5 = Timer 2 status.
//!   WRITE: Controls speaker gate (bit 0) and timer 2 gate (bit 1).
//!
//! Port 0x64 (Status Port / Command Port):
//!   READ:  Returns the Status Register (assembled from boolean flags):
//!     Bit 0: OBF   — Output Buffer Full (data available at port 0x60)
//!     Bit 1: IBF   — Input Buffer Full (controller busy processing input)
//!     Bit 2: SYSF  — System Flag (POST passed = 1)
//!     Bit 3: A2    — Address line A2 (0=data was written, 1=command was written)
//!     Bit 4: INH   — Keyboard Inhibit (0=keyboard locked, 1=keyboard enabled)
//!     Bit 5: AUXB  — Auxiliary Output Buffer (1=mouse data in output buffer)
//!     Bit 6: TIM   — Timeout Error (keyboard or mouse response timeout)
//!     Bit 7: PARE  — Parity Error
//!   WRITE: Sends a controller command (see Controller Commands below).
//! ```
//!
//! # Output Buffer Semantics (from Bochs keyboard.cc comments)
//!
//! The output buffer flag (OBF/outb) and auxiliary buffer flag (AUXB/auxb) work
//! together to indicate what data is available:
//!
//! ```text
//! auxb=0, outb=0 : Both buffers empty (nothing to read)
//! auxb=0, outb=1 : Keyboard data in output buffer
//! auxb=1, outb=0 : Not used
//! auxb=1, outb=1 : Mouse data in output buffer
//! ```
//!
//! # Data Flow Architecture
//!
//! The controller uses a multi-level buffering scheme:
//!
//! ```text
//! Keyboard Device                          Host CPU
//!   |                                        ^
//!   v                                        |
//! kbd_internal_buffer (ring, 16 entries)    port 0x60 read
//!   |                                        ^
//!   +---> controller_enQ() ----+             |
//!                              |       kbd_output_buffer (1 byte)
//!                              v             ^
//!                        controller_Q        |
//!                     (overflow, 5 entries)   |
//!                              |             |
//!                              +---> periodic() transfers
//!
//! Mouse Device
//!   |
//!   v
//! mouse_internal_buffer (ring, 48 entries)
//!   |
//!   +---> controller_enQ(source=1) ---> aux_output_buffer (1 byte)
//! ```
//!
//! ## Data Transfer Timing
//!
//! Data does not immediately appear in the output buffer. Instead:
//! 1. Device data is enqueued into the internal ring buffer
//! 2. `activate_timer()` sets `timer_pending = 1`
//! 3. On the next `periodic()` call (driven by the system timer):
//!    - If the output buffer is empty (`outb == 0`):
//!      - Dequeues from keyboard buffer (priority) or mouse buffer
//!      - Sets `outb = 1` and requests the appropriate IRQ
//!    - If the output buffer is full: waits until the host reads it
//!
//! # Controller Commands (port 0x64 write)
//!
//! ```text
//! 0x20: Read Command/Configuration Byte (CCB)
//!       Returns CCB with interrupt enables, translation mode, clock controls
//! 0x60: Write CCB (next byte written to port 0x60 is the CCB value)
//!       Bit 0: kbd_clock_enabled (keyboard IRQ1 enable)
//!       Bit 1: aux_clock_enabled (mouse IRQ12 enable)
//!       Bit 4: kbd_clock_enabled (0=disabled, 1=enabled)
//!       Bit 5: aux_clock_enabled (0=disabled, 1=enabled)
//!       Bit 6: scancodes_translate (1=translate set 2 to set 1)
//! 0xA7: Disable auxiliary (mouse) interface
//! 0xA8: Enable auxiliary (mouse) interface
//! 0xA9: Test mouse port (returns 0x00 = OK)
//! 0xAA: Controller self-test (returns 0x55 = passed)
//! 0xAB: Keyboard interface test (returns 0x00 = OK)
//! 0xAD: Disable keyboard interface (clears kbd_clock_enabled)
//! 0xAE: Enable keyboard interface (sets kbd_clock_enabled)
//! 0xD0: Read output port (A20 line state, system reset)
//! 0xD1: Write output port (next byte to port 0x60: bit 1 = A20, bit 0 = reset)
//! 0xD2: Write keyboard output buffer (next byte appears as keyboard data)
//! 0xD3: Write mouse output buffer (next byte appears as mouse data)
//! 0xD4: Write to mouse (next byte to port 0x60 is forwarded to mouse device)
//! 0xDD: Disable A20 gate
//! 0xDF: Enable A20 gate
//! 0xFE: System reset (pulse reset line)
//! ```
//!
//! # Keyboard Commands (port 0x60 write, forwarded to keyboard device)
//!
//! ```text
//! 0xED: Set LEDs (next byte: bit 0=ScrollLock, bit 1=NumLock, bit 2=CapsLock)
//! 0xEE: Echo (returns 0xEE)
//! 0xF0: Select scancode set (next byte: 0=query current, 1-3=select set)
//! 0xF2: Identify keyboard (AT: ACK, MF-II: ACK+0xAB+0x41/0x83)
//! 0xF3: Set typematic rate/delay (next byte: bits 6-5=delay, bits 4-0=rate)
//! 0xF4: Enable scanning
//! 0xF5: Reset to defaults + disable scanning
//! 0xF6: Reset to defaults + enable scanning
//! 0xF7-0xFD: PS/2 key type commands (silently ignored)
//! 0xFE: Resend (panics in Bochs)
//! 0xFF: Reset keyboard + BAT (returns ACK + 0xAA)
//! ```
//!
//! # Mouse Commands (via controller command 0xD4)
//!
//! ```text
//! 0xE6: Set scaling 1:1
//! 0xE7: Set scaling 2:1
//! 0xE8: Set resolution (next byte: 0=1cpmm, 1=2, 2=4, 3=8 counts/mm)
//! 0xE9: Status request (returns: status byte, resolution, sample rate)
//! 0xEA: Set stream mode
//! 0xEB: Read data (returns a movement packet in remote mode)
//! 0xEC: Reset wrap mode
//! 0xEE: Set wrap mode (echoes all bytes except 0xFF and 0xEC)
//! 0xF0: Set remote mode (host must poll with 0xEB)
//! 0xF2: Read device type (0x00=standard, 0x03=wheel/IntelliMouse)
//! 0xF3: Set sample rate (next byte: rate in Hz)
//!        Magic sequence 200,100,80 enables IntelliMouse wheel mode
//! 0xF4: Enable reporting (stream mode)
//! 0xF5: Disable reporting (stream mode)
//! 0xF6: Set defaults (100 Hz, 4 cpmm, 1:1 scaling, stream mode, disabled)
//! 0xFF: Reset (returns ACK + 0xAA + device ID 0x00)
//! ```

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

// ---- 8042 Controller Commands (written to port 0x64) ----
// Reference: keyboard.cc:469-640
const CTRL_CMD_GET_CCB: u8 = 0x20;
const CTRL_CMD_WRITE_CCB: u8 = 0x60;
const CTRL_CMD_BIOS_NAME: u8 = 0xA0;
const CTRL_CMD_BIOS_VERSION: u8 = 0xA1;
const CTRL_CMD_DISABLE_AUX: u8 = 0xA7;
const CTRL_CMD_ENABLE_AUX: u8 = 0xA8;
const CTRL_CMD_TEST_MOUSE_PORT: u8 = 0xA9;
const CTRL_CMD_SELF_TEST: u8 = 0xAA;
const CTRL_CMD_INTERFACE_TEST: u8 = 0xAB;
const CTRL_CMD_DISABLE_KBD: u8 = 0xAD;
const CTRL_CMD_ENABLE_KBD: u8 = 0xAE;
const CTRL_CMD_GET_VERSION: u8 = 0xAF;
const CTRL_CMD_READ_INPUT_PORT: u8 = 0xC0;
const CTRL_CMD_READ_KBD_MODE: u8 = 0xCA;
const CTRL_CMD_WRITE_KBD_MODE: u8 = 0xCB;
const CTRL_CMD_READ_OUTPUT_PORT: u8 = 0xD0;
const CTRL_CMD_WRITE_OUTPUT_PORT: u8 = 0xD1;
const CTRL_CMD_WRITE_KBD_OUTBUF: u8 = 0xD2;
const CTRL_CMD_WRITE_MOUSE_OUTBUF: u8 = 0xD3;
const CTRL_CMD_WRITE_TO_MOUSE: u8 = 0xD4;
const CTRL_CMD_DISABLE_A20: u8 = 0xDD;
const CTRL_CMD_ENABLE_A20: u8 = 0xDF;
const CTRL_CMD_SYSTEM_RESET: u8 = 0xFE;

// ---- Keyboard Commands (written to port 0x60 directly) ----
// Reference: keyboard.cc:865-1022
const KBD_CMD_SET_LEDS: u8 = 0xED;
const KBD_CMD_ECHO: u8 = 0xEE;
const KBD_CMD_SELECT_SCAN_SET: u8 = 0xF0;
const KBD_CMD_IDENTIFY: u8 = 0xF2;
const KBD_CMD_SET_TYPEMATIC: u8 = 0xF3;
const KBD_CMD_ENABLE_SCANNING: u8 = 0xF4;
const KBD_CMD_RESET_DISABLE: u8 = 0xF5;
const KBD_CMD_RESET_ENABLE: u8 = 0xF6;
const KBD_CMD_RESEND: u8 = 0xFE;
const KBD_CMD_RESET: u8 = 0xFF;

// ---- Mouse Commands (via 0xD4 controller command) ----
// Reference: keyboard.cc:1104-1339
const MOUSE_CMD_READ_SECONDARY_ID: u8 = 0xE1;
const MOUSE_CMD_SET_SCALING_1_1: u8 = 0xE6;
const MOUSE_CMD_SET_SCALING_2_1: u8 = 0xE7;
const MOUSE_CMD_SET_RESOLUTION: u8 = 0xE8;
const MOUSE_CMD_GET_INFO: u8 = 0xE9;
const MOUSE_CMD_SET_STREAM_MODE: u8 = 0xEA;
const MOUSE_CMD_READ_DATA: u8 = 0xEB;
const MOUSE_CMD_RESET_WRAP_MODE: u8 = 0xEC;
const MOUSE_CMD_SET_WRAP_MODE: u8 = 0xEE;
const MOUSE_CMD_SET_REMOTE_MODE: u8 = 0xF0;
const MOUSE_CMD_READ_DEVICE_TYPE: u8 = 0xF2;
const MOUSE_CMD_SET_SAMPLE_RATE: u8 = 0xF3;
const MOUSE_CMD_ENABLE: u8 = 0xF4;
const MOUSE_CMD_DISABLE: u8 = 0xF5;
const MOUSE_CMD_SET_DEFAULTS: u8 = 0xF6;
const MOUSE_CMD_RESET: u8 = 0xFF;

// ---- Keyboard/Controller Response Bytes ----
const KBD_RESP_ACK: u8 = 0xFA;
const KBD_RESP_RESEND: u8 = 0xFE;
const KBD_RESP_ERROR: u8 = 0xFF;
const KBD_RESP_ECHO: u8 = 0xEE;
const KBD_RESP_SELF_TEST_OK: u8 = 0x55;
const KBD_RESP_BAT_OK: u8 = 0xAA;
const KBD_RESP_TEST_OK: u8 = 0x00;
const KBD_ID_MF2_BYTE1: u8 = 0xAB;
const KBD_ID_MF2_XLAT: u8 = 0x41;
const KBD_ID_MF2_NO_XLAT: u8 = 0x83;
const MOUSE_ID_STANDARD: u8 = 0x00;
const MOUSE_ID_WHEEL: u8 = 0x03;

// ---- Output Port Bits (port 0xD1 write) ----
const OUT_PORT_A20_GATE: u8 = 0x02;
const OUT_PORT_CPU_RESET: u8 = 0x01;

// ---- System Control Port B (port 0x61) ----
const SYSCTL_B_PIT_CH2_OUT: u8 = 0x10;

// ---- Periodic return bitmask ----
const KBD_IRQ_BIT_KBD: u8 = 0x01;
const KBD_IRQ_BIT_MOUSE: u8 = 0x02;

// ---- Typematic sub-command masks ----
const TYPEMATIC_DELAY_MASK: u8 = 0x03;
const TYPEMATIC_RATE_MASK: u8 = 0x1F;

/// 8042 Controller state (Bochs `bx_keyb_c::s.kbd_controller`).
///
/// The status register is assembled from individual boolean flags on each read
/// of port 0x64, rather than stored as a single byte. This matches the Bochs
/// implementation and makes it easy to set/clear individual status bits from
/// various controller operations.
///
/// ## Status Register Layout (port 0x64 read)
///
/// ```text
/// Bit 7 (PARE): Parity error on last byte received from keyboard
/// Bit 6 (TIM):  General timeout — cleared on each status register read
/// Bit 5 (AUXB): Auxiliary output buffer full (1=mouse data, 0=keyboard data)
///               Only meaningful when OBF (bit 0) is also set
/// Bit 4 (KEYL): Keyboard inhibit — reflects keyboard lock switch state
///               1=keyboard enabled (not locked), 0=keyboard locked
/// Bit 3 (C/D):  Command/Data flag — 1=last write was to port 0x64 (command),
///               0=last write was to port 0x60 (data)
/// Bit 2 (SYSF): System Flag — 0 after power-on reset, set to 1 after
///               successful controller self-test (command 0xAA)
/// Bit 1 (IBF):  Input Buffer Full — 1=controller is processing a command/data
///               byte, software should wait for this to clear before writing
/// Bit 0 (OBF):  Output Buffer Full — 1=data available in output buffer
///               (port 0x60), cleared when host reads port 0x60
/// ```
#[derive(Debug, Clone)]
pub struct KbdController {
    // Status register bits (assembled on port 0x64 read)
    /// Bit 7: Parity error on last byte received from keyboard/mouse
    pub(crate) pare: bool,
    /// Bit 6: General timeout error. Cleared on each status register read.
    /// Set when a device fails to respond within the expected time.
    pub(crate) tim: bool,
    /// Bit 5: Auxiliary output buffer full. When both auxb=1 and outb=1,
    /// the data in the output buffer came from the mouse (auxiliary port).
    pub(crate) auxb: bool,
    /// Bit 4: Keyboard inhibit (lock). true=keyboard enabled (not locked).
    /// Reflects the physical keyboard lock switch; initialized to true.
    pub(crate) keyl: bool,
    /// Bit 3: Command/Data. Set to true when port 0x64 is written (command),
    /// cleared when port 0x60 is written (data).
    pub(crate) c_d: bool,
    /// Bit 2: System flag. Set after successful self-test (command 0xAA returns 0x55).
    /// BIOS checks this to determine if POST has completed.
    pub(crate) sysf: bool,
    /// Bit 1: Input buffer full. Set when software writes to port 0x60 or 0x64,
    /// cleared when the controller has processed the byte.
    pub(crate) inpb: bool,
    /// Bit 0: Output buffer full. Set when data is available at port 0x60,
    /// cleared when the host reads port 0x60.
    pub(crate) outb: bool,

    // Internal controller state (not directly visible via status register)
    /// Keyboard clock enable. When disabled, the keyboard's serial clock line
    /// is held low, preventing the keyboard from sending scancodes.
    /// Controlled by CCB bit 4 and commands 0xAD (disable) / 0xAE (enable).
    pub(crate) kbd_clock_enabled: bool,
    /// Auxiliary (mouse) clock enable. Similar to kbd_clock_enabled but for
    /// the mouse port. Controlled by CCB bit 5 and commands 0xA7/0xA8.
    pub(crate) aux_clock_enabled: bool,
    /// Allow keyboard IRQ1 — set from CCB bit 0. When true and OBF transitions
    /// to full with keyboard data, IRQ1 is requested.
    pub(crate) allow_irq1: bool,
    /// Allow mouse IRQ12 — set from CCB bit 1. When true and OBF transitions
    /// to full with mouse data, IRQ12 is requested.
    pub(crate) allow_irq12: bool,
    /// Keyboard output buffer — holds one byte of keyboard data ready for
    /// the host to read via port 0x60. Loaded from kbd_internal_buffer by periodic().
    pub(crate) kbd_output_buffer: u8,
    /// Auxiliary (mouse) output buffer — holds one byte of mouse data ready
    /// for the host to read via port 0x60. Loaded from mouse_internal_buffer.
    pub(crate) aux_output_buffer: u8,
    /// Last controller command received (port 0x64 write value).
    /// Used to determine how to handle the next data byte written to port 0x60.
    pub(crate) last_comm: u8,
    /// Tracks which controller command expects a follow-up byte on port 0x60.
    /// Non-zero means the next port 0x60 write is a parameter for this command
    /// (e.g., 0x60=write CCB, 0xD1=write output port, 0xD4=write to mouse).
    pub(crate) expecting_port60h: u8,
    /// Mouse is expecting a parameter byte (e.g., after 0xF3 Set Sample Rate)
    pub(crate) expecting_mouse_parameter: u8,
    /// Last mouse command (used to interpret the parameter byte)
    pub(crate) last_mouse_command: u8,
    /// Timer countdown. When non-zero, `periodic()` decrements it. When it
    /// reaches zero, periodic() transfers data from internal buffers to the
    /// output buffer. Simulates the serial transfer delay from device to controller.
    pub(crate) timer_pending: u32,
    /// IRQ1 (keyboard) request pending — checked and cleared by periodic()
    pub(crate) irq1_requested: bool,
    /// IRQ12 (mouse) request pending — checked and cleared by periodic()
    pub(crate) irq12_requested: bool,
    /// Scancode translation mode (CCB bit 6). When true, scan code set 2
    /// bytes are translated to scan code set 1 (legacy AT compatibility).
    pub(crate) scancodes_translate: bool,
    /// Keyboard is expecting a scancode set parameter (after 0xF0 command)
    pub(crate) expecting_scancodes_set: bool,
    /// Current scancode set (0=set 1, 1=set 2, 2=set 3). Default is set 1.
    pub(crate) current_scancodes_set: u8,
    /// BAT (Basic Assurance Test) is in progress. Set on keyboard reset (0xFF),
    /// cleared after BAT result (0xAA) is sent. During BAT, keyboard data is
    /// accepted even if kbd_clock_enabled is false.
    pub(crate) bat_in_progress: bool,
    /// Keyboard type (BX_KBD_XT_TYPE=0 or BX_KBD_MF_TYPE=2).
    /// Determines the response to the Identify command (0xF2):
    /// XT sends nothing, MF-II sends ACK + 0xAB + 0x41/0x83.
    pub(crate) kbd_type: u8,
}

/// Keyboard internal ring buffer (Bochs `bx_keyb_c::s.kbd_internal_buffer`).
///
/// This is a circular queue that holds scancodes from the keyboard device and
/// command responses (ACK=0xFA, BAT=0xAA, etc.) waiting to be transferred to
/// the controller's output buffer. The `periodic()` function dequeues from this
/// buffer into `kbd_output_buffer` when the output buffer is empty.
///
/// The buffer also holds keyboard device state for multi-byte command sequences:
/// - **Typematic**: After 0xF3 command, the next byte sets delay (bits 6-5:
///   250/500/750/1000 ms) and repeat rate (bits 4-0: 30 Hz down to 2 Hz)
/// - **LED write**: After 0xED command, the next byte sets LED state
///   (bit 0=ScrollLock, bit 1=NumLock, bit 2=CapsLock)
/// - **Scancode set**: After 0xF0, the next byte selects set 1/2/3 or queries
#[derive(Debug, Clone)]
pub struct KbdInternalBuffer {
    pub(crate) buffer: [u8; BX_KBD_ELEMENTS],
    pub(crate) head: usize,
    pub(crate) num_elements: usize,
    /// Keyboard expects typematic rate/delay parameter (after 0xF3)
    pub(crate) expecting_typematic: bool,
    /// Keyboard expects LED state byte (after 0xED)
    pub(crate) expecting_led_write: bool,
    /// Typematic delay (0=250ms, 1=500ms, 2=750ms, 3=1000ms)
    pub(crate) delay: u8,
    /// Typematic repeat rate (0-31, maps to 30.0-2.0 characters per second)
    pub(crate) repeat_rate: u8,
    /// Current LED state (bit 0=ScrollLock, bit 1=NumLock, bit 2=CapsLock)
    pub(crate) led_status: u8,
    /// Scanning enabled — when false, keyboard does not send scancodes.
    /// Set by 0xF4 (enable), cleared by 0xF5 (disable) or reset.
    pub(crate) scanning_enabled: bool,
}

/// Mouse internal ring buffer (Bochs `bx_keyb_c::s.mouse_internal_buffer`).
///
/// Larger than the keyboard buffer (48 vs 16 entries) because mouse packets
/// are 3-4 bytes each (standard PS/2 = 3 bytes, IntelliMouse = 4 bytes with
/// scroll wheel), and movement can generate many packets.
#[derive(Debug, Clone)]
pub struct MouseInternalBuffer {
    pub(crate) buffer: [u8; BX_MOUSE_BUFF_SIZE],
    pub(crate) head: usize,
    pub(crate) num_elements: usize,
}

/// Mouse device state (Bochs `bx_keyb_c::s.mouse`).
///
/// ## PS/2 Mouse Packet Format (standard, 3 bytes)
///
/// ```text
/// Byte 1: YO XO YS XS 1 MB RB LB
///   YO/XO: Y/X overflow (movement exceeded 9-bit range)
///   YS/XS: Y/X sign bits (1=negative)
///   Bit 3:  Always 1
///   MB/RB/LB: Middle, Right, Left button states
///
/// Byte 2: X movement (8 bits, sign in byte 1 bit 4)
/// Byte 3: Y movement (8 bits, sign in byte 1 bit 5)
/// ```
///
/// ## IntelliMouse Extension (4 bytes, im_mode=true)
///
/// Byte 4 contains the scroll wheel Z movement (-8 to +7).
/// IntelliMouse mode is enabled by a magic sample rate sequence:
/// set rate 200, set rate 100, set rate 80. If the mouse type is
/// IMPS2, `im_mode` is set to true and Read Device Type returns 0x03.
#[derive(Debug, Clone)]
pub struct MouseState {
    /// Mouse type (BX_MOUSE_TYPE_PS2=2, BX_MOUSE_TYPE_IMPS2=3)
    pub(crate) mouse_type: u8,
    /// Sample rate in Hz (default 100). Set by 0xF3 command.
    pub(crate) sample_rate: u8,
    /// Resolution in counts per millimeter (1, 2, 4, or 8). Set by 0xE8.
    pub(crate) resolution_cpmm: u8,
    /// Scaling factor (1=1:1, 2=2:1). Set by 0xE6/0xE7 commands.
    pub(crate) scaling: u8,
    /// Current operating mode (RESET=10, STREAM=11, REMOTE=12, WRAP=13)
    pub(crate) mode: u8,
    /// Saved mode before entering wrap mode (restored by 0xEC Reset Wrap Mode)
    pub(crate) saved_mode: u8,
    /// Reporting enabled (stream mode only). Set by 0xF4, cleared by 0xF5.
    pub(crate) enable: bool,
    /// Current button state (bit 0=left, bit 1=right, bit 2=middle)
    pub(crate) button_status: u8,
    /// Accumulated X movement not yet reported in a packet
    pub(crate) delayed_dx: i16,
    /// Accumulated Y movement not yet reported in a packet
    pub(crate) delayed_dy: i16,
    /// Accumulated Z (scroll wheel) movement
    pub(crate) delayed_dz: i16,
    /// IntelliMouse detection state machine counter.
    /// Tracks progress through the magic sequence: 200 -> 100 -> 80 Hz.
    /// 0=idle, 1=saw 200, 2=saw 100, triggers on 80.
    pub(crate) im_request: u8,
    /// IntelliMouse wheel mode active. When true, packets are 4 bytes
    /// (includes scroll wheel) and device ID reports 0x03 instead of 0x00.
    pub(crate) im_mode: bool,
}

/// 8042 Keyboard Controller — full Bochs-compatible implementation.
///
/// This is the top-level PS/2 controller structure containing all sub-components:
/// the controller state machine, keyboard and mouse device state, internal buffers,
/// and the controller overflow queue.
///
/// ## Controller Queue (controller_q)
///
/// When `controller_enQ()` is called but the output buffer is already full
/// (outb=1), the byte is stored in the controller overflow queue (max 5 entries).
/// When the host reads port 0x60 and empties the output buffer, `periodic()`
/// will drain the overflow queue before pulling from the internal device buffers.
///
/// This queue is necessary because some command responses require multiple bytes
/// (e.g., keyboard Identify returns ACK + 0xAB + 0x41/0x83, or mouse reset
/// returns ACK + 0xAA + 0x00) and they must be delivered in order even if the
/// host hasn't read the previous byte yet.
#[derive(Debug)]
pub struct BxKeyboardC {
    /// Controller state machine and status register bits
    pub(crate) kbd_controller: KbdController,
    /// Keyboard device internal ring buffer (scancodes and command responses)
    pub(crate) kbd_internal_buffer: KbdInternalBuffer,
    /// Mouse device internal ring buffer (packets and command responses)
    pub(crate) mouse_internal_buffer: MouseInternalBuffer,
    /// Mouse device state (mode, resolution, buttons, accumulated movement)
    pub(crate) mouse: MouseState,
    /// Controller overflow queue — holds bytes that could not be placed in the
    /// output buffer because it was already full. Max 5 entries.
    pub(crate) controller_q: [u8; BX_KBD_CONTROLLER_QSIZE],
    /// Number of bytes currently in the controller overflow queue
    pub(crate) controller_q_size: usize,
    /// Source of bytes in controller_q (0=keyboard, 1=mouse).
    /// All entries in the queue must be from the same source.
    pub(crate) controller_q_source: u8,
    /// System Control Port B (port 0x61). Bit 0=speaker gate, bit 1=timer 2 gate,
    /// bit 4=PIT channel 2 output (toggled on each read for BIOS delay_ms()).
    pub(crate) system_control_b: u8,
    /// A20 gate state. Controlled via output port (command 0xD1) bit 1 or
    /// dedicated commands 0xDD (disable) / 0xDF (enable).
    pub(crate) a20_enabled: bool,
    /// Flag set when A20 state changes, cleared by emulator after propagation
    /// to the memory subsystem.
    pub(crate) a20_change_pending: bool,
    /// First self-test flag (Bochs static `kbd_initialized`).
    /// Prevents double-initialization.
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
                saved_mode: 0, // Bochs: zeroed by memset, not explicitly set in init()
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
    // Port read handler (Bochs `bx_keyb_c::read`, keyboard.cc:271-410)
    // =========================================================================

    /// Read from keyboard controller I/O port.
    ///
    /// - **Port 0x60**: Returns data from the output buffer (keyboard or mouse byte
    ///   depending on AUXB flag). Clears OBF. If the controller overflow queue has
    ///   pending bytes, drains one into the output buffer immediately.
    /// - **Port 0x61**: System Control Port B. Bit 4 is toggled on each read to
    ///   simulate PIT channel 2 output transitions for BIOS `delay_ms()` loops.
    /// - **Port 0x64**: Status register (assembled from boolean flags each read).
    ///   TIM bit is cleared on each read.
    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        match port {
            KBD_DATA_PORT => self.read_port_60(),
            KBD_STATUS_PORT => self.read_port_64(),
            SYSTEM_CONTROL_B => {
                // Toggle bit 4 (PIT channel 2 output) on each read.
                // The BIOS delay_ms() polls this bit waiting for transitions.
                self.system_control_b ^= SYSCTL_B_PIT_CH2_OUT;
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
            tracing::debug!("Keyboard: Read port 0x60 [mouse] = {:#04x}", val);
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
    // Port write handler (Bochs `bx_keyb_c::write`, keyboard.cc:411-665)
    // =========================================================================

    /// Write to keyboard controller I/O port.
    ///
    /// - **Port 0x60 (Data)**: Context-dependent behavior:
    ///   - If `expecting_port60h` is set (from a prior controller command):
    ///     the byte is interpreted as a parameter for that command (CCB write,
    ///     output port write, write-to-mouse, etc.)
    ///   - Otherwise: the byte is forwarded to the keyboard device via
    ///     `kbd_ctrl_to_kbd()` for processing as a keyboard command
    ///
    /// - **Port 0x61 (System Control B)**: Updates speaker gate and timer 2 gate.
    ///   Bit 0 = speaker data enable, Bit 1 = timer 2 gate
    ///
    /// - **Port 0x64 (Command)**: Dispatches controller commands. Sets `c_d=1`
    ///   to indicate a command was written. Some commands set `expecting_port60h`
    ///   to capture the next port 0x60 write as a parameter.
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
                CTRL_CMD_WRITE_CCB => {
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
                CTRL_CMD_WRITE_KBD_MODE => {
                    // Write keyboard controller mode
                    tracing::debug!("Keyboard: Write controller mode {:#04x}", value);
                }
                CTRL_CMD_WRITE_OUTPUT_PORT => {
                    // Write output port — keyboard.cc:427-433
                    tracing::debug!("Keyboard: Write output port {:#04x}", value);
                    let new_a20 = (value & OUT_PORT_A20_GATE) != 0;
                    if self.a20_enabled != new_a20 {
                        self.a20_enabled = new_a20;
                        self.a20_change_pending = true;
                        tracing::debug!("Keyboard: A20 gate = {} via output port", new_a20);
                    }
                    if (value & OUT_PORT_CPU_RESET) == 0 {
                        tracing::warn!("Keyboard: Processor reset requested via output port!");
                    }
                }
                CTRL_CMD_WRITE_TO_MOUSE => {
                    // Write to mouse — keyboard.cc:435-439
                    self.kbd_ctrl_to_mouse(value);
                }
                CTRL_CMD_WRITE_MOUSE_OUTBUF => {
                    // Write mouse output buffer — keyboard.cc:442-445
                    self.controller_enq(value, 1);
                }
                CTRL_CMD_WRITE_KBD_OUTBUF => {
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
            CTRL_CMD_GET_CCB => {
                // Get keyboard command byte (CCB) — keyboard.cc:477-493
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for GET_CCB cmd");
                    return;
                }
                let command_byte = ((self.kbd_controller.scancodes_translate as u8) << 6)
                    | ((!self.kbd_controller.aux_clock_enabled as u8) << 5)
                    | ((!self.kbd_controller.kbd_clock_enabled as u8) << 4)
                    | (0 << 3)
                    | ((self.kbd_controller.sysf as u8) << 2)
                    | ((self.kbd_controller.allow_irq12 as u8) << 1)
                    | (self.kbd_controller.allow_irq1 as u8);
                self.controller_enq(command_byte, 0);
            }
            CTRL_CMD_WRITE_CCB => {
                // Write command byte — next byte to port 60h
                self.kbd_controller.expecting_port60h = 1;
            }
            CTRL_CMD_BIOS_NAME | CTRL_CMD_BIOS_VERSION => {
                // BIOS name / version — not supported
                tracing::trace!(
                    "Keyboard: BIOS name/version cmd {:#04x} (unsupported)",
                    value
                );
            }
            CTRL_CMD_DISABLE_AUX => {
                // Disable aux device — keyboard.cc:508-510
                self.set_aux_clock_enable(false);
                tracing::debug!("Keyboard: Aux device disabled");
            }
            CTRL_CMD_ENABLE_AUX => {
                // Enable aux device — keyboard.cc:512-514
                self.set_aux_clock_enable(true);
                tracing::debug!("Keyboard: Aux device enabled");
            }
            CTRL_CMD_TEST_MOUSE_PORT => {
                // Test mouse port — keyboard.cc:516-523
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for TEST_MOUSE_PORT cmd");
                    return;
                }
                self.controller_enq(KBD_RESP_TEST_OK, 0);
            }
            CTRL_CMD_SELF_TEST => {
                // Motherboard controller self test — keyboard.cc:524-539
                if !self.kbd_initialized {
                    self.controller_q_size = 0;
                    self.kbd_controller.outb = false;
                    self.kbd_initialized = true;
                }
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for SELF_TEST cmd");
                    return;
                }
                self.kbd_controller.sysf = true; // self test complete
                self.controller_enq(KBD_RESP_SELF_TEST_OK, 0);
                tracing::debug!("Keyboard: Self-test passed");
            }
            CTRL_CMD_INTERFACE_TEST => {
                // Interface test — keyboard.cc:540-547
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for INTERFACE_TEST cmd");
                    return;
                }
                self.controller_enq(KBD_RESP_TEST_OK, 0);
            }
            CTRL_CMD_DISABLE_KBD => {
                // Disable keyboard — keyboard.cc:548-550
                self.set_kbd_clock_enable(false);
                tracing::debug!("Keyboard: Keyboard disabled");
            }
            CTRL_CMD_ENABLE_KBD => {
                // Enable keyboard — keyboard.cc:552-554
                self.set_kbd_clock_enable(true);
                tracing::debug!("Keyboard: Keyboard enabled");
            }
            CTRL_CMD_GET_VERSION => {
                // Get controller version — not supported
                tracing::trace!("Keyboard: Get controller version (unsupported)");
            }
            CTRL_CMD_READ_INPUT_PORT => {
                // Read input port — keyboard.cc:559-567
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for READ_INPUT_PORT cmd");
                    return;
                }
                self.controller_enq(0x80, 0); // keyboard not inhibited
            }
            CTRL_CMD_READ_KBD_MODE => {
                // Read keyboard controller mode
                self.controller_enq(0x01, 0); // PS/2 (MCA) interface
            }
            CTRL_CMD_WRITE_KBD_MODE => {
                // Write keyboard controller mode
                self.kbd_controller.expecting_port60h = 1;
            }
            CTRL_CMD_READ_OUTPUT_PORT => {
                // Read output port — keyboard.cc:576-588
                if self.kbd_controller.outb {
                    tracing::warn!("Keyboard: OUTB set for READ_OUTPUT_PORT cmd");
                    return;
                }
                let output_port_val = ((self.kbd_controller.irq12_requested as u8) << 5)
                    | ((self.kbd_controller.irq1_requested as u8) << 4)
                    | ((self.a20_enabled as u8) << 1)
                    | OUT_PORT_CPU_RESET;
                self.controller_enq(output_port_val, 0);
            }
            CTRL_CMD_WRITE_OUTPUT_PORT => {
                // Write output port — next byte to port 60h
                self.kbd_controller.expecting_port60h = 1;
            }
            CTRL_CMD_WRITE_KBD_OUTBUF => {
                // Write keyboard output buffer — keyboard.cc:609-611
                self.kbd_controller.expecting_port60h = 1;
            }
            CTRL_CMD_WRITE_MOUSE_OUTBUF => {
                // Write mouse output buffer — keyboard.cc:596-601
                self.kbd_controller.expecting_port60h = 1;
            }
            CTRL_CMD_WRITE_TO_MOUSE => {
                // Write to mouse — keyboard.cc:603-607
                self.kbd_controller.expecting_port60h = 1;
            }
            CTRL_CMD_DISABLE_A20 => {
                // Disable A20 Address Line — keyboard.cc:613-614
                self.a20_enabled = false;
                self.a20_change_pending = true;
                tracing::debug!("Keyboard: A20 disabled via 0xDD");
            }
            CTRL_CMD_ENABLE_A20 => {
                // Enable A20 Address Line — keyboard.cc:616-617
                self.a20_enabled = true;
                self.a20_change_pending = true;
                tracing::debug!("Keyboard: A20 enabled via 0xDF");
            }
            CTRL_CMD_SYSTEM_RESET => {
                // System reset — keyboard.cc:625-627
                tracing::warn!("Keyboard: System reset via 0xFE");
            }
            _ => {
                if value == 0xFF || (value >= 0xF0 && value <= 0xFD) {
                    // Useless pulse output bit commands
                    tracing::trace!("Keyboard: Pulse command {:#04x}", value);
                } else {
                    tracing::warn!("Keyboard: Unknown command {:#04x}", value);
                }
            }
        }
    }

    // =========================================================================
    // Controller queue and buffer management
    // =========================================================================

    /// Queue data from controller to output buffer (Bochs `controller_enQ`, keyboard.cc:752-784).
    ///
    /// `source`: 0 = keyboard, 1 = mouse.
    ///
    /// If the output buffer is already full (`outb == true`), the data byte is
    /// pushed into the controller overflow queue (`controller_q`, max 5 entries).
    /// If the overflow queue is also full, this panics (matching Bochs behavior).
    ///
    /// If the output buffer is empty, the data goes directly into either
    /// `kbd_output_buffer` (source=0) or `aux_output_buffer` (source=1), and
    /// `outb` is set to true. The appropriate IRQ flag is set if enabled:
    /// - source=0, allow_irq1=true: sets irq1_requested
    /// - source=1, allow_irq12=true: sets irq12_requested
    ///
    /// The `auxb` flag is set to match `source` so the host knows which buffer
    /// to read on the next port 0x60 read. `inpb` is cleared since the
    /// controller is no longer busy processing input.
    fn controller_enq(&mut self, data: u8, source: u8) {
        tracing::debug!("Keyboard: controller_enQ({:#04x}) source={}", data, source);

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
            tracing::warn!("Keyboard: Internal buffer full, ignoring {:#04x}", scancode);
            return;
        }

        let tail = (self.kbd_internal_buffer.head + self.kbd_internal_buffer.num_elements)
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
            tracing::warn!("Keyboard: Mouse buffer full, ignoring {:#04x}", data);
            return;
        }

        let tail = (self.mouse_internal_buffer.head + self.mouse_internal_buffer.num_elements)
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
    // Keyboard command handler (Bochs `kbd_ctrl_to_kbd`, keyboard.cc:865-1022)
    // =========================================================================

    /// Process a command byte sent to the keyboard device.
    ///
    /// Called when the host writes to port 0x60 and no controller command is
    /// pending (`expecting_port60h == 0`). The byte is interpreted as a keyboard
    /// device command.
    ///
    /// ## Multi-byte Command Protocol
    ///
    /// Some commands expect a parameter byte. The keyboard sets a flag
    /// (e.g., `expecting_typematic`, `expecting_led_write`, `expecting_scancodes_set`)
    /// and returns ACK (0xFA). The next call to this function receives the
    /// parameter byte, processes it, and returns another ACK.
    ///
    /// ## Command Summary
    ///
    /// | Cmd  | Name                     | Response                        |
    /// |------|--------------------------|---------------------------------|
    /// | 0xED | Set LEDs                 | ACK, then wait for LED byte     |
    /// | 0xEE | Echo                     | 0xEE                            |
    /// | 0xF0 | Select scancode set      | ACK, then wait for set number   |
    /// | 0xF2 | Identify keyboard        | ACK (+0xAB+0x41 for MF-II)      |
    /// | 0xF3 | Set typematic rate       | ACK, then wait for rate byte    |
    /// | 0xF4 | Enable scanning          | ACK                             |
    /// | 0xF5 | Reset + disable scanning | ACK                             |
    /// | 0xF6 | Reset + enable scanning  | ACK                             |
    /// | 0xFF | Reset + BAT              | ACK + 0xAA (BAT passed)         |
    fn kbd_ctrl_to_kbd(&mut self, value: u8) {
        tracing::debug!("Keyboard: kbd_ctrl_to_kbd({:#04x})", value);

        if self.kbd_internal_buffer.expecting_typematic {
            self.kbd_internal_buffer.expecting_typematic = false;
            self.kbd_internal_buffer.delay = (value >> 5) & TYPEMATIC_DELAY_MASK;
            self.kbd_internal_buffer.repeat_rate = value & TYPEMATIC_RATE_MASK;
            self.kbd_enq(KBD_RESP_ACK);
            return;
        }

        if self.kbd_internal_buffer.expecting_led_write {
            self.kbd_internal_buffer.led_status = value;
            self.kbd_internal_buffer.expecting_led_write = false;
            self.kbd_enq(KBD_RESP_ACK);
            return;
        }

        if self.kbd_controller.expecting_scancodes_set {
            self.kbd_controller.expecting_scancodes_set = false;
            if value != 0 {
                if value < 4 {
                    self.kbd_controller.current_scancodes_set = value - 1;
                    self.kbd_enq(KBD_RESP_ACK);
                } else {
                    self.kbd_enq(KBD_RESP_ERROR);
                }
            } else {
                // Query current set: send ACK then set number
                self.kbd_enq(KBD_RESP_ACK);
                self.kbd_enq(1 + self.kbd_controller.current_scancodes_set);
            }
            return;
        }

        match value {
            0x00 => {
                self.kbd_enq(KBD_RESP_ACK);
            }
            0x05 => {
                // (mch) trying to get this to work...
                self.kbd_controller.sysf = true;
                self.kbd_enq_imm(KBD_RESP_RESEND);
            }
            KBD_CMD_SET_LEDS => {
                // LED Write
                self.kbd_internal_buffer.expecting_led_write = true;
                self.kbd_enq_imm(KBD_RESP_ACK);
            }
            KBD_CMD_ECHO => {
                // Echo
                self.kbd_enq(KBD_RESP_ECHO);
            }
            KBD_CMD_SELECT_SCAN_SET => {
                // Select alternate scan code set
                self.kbd_controller.expecting_scancodes_set = true;
                self.kbd_enq(KBD_RESP_ACK);
            }
            KBD_CMD_IDENTIFY => {
                // Identify keyboard — keyboard.cc:950-967
                if self.kbd_controller.kbd_type != BX_KBD_XT_TYPE {
                    self.kbd_enq(KBD_RESP_ACK);
                    if self.kbd_controller.kbd_type == BX_KBD_MF_TYPE {
                        self.kbd_enq(KBD_ID_MF2_BYTE1);
                        if self.kbd_controller.scancodes_translate {
                            self.kbd_enq(KBD_ID_MF2_XLAT);
                        } else {
                            self.kbd_enq(KBD_ID_MF2_NO_XLAT);
                        }
                    }
                }
            }
            KBD_CMD_SET_TYPEMATIC => {
                // Set typematic rate
                self.kbd_internal_buffer.expecting_typematic = true;
                self.kbd_enq(KBD_RESP_ACK);
            }
            KBD_CMD_ENABLE_SCANNING => {
                // Enable scanning
                self.kbd_internal_buffer.scanning_enabled = true;
                self.kbd_enq(KBD_RESP_ACK);
            }
            KBD_CMD_RESET_DISABLE => {
                // Reset keyboard and disable scanning
                self.resetinternals(true);
                self.kbd_enq(KBD_RESP_ACK);
                self.kbd_internal_buffer.scanning_enabled = false;
            }
            KBD_CMD_RESET_ENABLE => {
                // Reset keyboard and enable scanning
                self.resetinternals(true);
                self.kbd_enq(KBD_RESP_ACK);
                self.kbd_internal_buffer.scanning_enabled = true;
            }
            KBD_CMD_RESEND => {
                // Resend — not supported
                tracing::warn!("Keyboard: Resend command (0xFE) received");
            }
            KBD_CMD_RESET => {
                // Reset keyboard + BAT — keyboard.cc:998-1004
                tracing::debug!("Keyboard: Reset command received");
                self.resetinternals(true);
                self.kbd_enq(KBD_RESP_ACK);
                self.kbd_controller.bat_in_progress = true;
                self.kbd_enq(KBD_RESP_BAT_OK);
            }
            0xD3 => {
                self.kbd_enq(KBD_RESP_ACK);
            }
            0xF7..=0xFD => {
                // PS/2 extensions — silently ignored with NACK
                self.kbd_enq(KBD_RESP_RESEND);
            }
            _ => {
                tracing::warn!("Keyboard: Unknown kbd command {:#04x}", value);
                self.kbd_enq(KBD_RESP_RESEND);
            }
        }
    }

    // =========================================================================
    // Mouse command handler (Bochs `kbd_ctrl_to_mouse`, keyboard.cc:1104-1339)
    // =========================================================================

    /// Process a command byte sent to the PS/2 mouse device.
    ///
    /// Called when the host has previously written 0xD4 to port 0x64 (Write to Mouse),
    /// and then writes the mouse command byte to port 0x60. The response bytes are
    /// enqueued via `controller_enq(data, 1)` (source=1 for mouse).
    ///
    /// ## ACK Protocol
    ///
    /// An ACK (0xFA) is always the first response to any valid command, except
    /// for Set-Wrap-Mode (0xEE) and Resend (0xFE) which have special handling.
    ///
    /// ## Wrap Mode (0xEE)
    ///
    /// In wrap mode, the mouse echoes all received bytes back to the host
    /// unchanged, except for 0xFF (Reset) and 0xEC (Reset Wrap Mode) which
    /// are processed normally. This is used for diagnostics.
    ///
    /// ## IntelliMouse Detection Sequence
    ///
    /// The host enables wheel mouse (IntelliMouse) mode by sending a specific
    /// sequence of Set Sample Rate (0xF3) commands:
    /// 1. Set rate to 200 Hz
    /// 2. Set rate to 100 Hz
    /// 3. Set rate to 80 Hz
    ///
    /// If the mouse type supports it (BX_MOUSE_TYPE_IMPS2), `im_mode` is set
    /// to true, packets become 4 bytes (with scroll wheel), and Read Device
    /// Type (0xF2) returns 0x03 instead of 0x00.
    fn kbd_ctrl_to_mouse(&mut self, value: u8) {
        let is_ps2 = self.mouse.mouse_type == BX_MOUSE_TYPE_PS2
            || self.mouse.mouse_type == BX_MOUSE_TYPE_IMPS2;

        tracing::debug!("Keyboard: kbd_ctrl_to_mouse({:#04x})", value);

        if self.kbd_controller.expecting_mouse_parameter != 0 {
            self.kbd_controller.expecting_mouse_parameter = 0;
            match self.kbd_controller.last_mouse_command {
                MOUSE_CMD_SET_SAMPLE_RATE => {
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
                    self.controller_enq(KBD_RESP_ACK, 1);
                }
                MOUSE_CMD_SET_RESOLUTION => {
                    // Set resolution
                    self.mouse.resolution_cpmm = match value {
                        0 => 1,
                        1 => 2,
                        2 => 4,
                        3 => 8,
                        _ => 4,
                    };
                    self.controller_enq(KBD_RESP_ACK, 1);
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
            if value != MOUSE_CMD_RESET && value != MOUSE_CMD_RESET_WRAP_MODE {
                self.controller_enq(value, 1);
                return;
            }
        }

        match value {
            MOUSE_CMD_SET_SCALING_1_1 => {
                // Scaling 1:1
                self.controller_enq(KBD_RESP_ACK, 1);
                self.mouse.scaling = 1;
            }
            MOUSE_CMD_SET_SCALING_2_1 => {
                // Scaling 2:1
                self.controller_enq(KBD_RESP_ACK, 1);
                self.mouse.scaling = 2;
            }
            MOUSE_CMD_SET_RESOLUTION => {
                // Set resolution (next byte)
                self.controller_enq(KBD_RESP_ACK, 1);
                self.kbd_controller.expecting_mouse_parameter = 1;
            }
            MOUSE_CMD_GET_INFO => {
                // Get mouse information
                self.controller_enq(KBD_RESP_ACK, 1);
                let status = self.get_mouse_status_byte();
                self.controller_enq(status, 1);
                let resolution = self.get_mouse_resolution_byte();
                self.controller_enq(resolution, 1);
                self.controller_enq(self.mouse.sample_rate, 1);
            }
            MOUSE_CMD_SET_STREAM_MODE => {
                // Set stream mode
                self.mouse.mode = MOUSE_MODE_STREAM;
                self.controller_enq(KBD_RESP_ACK, 1);
            }
            MOUSE_CMD_RESET_WRAP_MODE => {
                // Reset wrap mode
                if self.mouse.mode == MOUSE_MODE_WRAP {
                    self.mouse.mode = self.mouse.saved_mode;
                    self.controller_enq(KBD_RESP_ACK, 1);
                }
            }
            MOUSE_CMD_SET_WRAP_MODE => {
                // Set wrap mode
                self.mouse.saved_mode = self.mouse.mode;
                self.mouse.mode = MOUSE_MODE_WRAP;
                self.controller_enq(KBD_RESP_ACK, 1);
            }
            MOUSE_CMD_SET_REMOTE_MODE => {
                // Set remote mode
                self.mouse.mode = MOUSE_MODE_REMOTE;
                self.controller_enq(KBD_RESP_ACK, 1);
            }
            MOUSE_CMD_READ_DEVICE_TYPE => {
                // Read device type
                self.controller_enq(KBD_RESP_ACK, 1);
                if self.mouse.im_mode {
                    self.controller_enq(MOUSE_ID_WHEEL, 1);
                } else {
                    self.controller_enq(MOUSE_ID_STANDARD, 1);
                }
            }
            MOUSE_CMD_SET_SAMPLE_RATE => {
                // Set sample rate (next byte)
                self.controller_enq(KBD_RESP_ACK, 1);
                self.kbd_controller.expecting_mouse_parameter = 1;
            }
            MOUSE_CMD_ENABLE => {
                // Enable (stream mode)
                if is_ps2 {
                    self.mouse.enable = true;
                    self.controller_enq(KBD_RESP_ACK, 1);
                } else {
                    self.controller_enq(KBD_RESP_RESEND, 1);
                    self.kbd_controller.tim = true;
                }
            }
            MOUSE_CMD_DISABLE => {
                // Disable
                self.mouse.enable = false;
                self.controller_enq(KBD_RESP_ACK, 1);
            }
            MOUSE_CMD_SET_DEFAULTS => {
                // Set defaults
                self.mouse.sample_rate = 100;
                self.mouse.resolution_cpmm = 4;
                self.mouse.scaling = 1;
                self.mouse.enable = false;
                self.mouse.mode = MOUSE_MODE_STREAM;
                self.controller_enq(KBD_RESP_ACK, 1);
            }
            MOUSE_CMD_RESET => {
                // Reset mouse
                if is_ps2 {
                    self.mouse.sample_rate = 100;
                    self.mouse.resolution_cpmm = 4;
                    self.mouse.scaling = 1;
                    self.mouse.mode = MOUSE_MODE_RESET;
                    self.mouse.enable = false;
                    self.mouse.im_mode = false;
                    self.controller_enq(KBD_RESP_ACK, 1);
                    self.controller_enq(KBD_RESP_BAT_OK, 1); // Completion
                    self.controller_enq(MOUSE_ID_STANDARD, 1); // ID
                } else {
                    self.controller_enq(KBD_RESP_RESEND, 1);
                    self.kbd_controller.tim = true;
                }
            }
            MOUSE_CMD_READ_SECONDARY_ID => {
                // Read secondary ID
                self.controller_enq(KBD_RESP_ACK, 1);
                self.controller_enq(MOUSE_ID_STANDARD, 1);
            }
            MOUSE_CMD_READ_DATA => {
                // Read data (remote mode)
                self.controller_enq(KBD_RESP_ACK, 1);
                // Send empty packet
                self.controller_enq(0x08 | (self.mouse.button_status & 0x0F), 1);
                self.controller_enq(0x00, 1);
                self.controller_enq(0x00, 1);
            }
            _ => {
                if is_ps2 {
                    tracing::warn!("Keyboard: Unknown mouse command {:#04x}", value);
                    self.controller_enq(KBD_RESP_RESEND, 1);
                }
            }
        }
    }

    fn get_mouse_status_byte(&self) -> u8 {
        let mut ret: u8 = if self.mouse.mode == MOUSE_MODE_REMOTE {
            0x40
        } else {
            0
        };
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
    // Clock enable (Bochs keyboard.cc:710-742)
    //
    // The keyboard and mouse have serial clock lines controlled by the 8042.
    // When the clock is disabled (held low), the device cannot send data.
    // When the clock transitions from disabled to enabled and the output
    // buffer is empty, activate_timer() is called to start transferring
    // any queued data from the device's internal buffer.
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
    // Periodic timer (Bochs `bx_keyb_c::periodic`, keyboard.cc:1037-1095)
    // =========================================================================

    /// Timer-driven transfer from internal device buffers to the output buffer.
    ///
    /// This is the core data pump of the PS/2 controller. Called from
    /// `DeviceManager::tick()` on each timer tick with the elapsed microseconds.
    ///
    /// Returns an IRQ bitmask: bit 0 = raise IRQ1 (keyboard), bit 1 = raise IRQ12 (mouse).
    ///
    /// ## Algorithm (matching Bochs keyboard.cc:1037-1095)
    ///
    /// 1. Collect any pending IRQ requests and clear them
    /// 2. If `timer_pending == 0`, return immediately (nothing to transfer)
    /// 3. Decrement `timer_pending` by `usec_delta`. If still non-zero, return
    /// 4. Timer has expired. If output buffer is full (`outb == true`), return
    ///    (host hasn't read the previous byte yet)
    /// 5. Transfer priority:
    ///    a. **Keyboard buffer** (if `kbd_clock_enabled` or `bat_in_progress`):
    ///       Dequeue from `kbd_internal_buffer.head`, set `outb=true`, request IRQ1
    ///    b. **Mouse buffer** (if `aux_clock_enabled`):
    ///       Dequeue from `mouse_internal_buffer.head`, set `outb=true`, `auxb=true`,
    ///       request IRQ12
    ///    c. Neither: log "no keys waiting"
    pub fn periodic(&mut self, usec_delta: u32) -> u8 {
        // Collect pending IRQ requests
        let mut retval: u8 = 0;
        if self.kbd_controller.irq1_requested {
            retval |= KBD_IRQ_BIT_KBD;
        }
        if self.kbd_controller.irq12_requested {
            retval |= KBD_IRQ_BIT_MOUSE;
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
            && (self.kbd_controller.kbd_clock_enabled || self.kbd_controller.bat_in_progress)
        {
            self.kbd_controller.kbd_output_buffer =
                self.kbd_internal_buffer.buffer[self.kbd_internal_buffer.head];
            self.kbd_controller.outb = true;
            self.kbd_internal_buffer.head = (self.kbd_internal_buffer.head + 1) % BX_KBD_ELEMENTS;
            self.kbd_internal_buffer.num_elements -= 1;
            if self.kbd_controller.allow_irq1 {
                self.kbd_controller.irq1_requested = true;
                retval |= KBD_IRQ_BIT_KBD;
            }
        } else {
            // Try mouse internal buffer
            if self.kbd_controller.aux_clock_enabled && self.mouse_internal_buffer.num_elements > 0
            {
                self.kbd_controller.aux_output_buffer =
                    self.mouse_internal_buffer.buffer[self.mouse_internal_buffer.head];
                self.kbd_controller.outb = true;
                self.kbd_controller.auxb = true;
                self.mouse_internal_buffer.head =
                    (self.mouse_internal_buffer.head + 1) % BX_MOUSE_BUFF_SIZE;
                self.mouse_internal_buffer.num_elements -= 1;
                if self.kbd_controller.allow_irq12 {
                    self.kbd_controller.irq12_requested = true;
                    retval |= KBD_IRQ_BIT_MOUSE;
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
        if self.kbd_controller.kbd_clock_enabled && self.kbd_internal_buffer.scanning_enabled {
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
        assert!(!kbd.kbd_controller.sysf); // Not set until self-test
        assert!(kbd.kbd_controller.keyl);
        assert!(kbd.kbd_controller.kbd_clock_enabled);
        assert!(!kbd.kbd_controller.aux_clock_enabled);
    }

    #[test]
    fn test_keyboard_self_test() {
        let mut kbd = BxKeyboardC::new();

        // Send self-test command to port 0x64
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_SELF_TEST as u32, 1);

        // Should have self-test OK in output buffer immediately (via controller_enQ)
        assert!(kbd.kbd_controller.outb);
        assert!(kbd.kbd_controller.sysf);

        let response = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(response, KBD_RESP_SELF_TEST_OK as u32);
    }

    #[test]
    fn test_keyboard_interface_test() {
        let mut kbd = BxKeyboardC::new();

        // Self test first
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_SELF_TEST as u32, 1);
        let _ = kbd.read(KBD_DATA_PORT, 1);

        // Interface test
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_INTERFACE_TEST as u32, 1);
        assert!(kbd.kbd_controller.outb);

        let response = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(response, KBD_RESP_TEST_OK as u32);
    }

    #[test]
    fn test_keyboard_reset_bat() {
        let mut kbd = BxKeyboardC::new();

        // Self-test first
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_SELF_TEST as u32, 1);
        let _ = kbd.read(KBD_DATA_PORT, 1);

        // Enable keyboard
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_ENABLE_KBD as u32, 1);

        // Send reset (0xFF to port 0x60)
        kbd.write(KBD_DATA_PORT, KBD_CMD_RESET as u32, 1);

        // ACK and BAT are in internal buffer, need periodic() to transfer
        assert_eq!(kbd.kbd_internal_buffer.num_elements, 2);
        assert!(kbd.kbd_controller.bat_in_progress);

        // Transfer ACK
        let irq = kbd.periodic(10);
        assert!(kbd.kbd_controller.outb);
        let ack = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(ack, KBD_RESP_ACK as u32);

        // Activate timer for next transfer
        // (read_port_60 calls activate_timer internally)
        let irq = kbd.periodic(10);
        assert!(kbd.kbd_controller.outb);
        let bat = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(bat, KBD_RESP_BAT_OK as u32);
        let _ = irq; // suppress unused warning
    }

    #[test]
    fn test_keyboard_disable_enable() {
        let mut kbd = BxKeyboardC::new();

        // Disable keyboard
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_DISABLE_KBD as u32, 1);
        assert!(!kbd.kbd_controller.kbd_clock_enabled);

        // Enable keyboard
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_ENABLE_KBD as u32, 1);
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
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_SELF_TEST as u32, 1);
        let status = kbd.read(KBD_STATUS_PORT, 1);
        // keyl=1, c_d=1, sysf=1, outb=1 => 0x1D
        assert_eq!(status, 0x1D);
    }

    #[test]
    fn test_ccb_write_read() {
        let mut kbd = BxKeyboardC::new();

        // Self-test first
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_SELF_TEST as u32, 1);
        let _ = kbd.read(KBD_DATA_PORT, 1);

        // Write CCB: translate=1, disable_aux=1, sysf=1, irq1=1
        // = 0b01100101 = 0x65
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_WRITE_CCB as u32, 1);
        kbd.write(KBD_DATA_PORT, 0x65, 1);

        assert!(kbd.kbd_controller.scancodes_translate);
        assert!(!kbd.kbd_controller.aux_clock_enabled);
        assert!(kbd.kbd_controller.kbd_clock_enabled);
        assert!(kbd.kbd_controller.sysf);
        assert!(kbd.kbd_controller.allow_irq1);
        assert!(!kbd.kbd_controller.allow_irq12);

        // Read CCB back
        kbd.write(KBD_COMMAND_PORT, CTRL_CMD_GET_CCB as u32, 1);
        let ccb = kbd.read(KBD_DATA_PORT, 1);
        assert_eq!(ccb, 0x65);
    }
}
