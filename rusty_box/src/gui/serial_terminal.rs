//! The serial console as a terminal.
//!
//! A guest drives a serial console the way it drives any terminal: with
//! VT100/xterm control sequences that clear the screen, move the cursor and
//! set colours (`ESC [ 2 J`, `ESC [ row ; col H`, `ESC [ 31 m`, ...). The
//! guest's bytes go through a parser for those sequences (the `vt100`
//! crate), which keeps the screen they draw and the lines scrolled off its
//! top; the serial panel shows that, as a terminal window would, so firmware
//! menus, progress bars and full-screen programs read as they were drawn.

/// A terminal screen's size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScreenSize {
    rows: u16,
    cols: u16,
}

impl ScreenSize {
    /// The size of `screen`, which `vt100` reports as `(rows, cols)`.
    fn of(screen: &vt100::Screen) -> Self {
        let (rows, cols) = screen.size();
        Self { rows, cols }
    }
}

/// Where a screen's cursor is, in character cells from the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CursorPosition {
    row: u16,
    col: u16,
}

impl CursorPosition {
    /// The cursor of `screen`, which `vt100` reports as `(row, col)`.
    fn of(screen: &vt100::Screen) -> Self {
        let (row, col) = screen.cursor_position();
        Self { row, col }
    }
}

/// A character cell of the drawn text: its line, counting the scrollback
/// above the screen, and its column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextCell {
    pub(crate) line: usize,
    pub(crate) col: usize,
}

/// The lines of the drawn text the screen takes: from line `first`, after
/// the scrollback, `count` of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScreenLines {
    pub(crate) first: usize,
    pub(crate) count: usize,
}

/// The screen before the guest asks for one: a VT100's 24 rows of 80.
const DEFAULT_SIZE: ScreenSize = ScreenSize { rows: 24, cols: 80 };

/// The largest screen a guest's resize request may make. GRUB on an EDK2
/// console asks for 56 rows of 240.
const MAX_SIZE: ScreenSize = ScreenSize { rows: 200, cols: 500 };

/// Lines kept above the screen.
const SCROLLBACK_LINES: usize = 1000;

/// The xterm palette's sixteen base colours (black, red, green, yellow,
/// blue, magenta, cyan, white, then the bright eight).
const BASE_COLOURS: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0xCD, 0x00, 0x00],
    [0x00, 0xCD, 0x00],
    [0xCD, 0xCD, 0x00],
    [0x00, 0x00, 0xEE],
    [0xCD, 0x00, 0xCD],
    [0x00, 0xCD, 0xCD],
    [0xE5, 0xE5, 0xE5],
    [0x7F, 0x7F, 0x7F],
    [0xFF, 0x00, 0x00],
    [0x00, 0xFF, 0x00],
    [0xFF, 0xFF, 0x00],
    [0x5C, 0x5C, 0xFF],
    [0xFF, 0x00, 0xFF],
    [0x00, 0xFF, 0xFF],
    [0xFF, 0xFF, 0xFF],
];

/// The levels of each channel in xterm's 6x6x6 colour cube.
const CUBE_LEVELS: [u8; 6] = [0x00, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];

/// Takes the guest's xterm window resize (`ESC [ 8 ; rows ; cols t`) as
/// xterm does, by resizing the screen, within [`MAX_SIZE`].
struct GuestResize;

impl vt100::Callbacks for GuestResize {
    // `vt100` hands the request over as `(rows, cols)`.
    fn resize(&mut self, screen: &mut vt100::Screen, (rows, cols): (u16, u16)) {
        let wanted = ScreenSize { rows, cols };
        if (1..=MAX_SIZE.rows).contains(&wanted.rows) && (1..=MAX_SIZE.cols).contains(&wanted.cols) {
            screen.set_size(wanted.rows, wanted.cols);
        }
    }
}

/// The colours the panel draws a terminal in where the guest set none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TerminalColours {
    pub(crate) foreground: egui::Color32,
    pub(crate) background: egui::Color32,
}

