//! Keyboard input as every rusty_box machine sees it, whatever its
//! architecture: the keys of a PC keyboard, the US layout that spells
//! characters with them, what became of typing text into a guest, and the
//! [`KeyboardInput`] verbs a host drives a guest's keyboard through.
//!
//! What a key becomes on the wire is the machine's business: an x86 PC's
//! 8042 renders it as PS/2 scancodes in the guest's selected set, while a
//! serial-console machine renders it as the bytes a terminal sends
//! ([`terminal`]).

pub mod terminal;

/// A key, numbered as Bochs's `BX_KEY_*` enumeration (`gui/gui.h`), so the
/// discriminant indexes Bochs's per-key tables (`iodev/scancodes.cc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Key {
    /// Bochs `BX_KEY_CTRL_L`
    CtrlL = 0,
    /// Bochs `BX_KEY_SHIFT_L`
    ShiftL = 1,
    /// Bochs `BX_KEY_F1`
    F1 = 2,
    /// Bochs `BX_KEY_F2`
    F2 = 3,
    /// Bochs `BX_KEY_F3`
    F3 = 4,
    /// Bochs `BX_KEY_F4`
    F4 = 5,
    /// Bochs `BX_KEY_F5`
    F5 = 6,
    /// Bochs `BX_KEY_F6`
    F6 = 7,
    /// Bochs `BX_KEY_F7`
    F7 = 8,
    /// Bochs `BX_KEY_F8`
    F8 = 9,
    /// Bochs `BX_KEY_F9`
    F9 = 10,
    /// Bochs `BX_KEY_F10`
    F10 = 11,
    /// Bochs `BX_KEY_F11`
    F11 = 12,
    /// Bochs `BX_KEY_F12`
    F12 = 13,
    /// Bochs `BX_KEY_CTRL_R`
    CtrlR = 14,
    /// Bochs `BX_KEY_SHIFT_R`
    ShiftR = 15,
    /// Bochs `BX_KEY_CAPS_LOCK`
    CapsLock = 16,
    /// Bochs `BX_KEY_NUM_LOCK`
    NumLock = 17,
    /// Bochs `BX_KEY_ALT_L`
    AltL = 18,
    /// Bochs `BX_KEY_ALT_R`
    AltR = 19,
    /// Bochs `BX_KEY_A`
    A = 20,
    /// Bochs `BX_KEY_B`
    B = 21,
    /// Bochs `BX_KEY_C`
    C = 22,
    /// Bochs `BX_KEY_D`
    D = 23,
    /// Bochs `BX_KEY_E`
    E = 24,
    /// Bochs `BX_KEY_F`
    F = 25,
    /// Bochs `BX_KEY_G`
    G = 26,
    /// Bochs `BX_KEY_H`
    H = 27,
    /// Bochs `BX_KEY_I`
    I = 28,
    /// Bochs `BX_KEY_J`
    J = 29,
    /// Bochs `BX_KEY_K`
    K = 30,
    /// Bochs `BX_KEY_L`
    L = 31,
    /// Bochs `BX_KEY_M`
    M = 32,
    /// Bochs `BX_KEY_N`
    N = 33,
    /// Bochs `BX_KEY_O`
    O = 34,
    /// Bochs `BX_KEY_P`
    P = 35,
    /// Bochs `BX_KEY_Q`
    Q = 36,
    /// Bochs `BX_KEY_R`
    R = 37,
    /// Bochs `BX_KEY_S`
    S = 38,
    /// Bochs `BX_KEY_T`
    T = 39,
    /// Bochs `BX_KEY_U`
    U = 40,
    /// Bochs `BX_KEY_V`
    V = 41,
    /// Bochs `BX_KEY_W`
    W = 42,
    /// Bochs `BX_KEY_X`
    X = 43,
    /// Bochs `BX_KEY_Y`
    Y = 44,
    /// Bochs `BX_KEY_Z`
    Z = 45,
    /// Bochs `BX_KEY_0`
    K0 = 46,
    /// Bochs `BX_KEY_1`
    K1 = 47,
    /// Bochs `BX_KEY_2`
    K2 = 48,
    /// Bochs `BX_KEY_3`
    K3 = 49,
    /// Bochs `BX_KEY_4`
    K4 = 50,
    /// Bochs `BX_KEY_5`
    K5 = 51,
    /// Bochs `BX_KEY_6`
    K6 = 52,
    /// Bochs `BX_KEY_7`
    K7 = 53,
    /// Bochs `BX_KEY_8`
    K8 = 54,
    /// Bochs `BX_KEY_9`
    K9 = 55,
    /// Bochs `BX_KEY_ESC`
    Esc = 56,
    /// Bochs `BX_KEY_SPACE`
    Space = 57,
    /// Bochs `BX_KEY_SINGLE_QUOTE`
    SingleQuote = 58,
    /// Bochs `BX_KEY_COMMA`
    Comma = 59,
    /// Bochs `BX_KEY_PERIOD`
    Period = 60,
    /// Bochs `BX_KEY_SLASH`
    Slash = 61,
    /// Bochs `BX_KEY_SEMICOLON`
    Semicolon = 62,
    /// Bochs `BX_KEY_EQUALS`
    Equals = 63,
    /// Bochs `BX_KEY_LEFT_BRACKET`
    LeftBracket = 64,
    /// Bochs `BX_KEY_BACKSLASH`
    Backslash = 65,
    /// Bochs `BX_KEY_RIGHT_BRACKET`
    RightBracket = 66,
    /// Bochs `BX_KEY_MINUS`
    Minus = 67,
    /// Bochs `BX_KEY_GRAVE`
    Grave = 68,
    /// Bochs `BX_KEY_BACKSPACE`
    Backspace = 69,
    /// Bochs `BX_KEY_ENTER`
    Enter = 70,
    /// Bochs `BX_KEY_TAB`
    Tab = 71,
    /// Bochs `BX_KEY_LEFT_BACKSLASH`
    LeftBackslash = 72,
    /// Bochs `BX_KEY_PRINT`
    Print = 73,
    /// Bochs `BX_KEY_SCRL_LOCK`
    ScrlLock = 74,
    /// Bochs `BX_KEY_PAUSE`
    Pause = 75,
    /// Bochs `BX_KEY_INSERT`
    Insert = 76,
    /// Bochs `BX_KEY_DELETE`
    Delete = 77,
    /// Bochs `BX_KEY_HOME`
    Home = 78,
    /// Bochs `BX_KEY_END`
    End = 79,
    /// Bochs `BX_KEY_PAGE_UP`
    PageUp = 80,
    /// Bochs `BX_KEY_PAGE_DOWN`
    PageDown = 81,
    /// Bochs `BX_KEY_KP_ADD`
    KpAdd = 82,
    /// Bochs `BX_KEY_KP_SUBTRACT`
    KpSubtract = 83,
    /// Bochs `BX_KEY_KP_END`
    KpEnd = 84,
    /// Bochs `BX_KEY_KP_DOWN`
    KpDown = 85,
    /// Bochs `BX_KEY_KP_PAGE_DOWN`
    KpPageDown = 86,
    /// Bochs `BX_KEY_KP_LEFT`
    KpLeft = 87,
    /// Bochs `BX_KEY_KP_RIGHT`
    KpRight = 88,
    /// Bochs `BX_KEY_KP_HOME`
    KpHome = 89,
    /// Bochs `BX_KEY_KP_UP`
    KpUp = 90,
    /// Bochs `BX_KEY_KP_PAGE_UP`
    KpPageUp = 91,
    /// Bochs `BX_KEY_KP_INSERT`
    KpInsert = 92,
    /// Bochs `BX_KEY_KP_DELETE`
    KpDelete = 93,
    /// Bochs `BX_KEY_KP_5`
    Kp5 = 94,
    /// Bochs `BX_KEY_UP`
    Up = 95,
    /// Bochs `BX_KEY_DOWN`
    Down = 96,
    /// Bochs `BX_KEY_LEFT`
    Left = 97,
    /// Bochs `BX_KEY_RIGHT`
    Right = 98,
    /// Bochs `BX_KEY_KP_ENTER`
    KpEnter = 99,
    /// Bochs `BX_KEY_KP_MULTIPLY`
    KpMultiply = 100,
    /// Bochs `BX_KEY_KP_DIVIDE`
    KpDivide = 101,
    /// Bochs `BX_KEY_WIN_L`
    WinL = 102,
    /// Bochs `BX_KEY_WIN_R`
    WinR = 103,
    /// Bochs `BX_KEY_MENU`
    Menu = 104,
    /// Bochs `BX_KEY_ALT_SYSREQ`
    AltSysreq = 105,
    /// Bochs `BX_KEY_CTRL_BREAK`
    CtrlBreak = 106,
    /// Bochs `BX_KEY_INT_BACK`
    IntBack = 107,
    /// Bochs `BX_KEY_INT_FORWARD`
    IntForward = 108,
    /// Bochs `BX_KEY_INT_STOP`
    IntStop = 109,
    /// Bochs `BX_KEY_INT_MAIL`
    IntMail = 110,
    /// Bochs `BX_KEY_INT_SEARCH`
    IntSearch = 111,
    /// Bochs `BX_KEY_INT_FAV`
    IntFav = 112,
    /// Bochs `BX_KEY_INT_HOME`
    IntHome = 113,
    /// Bochs `BX_KEY_POWER_MYCOMP`
    PowerMycomp = 114,
    /// Bochs `BX_KEY_POWER_CALC`
    PowerCalc = 115,
    /// Bochs `BX_KEY_POWER_SLEEP`
    PowerSleep = 116,
    /// Bochs `BX_KEY_POWER_POWER`
    PowerPower = 117,
    /// Bochs `BX_KEY_POWER_WAKE`
    PowerWake = 118,
}

