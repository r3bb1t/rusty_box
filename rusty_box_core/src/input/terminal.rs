//! A keyboard attached to a serial terminal: key events in, the bytes an
//! xterm-compatible terminal sends for them out.
//!
//! A machine whose console is a UART has no keyboard device; what a person
//! types reaches the guest as the byte stream their terminal emulator
//! produces, which is what QEMU's `-nographic` console and every serial
//! terminal do. This models that terminal's keyboard half: it tracks the
//! modifier and lock keys, sends nothing on a release, and renders a press
//! with xterm's conventions (xterm `ctlseqs`, "PC-Style Function Keys"):
//! printable keys as their US-layout character, Ctrl as the C0 control code,
//! Alt as an `ESC` prefix, cursor keys as `CSI A`..`CSI D`, and the editing
//! and function keys as `CSI n ~` / `SS3 P`..`SS3 S`, with xterm's
//! `;modifier` parameter when Shift, Alt or Ctrl is held.

use super::{Key, KeyAction};

const ESC: u8 = 0x1B;

/// The bytes one key event sends: at most eight (`ESC [ 1 5 ; 8 ~` and an
/// Alt prefix fit), and none for a release or a key with no terminal byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalBytes {
    bytes: [u8; 8],
    len: usize,
}

impl TerminalBytes {
    const NONE: Self = Self { bytes: [0; 8], len: 0 };

    fn push(&mut self, byte: u8) {
        if let Some(slot) = self.bytes.get_mut(self.len) {
            *slot = byte;
            self.len += 1;
        }
    }

    fn push_number(&mut self, n: u8) {
        if n >= 10 {
            self.push(b'0' + n / 10);
        }
        self.push(b'0' + n % 10);
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The modifiers held and the locks engaged on a terminal's keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerminalKeyboard {
    shift_left: bool,
    shift_right: bool,
    ctrl_left: bool,
    ctrl_right: bool,
    alt_left: bool,
    alt_right: bool,
    caps_lock: bool,
    /// Num Lock starts engaged, as a PC keyboard's does after POST.
    num_lock_off: bool,
}

/// A modifier key: held state, no byte of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modifier {
    ShiftLeft,
    ShiftRight,
    CtrlLeft,
    CtrlRight,
    AltLeft,
    AltRight,
}

/// A lock key: a press toggles it, and it sends no byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lock {
    Caps,
    Num,
}

/// A key that sends an escape sequence: `CSI <final>` or `SS3 <final>` for
/// the cursor and F1-F4 keys, `CSI <number> ~` for the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sequence {
    Cursor(u8),
    Ss3(u8),
    Tilde(u8),
}

/// What a key does on a terminal's keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Modifier(Modifier),
    Lock(Lock),
    /// A printable key: its US-layout character without and with Shift.
    Character { plain: u8, shifted: u8 },
    /// A keypad key: `digit` under Num Lock, the editing key's `sequence`
    /// without it.
    Keypad { digit: u8, sequence: Sequence },
    /// Enter, Backspace and Esc: one control byte.
    Control(u8),
    /// Tab: `HT`, or xterm's back-tab `CSI Z` with Shift.
    Tab,
    Sequence(Sequence),
    /// Print, Scroll Lock, Pause, the Windows and menu keys, SysRq, Break and
    /// the multimedia keys: a terminal sends nothing for them.
    Silent,
}

const fn character(plain: u8, shifted: u8) -> Role {
    Role::Character { plain, shifted }
}

const fn keypad(digit: u8, sequence: Sequence) -> Role {
    Role::Keypad { digit, sequence }
}