/// A serial console's screen and scrollback, fed from the log the emulator
/// writes, and the text that draws them.
pub(crate) struct SerialTerminal {
    parser: vt100::Parser<GuestResize>,
    /// How many of the bytes the log was ever given have been fed.
    fed: u64,
    /// Output arrived since [`Self::take_new_output`] last asked.
    new_output: bool,
    /// The text last built, and the font and colours it was built in;
    /// `None` once the screen changes.
    drawn: Option<Drawn>,
}

struct Drawn {
    job: egui::text::LayoutJob,
    font: egui::FontId,
    colours: TerminalColours,
}

impl SerialTerminal {
    pub(crate) fn new() -> Self {
        Self {
            parser: vt100::Parser::new_with_callbacks(
                DEFAULT_SIZE.rows,
                DEFAULT_SIZE.cols,
                SCROLLBACK_LINES,
                GuestResize,
            ),
            fed: 0,
            new_output: false,
            drawn: None,
        }
    }

    /// Whether output arrived since the last call: the panel then brings the
    /// cursor into view, as a terminal window follows its cursor.
    pub(crate) fn take_new_output(&mut self) -> bool {
        core::mem::take(&mut self.new_output)
    }

    /// The lines of the drawn text the screen takes, below the scrollback.
    pub(crate) fn screen_lines(&mut self) -> ScreenLines {
        let screen = self.parser.screen_mut();
        screen.set_scrollback(usize::MAX);
        let first = screen.scrollback();
        screen.set_scrollback(0);
        ScreenLines {
            first,
            count: usize::from(ScreenSize::of(screen).rows),
        }
    }

    /// Where the cursor is in the drawn text, when the guest shows it: its
    /// line (counting the scrollback above the screen) and column.
    pub(crate) fn cursor_cell(&mut self) -> Option<TextCell> {
        let screen = self.parser.screen_mut();
        if screen.hide_cursor() {
            return None;
        }
        screen.set_scrollback(usize::MAX);
        let history = screen.scrollback();
        screen.set_scrollback(0);
        let cursor = CursorPosition::of(screen);
        Some(TextCell {
            line: history.saturating_add(usize::from(cursor.row)),
            col: usize::from(cursor.col),
        })
    }

    /// Feeds what the log gained since the last call. `written` counts every
    /// byte the log was ever given and `log` holds its newest part; when the
    /// log no longer holds all that is new, or was started again, the
    /// terminal starts over from what the log holds.
    pub(crate) fn catch_up(&mut self, log: &str, written: u64) {
        if written == self.fed {
            return;
        }
        let bytes = log.as_bytes();
        let new = written
            .checked_sub(self.fed)
            .and_then(|new| usize::try_from(new).ok())
            .and_then(|new| bytes.len().checked_sub(new))
            .and_then(|start| bytes.get(start..));
        match new {
            Some(new) => self.parser.process(new),
            None => {
                *self = Self::new();
                self.parser.process(bytes);
            }
        }
        self.fed = written;
        self.new_output = true;
        self.drawn = None;
    }

    /// The screen's size.
    #[cfg(test)]
    fn size(&self) -> ScreenSize {
        ScreenSize::of(self.parser.screen())
    }

    /// The scrollback and the screen as text, oldest line first, without
    /// the control sequences that drew them.
    pub(crate) fn text(&mut self) -> String {
        let mut text = String::new();
        self.for_each_line(|screen, row, _cursor| {
            text.push_str(row_text(screen, row).trim_end());
            text.push('\n');
        });
        text
    }

    /// The scrollback and the screen as one block of text, oldest line
    /// first, in `font` and the guest's colours over `colours`, with the
    /// cursor shown where the guest left it visible.
    pub(crate) fn layout(&mut self, font: &egui::FontId, colours: TerminalColours) -> egui::text::LayoutJob {
        if let Some(drawn) = &self.drawn {
            if drawn.font == *font && drawn.colours == colours {
                return drawn.job.clone();
            }
        }
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        self.for_each_line(|screen, row, cursor| append_row(&mut job, screen, row, cursor, font, colours));
        // Lines are separated, not ended: a final newline would lay out one
        // more, empty, row, and the text's height divided by its lines is
        // the row height the panel scrolls by.
        if job.text.ends_with('\n') {
            job.text.pop();
            job.sections.pop();
        }
        self.drawn = Some(Drawn {
            job: job.clone(),
            font: font.clone(),
            colours,
        });
        job
    }