// `PowerWake` is Bochs's last key (`BX_KEY_NBKEYS` follows it), and the
// per-key tables are `COUNT` long. A key added to the enum does not compile
// until every exhaustive per-key match (the terminal's role table) places it,
// and moving the last key without raising `COUNT` stops the build here.
const _: () = assert!(Key::PowerWake as usize + 1 == Key::COUNT);

impl Key {
    /// How many keys there are (`BX_KEY_NBKEYS`).
    pub const COUNT: usize = 119;

    /// The key's index into Bochs's per-key tables (Bochs uses `key & 0xFF`).
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Whether a key goes down or comes up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyAction {
    Press,
    Release,
}

/// The key that types a character on a US layout, and whether Shift is held
/// for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyStroke {
    pub key: Key,
    pub shift: bool,
}

const LETTERS: [Key; 26] = [
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
];

const DIGITS: [Key; 10] = [
    Key::K0,
    Key::K1,
    Key::K2,
    Key::K3,
    Key::K4,
    Key::K5,
    Key::K6,
    Key::K7,
    Key::K8,
    Key::K9,
];

impl KeyStroke {
    const fn plain(key: Key) -> Option<Self> {
        Some(Self { key, shift: false })
    }

    const fn shifted(key: Key) -> Option<Self> {
        Some(Self { key, shift: true })
    }