/// Every key's role. The match names every key, so a key added to [`Key`]
/// does not compile until it has one here (R5).
const fn role(key: Key) -> Role {
    match key {
        Key::ShiftL => Role::Modifier(Modifier::ShiftLeft),
        Key::ShiftR => Role::Modifier(Modifier::ShiftRight),
        Key::CtrlL => Role::Modifier(Modifier::CtrlLeft),
        Key::CtrlR => Role::Modifier(Modifier::CtrlRight),
        Key::AltL => Role::Modifier(Modifier::AltLeft),
        Key::AltR => Role::Modifier(Modifier::AltRight),
        Key::CapsLock => Role::Lock(Lock::Caps),
        Key::NumLock => Role::Lock(Lock::Num),
        Key::A => character(b'a', b'A'),
        Key::B => character(b'b', b'B'),
        Key::C => character(b'c', b'C'),
        Key::D => character(b'd', b'D'),
        Key::E => character(b'e', b'E'),
        Key::F => character(b'f', b'F'),
        Key::G => character(b'g', b'G'),
        Key::H => character(b'h', b'H'),
        Key::I => character(b'i', b'I'),
        Key::J => character(b'j', b'J'),
        Key::K => character(b'k', b'K'),
        Key::L => character(b'l', b'L'),
        Key::M => character(b'm', b'M'),
        Key::N => character(b'n', b'N'),
        Key::O => character(b'o', b'O'),
        Key::P => character(b'p', b'P'),
        Key::Q => character(b'q', b'Q'),
        Key::R => character(b'r', b'R'),
        Key::S => character(b's', b'S'),
        Key::T => character(b't', b'T'),
        Key::U => character(b'u', b'U'),
        Key::V => character(b'v', b'V'),
        Key::W => character(b'w', b'W'),
        Key::X => character(b'x', b'X'),
        Key::Y => character(b'y', b'Y'),
        Key::Z => character(b'z', b'Z'),
        Key::K1 => character(b'1', b'!'),
        Key::K2 => character(b'2', b'@'),
        Key::K3 => character(b'3', b'#'),
        Key::K4 => character(b'4', b'$'),
        Key::K5 => character(b'5', b'%'),
        Key::K6 => character(b'6', b'^'),
        Key::K7 => character(b'7', b'&'),
        Key::K8 => character(b'8', b'*'),
        Key::K9 => character(b'9', b'('),
        Key::K0 => character(b'0', b')'),
        Key::Space => character(b' ', b' '),
        Key::Minus => character(b'-', b'_'),
        Key::Equals => character(b'=', b'+'),
        Key::LeftBracket => character(b'[', b'{'),
        Key::RightBracket => character(b']', b'}'),
        Key::Backslash | Key::LeftBackslash => character(b'\\', b'|'),
        Key::Semicolon => character(b';', b':'),
        Key::SingleQuote => character(b'\'', b'"'),
        Key::Grave => character(b'`', b'~'),
        Key::Comma => character(b',', b'<'),
        Key::Period => character(b'.', b'>'),
        Key::Slash => character(b'/', b'?'),
        Key::KpAdd => character(b'+', b'+'),
        Key::KpSubtract => character(b'-', b'-'),
        Key::KpMultiply => character(b'*', b'*'),
        Key::KpDivide => character(b'/', b'/'),
        Key::KpInsert => keypad(b'0', Sequence::Tilde(2)),
        Key::KpEnd => keypad(b'1', Sequence::Cursor(b'F')),
        Key::KpDown => keypad(b'2', Sequence::Cursor(b'B')),
        Key::KpPageDown => keypad(b'3', Sequence::Tilde(6)),
        Key::KpLeft => keypad(b'4', Sequence::Cursor(b'D')),
        Key::Kp5 => keypad(b'5', Sequence::Cursor(b'E')),
        Key::KpRight => keypad(b'6', Sequence::Cursor(b'C')),
        Key::KpHome => keypad(b'7', Sequence::Cursor(b'H')),
        Key::KpUp => keypad(b'8', Sequence::Cursor(b'A')),
        Key::KpPageUp => keypad(b'9', Sequence::Tilde(5)),
        Key::KpDelete => keypad(b'.', Sequence::Tilde(3)),
        Key::Enter | Key::KpEnter => Role::Control(b'\r'),
        Key::Backspace => Role::Control(0x7F),
        Key::Esc => Role::Control(ESC),
        Key::Tab => Role::Tab,
        Key::Up => Role::Sequence(Sequence::Cursor(b'A')),
        Key::Down => Role::Sequence(Sequence::Cursor(b'B')),
        Key::Right => Role::Sequence(Sequence::Cursor(b'C')),
        Key::Left => Role::Sequence(Sequence::Cursor(b'D')),
        Key::Home => Role::Sequence(Sequence::Cursor(b'H')),
        Key::End => Role::Sequence(Sequence::Cursor(b'F')),
        Key::F1 => Role::Sequence(Sequence::Ss3(b'P')),
        Key::F2 => Role::Sequence(Sequence::Ss3(b'Q')),
        Key::F3 => Role::Sequence(Sequence::Ss3(b'R')),
        Key::F4 => Role::Sequence(Sequence::Ss3(b'S')),
        Key::Insert => Role::Sequence(Sequence::Tilde(2)),
        Key::Delete => Role::Sequence(Sequence::Tilde(3)),
        Key::PageUp => Role::Sequence(Sequence::Tilde(5)),
        Key::PageDown => Role::Sequence(Sequence::Tilde(6)),
        Key::F5 => Role::Sequence(Sequence::Tilde(15)),
        Key::F6 => Role::Sequence(Sequence::Tilde(17)),
        Key::F7 => Role::Sequence(Sequence::Tilde(18)),
        Key::F8 => Role::Sequence(Sequence::Tilde(19)),
        Key::F9 => Role::Sequence(Sequence::Tilde(20)),
        Key::F10 => Role::Sequence(Sequence::Tilde(21)),
        Key::F11 => Role::Sequence(Sequence::Tilde(23)),
        Key::F12 => Role::Sequence(Sequence::Tilde(24)),
        Key::Print
        | Key::ScrlLock
        | Key::Pause
        | Key::WinL
        | Key::WinR
        | Key::Menu
        | Key::AltSysreq
        | Key::CtrlBreak
        | Key::IntBack
        | Key::IntForward
        | Key::IntStop
        | Key::IntMail
        | Key::IntSearch
        | Key::IntFav
        | Key::IntHome
        | Key::PowerMycomp
        | Key::PowerCalc
        | Key::PowerSleep
        | Key::PowerPower
        | Key::PowerWake => Role::Silent,
    }
}

