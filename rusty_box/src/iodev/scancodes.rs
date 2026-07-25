//! PS/2 scancode tables for sets 1, 2 and 3.
//!
//! Generated from Bochs `iodev/scancodes.cc` (`scancode scancodes[BX_KEY_NBKEYS][3]`)
//! and the `BX_KEY_*` enumeration in `gui/gui.h`. Each key has a make and a
//! break byte sequence per scancode set; the keyboard controller selects the
//! set with the 0xF0 command and may additionally translate to set 1 through
//! the 8042 translation table (Bochs keyboard.cc `gen_scancode`).

/// Number of keys in the Bochs key enumeration (`BX_KEY_NBKEYS`, gui.h).
pub const BX_KEY_NBKEYS: usize = 119;

/// A guest key, matching the Bochs `BX_KEY_*` enumeration order - the index
/// into [`SCANCODES`]. GUI front ends emit these rather than raw scancodes so
/// the active scancode set is honored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum BxKey {
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

impl BxKey {
    /// Table index for this key (Bochs uses `key & 0xFF`).
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }
}

/// The make and break byte sequences for one key in one scancode set.
/// Bochs: `typedef struct { const char *make; const char *brek; } scancode;`
#[derive(Debug, Clone, Copy)]
pub struct Scancode {
    /// Bytes emitted when the key is pressed.
    pub make: &'static [u8],
    /// Bytes emitted when the key is released.
    pub brek: &'static [u8],
}

