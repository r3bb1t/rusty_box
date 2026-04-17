//! Terminal/Text GUI implementation
//!
//! A simple text-based GUI that displays VGA text mode output to the terminal.
//! Based on the "term" GUI from original Bochs.
//!
//! Uses `crossterm` for cross-platform raw mode and keyboard event handling —
//! no platform-specific cfg(windows) / cfg(unix) code.

use super::gui_trait::{BxGui, DisplayMode, VgaTextModeInfo};
use super::keymap::char_to_scancode_sequence;

/// Terminal GUI implementation
pub struct TermGui {
    display_mode: DisplayMode,
    screen_width: u32,
    screen_height: u32,
    /// Raw VGA text buffer (full aperture, e.g. 0x8000 bytes)
    vga_text: Vec<u8>,
    /// Derived visible window (screen_width * screen_height * 2)
    text_buffer: Vec<u8>,
    cursor_x: u32,
    cursor_y: u32,
    pending_scancodes: alloc::collections::VecDeque<u8>,
    last_tm_info: Option<VgaTextModeInfo>,
    raw_mode_active: bool,
}

impl TermGui {
    pub fn new() -> Self {
        Self {
            display_mode: DisplayMode::Sim,
            screen_width: 80,
            screen_height: 25,
            vga_text: Vec::new(),
            text_buffer: vec![0; 80 * 25 * 2],
            cursor_x: 0,
            cursor_y: 0,
            pending_scancodes: alloc::collections::VecDeque::new(),
            last_tm_info: None,
            raw_mode_active: false,
        }
    }

    fn setup_raw_mode(&mut self) {
        if crossterm::terminal::enable_raw_mode().is_ok() {
            self.raw_mode_active = true;
        } else {
            tracing::warn!("TermGUI: Failed to enable raw mode");
        }
    }

    fn restore_terminal_mode(&mut self) {
        if self.raw_mode_active {
            let _ = crossterm::terminal::disable_raw_mode();
            self.raw_mode_active = false;
        }
    }
}

impl Default for TermGui {
    fn default() -> Self {
        Self::new()
    }
}

impl BxGui for TermGui {
    fn specific_init(&mut self, _argc: i32, _argv: &[String], _header_bar_y: u32) {
        tracing::debug!("TermGUI: Initialized (terminal text mode)");
        print!("\x1b[2J\x1b[H");
        self.setup_raw_mode();
    }

    fn text_update(
        &mut self,
        _old_text: &[u8],
        new_text: &[u8],
        cursor_x: u32,
        cursor_y: u32,
        tm_info: &VgaTextModeInfo,
    ) {
        self.vga_text.clear();
        self.vga_text.extend_from_slice(new_text);
        self.last_tm_info = Some(tm_info.clone());
        self.cursor_x = cursor_x;
        self.cursor_y = cursor_y;
        self.render_text_mode();
    }

    fn graphics_tile_update(&mut self, _tile: &[u8], _x: u32, _y: u32) {
    }