/// The control code Ctrl turns `byte` into (xterm's `modifyOtherKeys`
/// off): letters and `@[\]^_` to C0, `?` to DEL, Space and `2` to NUL.
fn with_ctrl(byte: u8) -> u8 {
    match byte {
        b'?' => 0x7F,
        b' ' | b'2' => 0x00,
        b'3'..=b'7' => byte - b'3' + 0x1B,
        b'8' => 0x7F,
        b'/' => 0x1F,
        0x40..=0x7F => byte & 0x1F,
        _ => byte,
    }
}

impl TerminalKeyboard {
    pub const fn new() -> Self {
        Self {
            shift_left: false,
            shift_right: false,
            ctrl_left: false,
            ctrl_right: false,
            alt_left: false,
            alt_right: false,
            caps_lock: false,
            num_lock_off: false,
        }
    }

    fn shift(&self) -> bool {
        self.shift_left || self.shift_right
    }

    fn ctrl(&self) -> bool {
        self.ctrl_left || self.ctrl_right
    }

    fn alt(&self) -> bool {
        self.alt_left || self.alt_right
    }

    /// xterm's modifier parameter: 1 + Shift + 2·Alt + 4·Ctrl.
    fn modifier_parameter(&self) -> u8 {
        1 + u8::from(self.shift()) + 2 * u8::from(self.alt()) + 4 * u8::from(self.ctrl())
    }

    /// The bytes `key` going `action` sends, without changing the
    /// keyboard's state: what [`Self::apply`] would send.
    pub fn preview(&self, key: Key, action: KeyAction) -> TerminalBytes {
        let mut copy = *self;
        copy.apply(key, action)
    }