/// `SCANCODES[key][set]`, where `set` is 0 for scancode set 1, 1 for set 2 and
/// 2 for set 3 - the same 0-based indexing as Bochs `current_scancodes_set`.
pub static SCANCODES: [[Scancode; 3]; BX_KEY_NBKEYS] = [
    // BX_KEY_CTRL_L
    [
        Scancode { make: &[0x1D], brek: &[0x9D] },
        Scancode { make: &[0x14], brek: &[0xF0, 0x14] },
        Scancode { make: &[0x11], brek: &[0xF0, 0x11] },
    ],
    // BX_KEY_SHIFT_L
    [
        Scancode { make: &[0x2A], brek: &[0xAA] },
        Scancode { make: &[0x12], brek: &[0xF0, 0x12] },
        Scancode { make: &[0x12], brek: &[0xF0, 0x12] },
    ],
    // BX_KEY_F1
    [
        Scancode { make: &[0x3B], brek: &[0xBB] },
        Scancode { make: &[0x05], brek: &[0xF0, 0x05] },
        Scancode { make: &[0x07], brek: &[0xF0, 0x07] },
    ],
    // BX_KEY_F2
    [
        Scancode { make: &[0x3C], brek: &[0xBC] },
        Scancode { make: &[0x06], brek: &[0xF0, 0x06] },
        Scancode { make: &[0x0F], brek: &[0xF0, 0x0F] },
    ],
    // BX_KEY_F3
    [
        Scancode { make: &[0x3D], brek: &[0xBD] },
        Scancode { make: &[0x04], brek: &[0xF0, 0x04] },
        Scancode { make: &[0x17], brek: &[0xF0, 0x17] },
    ],
    // BX_KEY_F4
    [
        Scancode { make: &[0x3E], brek: &[0xBE] },
        Scancode { make: &[0x0C], brek: &[0xF0, 0x0C] },
        Scancode { make: &[0x1F], brek: &[0xF0, 0x1F] },
    ],
    // BX_KEY_F5
    [
        Scancode { make: &[0x3F], brek: &[0xBF] },
        Scancode { make: &[0x03], brek: &[0xF0, 0x03] },
        Scancode { make: &[0x27], brek: &[0xF0, 0x27] },
    ],
    // BX_KEY_F6
    [
        Scancode { make: &[0x40], brek: &[0xC0] },
        Scancode { make: &[0x0B], brek: &[0xF0, 0x0B] },
        Scancode { make: &[0x2F], brek: &[0xF0, 0x2F] },
    ],
    // BX_KEY_F7
    [
        Scancode { make: &[0x41], brek: &[0xC1] },
        Scancode { make: &[0x83], brek: &[0xF0, 0x83] },
        Scancode { make: &[0x37], brek: &[0xF0, 0x37] },
    ],
    // BX_KEY_F8
    [
        Scancode { make: &[0x42], brek: &[0xC2] },
        Scancode { make: &[0x0A], brek: &[0xF0, 0x0A] },
        Scancode { make: &[0x3F], brek: &[0xF0, 0x3F] },
    ],
    // BX_KEY_F9
    [
        Scancode { make: &[0x43], brek: &[0xC3] },
        Scancode { make: &[0x01], brek: &[0xF0, 0x01] },
        Scancode { make: &[0x47], brek: &[0xF0, 0x47] },
    ],
    // BX_KEY_F10
    [
        Scancode { make: &[0x44], brek: &[0xC4] },
        Scancode { make: &[0x09], brek: &[0xF0, 0x09] },
        Scancode { make: &[0x4F], brek: &[0xF0, 0x4F] },
    ],
    // BX_KEY_F11
    [
        Scancode { make: &[0x57], brek: &[0xD7] },
        Scancode { make: &[0x78], brek: &[0xF0, 0x78] },
        Scancode { make: &[0x56], brek: &[0xF0, 0x56] },
    ],
    // BX_KEY_F12
    [
        Scancode { make: &[0x58], brek: &[0xD8] },
        Scancode { make: &[0x07], brek: &[0xF0, 0x07] },
        Scancode { make: &[0x5E], brek: &[0xF0, 0x5E] },
    ],
    // BX_KEY_CTRL_R
    [
        Scancode { make: &[0xE0, 0x1D], brek: &[0xE0, 0x9D] },
        Scancode { make: &[0xE0, 0x14], brek: &[0xE0, 0xF0, 0x14] },
        Scancode { make: &[0x58], brek: &[0xF0, 0x58] },
    ],
    // BX_KEY_SHIFT_R
    [
        Scancode { make: &[0x36], brek: &[0xB6] },
        Scancode { make: &[0x59], brek: &[0xF0, 0x59] },
        Scancode { make: &[0x59], brek: &[0xF0, 0x59] },
    ],
    // BX_KEY_CAPS_LOCK
    [
        Scancode { make: &[0x3A], brek: &[0xBA] },
        Scancode { make: &[0x58], brek: &[0xF0, 0x58] },
        Scancode { make: &[0x14], brek: &[0xF0, 0x14] },
    ],
    // BX_KEY_NUM_LOCK
    [
        Scancode { make: &[0x45], brek: &[0xC5] },
        Scancode { make: &[0x77], brek: &[0xF0, 0x77] },
        Scancode { make: &[0x76], brek: &[0xF0, 0x76] },
    ],
    // BX_KEY_ALT_L
    [
        Scancode { make: &[0x38], brek: &[0xB8] },
        Scancode { make: &[0x11], brek: &[0xF0, 0x11] },
        Scancode { make: &[0x19], brek: &[0xF0, 0x19] },
    ],
    // BX_KEY_ALT_R
    [
        Scancode { make: &[0xE0, 0x38], brek: &[0xE0, 0xB8] },
        Scancode { make: &[0xE0, 0x11], brek: &[0xE0, 0xF0, 0x11] },
        Scancode { make: &[0x39], brek: &[0xF0, 0x39] },
    ],
    // BX_KEY_A
    [
        Scancode { make: &[0x1E], brek: &[0x9E] },
        Scancode { make: &[0x1C], brek: &[0xF0, 0x1C] },
        Scancode { make: &[0x1C], brek: &[0xF0, 0x1C] },
    ],
    // BX_KEY_B
    [
        Scancode { make: &[0x30], brek: &[0xB0] },
        Scancode { make: &[0x32], brek: &[0xF0, 0x32] },
        Scancode { make: &[0x32], brek: &[0xF0, 0x32] },
    ],
    // BX_KEY_C
    [
        Scancode { make: &[0x2E], brek: &[0xAE] },
        Scancode { make: &[0x21], brek: &[0xF0, 0x21] },
        Scancode { make: &[0x21], brek: &[0xF0, 0x21] },
    ],
    // BX_KEY_D
    [
        Scancode { make: &[0x20], brek: &[0xA0] },
        Scancode { make: &[0x23], brek: &[0xF0, 0x23] },
        Scancode { make: &[0x23], brek: &[0xF0, 0x23] },
    ],
    // BX_KEY_E
    [
        Scancode { make: &[0x12], brek: &[0x92] },
        Scancode { make: &[0x24], brek: &[0xF0, 0x24] },
        Scancode { make: &[0x24], brek: &[0xF0, 0x24] },
    ],
    // BX_KEY_F
    [
        Scancode { make: &[0x21], brek: &[0xA1] },
        Scancode { make: &[0x2B], brek: &[0xF0, 0x2B] },
        Scancode { make: &[0x2B], brek: &[0xF0, 0x2B] },
    ],
    // BX_KEY_G
    [
        Scancode { make: &[0x22], brek: &[0xA2] },
        Scancode { make: &[0x34], brek: &[0xF0, 0x34] },
        Scancode { make: &[0x34], brek: &[0xF0, 0x34] },
    ],
    // BX_KEY_H
    [
        Scancode { make: &[0x23], brek: &[0xA3] },
        Scancode { make: &[0x33], brek: &[0xF0, 0x33] },
        Scancode { make: &[0x33], brek: &[0xF0, 0x33] },
    ],
    // BX_KEY_I
    [
        Scancode { make: &[0x17], brek: &[0x97] },
        Scancode { make: &[0x43], brek: &[0xF0, 0x43] },
        Scancode { make: &[0x43], brek: &[0xF0, 0x43] },
    ],
    // BX_KEY_J
    [
        Scancode { make: &[0x24], brek: &[0xA4] },
        Scancode { make: &[0x3B], brek: &[0xF0, 0x3B] },
        Scancode { make: &[0x3B], brek: &[0xF0, 0x3B] },
    ],
    // BX_KEY_K
    [
        Scancode { make: &[0x25], brek: &[0xA5] },
        Scancode { make: &[0x42], brek: &[0xF0, 0x42] },
        Scancode { make: &[0x42], brek: &[0xF0, 0x42] },
    ],
    // BX_KEY_L
    [
        Scancode { make: &[0x26], brek: &[0xA6] },
        Scancode { make: &[0x4B], brek: &[0xF0, 0x4B] },
        Scancode { make: &[0x4B], brek: &[0xF0, 0x4B] },
    ],
    // BX_KEY_M
    [
        Scancode { make: &[0x32], brek: &[0xB2] },
        Scancode { make: &[0x3A], brek: &[0xF0, 0x3A] },
        Scancode { make: &[0x3A], brek: &[0xF0, 0x3A] },
    ],
    // BX_KEY_N
    [
        Scancode { make: &[0x31], brek: &[0xB1] },
        Scancode { make: &[0x31], brek: &[0xF0, 0x31] },
        Scancode { make: &[0x31], brek: &[0xF0, 0x31] },
    ],
    // BX_KEY_O
    [
        Scancode { make: &[0x18], brek: &[0x98] },
        Scancode { make: &[0x44], brek: &[0xF0, 0x44] },
        Scancode { make: &[0x44], brek: &[0xF0, 0x44] },
    ],
    // BX_KEY_P
    [
        Scancode { make: &[0x19], brek: &[0x99] },
        Scancode { make: &[0x4D], brek: &[0xF0, 0x4D] },
        Scancode { make: &[0x4D], brek: &[0xF0, 0x4D] },
    ],
    // BX_KEY_Q
    [
        Scancode { make: &[0x10], brek: &[0x90] },
        Scancode { make: &[0x15], brek: &[0xF0, 0x15] },
        Scancode { make: &[0x15], brek: &[0xF0, 0x15] },
    ],
    // BX_KEY_R
    [
        Scancode { make: &[0x13], brek: &[0x93] },
        Scancode { make: &[0x2D], brek: &[0xF0, 0x2D] },
        Scancode { make: &[0x2D], brek: &[0xF0, 0x2D] },
    ],
    // BX_KEY_S
    [
        Scancode { make: &[0x1F], brek: &[0x9F] },
        Scancode { make: &[0x1B], brek: &[0xF0, 0x1B] },
        Scancode { make: &[0x1B], brek: &[0xF0, 0x1B] },
    ],
    // BX_KEY_T
    [
        Scancode { make: &[0x14], brek: &[0x94] },
        Scancode { make: &[0x2C], brek: &[0xF0, 0x2C] },
        Scancode { make: &[0x2C], brek: &[0xF0, 0x2C] },
    ],
    // BX_KEY_U
    [
        Scancode { make: &[0x16], brek: &[0x96] },
        Scancode { make: &[0x3C], brek: &[0xF0, 0x3C] },
        Scancode { make: &[0x3C], brek: &[0xF0, 0x3C] },
    ],
    // BX_KEY_V
    [
        Scancode { make: &[0x2F], brek: &[0xAF] },
        Scancode { make: &[0x2A], brek: &[0xF0, 0x2A] },
        Scancode { make: &[0x2A], brek: &[0xF0, 0x2A] },
    ],
    // BX_KEY_W
    [
        Scancode { make: &[0x11], brek: &[0x91] },
        Scancode { make: &[0x1D], brek: &[0xF0, 0x1D] },
        Scancode { make: &[0x1D], brek: &[0xF0, 0x1D] },
    ],
    // BX_KEY_X
    [
        Scancode { make: &[0x2D], brek: &[0xAD] },
        Scancode { make: &[0x22], brek: &[0xF0, 0x22] },
        Scancode { make: &[0x22], brek: &[0xF0, 0x22] },
    ],
    // BX_KEY_Y
    [
        Scancode { make: &[0x15], brek: &[0x95] },
        Scancode { make: &[0x35], brek: &[0xF0, 0x35] },
        Scancode { make: &[0x35], brek: &[0xF0, 0x35] },
    ],
    // BX_KEY_Z
    [
        Scancode { make: &[0x2C], brek: &[0xAC] },
        Scancode { make: &[0x1A], brek: &[0xF0, 0x1A] },
        Scancode { make: &[0x1A], brek: &[0xF0, 0x1A] },
    ],
    // BX_KEY_0
    [
        Scancode { make: &[0x0B], brek: &[0x8B] },
        Scancode { make: &[0x45], brek: &[0xF0, 0x45] },
        Scancode { make: &[0x45], brek: &[0xF0, 0x45] },
    ],
    // BX_KEY_1
    [
        Scancode { make: &[0x02], brek: &[0x82] },
        Scancode { make: &[0x16], brek: &[0xF0, 0x16] },
        Scancode { make: &[0x16], brek: &[0xF0, 0x16] },
    ],
    // BX_KEY_2
    [
        Scancode { make: &[0x03], brek: &[0x83] },
        Scancode { make: &[0x1E], brek: &[0xF0, 0x1E] },
        Scancode { make: &[0x1E], brek: &[0xF0, 0x1E] },
    ],
    // BX_KEY_3
    [
        Scancode { make: &[0x04], brek: &[0x84] },
        Scancode { make: &[0x26], brek: &[0xF0, 0x26] },
        Scancode { make: &[0x26], brek: &[0xF0, 0x26] },
    ],
    // BX_KEY_4
    [
        Scancode { make: &[0x05], brek: &[0x85] },
        Scancode { make: &[0x25], brek: &[0xF0, 0x25] },
        Scancode { make: &[0x25], brek: &[0xF0, 0x25] },
    ],
    // BX_KEY_5
    [
        Scancode { make: &[0x06], brek: &[0x86] },
        Scancode { make: &[0x2E], brek: &[0xF0, 0x2E] },
        Scancode { make: &[0x2E], brek: &[0xF0, 0x2E] },
    ],
    // BX_KEY_6
    [
        Scancode { make: &[0x07], brek: &[0x87] },
        Scancode { make: &[0x36], brek: &[0xF0, 0x36] },
        Scancode { make: &[0x36], brek: &[0xF0, 0x36] },
    ],
    // BX_KEY_7
    [
        Scancode { make: &[0x08], brek: &[0x88] },
        Scancode { make: &[0x3D], brek: &[0xF0, 0x3D] },
        Scancode { make: &[0x3D], brek: &[0xF0, 0x3D] },
    ],
    // BX_KEY_8
    [
        Scancode { make: &[0x09], brek: &[0x89] },
        Scancode { make: &[0x3E], brek: &[0xF0, 0x3E] },
        Scancode { make: &[0x3E], brek: &[0xF0, 0x3E] },
    ],
    // BX_KEY_9
    [
        Scancode { make: &[0x0A], brek: &[0x8A] },
        Scancode { make: &[0x46], brek: &[0xF0, 0x46] },
        Scancode { make: &[0x46], brek: &[0xF0, 0x46] },
    ],
    // BX_KEY_ESC
    [
        Scancode { make: &[0x01], brek: &[0x81] },
        Scancode { make: &[0x76], brek: &[0xF0, 0x76] },
        Scancode { make: &[0x08], brek: &[0xF0, 0x08] },
    ],
    // BX_KEY_SPACE
    [
        Scancode { make: &[0x39], brek: &[0xB9] },
        Scancode { make: &[0x29], brek: &[0xF0, 0x29] },
        Scancode { make: &[0x29], brek: &[0xF0, 0x29] },
    ],
    // BX_KEY_SINGLE_QUOTE
    [
        Scancode { make: &[0x28], brek: &[0xA8] },
        Scancode { make: &[0x52], brek: &[0xF0, 0x52] },
        Scancode { make: &[0x52], brek: &[0xF0, 0x52] },
    ],
    // BX_KEY_COMMA
    [
        Scancode { make: &[0x33], brek: &[0xB3] },
        Scancode { make: &[0x41], brek: &[0xF0, 0x41] },
        Scancode { make: &[0x41], brek: &[0xF0, 0x41] },
    ],
    // BX_KEY_PERIOD
    [
        Scancode { make: &[0x34], brek: &[0xB4] },
        Scancode { make: &[0x49], brek: &[0xF0, 0x49] },
        Scancode { make: &[0x49], brek: &[0xF0, 0x49] },
    ],
    // BX_KEY_SLASH
    [
        Scancode { make: &[0x35], brek: &[0xB5] },
        Scancode { make: &[0x4A], brek: &[0xF0, 0x4A] },
        Scancode { make: &[0x4A], brek: &[0xF0, 0x4A] },
    ],
    // BX_KEY_SEMICOLON
    [
        Scancode { make: &[0x27], brek: &[0xA7] },
        Scancode { make: &[0x4C], brek: &[0xF0, 0x4C] },
        Scancode { make: &[0x4C], brek: &[0xF0, 0x4C] },
    ],
    // BX_KEY_EQUALS
    [
        Scancode { make: &[0x0D], brek: &[0x8D] },
        Scancode { make: &[0x55], brek: &[0xF0, 0x55] },
        Scancode { make: &[0x55], brek: &[0xF0, 0x55] },
    ],
    // BX_KEY_LEFT_BRACKET
    [
        Scancode { make: &[0x1A], brek: &[0x9A] },
        Scancode { make: &[0x54], brek: &[0xF0, 0x54] },
        Scancode { make: &[0x54], brek: &[0xF0, 0x54] },
    ],
    // BX_KEY_BACKSLASH
    [
        Scancode { make: &[0x2B], brek: &[0xAB] },
        Scancode { make: &[0x5D], brek: &[0xF0, 0x5D] },
        Scancode { make: &[0x53], brek: &[0xF0, 0x53] },
    ],
    // BX_KEY_RIGHT_BRACKET
    [
        Scancode { make: &[0x1B], brek: &[0x9B] },
        Scancode { make: &[0x5B], brek: &[0xF0, 0x5B] },
        Scancode { make: &[0x5B], brek: &[0xF0, 0x5B] },
    ],
    // BX_KEY_MINUS
    [
        Scancode { make: &[0x0C], brek: &[0x8C] },
        Scancode { make: &[0x4E], brek: &[0xF0, 0x4E] },
        Scancode { make: &[0x4E], brek: &[0xF0, 0x4E] },
    ],
    // BX_KEY_GRAVE
    [
        Scancode { make: &[0x29], brek: &[0xA9] },
        Scancode { make: &[0x0E], brek: &[0xF0, 0x0E] },
        Scancode { make: &[0x0E], brek: &[0xF0, 0x0E] },
    ],
    // BX_KEY_BACKSPACE
    [
        Scancode { make: &[0x0E], brek: &[0x8E] },
        Scancode { make: &[0x66], brek: &[0xF0, 0x66] },
        Scancode { make: &[0x66], brek: &[0xF0, 0x66] },
    ],
    // BX_KEY_ENTER
    [
        Scancode { make: &[0x1C], brek: &[0x9C] },
        Scancode { make: &[0x5A], brek: &[0xF0, 0x5A] },
        Scancode { make: &[0x5A], brek: &[0xF0, 0x5A] },
    ],
    // BX_KEY_TAB
    [
        Scancode { make: &[0x0F], brek: &[0x8F] },
        Scancode { make: &[0x0D], brek: &[0xF0, 0x0D] },
        Scancode { make: &[0x0D], brek: &[0xF0, 0x0D] },
    ],
    // BX_KEY_LEFT_BACKSLASH
    [
        Scancode { make: &[0x56], brek: &[0xD6] },
        Scancode { make: &[0x61], brek: &[0xF0, 0x61] },
        Scancode { make: &[0x13], brek: &[0xF0, 0x13] },
    ],
    // BX_KEY_PRINT
    [
        Scancode { make: &[0xE0, 0x2A, 0xE0, 0x37], brek: &[0xE0, 0xB7, 0xE0, 0xAA] },
        Scancode { make: &[0xE0, 0x12, 0xE0, 0x7C], brek: &[0xE0, 0xF0, 0x7C, 0xE0, 0xF0, 0x12] },
        Scancode { make: &[0x57], brek: &[0xF0, 0x57] },
    ],
    // BX_KEY_SCRL_LOCK
    [
        Scancode { make: &[0x46], brek: &[0xC6] },
        Scancode { make: &[0x7E], brek: &[0xF0, 0x7E] },
        Scancode { make: &[0x5F], brek: &[0xF0, 0x5F] },
    ],
    // BX_KEY_PAUSE
    [
        Scancode { make: &[0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5], brek: &[] },
        Scancode { make: &[0xE1, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77], brek: &[] },
        Scancode { make: &[0x62], brek: &[0xF0, 0x62] },
    ],
    // BX_KEY_INSERT
    [
        Scancode { make: &[0xE0, 0x52], brek: &[0xE0, 0xD2] },
        Scancode { make: &[0xE0, 0x70], brek: &[0xE0, 0xF0, 0x70] },
        Scancode { make: &[0x67], brek: &[0xF0, 0x67] },
    ],
    // BX_KEY_DELETE
    [
        Scancode { make: &[0xE0, 0x53], brek: &[0xE0, 0xD3] },
        Scancode { make: &[0xE0, 0x71], brek: &[0xE0, 0xF0, 0x71] },
        Scancode { make: &[0x64], brek: &[0xF0, 0x64] },
    ],
    // BX_KEY_HOME
    [
        Scancode { make: &[0xE0, 0x47], brek: &[0xE0, 0xC7] },
        Scancode { make: &[0xE0, 0x6C], brek: &[0xE0, 0xF0, 0x6C] },
        Scancode { make: &[0x6E], brek: &[0xF0, 0x6E] },
    ],
    // BX_KEY_END
    [
        Scancode { make: &[0xE0, 0x4F], brek: &[0xE0, 0xCF] },
        Scancode { make: &[0xE0, 0x69], brek: &[0xE0, 0xF0, 0x69] },
        Scancode { make: &[0x65], brek: &[0xF0, 0x65] },
    ],
    // BX_KEY_PAGE_UP
    [
        Scancode { make: &[0xE0, 0x49], brek: &[0xE0, 0xC9] },
        Scancode { make: &[0xE0, 0x7D], brek: &[0xE0, 0xF0, 0x7D] },
        Scancode { make: &[0x6F], brek: &[0xF0, 0x6F] },
    ],
    // BX_KEY_PAGE_DOWN
    [
        Scancode { make: &[0xE0, 0x51], brek: &[0xE0, 0xD1] },
        Scancode { make: &[0xE0, 0x7A], brek: &[0xE0, 0xF0, 0x7A] },
        Scancode { make: &[0x6D], brek: &[0xF0, 0x6D] },
    ],
    // BX_KEY_KP_ADD
    [
        Scancode { make: &[0x4E], brek: &[0xCE] },
        Scancode { make: &[0x79], brek: &[0xF0, 0x79] },
        Scancode { make: &[0x7C], brek: &[0xF0, 0x7C] },
    ],
    // BX_KEY_KP_SUBTRACT
    [
        Scancode { make: &[0x4A], brek: &[0xCA] },
        Scancode { make: &[0x7B], brek: &[0xF0, 0x7B] },
        Scancode { make: &[0x84], brek: &[0xF0, 0x84] },
    ],
    // BX_KEY_KP_END
    [
        Scancode { make: &[0x4F], brek: &[0xCF] },
        Scancode { make: &[0x69], brek: &[0xF0, 0x69] },
        Scancode { make: &[0x69], brek: &[0xF0, 0x69] },
    ],
    // BX_KEY_KP_DOWN
    [
        Scancode { make: &[0x50], brek: &[0xD0] },
        Scancode { make: &[0x72], brek: &[0xF0, 0x72] },
        Scancode { make: &[0x72], brek: &[0xF0, 0x72] },
    ],
    // BX_KEY_KP_PAGE_DOWN
    [
        Scancode { make: &[0x51], brek: &[0xD1] },
        Scancode { make: &[0x7A], brek: &[0xF0, 0x7A] },
        Scancode { make: &[0x7A], brek: &[0xF0, 0x7A] },
    ],
    // BX_KEY_KP_LEFT
    [
        Scancode { make: &[0x4B], brek: &[0xCB] },
        Scancode { make: &[0x6B], brek: &[0xF0, 0x6B] },
        Scancode { make: &[0x6B], brek: &[0xF0, 0x6B] },
    ],
    // BX_KEY_KP_RIGHT
    [
        Scancode { make: &[0x4D], brek: &[0xCD] },
        Scancode { make: &[0x74], brek: &[0xF0, 0x74] },
        Scancode { make: &[0x74], brek: &[0xF0, 0x74] },
    ],
    // BX_KEY_KP_HOME
    [
        Scancode { make: &[0x47], brek: &[0xC7] },
        Scancode { make: &[0x6C], brek: &[0xF0, 0x6C] },
        Scancode { make: &[0x6C], brek: &[0xF0, 0x6C] },
    ],
    // BX_KEY_KP_UP
    [
        Scancode { make: &[0x48], brek: &[0xC8] },
        Scancode { make: &[0x75], brek: &[0xF0, 0x75] },
        Scancode { make: &[0x75], brek: &[0xF0, 0x75] },
    ],
    // BX_KEY_KP_PAGE_UP
    [
        Scancode { make: &[0x49], brek: &[0xC9] },
        Scancode { make: &[0x7D], brek: &[0xF0, 0x7D] },
        Scancode { make: &[0x7D], brek: &[0xF0, 0x7D] },
    ],
    // BX_KEY_KP_INSERT
    [
        Scancode { make: &[0x52], brek: &[0xD2] },
        Scancode { make: &[0x70], brek: &[0xF0, 0x70] },
        Scancode { make: &[0x70], brek: &[0xF0, 0x70] },
    ],
    // BX_KEY_KP_DELETE
    [
        Scancode { make: &[0x53], brek: &[0xD3] },
        Scancode { make: &[0x71], brek: &[0xF0, 0x71] },
        Scancode { make: &[0x71], brek: &[0xF0, 0x71] },
    ],
    // BX_KEY_KP_5
    [
        Scancode { make: &[0x4C], brek: &[0xCC] },
        Scancode { make: &[0x73], brek: &[0xF0, 0x73] },
        Scancode { make: &[0x73], brek: &[0xF0, 0x73] },
    ],
    // BX_KEY_UP
    [
        Scancode { make: &[0xE0, 0x48], brek: &[0xE0, 0xC8] },
        Scancode { make: &[0xE0, 0x75], brek: &[0xE0, 0xF0, 0x75] },
        Scancode { make: &[0x63], brek: &[0xF0, 0x63] },
    ],
    // BX_KEY_DOWN
    [
        Scancode { make: &[0xE0, 0x50], brek: &[0xE0, 0xD0] },
        Scancode { make: &[0xE0, 0x72], brek: &[0xE0, 0xF0, 0x72] },
        Scancode { make: &[0x60], brek: &[0xF0, 0x60] },
    ],
    // BX_KEY_LEFT
    [
        Scancode { make: &[0xE0, 0x4B], brek: &[0xE0, 0xCB] },
        Scancode { make: &[0xE0, 0x6B], brek: &[0xE0, 0xF0, 0x6B] },
        Scancode { make: &[0x61], brek: &[0xF0, 0x61] },
    ],
    // BX_KEY_RIGHT
    [
        Scancode { make: &[0xE0, 0x4D], brek: &[0xE0, 0xCD] },
        Scancode { make: &[0xE0, 0x74], brek: &[0xE0, 0xF0, 0x74] },
        Scancode { make: &[0x6A], brek: &[0xF0, 0x6A] },
    ],
    // BX_KEY_KP_ENTER
    [
        Scancode { make: &[0xE0, 0x1C], brek: &[0xE0, 0x9C] },
        Scancode { make: &[0xE0, 0x5A], brek: &[0xE0, 0xF0, 0x5A] },
        Scancode { make: &[0x79], brek: &[0xF0, 0x79] },
    ],
    // BX_KEY_KP_MULTIPLY
    [
        Scancode { make: &[0x37], brek: &[0xB7] },
        Scancode { make: &[0x7C], brek: &[0xF0, 0x7C] },
        Scancode { make: &[0x7E], brek: &[0xF0, 0x7E] },
    ],
    // BX_KEY_KP_DIVIDE
    [
        Scancode { make: &[0xE0, 0x35], brek: &[0xE0, 0xB5] },
        Scancode { make: &[0xE0, 0x4A], brek: &[0xE0, 0xF0, 0x4A] },
        Scancode { make: &[0x77], brek: &[0xF0, 0x77] },
    ],
    // BX_KEY_WIN_L
    [
        Scancode { make: &[0xE0, 0x5B], brek: &[0xE0, 0xDB] },
        Scancode { make: &[0xE0, 0x1F], brek: &[0xE0, 0xF0, 0x1F] },
        Scancode { make: &[0x8B], brek: &[0xF0, 0x8B] },
    ],
    // BX_KEY_WIN_R
    [
        Scancode { make: &[0xE0, 0x5C], brek: &[0xE0, 0xDC] },
        Scancode { make: &[0xE0, 0x27], brek: &[0xE0, 0xF0, 0x27] },
        Scancode { make: &[0x8C], brek: &[0xF0, 0x8C] },
    ],
    // BX_KEY_MENU
    [
        Scancode { make: &[0xE0, 0x5D], brek: &[0xE0, 0xDD] },
        Scancode { make: &[0xE0, 0x2F], brek: &[0xE0, 0xF0, 0x2F] },
        Scancode { make: &[0x8D], brek: &[0xF0, 0x8D] },
    ],
    // BX_KEY_ALT_SYSREQ
    [
        Scancode { make: &[0x54], brek: &[0xD4] },
        Scancode { make: &[0x84], brek: &[0xF0, 0x84] },
        Scancode { make: &[0x57], brek: &[0xF0, 0x57] },
    ],
    // BX_KEY_CTRL_BREAK
    [
        Scancode { make: &[0xE0, 0x46], brek: &[0xE0, 0xC6] },
        Scancode { make: &[0xE0, 0x7E], brek: &[0xE0, 0xF0, 0x7E] },
        Scancode { make: &[0x62], brek: &[0xF0, 0x62] },
    ],
    // BX_KEY_INT_BACK
    [
        Scancode { make: &[0xE0, 0x6A], brek: &[0xE0, 0xEA] },
        Scancode { make: &[0xE0, 0x38], brek: &[0xE0, 0xF0, 0x38] },
        Scancode { make: &[0x38], brek: &[0xF0, 0x38] },
    ],
    // BX_KEY_INT_FORWARD
    [
        Scancode { make: &[0xE0, 0x69], brek: &[0xE0, 0xE9] },
        Scancode { make: &[0xE0, 0x30], brek: &[0xE0, 0xF0, 0x30] },
        Scancode { make: &[0x30], brek: &[0xF0, 0x30] },
    ],
    // BX_KEY_INT_STOP
    [
        Scancode { make: &[0xE0, 0x68], brek: &[0xE0, 0xE8] },
        Scancode { make: &[0xE0, 0x28], brek: &[0xE0, 0xF0, 0x28] },
        Scancode { make: &[0x28], brek: &[0xF0, 0x28] },
    ],
    // BX_KEY_INT_MAIL
    [
        Scancode { make: &[0xE0, 0x6C], brek: &[0xE0, 0xEC] },
        Scancode { make: &[0xE0, 0x48], brek: &[0xE0, 0xF0, 0x48] },
        Scancode { make: &[0x48], brek: &[0xF0, 0x48] },
    ],
    // BX_KEY_INT_SEARCH
    [
        Scancode { make: &[0xE0, 0x65], brek: &[0xE0, 0xE5] },
        Scancode { make: &[0xE0, 0x10], brek: &[0xE0, 0xF0, 0x10] },
        Scancode { make: &[0x10], brek: &[0xF0, 0x10] },
    ],
    // BX_KEY_INT_FAV
    [
        Scancode { make: &[0xE0, 0x66], brek: &[0xE0, 0xE6] },
        Scancode { make: &[0xE0, 0x18], brek: &[0xE0, 0xF0, 0x18] },
        Scancode { make: &[0x18], brek: &[0xF0, 0x18] },
    ],
    // BX_KEY_INT_HOME
    [
        Scancode { make: &[0xE0, 0x32], brek: &[0xE0, 0xB2] },
        Scancode { make: &[0xE0, 0x3A], brek: &[0xE0, 0xF0, 0x3A] },
        Scancode { make: &[0x97], brek: &[0xF0, 0x97] },
    ],
    // BX_KEY_POWER_MYCOMP
    [
        Scancode { make: &[0xE0, 0x6B], brek: &[0xE0, 0xEB] },
        Scancode { make: &[0xE0, 0x40], brek: &[0xE0, 0xF0, 0x40] },
        Scancode { make: &[0x40], brek: &[0xF0, 0x40] },
    ],
    // BX_KEY_POWER_CALC
    [
        Scancode { make: &[0xE0, 0x21], brek: &[0xE0, 0xA1] },
        Scancode { make: &[0xE0, 0x2B], brek: &[0xE0, 0xF0, 0x2B] },
        Scancode { make: &[0x99], brek: &[0xF0, 0x99] },
    ],
    // BX_KEY_POWER_SLEEP
    [
        Scancode { make: &[0xE0, 0x5F], brek: &[0xE0, 0xDF] },
        Scancode { make: &[0xE0, 0x3F], brek: &[0xE0, 0xF0, 0x3F] },
        Scancode { make: &[0x7F], brek: &[0xF0, 0x7F] },
    ],
    // BX_KEY_POWER_POWER
    [
        Scancode { make: &[0xE0, 0x5E], brek: &[0xE0, 0xDE] },
        Scancode { make: &[0xE0, 0x37], brek: &[0xE0, 0xF0, 0x37] },
        Scancode { make: &[], brek: &[] },
    ],
    // BX_KEY_POWER_WAKE
    [
        Scancode { make: &[0xE0, 0x63], brek: &[0xE0, 0xE3] },
        Scancode { make: &[0xE0, 0x5E], brek: &[0xE0, 0xF0, 0x5E] },
        Scancode { make: &[], brek: &[] },
    ],
];