    fn handle_events(&mut self) {
        use crossterm::event::{self, Event};

        while event::poll(std::time::Duration::ZERO).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => {
                    let scancodes = crossterm_key_to_scancodes(key);
                    self.pending_scancodes.extend(scancodes);
                }
                _ => break,
            }
        }
    }

    fn flush(&mut self) {
        self.render_text_mode();
    }

    fn clear_screen(&mut self) {
        print!("\x1b[2J\x1b[H");
        self.text_buffer.fill(0);
    }

    fn palette_change(&mut self, _index: u8, _red: u8, _green: u8, _blue: u8) -> bool {
        true
    }

    fn dimension_update(&mut self, x: u32, y: u32, fheight: u32, fwidth: u32, _bpp: u32) {
        if fheight > 0 && fwidth > 0 {
            self.screen_width = x / fwidth;
            self.screen_height = y / fheight;
        } else {
            self.screen_width = x;
            self.screen_height = y;
        }
        let buf_size = (self.screen_width * self.screen_height * 2) as usize;
        self.text_buffer.resize(buf_size, 0);
        tracing::trace!(
            "TermGUI: Dimensions updated to {}x{} ({}x{} pixels, font {}x{})",
            self.screen_width,
            self.screen_height,
            x,
            y,
            fwidth,
            fheight
        );
    }

    fn create_bitmap(&mut self, _bmap: &[u8], _xdim: u32, _ydim: u32) -> u32 {
        0
    }

    fn headerbar_bitmap(
        &mut self,
        _bmap_id: u32,
        _alignment: u32,
        _callback: Box<dyn Fn()>,
    ) -> u32 {
        0
    }

    fn replace_bitmap(&mut self, _hbar_id: u32, _bmap_id: u32) {}

    fn show_headerbar(&mut self) {}

    fn get_clipboard_text(&mut self) -> Option<Vec<u8>> {
        None
    }

    fn set_clipboard_text(&mut self, _text: &str) -> bool {
        false
    }

    fn mouse_enabled_changed_specific(&mut self, _val: bool) {}

    fn exit(&mut self) {
        self.restore_terminal_mode();
        print!("\x1b[0m\x1b[2J\x1b[H");
        tracing::debug!("TermGUI: Exiting");
    }

    fn set_display_mode(&mut self, mode: DisplayMode) {
        self.display_mode = mode;
        tracing::trace!("TermGUI: Display mode changed to {:?}", mode);
    }

    fn show_ips(&mut self, _ips_count: u32) {
    }

    fn get_pending_scancodes(&mut self) -> Vec<u8> {
        let mut result = Vec::new();
        while let Some(scancode) = self.pending_scancodes.pop_front() {
            result.push(scancode);
        }
        result
    }
}

impl TermGui {
    fn render_text_mode(&mut self) {
        use std::io::Write;

        print!("\x1b[H");

        let cols = self.screen_width as usize;
        let rows = self.screen_height as usize;
        let needed = cols * rows * 2;
        if self.text_buffer.len() != needed {
            self.text_buffer.resize(needed, 0);
        }

        if let Some(ref tm_info) = self.last_tm_info {
            if self.vga_text.len() >= 2 {
                let base = tm_info.start_address as usize;
                let stride = tm_info.line_offset as usize;
                for row in 0..rows {
                    let src_row = base + row * stride;
                    for col in 0..cols {
                        let dst = (row * cols + col) * 2;
                        let src = src_row + col * 2;
                        if src + 1 < self.vga_text.len() {
                            self.text_buffer[dst] = self.vga_text[src];
                            self.text_buffer[dst + 1] = self.vga_text[src + 1];
                        } else {
                            self.text_buffer[dst] = 0;
                            self.text_buffer[dst + 1] = 0x07;
                        }
                    }
                }
            } else {
                self.text_buffer.fill(0);
            }
        } else {
            if self.vga_text.len() >= self.text_buffer.len() {
                let n = self.text_buffer.len();
                self.text_buffer.copy_from_slice(&self.vga_text[..n]);
            } else if !self.vga_text.is_empty() {
                self.text_buffer.fill(0);
                self.text_buffer[..self.vga_text.len()].copy_from_slice(&self.vga_text);
            } else {
                self.text_buffer.fill(0);
            }
        }

        for row in 0..self.screen_height {
            for col in 0..self.screen_width {
                let idx = ((row * self.screen_width + col) * 2) as usize;
                if idx + 1 < self.text_buffer.len() {
                    let ch_byte = self.text_buffer[idx];
                    let attr_byte = self.text_buffer[idx + 1];

                    let fg_color = attr_byte & 0x0F;
                    let bg_color = (attr_byte >> 4) & 0x07;
                    let bright = (fg_color & 0x08) != 0;
                    let blink = (attr_byte & 0x80) != 0;

                    let ansi_fg = match fg_color & 0x07 {
                        0 => 30,
                        1 => 34,
                        2 => 32,
                        3 => 36,
                        4 => 31,
                        5 => 35,
                        6 => 33,
                        7 => 37,
                        _ => 37,
                    };
                    let ansi_bg = match bg_color {
                        0 => 40,
                        1 => 44,
                        2 => 42,
                        3 => 46,
                        4 => 41,
                        5 => 45,
                        6 => 43,
                        7 => 47,
                        _ => 47,
                    };

                    let mut ansi_seq = String::new();
                    if bright {
                        ansi_seq.push_str("\x1b[1m");
                    }
                    if blink {
                        ansi_seq.push_str("\x1b[5m");
                    }
                    ansi_seq.push_str(&format!("\x1b[{};{}m", ansi_fg, ansi_bg));
                    print!("{}", ansi_seq);

                    let ch_to_print = if ch_byte == 0 {
                        ' '
                    } else if ch_byte.is_ascii() && !ch_byte.is_ascii_control() {
                        ch_byte as char
                    } else {
                        ' '
                    };
                    print!("{}", ch_to_print);
                }
            }
            println!();
        }

        print!("\x1b[0m");

        if self.cursor_x < self.screen_width && self.cursor_y < self.screen_height {
            print!("\x1b[{};{}H", self.cursor_y + 1, self.cursor_x + 1);
            print!("\x1b[?25h");
        } else {
            print!("\x1b[?25l");
            print!("\x1b[{};{}H", self.screen_height, self.screen_width);
        }

        std::io::stdout().flush().ok();
    }
}