    /// Record `key` going `action` and return the bytes the terminal sends.
    pub fn apply(&mut self, key: Key, action: KeyAction) -> TerminalBytes {
        let down = action == KeyAction::Press;
        let role = role(key);
        match role {
            Role::Modifier(modifier) => {
                let held = match modifier {
                    Modifier::ShiftLeft => &mut self.shift_left,
                    Modifier::ShiftRight => &mut self.shift_right,
                    Modifier::CtrlLeft => &mut self.ctrl_left,
                    Modifier::CtrlRight => &mut self.ctrl_right,
                    Modifier::AltLeft => &mut self.alt_left,
                    Modifier::AltRight => &mut self.alt_right,
                };
                *held = down;
            }
            Role::Lock(lock) => {
                let engaged = match lock {
                    Lock::Caps => &mut self.caps_lock,
                    Lock::Num => &mut self.num_lock_off,
                };
                *engaged ^= down;
            }
            Role::Character { .. } | Role::Keypad { .. } | Role::Control(_) | Role::Tab | Role::Sequence(_) | Role::Silent => {}
        }
        if down { self.press(role) } else { TerminalBytes::NONE }
    }

    fn press(&self, role: Role) -> TerminalBytes {
        match role {
            Role::Modifier(_) | Role::Lock(_) | Role::Silent => TerminalBytes::NONE,
            Role::Character { plain, shifted } => self.character(plain, shifted),
            // The keypad's digit keys type digits under Num Lock and are the
            // editing keys without it.
            Role::Keypad { digit, .. } if !self.num_lock_off => self.character(digit, digit),
            Role::Keypad { sequence, .. } | Role::Sequence(sequence) => self.sequence(sequence),
            Role::Control(byte) => self.control(byte),
            Role::Tab if self.shift() => {
                let mut out = TerminalBytes::NONE;
                out.push(ESC);
                out.push(b'[');
                out.push(b'Z');
                out
            }
            Role::Tab => self.control(b'\t'),
        }
    }

    /// An escape sequence, with xterm's `;modifier` parameter when Shift, Alt
    /// or Ctrl is held.
    fn sequence(&self, sequence: Sequence) -> TerminalBytes {
        let mut out = TerminalBytes::NONE;
        let modifier = self.modifier_parameter();
        out.push(ESC);
        match sequence {
            Sequence::Cursor(last) | Sequence::Ss3(last) if modifier > 1 => {
                out.push(b'[');
                out.push(b'1');
                out.push(b';');
                out.push_number(modifier);
                out.push(last);
            }
            Sequence::Cursor(last) => {
                out.push(b'[');
                out.push(last);
            }
            Sequence::Ss3(last) => {
                out.push(b'O');
                out.push(last);
            }
            Sequence::Tilde(number) => {
                out.push(b'[');
                out.push_number(number);
                if modifier > 1 {
                    out.push(b';');
                    out.push_number(modifier);
                }
                out.push(b'~');
            }
        }
        out
    }

    /// A printable key: Shift (or Caps Lock, for letters) picks the shifted
    /// character, Ctrl turns it into a control code, Alt prefixes `ESC`.
    fn character(&self, plain: u8, shifted: u8) -> TerminalBytes {
        let letter = plain.is_ascii_lowercase();
        let upper = self.shift() != (letter && self.caps_lock);
        let byte = if upper { shifted } else { plain };
        let byte = if self.ctrl() { with_ctrl(byte) } else { byte };
        self.control(byte)
    }

