//! What a display device is, independent of any machine that owns one.
//!
//! The role types below are the vocabulary a *reader* of the screen uses —
//! resolution, a character grid, a cell. The handle that hands them to a
//! caller (`Display<'_, D>`) stays with the machine, because a handle borrows
//! a machine; the role it is generic over is what lives here.

pub mod card;
pub mod ddc;
/// The NV card's register file and RAMIN are sized from guest configuration,
/// so the model needs an allocator where the standard adapter does not.
#[cfg(feature = "alloc")]
pub mod geforce;
pub mod sink;
pub mod vga;

/// The displayed picture size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// A cell of the character grid, counted from the top-left of the displayed
/// page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPos {
    pub row: usize,
    pub col: usize,
}

impl TextPos {
    pub const fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }
}

/// The shape of the character grid currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextGrid {
    pub rows: usize,
    pub cols: usize,
    /// Where the hardware cursor sits, or `None` when it is parked off the
    /// displayed page.
    pub cursor: Option<TextPos>,
}

/// What reading the screen needs from whatever is driving it.
///
/// Deliberately small: an adapter answers what is on screen, not how it got
/// there. Mode programming, banking and the frame pump belong to the device.
pub trait DisplaySource {
    /// The displayed picture size in pixels.
    fn resolution(&self) -> Resolution;

    /// The character grid, or `None` when the adapter is presenting pixels.
    fn text_grid(&self) -> Option<TextGrid>;

    /// The character at `pos`, rendered for reading: a blank cell is a space
    /// and anything unprintable is `?`. Positions outside the grid read as
    /// spaces.
    fn text_char(&self, pos: TextPos) -> char;
}

