//! Reading the screen.
//!
//! Automation scrapes a guest's display the way a person reads it: as a grid
//! of characters with a cursor somewhere in it. [`DisplaySource`] is what any
//! adapter has to answer to be read that way, and [`Display`] is the transient
//! handle a machine hands out.
//!
//! Doctrine: R0 (named `Resolution`/`TextPos`/`RowChars`, no tuples and no
//! anonymous iterators), R2 (a graphics mode has no text grid, so `text()`
//! is `Option` rather than a flag plus an accessor that lies).

use crate::cpu::instrumentation::Instrumentation;

use super::{Emulator, SliceEngine};

/// The display vocabulary, re-exported deliberately rather than as a shim.
///
/// A display model lives in `rusty_box_devices`, but [`Emulator::display`]
/// *returns* one of its types, so the signature would otherwise oblige every
/// caller to add a dependency on that crate just to name what this one already
/// handed them. A facade is the crate keeping its public API self-contained.
///
/// This is the only re-export of its kind. In-crate code says
/// `rusty_box_devices::…` outright, so a reader of a device file can see that
/// it is reaching across a boundary.
pub use rusty_box_devices::display::{
    card::{StdVga, VgaCard},
    DisplaySource, Resolution, TextGrid, TextPos,
};

/// A machine's display, borrowed for as long as the handle lives.
pub struct Display<'m, D: DisplaySource> {
    source: &'m mut D,
}

impl<'m, D: DisplaySource> Display<'m, D> {
    pub(crate) fn new(source: &'m mut D) -> Self {
        Self { source }
    }

    /// The displayed picture size in pixels.
    pub fn resolution(&self) -> Resolution {
        self.source.resolution()
    }

    /// The screen as characters, or `None` in a graphics mode.
    ///
    /// Consumes the handle so the view borrows the machine directly rather
    /// than the handle: `machine.display().text()` is one expression, not two
    /// statements with a binding in between.
    pub fn text(self) -> Option<TextView<'m, D>> {
        let grid = self.source.text_grid()?;
        Some(TextView {
            source: self.source,
            grid,
        })
    }
}

/// The character grid on screen, and the ways automation reads it.
///
/// A row is what the guest sees as a line; matches never span a row boundary,
/// because on a real screen they do not either.
pub struct TextView<'d, D: DisplaySource> {
    source: &'d D,
    grid: TextGrid,
}

impl<'d, D: DisplaySource> TextView<'d, D> {
    pub fn rows(&self) -> usize {
        self.grid.rows
    }

    pub fn cols(&self) -> usize {
        self.grid.cols
    }

    /// Where the hardware cursor sits, or `None` when it is parked off the
    /// displayed page.
    pub fn cursor(&self) -> Option<TextPos> {
        self.grid.cursor
    }

    /// The characters of one row, left to right. A row past the bottom of the
    /// grid yields nothing.
    pub fn row_chars(&self, row: usize) -> RowChars<'_, D> {
        RowChars {
            source: self.source,
            row,
            col: 0,
            cols: if row < self.grid.rows {
                self.grid.cols
            } else {
                0
            },
        }
    }

    /// Whether any single row contains `needle`.
    pub fn contains(&self, needle: &str) -> bool {
        self.find(needle).is_some()
    }

    /// Where `needle` first appears, scanning rows top to bottom and each row
    /// left to right. An empty needle matches the top-left cell of a grid that
    /// has one.
    pub fn find(&self, needle: &str) -> Option<TextPos> {
        for row in 0..self.grid.rows {
            for col in 0..self.grid.cols {
                if self.matches_at(row, col, needle) {
                    return Some(TextPos::new(row, col));
                }
            }
        }
        None
    }

    fn matches_at(&self, row: usize, col: usize, needle: &str) -> bool {
        let mut offset = col;
        for wanted in needle.chars() {
            if offset >= self.grid.cols {
                return false;
            }
            if self.source.text_char(TextPos::new(row, offset)) != wanted {
                return false;
            }
            offset += 1;
        }
        true
    }

    /// The whole screen as one string, rows separated by newlines and trailing
    /// blanks trimmed off each row.
    #[cfg(feature = "alloc")]
    pub fn to_text(&self) -> alloc::string::String {
        let mut text = alloc::string::String::new();
        for row in 0..self.grid.rows {
            let start = text.len();
            text.extend(self.row_chars(row));
            let trimmed = text[start..].trim_end_matches(' ').len();
            text.truncate(start + trimmed);
            text.push('\n');
        }
        text
    }
}