    /// Calls `line` for every scrollback line, oldest first, then for every
    /// screen row, each with the screen positioned to show it as `row` and
    /// the column of the cursor when it is visible on that row.
    fn for_each_line(&mut self, mut line: impl FnMut(&vt100::Screen, u16, Option<u16>)) {
        let screen = self.parser.screen_mut();
        // Scrolled back by `n` lines, the screen's top row is the `n`th line
        // above it; the clamp to the history's length tells how long it is.
        screen.set_scrollback(usize::MAX);
        let history = screen.scrollback();
        for back in (1..=history).rev() {
            screen.set_scrollback(back);
            line(screen, 0, None);
        }
        screen.set_scrollback(0);
        let size = ScreenSize::of(screen);
        let cursor = CursorPosition::of(screen);
        let cursor_shown = !screen.hide_cursor();
        for row in 0..size.rows {
            let cursor_col = (cursor_shown && row == cursor.row).then_some(cursor.col);
            line(screen, row, cursor_col);
        }
    }
}

impl SerialTerminal {
    /// Whether the guest switched the cursor keys to application mode
    /// (`ESC [ ? 1 h`), in which an arrow is sent as `ESC O A` rather than
    /// `ESC [ A`.
    pub(crate) fn application_cursor(&self) -> bool {
        self.parser.screen().application_cursor()
    }
}

/// What a key or a paste typed into the terminal sends to the guest, as
/// xterm sends it; `None` for an event that sends nothing. Ctrl+C and
/// Ctrl+X reach a window as Copy and Cut, never as keys, so they are taken
/// as the control characters they are on a terminal; Ctrl+V pastes the host
/// clipboard, as Windows Terminal does.
pub(crate) fn terminal_input(event: &egui::Event, application_cursor: bool) -> Option<Vec<u8>> {
    match event {
        egui::Event::Text(text) | egui::Event::Paste(text) => Some(text.as_bytes().to_vec()),
        egui::Event::Copy => Some(vec![0x03]),
        egui::Event::Cut => Some(vec![0x18]),
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => key_input(*key, *modifiers, application_cursor),
        _ => None,
    }
}

/// xterm's modifier parameter: 1, plus 1 for Shift, 2 for Alt, 4 for Ctrl;
/// `None` when no modifier is held.
fn modifier_parameter(modifiers: egui::Modifiers) -> Option<u8> {
    let parameter = 1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.ctrl);
    (parameter > 1).then_some(parameter)
}