    /// The stroke that types `ch` on a US layout, or `None` when no key
    /// does. Newline and carriage return are both Enter.
    pub fn for_char(ch: char) -> Option<Self> {
        match ch {
            'a'..='z' => Self::plain(LETTERS[ch as usize - 'a' as usize]),
            'A'..='Z' => Self::shifted(LETTERS[ch as usize - 'A' as usize]),
            '0'..='9' => Self::plain(DIGITS[ch as usize - '0' as usize]),
            ')' => Self::shifted(Key::K0),
            '!' => Self::shifted(Key::K1),
            '@' => Self::shifted(Key::K2),
            '#' => Self::shifted(Key::K3),
            '$' => Self::shifted(Key::K4),
            '%' => Self::shifted(Key::K5),
            '^' => Self::shifted(Key::K6),
            '&' => Self::shifted(Key::K7),
            '*' => Self::shifted(Key::K8),
            '(' => Self::shifted(Key::K9),
            ' ' => Self::plain(Key::Space),
            '\n' | '\r' => Self::plain(Key::Enter),
            '\t' => Self::plain(Key::Tab),
            '\u{8}' => Self::plain(Key::Backspace),
            '\u{1B}' => Self::plain(Key::Esc),
            '-' => Self::plain(Key::Minus),
            '_' => Self::shifted(Key::Minus),
            '=' => Self::plain(Key::Equals),
            '+' => Self::shifted(Key::Equals),
            '[' => Self::plain(Key::LeftBracket),
            '{' => Self::shifted(Key::LeftBracket),
            ']' => Self::plain(Key::RightBracket),
            '}' => Self::shifted(Key::RightBracket),
            '\\' => Self::plain(Key::Backslash),
            '|' => Self::shifted(Key::Backslash),
            ';' => Self::plain(Key::Semicolon),
            ':' => Self::shifted(Key::Semicolon),
            '\'' => Self::plain(Key::SingleQuote),
            '"' => Self::shifted(Key::SingleQuote),
            ',' => Self::plain(Key::Comma),
            '<' => Self::shifted(Key::Comma),
            '.' => Self::plain(Key::Period),
            '>' => Self::shifted(Key::Period),
            '/' => Self::plain(Key::Slash),
            '?' => Self::shifted(Key::Slash),
            '`' => Self::plain(Key::Grave),
            '~' => Self::shifted(Key::Grave),
            _ => None,
        }
    }