    /// One byte, with Alt's `ESC` prefix when Alt is held.
    fn control(&self, byte: u8) -> TerminalBytes {
        let mut out = TerminalBytes::NONE;
        if self.alt() {
            out.push(ESC);
        }
        out.push(byte);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(keyboard: &mut TerminalKeyboard, key: Key) -> TerminalBytes {
        keyboard.apply(key, KeyAction::Press)
    }

    /// Letters, Shift, Caps Lock and Ctrl follow a terminal's rules: Caps Lock
    /// flips letters only, and Ctrl+C is ETX.
    #[test]
    fn printable_keys_type_their_characters() {
        let mut kb = TerminalKeyboard::new();
        assert_eq!(press(&mut kb, Key::A).as_slice(), b"a");
        assert_eq!(press(&mut kb, Key::K1).as_slice(), b"1");
        kb.apply(Key::ShiftL, KeyAction::Press);
        assert_eq!(press(&mut kb, Key::A).as_slice(), b"A");
        assert_eq!(press(&mut kb, Key::K1).as_slice(), b"!");
        kb.apply(Key::ShiftL, KeyAction::Release);
        kb.apply(Key::CapsLock, KeyAction::Press);
        kb.apply(Key::CapsLock, KeyAction::Release);
        assert_eq!(press(&mut kb, Key::A).as_slice(), b"A");
        assert_eq!(press(&mut kb, Key::K1).as_slice(), b"1", "Caps Lock leaves digits alone");
        kb.apply(Key::CapsLock, KeyAction::Press);
        kb.apply(Key::CtrlL, KeyAction::Press);
        assert_eq!(press(&mut kb, Key::C).as_slice(), &[0x03]);
        assert_eq!(kb.apply(Key::C, KeyAction::Release).as_slice(), b"", "a release sends nothing");
    }

    /// Cursor, editing and function keys send xterm's sequences, with the
    /// modifier parameter when one is held.
    #[test]
    fn special_keys_send_xterm_sequences() {
        let mut kb = TerminalKeyboard::new();
        assert_eq!(press(&mut kb, Key::Up).as_slice(), b"\x1b[A");
        assert_eq!(press(&mut kb, Key::F1).as_slice(), b"\x1bOP");
        assert_eq!(press(&mut kb, Key::F5).as_slice(), b"\x1b[15~");
        assert_eq!(press(&mut kb, Key::Delete).as_slice(), b"\x1b[3~");
        assert_eq!(press(&mut kb, Key::Enter).as_slice(), b"\r");
        assert_eq!(press(&mut kb, Key::Backspace).as_slice(), &[0x7F]);
        kb.apply(Key::CtrlL, KeyAction::Press);
        assert_eq!(press(&mut kb, Key::Right).as_slice(), b"\x1b[1;5C");
        assert_eq!(press(&mut kb, Key::F12).as_slice(), b"\x1b[24;5~");
        kb.apply(Key::CtrlL, KeyAction::Release);
        kb.apply(Key::AltL, KeyAction::Press);
        assert_eq!(press(&mut kb, Key::X).as_slice(), b"\x1bx");
        kb.apply(Key::AltL, KeyAction::Release);
        kb.apply(Key::ShiftR, KeyAction::Press);
        assert_eq!(press(&mut kb, Key::Tab).as_slice(), b"\x1b[Z");
        assert!(press(&mut kb, Key::Print).is_empty());
    }

    /// The keypad types digits under Num Lock and moves the cursor without it.
    #[test]
    fn the_keypad_follows_num_lock() {
        let mut kb = TerminalKeyboard::new();
        assert_eq!(press(&mut kb, Key::KpHome).as_slice(), b"7");
        assert_eq!(press(&mut kb, Key::KpDelete).as_slice(), b".");
        kb.apply(Key::NumLock, KeyAction::Press);
        kb.apply(Key::NumLock, KeyAction::Release);
        assert_eq!(press(&mut kb, Key::KpHome).as_slice(), b"\x1b[H");
        assert_eq!(press(&mut kb, Key::KpPageUp).as_slice(), b"\x1b[5~");
        assert_eq!(press(&mut kb, Key::Kp5).as_slice(), b"\x1b[E");
        assert_eq!(press(&mut kb, Key::KpAdd).as_slice(), b"+");
    }

    /// Modifier and lock keys send nothing either way, and a released
    /// modifier stops applying.
    #[test]
    fn modifiers_and_locks_are_silent() {
        let mut kb = TerminalKeyboard::new();
        for key in [Key::ShiftL, Key::CtrlR, Key::AltR, Key::CapsLock, Key::NumLock] {
            assert!(kb.apply(key, KeyAction::Press).is_empty());
            assert!(kb.apply(key, KeyAction::Release).is_empty());
        }
        kb.apply(Key::ShiftL, KeyAction::Press);
        kb.apply(Key::ShiftL, KeyAction::Release);
        assert_eq!(press(&mut kb, Key::Q).as_slice(), b"Q", "Caps Lock was toggled on above");
    }
}