/// The characters of one screen row.
pub struct RowChars<'d, D: DisplaySource> {
    source: &'d D,
    row: usize,
    col: usize,
    cols: usize,
}

impl<D: DisplaySource> Iterator for RowChars<'_, D> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        if self.col >= self.cols {
            return None;
        }
        let ch = self.source.text_char(TextPos::new(self.row, self.col));
        self.col += 1;
        Some(ch)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.cols - self.col;
        (remaining, Some(remaining))
    }
}

impl<D: DisplaySource> ExactSizeIterator for RowChars<'_, D> {}

/// Driving the VGA adapter, and diagnostics that only make sense for it — so
/// neither is part of the role every adapter answers to.
#[cfg(feature = "alloc")]
impl Display<'_, VgaCard<StdVga>> {
    /// Render the current frame into a shared framebuffer.
    ///
    /// The pump a front end that owns the machine on its own thread calls once
    /// per frame; the `BxGui` path does the same thing through a trait object.
    ///
    /// Reports whether the frame changed, which a caller may use to skip the
    /// upload of a framebuffer it already has on screen. Ignoring it is
    /// correct: the same buffer carries text written by other paths, so an
    /// unconditional upload is never wrong, only sometimes wasteful.
    pub fn render_into(
        &mut self,
        framebuffer: &mut crate::gui::shared_display::SharedDisplay,
    ) -> rusty_box_devices::display::sink::Refreshed {
        super::run::render_vga_into(self.source, framebuffer)
    }

    /// Make the next render redraw everything.
    ///
    /// A freshly built machine has drawn nothing, so the first frame has no
    /// previous state to diff against — Bochs vgacore.cc forces the same full
    /// redraw after a mode set.
    pub fn force_update(&mut self) {
        self.source.force_initial_update();
    }

    /// Program standard text mode 3 (80x25 colour).
    ///
    /// Needed for a direct kernel boot, where no video BIOS runs to do it and
    /// the kernel's console driver expects the adapter already in text mode.
    pub fn init_text_mode3(&mut self) {
        self.source.init_text_mode3();
    }

    /// Raise the DISPI capability ceiling and seed the power-on dimensions the
    /// guest's video BIOS reports. Survives guest resets.
    pub fn set_preferred_mode(&mut self, width: u16, height: u16, bpp: u16) {
        self.source.set_preferred_mode(width, height, bpp);
    }

    /// Every row the text aperture holds, displayed or not, one string each.
    ///
    /// [`TextView`] reads the page the CRTC start address points at. This
    /// reads the whole 32 KiB, which is what answers "the screen was cleared,
    /// so where did the guest actually write?".
    pub fn dump_text_aperture(&self) -> alloc::vec::Vec<alloc::string::String> {
        self.source.get_all_text_rows()
    }

    /// A one-paragraph summary of the text aperture: the CRTC start address,
    /// whether the adapter is in a graphics mode, and where printable
    /// characters were found.
    pub fn describe_text_aperture(&self) -> alloc::string::String {
        self.source.scan_all_text_memory()
    }
}