/// Map a crossterm KeyEvent to PS/2 Set 2 scancode sequence.
/// Returns make codes; release events are ignored (terminal only sends make).
fn crossterm_key_to_scancodes(key: crossterm::event::KeyEvent) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};

    // Only process press events (crossterm also reports Release on some platforms)
    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }

    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        // Printable characters — delegate to existing char→scancode mapping
        KeyCode::Char(c) => {
            let c = if shift { c.to_ascii_uppercase() } else { c };
            char_to_scancode_sequence(c).to_vec()
        }
        // Special keys (PS/2 Set 2 make codes)
        KeyCode::Enter => vec![0x5A, 0xF0, 0x5A],
        KeyCode::Backspace => vec![0x66, 0xF0, 0x66],
        KeyCode::Tab => vec![0x0D, 0xF0, 0x0D],
        KeyCode::Esc => vec![0x76, 0xF0, 0x76],
        KeyCode::Delete => vec![0xE0, 0x71, 0xE0, 0xF0, 0x71],
        KeyCode::Insert => vec![0xE0, 0x70, 0xE0, 0xF0, 0x70],
        KeyCode::Home => vec![0xE0, 0x6C, 0xE0, 0xF0, 0x6C],
        KeyCode::End => vec![0xE0, 0x69, 0xE0, 0xF0, 0x69],
        KeyCode::PageUp => vec![0xE0, 0x7D, 0xE0, 0xF0, 0x7D],
        KeyCode::PageDown => vec![0xE0, 0x7A, 0xE0, 0xF0, 0x7A],
        KeyCode::Up => vec![0xE0, 0x75, 0xE0, 0xF0, 0x75],
        KeyCode::Down => vec![0xE0, 0x72, 0xE0, 0xF0, 0x72],
        KeyCode::Left => vec![0xE0, 0x6B, 0xE0, 0xF0, 0x6B],
        KeyCode::Right => vec![0xE0, 0x74, 0xE0, 0xF0, 0x74],
        KeyCode::F(1) => vec![0x05, 0xF0, 0x05],
        KeyCode::F(2) => vec![0x06, 0xF0, 0x06],
        KeyCode::F(3) => vec![0x04, 0xF0, 0x04],
        KeyCode::F(4) => vec![0x0C, 0xF0, 0x0C],
        KeyCode::F(5) => vec![0x03, 0xF0, 0x03],
        KeyCode::F(6) => vec![0x0B, 0xF0, 0x0B],
        KeyCode::F(7) => vec![0x83, 0xF0, 0x83],
        KeyCode::F(8) => vec![0x0A, 0xF0, 0x0A],
        KeyCode::F(9) => vec![0x01, 0xF0, 0x01],
        KeyCode::F(10) => vec![0x09, 0xF0, 0x09],
        KeyCode::F(11) => vec![0x78, 0xF0, 0x78],
        KeyCode::F(12) => vec![0x07, 0xF0, 0x07],
        _ => Vec::new(),
    }
}