/// A key that types no text, as xterm sends it.
fn key_input(key: egui::Key, modifiers: egui::Modifiers, application_cursor: bool) -> Option<Vec<u8>> {
    use egui::Key;
    // Cursor keys and Home/End: `ESC [ final`, or `ESC O final` in
    // application mode; with a modifier, `ESC [ 1 ; modifier final`.
    let cursor = |final_byte: u8| match modifier_parameter(modifiers) {
        Some(parameter) => format!("\x1b[1;{parameter}{}", char::from(final_byte)).into_bytes(),
        None if application_cursor => vec![0x1b, b'O', final_byte],
        None => vec![0x1b, b'[', final_byte],
    };
    // The editing keys and F5 up: `ESC [ number ~`, with a modifier
    // `ESC [ number ; modifier ~`.
    let tilde = |number: u8| match modifier_parameter(modifiers) {
        Some(parameter) => format!("\x1b[{number};{parameter}~").into_bytes(),
        None => format!("\x1b[{number}~").into_bytes(),
    };
    // F1 to F4: `ESC O final`, with a modifier `ESC [ 1 ; modifier final`.
    let function = |final_byte: u8| match modifier_parameter(modifiers) {
        Some(parameter) => format!("\x1b[1;{parameter}{}", char::from(final_byte)).into_bytes(),
        None => vec![0x1b, b'O', final_byte],
    };
    let bytes = match key {
        Key::Enter => vec![b'\r'],
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![b'\t'],
        Key::Backspace => vec![0x7F],
        Key::Escape => vec![0x1B],
        Key::ArrowUp => cursor(b'A'),
        Key::ArrowDown => cursor(b'B'),
        Key::ArrowRight => cursor(b'C'),
        Key::ArrowLeft => cursor(b'D'),
        Key::Home => cursor(b'H'),
        Key::End => cursor(b'F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => function(b'P'),
        Key::F2 => function(b'Q'),
        Key::F3 => function(b'R'),
        Key::F4 => function(b'S'),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        _ if modifiers.ctrl => vec![control_character(key)?],
        _ => return None,
    };
    Some(bytes)
}

/// The control character Ctrl and `key` type: Ctrl+A to Ctrl+Z are 1 to 26,
/// Ctrl+Space is NUL, Ctrl+[ is ESC, Ctrl+\ is FS and Ctrl+] is GS.
fn control_character(key: egui::Key) -> Option<u8> {
    use egui::Key;
    let letters = [
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
    if let Some(index) = letters.iter().position(|&letter| letter == key) {
        return u8::try_from(index).ok().map(|index| index + 1);
    }
    match key {
        Key::Space => Some(0x00),
        Key::OpenBracket => Some(0x1B),
        Key::Backslash => Some(0x1C),
        Key::CloseBracket => Some(0x1D),
        _ => None,
    }
}

impl Default for SerialTerminal {
    fn default() -> Self {
        Self::new()
    }
}

/// Row `row` of `screen` as plain text, a space for each empty cell.
fn row_text(screen: &vt100::Screen, row: u16) -> String {
    let mut text = String::new();
    for col in 0..ScreenSize::of(screen).cols {
        match screen.cell(row, col) {
            Some(cell) if cell.is_wide_continuation() => {}
            Some(cell) if cell.has_contents() => text.push_str(cell.contents()),
            _ => text.push(' '),
        }
    }
    text
}

/// Appends row `row` of `screen` to `job` and ends the line: runs of cells
/// drawn alike become one section; the row stops after its last cell that
/// shows anything, and the cursor at `cursor` is drawn in inverse.
fn append_row(
    job: &mut egui::text::LayoutJob,
    screen: &vt100::Screen,
    row: u16,
    cursor: Option<u16>,
    font: &egui::FontId,
    colours: TerminalColours,
) {
    let cols = ScreenSize::of(screen).cols;
    let shown = |col: u16| {
        Some(col) == cursor
            || screen
                .cell(row, col)
                .is_some_and(|cell| cell.has_contents() || cell.bgcolor() != vt100::Color::Default || cell.inverse())
    };
    let end = (0..cols).rev().find(|&col| shown(col)).map_or(0, |col| col.saturating_add(1));
    let mut run = String::new();
    let mut run_format: Option<egui::TextFormat> = None;
    for col in 0..end {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        let format = cell_format(cell, Some(col) == cursor, font, colours);
        if run_format.as_ref() != Some(&format) {
            if let Some(previous) = run_format.take() {
                job.append(&run, 0.0, previous);
                run.clear();
            }
            run_format = Some(format);
        }
        if cell.has_contents() {
            run.push_str(cell.contents());
        } else {
            run.push(' ');
        }
    }
    if let Some(format) = run_format {
        job.append(&run, 0.0, format);
    }
    job.append(
        "\n",
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            ..Default::default()
        },
    );
}

/// How a cell is drawn: its colours, bold brightening the eight base
/// colours as on a VT100 console, inverse (or the cursor) swapping them.
fn cell_format(cell: &vt100::Cell, cursor: bool, font: &egui::FontId, colours: TerminalColours) -> egui::TextFormat {
    let foreground = match (cell.fgcolor(), cell.bold()) {
        (vt100::Color::Idx(index @ 0..=7), true) => palette(index.saturating_add(8)),
        (colour, _) => colour_of(colour).unwrap_or(colours.foreground),
    };
    let background = colour_of(cell.bgcolor());
    let (foreground, background) = if cell.inverse() != cursor {
        (background.unwrap_or(colours.background), Some(foreground))
    } else {
        (foreground, background)
    };
    egui::TextFormat {
        font_id: font.clone(),
        color: foreground,
        background: background.unwrap_or(egui::Color32::TRANSPARENT),
        // A cell's background fills its cell and no more: egui's default
        // 1-point overhang makes each row's background overlap its
        // neighbours', leaving seams where they meet at fractional scales.
        expand_bg: 0.0,
        ..Default::default()
    }
}

/// A guest colour, or `None` for the terminal's default.
fn colour_of(colour: vt100::Color) -> Option<egui::Color32> {
    match colour {
        vt100::Color::Default => None,
        vt100::Color::Idx(index) => Some(palette(index)),
        vt100::Color::Rgb(r, g, b) => Some(egui::Color32::from_rgb(r, g, b)),
    }
}

/// Colour `index` of xterm's 256: the sixteen base colours, the 6x6x6
/// cube, then 24 greys from 8 to 238.
fn palette(index: u8) -> egui::Color32 {
    let rgb = match index {
        0..=15 => BASE_COLOURS.get(usize::from(index)).copied().unwrap_or([0; 3]),
        16..=231 => {
            let cube = index - 16;
            let level = |n: u8| CUBE_LEVELS.get(usize::from(n % 6)).copied().unwrap_or(0);
            [level(cube / 36), level(cube / 6), level(cube)]
        }
        232..=255 => {
            let grey = 8 + (index - 232) * 10;
            [grey; 3]
        }
    };
    egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(bytes: &str) -> SerialTerminal {
        let mut terminal = SerialTerminal::new();
        terminal.catch_up(bytes, bytes.len() as u64);
        terminal
    }

    /// The panel shows what a terminal shows: a clear and a cursor move put
    /// text where the guest put it, not the sequences themselves.
    #[test]
    fn cursor_moves_place_text_and_leave_no_sequences() {
        let mut terminal = fed("junk\x1b[2J\x1b[3;5HHi");
        let text = terminal.text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.get(2).copied(), Some("    Hi"));
        assert!(!text.contains('\x1b'));
        assert!(!text.contains("junk"), "the clear wiped it");
    }

    /// GRUB asks an xterm for EDK2's console size; the screen takes it.
    #[test]
    fn a_guest_resize_request_sizes_the_screen() {
        let mut terminal = fed("\x1b[8;056;240t");
        let grub = ScreenSize { rows: 56, cols: 240 };
        assert_eq!(terminal.size(), grub);
        terminal.catch_up("\x1b[8;056;240t\x1b[8;9999;9999t", 30);
        assert_eq!(terminal.size(), grub, "a size past the limit is refused");
    }

    /// Only what the log gained is fed; once the log has dropped bytes the
    /// terminal never saw, it starts over from what the log holds.
    #[test]
    fn catching_up_feeds_the_new_bytes_once() {
        let mut terminal = fed("one\r\n");
        terminal.catch_up("one\r\ntwo\r\n", 10);
        assert_eq!(terminal.text().lines().take(2).collect::<Vec<_>>(), ["one", "two"]);

        // The log kept only its last line of thirty bytes written.
        terminal.catch_up("ten\r\n", 30);
        assert_eq!(terminal.text().lines().next(), Some("ten"));
    }

    /// Colour sequences colour the text; bold brightens a base colour.
    #[test]
    fn colours_reach_the_drawn_text() {
        let mut terminal = fed("\x1b[31mR\x1b[1;34mB");
        let colours = TerminalColours {
            foreground: egui::Color32::WHITE,
            background: egui::Color32::BLACK,
        };
        let job = terminal.layout(&egui::FontId::monospace(11.0), colours);
        assert!(job.text.starts_with("RB"));
        // One section per run of cells drawn alike: "R", then "B".
        let colours: Vec<egui::Color32> = job.sections.iter().map(|section| section.format.color).collect();
        assert_eq!(colours.first(), Some(&palette(1)));
        assert_eq!(colours.get(1), Some(&palette(12)));
    }

    /// Lines scrolled off the screen's top stay above it, oldest first.
    #[test]
    fn scrolled_lines_stay_above_the_screen() {
        let lines: String = (0..30).map(|n| format!("line {n}\r\n")).collect();
        let mut terminal = fed(&lines);
        let text = terminal.text();
        assert_eq!(text.lines().next(), Some("line 0"));
        assert!(text.contains("line 29"));
    }

    /// The panel follows the cursor: it is reported in drawn-text lines,
    /// counting the scrollback, and new output is reported once.
    #[test]
    fn the_cursor_is_reported_where_the_text_puts_it() {
        let lines: String = (0..30).map(|n| format!("line {n}\r\n")).collect();
        let mut terminal = fed(&format!("{lines}ab"));
        assert!(terminal.take_new_output());
        assert!(!terminal.take_new_output(), "reported once");
        // 31 lines on a 24-row screen: 7 above it, the cursor on row 23.
        assert_eq!(terminal.cursor_cell(), Some(TextCell { line: 30, col: 2 }));
        terminal.catch_up(&format!("{lines}ab\x1b[?25l"), (lines.len() + 8) as u64);
        assert_eq!(terminal.cursor_cell(), None, "a hidden cursor is not followed");
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// Keys reach the guest as xterm sends them.
    #[test]
    fn keys_are_sent_as_xterm_sends_them() {
        let none = egui::Modifiers::NONE;
        let input = |event: egui::Event| terminal_input(&event, false);
        assert_eq!(input(egui::Event::Text("ls".to_owned())), Some(b"ls".to_vec()));
        assert_eq!(input(key(egui::Key::Enter, none)), Some(b"\r".to_vec()));
        assert_eq!(input(key(egui::Key::Backspace, none)), Some(vec![0x7F]));
        assert_eq!(input(key(egui::Key::ArrowUp, none)), Some(b"\x1b[A".to_vec()));
        assert_eq!(input(key(egui::Key::ArrowUp, egui::Modifiers::CTRL)), Some(b"\x1b[1;5A".to_vec()));
        assert_eq!(input(key(egui::Key::Delete, none)), Some(b"\x1b[3~".to_vec()));
        assert_eq!(input(key(egui::Key::F1, none)), Some(b"\x1bOP".to_vec()));
        assert_eq!(input(key(egui::Key::F5, none)), Some(b"\x1b[15~".to_vec()));
        assert_eq!(input(key(egui::Key::D, egui::Modifiers::CTRL)), Some(vec![0x04]));
        assert_eq!(input(egui::Event::Copy), Some(vec![0x03]), "Ctrl+C interrupts");
        assert_eq!(input(egui::Event::Paste("a\nb".to_owned())), Some(b"a\nb".to_vec()));
        // A letter without Ctrl types text, which arrives as Text.
        assert_eq!(input(key(egui::Key::D, none)), None);
    }

    /// In application cursor mode an arrow is `ESC O`, as `vi` asks for.
    #[test]
    fn application_cursor_mode_changes_the_arrows() {
        let mut terminal = fed("\x1b[?1h");
        assert!(terminal.application_cursor());
        let up = key(egui::Key::ArrowUp, egui::Modifiers::NONE);
        assert_eq!(terminal_input(&up, terminal.application_cursor()), Some(b"\x1bOA".to_vec()));
        terminal.catch_up("\x1b[?1h\x1b[?1l", 10);
        assert_eq!(terminal_input(&up, terminal.application_cursor()), Some(b"\x1b[A".to_vec()));
    }

    #[test]
    fn the_palette_matches_xterm() {
        assert_eq!(palette(196), egui::Color32::from_rgb(0xFF, 0, 0));
        assert_eq!(palette(244), egui::Color32::from_rgb(128, 128, 128));
    }
}