impl<T: Instrumentation, E: SliceEngine<T>> Emulator<T, E> {
    /// Read this machine's screen.
    ///
    /// The handle borrows the machine, so it is taken, used and dropped; a
    /// scrape between two `step_batch` calls sees the frame as it stood when
    /// the batch ended.
    pub fn display(
        &mut self,
    ) -> Display<'_, VgaCard<StdVga>> {
        Display::new(&mut self.device_manager.vga)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid whose cells come from a fixed screen image, so the view can be
    /// exercised without a machine.
    struct FakeScreen {
        rows: usize,
        cols: usize,
        cursor: Option<TextPos>,
        cells: alloc::vec::Vec<char>,
        graphics: bool,
    }

    impl FakeScreen {
        fn new(lines: &[&str]) -> Self {
            let cols = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0);
            let mut cells = alloc::vec![' '; lines.len() * cols];
            for (row, line) in lines.iter().enumerate() {
                for (col, ch) in line.chars().enumerate() {
                    cells[row * cols + col] = ch;
                }
            }
            Self {
                rows: lines.len(),
                cols,
                cursor: None,
                cells,
                graphics: false,
            }
        }
    }

    impl DisplaySource for FakeScreen {
        fn resolution(&self) -> Resolution {
            Resolution::new(self.cols as u32 * 9, self.rows as u32 * 16)
        }

        fn text_grid(&self) -> Option<TextGrid> {
            if self.graphics {
                return None;
            }
            Some(TextGrid {
                rows: self.rows,
                cols: self.cols,
                cursor: self.cursor,
            })
        }

        fn text_char(&self, pos: TextPos) -> char {
            if pos.row >= self.rows || pos.col >= self.cols {
                return ' ';
            }
            self.cells[pos.row * self.cols + pos.col]
        }
    }

    #[test]
    fn a_row_reads_left_to_right_and_stops_at_the_last_column() {
        let mut screen = FakeScreen::new(&["ab", "cd"]);
        let display = Display::new(&mut screen);
        let text = display.text().expect("text mode");

        assert_eq!(text.row_chars(0).collect::<alloc::string::String>(), "ab");
        assert_eq!(text.row_chars(1).count(), 2);
        assert_eq!(text.row_chars(2).count(), 0, "a row past the grid is empty");
    }

    #[test]
    fn a_search_finds_the_first_match_scanning_top_to_bottom() {
        let mut screen = FakeScreen::new(&["  hello ", "hello   "]);
        let display = Display::new(&mut screen);
        let text = display.text().expect("text mode");

        assert!(text.contains("hello"));
        assert_eq!(text.find("hello"), Some(TextPos::new(0, 2)));
    }

    /// A screen is read as lines, so a run of characters that only exists by
    /// joining the end of one row to the start of the next is not on it.
    #[test]
    fn a_match_never_spans_a_row_boundary() {
        let mut screen = FakeScreen::new(&["ab", "cd"]);
        let display = Display::new(&mut screen);
        let text = display.text().expect("text mode");

        assert!(text.contains("ab"));
        assert!(!text.contains("bc"));
    }

    #[test]
    fn the_screen_renders_with_trailing_blanks_trimmed() {
        let mut screen = FakeScreen::new(&["hi   ", "     ", "there"]);
        let display = Display::new(&mut screen);
        let text = display.text().expect("text mode");

        assert_eq!(text.to_text(), "hi\n\nthere\n");
    }

    #[test]
    fn a_graphics_mode_has_no_text_to_read() {
        let mut screen = FakeScreen::new(&["ab"]);
        screen.graphics = true;
        let display = Display::new(&mut screen);

        assert_eq!(display.resolution(), Resolution::new(18, 16));
        assert!(display.text().is_none());
    }

    #[test]
    fn the_cursor_position_is_reported_when_it_is_on_the_page() {
        let mut screen = FakeScreen::new(&["ab", "cd"]);
        screen.cursor = Some(TextPos::new(1, 1));
        let display = Display::new(&mut screen);

        assert_eq!(
            display.text().expect("text mode").cursor(),
            Some(TextPos::new(1, 1))
        );
    }
}