    /// The key events that type this stroke: Shift down, the key down and up,
    /// Shift up — the Shift pair only when the stroke needs it.
    pub fn events(self) -> KeyEvents {
        let mut events = [(self.key, KeyAction::Press); 4];
        let len = if self.shift {
            events = [
                (Key::ShiftL, KeyAction::Press),
                (self.key, KeyAction::Press),
                (self.key, KeyAction::Release),
                (Key::ShiftL, KeyAction::Release),
            ];
            4
        } else {
            events[1] = (self.key, KeyAction::Release);
            2
        };
        KeyEvents { events, len, next: 0 }
    }
}

/// The key events of one [`KeyStroke`], in order.
#[derive(Debug, Clone)]
pub struct KeyEvents {
    events: [(Key, KeyAction); 4],
    len: usize,
    next: usize,
}

impl Iterator for KeyEvents {
    type Item = (Key, KeyAction);

    fn next(&mut self) -> Option<Self::Item> {
        let event = self.events.get(self.next).filter(|_| self.next < self.len).copied();
        if event.is_some() {
            self.next += 1;
        }
        event
    }
}

/// What became of a [`KeyboardInput::type_text`] call.
///
/// A bare count cannot say why typing stopped, and the two reasons need
/// opposite responses (R2): a guest that is not draining its input will take
/// the rest once it has, while a character the modelled US keyboard has no
/// key for will never be typed however often it is offered. A caller that
/// resumed from a count alone would retry such a character forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "text that did not reach the guest is lost unless the caller resumes it"]
pub enum Typed {
    /// Every character reached the guest.
    All {
        /// How many characters that was.
        delivered: usize,
    },
    /// The guest's keyboard took no more. Step the machine so it drains its
    /// input, then resume with `text.chars().skip(delivered)`.
    ///
    /// Nothing of the character this stopped on reached the guest: one starts
    /// only while its whole key sequence fits (Bochs devices.cc
    /// `service_paste_buf`), so resuming sends each character exactly once.
    Refused {
        /// Characters delivered whole before the refusal.
        delivered: usize,
    },
    /// `ch` has no key on the modelled US keyboard, so neither it nor
    /// anything after it was sent. Resume past it with
    /// `text.chars().skip(delivered + 1)`.
    Unmappable {
        /// Characters delivered whole before this one.
        delivered: usize,
        /// The character that cannot be typed.
        ch: char,
    },
}

impl Typed {
    /// How many characters the guest received in full.
    pub fn delivered(self) -> usize {
        match self {
            Self::All { delivered } | Self::Refused { delivered } | Self::Unmappable { delivered, .. } => delivered,
        }
    }

    /// Whether the whole text arrived.
    pub fn is_complete(self) -> bool {
        matches!(self, Self::All { .. })
    }
}

/// The verbs a host drives a guest's keyboard with, the same on every
/// machine. Every verb reports how much got through. A refusal means the
/// guest is not taking keys right now — it has not drained what it was sent,
/// or (on a PC) it has the keyboard's clock or scanning switched off — and
/// every cause has the same answer: step the machine and send again.
pub trait KeyboardInput {
    /// Press or release `key`. Returns whether it reached the guest, which
    /// it does whole or not at all.
    #[must_use = "a refused key never reached the guest"]
    fn key(&mut self, key: Key, action: KeyAction) -> bool;

    /// Press and release `key`, whole: `false` means neither half was sent.
    #[must_use = "a refused tap never reached the guest"]
    fn tap(&mut self, key: Key) -> bool;

    /// Type `text` as US-layout key strokes, one character at a time, each
    /// starting only while all of it fits.
    fn type_text(&mut self, text: &str) -> Typed;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shifted stroke wraps its key in a Shift press and release; a plain
    /// one is just the key.
    #[test]
    fn strokes_spell_shift_around_the_key() {
        let upper: [(Key, KeyAction); 4] = [
            (Key::ShiftL, KeyAction::Press),
            (Key::Q, KeyAction::Press),
            (Key::Q, KeyAction::Release),
            (Key::ShiftL, KeyAction::Release),
        ];
        let stroke = KeyStroke::for_char('Q');
        assert_eq!(stroke, Some(KeyStroke { key: Key::Q, shift: true }));
        assert!(stroke.is_some_and(|s| s.events().eq(upper)));
        let lower = KeyStroke::for_char('q').map(|s| s.events().count());
        assert_eq!(lower, Some(2));
        assert_eq!(KeyStroke::for_char('é'), None);
    }

    /// The enum's order is Bochs's, so its discriminants index Bochs tables.
    #[test]
    fn keys_keep_the_bochs_numbering() {
        assert_eq!(Key::CtrlL.index(), 0);
        assert_eq!(Key::A.index(), 20);
        assert_eq!(Key::Enter.index(), 70);
        assert_eq!(Key::PowerWake.index(), Key::COUNT - 1);
    }
}
