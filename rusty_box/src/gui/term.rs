//! Terminal/Text GUI implementation
//!
//! A simple text-based GUI that displays VGA text mode output to the terminal.
//! Based on the "term" GUI from original Bochs.

use super::gui_trait::{BxGui, DisplayMode, VgaTextModeInfo};
use super::keymap::char_to_scancode_sequence;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

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
    #[cfg(unix)]
    original_termios: Option<libc::termios>,
    #[cfg(windows)]
    original_console_mode: Option<u32>,
}

impl TermGui {
    pub fn new() -> Self {
        Self {
            display_mode: DisplayMode::Sim,
            screen_width: 80,
            screen_height: 25,
            vga_text: Vec::new(),
            text_buffer: vec![0; 80 * 25 * 2], // visible window: 80x25, 2 bytes per char
            cursor_x: 0,
            cursor_y: 0,
            pending_scancodes: alloc::collections::VecDeque::new(),
            last_tm_info: None,
            #[cfg(unix)]
            original_termios: None,
            #[cfg(windows)]
            original_console_mode: None,
        }
    }

    /// Setup terminal for raw input mode
    fn setup_raw_mode(&mut self) {
        #[cfg(unix)]
        {
            use std::io::{stdin, Read};
            let stdin_fd = stdin().as_raw_fd();
            unsafe {
                let mut termios: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(stdin_fd, &mut termios) == 0 {
                    self.original_termios = Some(termios);
                    // Disable canonical mode, echo, and line buffering
                    termios.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHONL);
                    termios.c_cc[libc::VMIN] = 0; // Non-blocking reads
                    termios.c_cc[libc::VTIME] = 0;
                    let _ = libc::tcsetattr(stdin_fd, libc::TCSANOW, &termios);
                }
            }
        }

        #[cfg(windows)]
        {
            use winapi::um::consoleapi::{GetConsoleMode, SetConsoleMode};
            use winapi::um::processenv::GetStdHandle;
            use winapi::um::winbase::STD_INPUT_HANDLE;
            use winapi::um::wincon::{
                ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
            };

            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE);
                if handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
                    let mut mode: u32 = 0;
                    if GetConsoleMode(handle, &mut mode) != 0 {
                        self.original_console_mode = Some(mode);
                        // Disable echo, line input, and processed input
                        let new_mode = mode
                            & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT);
                        let _ = SetConsoleMode(handle, new_mode);
                    }
                }
            }
        }
    }

    /// Restore terminal to original mode
    fn restore_terminal_mode(&mut self) {
        #[cfg(unix)]
        {
            use std::io::stdin;
            if let Some(termios) = self.original_termios.take() {
                let stdin_fd = stdin().as_raw_fd();
                unsafe {
                    let _ = libc::tcsetattr(stdin_fd, libc::TCSANOW, &termios);
                }
            }
        }

        #[cfg(windows)]
        {
            use winapi::um::consoleapi::SetConsoleMode;
            use winapi::um::processenv::GetStdHandle;
            use winapi::um::winbase::STD_INPUT_HANDLE;

            if let Some(mode) = self.original_console_mode.take() {
                unsafe {
                    let handle = GetStdHandle(STD_INPUT_HANDLE);
                    if handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
                        let _ = SetConsoleMode(handle, mode);
                    }
                }
            }
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
        tracing::info!("TermGUI: Initialized (terminal text mode)");
        // Clear terminal and set up for text mode
        print!("\x1b[2J\x1b[H"); // Clear screen and move cursor to home
                                 // Setup raw mode for keyboard input
        self.setup_raw_mode();
    }

    fn text_update(
        &mut self,
        old_text: &[u8],
        new_text: &[u8],
        cursor_x: u32,
        cursor_y: u32,
        tm_info: &VgaTextModeInfo,
    ) {
        // Store raw VGA text + tm_info (Bochs passes a full text buffer and uses line_offset).
        self.vga_text.clear();
        self.vga_text.extend_from_slice(new_text);
        self.last_tm_info = Some(tm_info.clone());

        // Update cursor position
        self.cursor_x = cursor_x;
        self.cursor_y = cursor_y;

        // For correctness (Bochs uses line_offset and start_address), render using tm_info.
        // This is fast enough for terminal output and avoids false negatives when buffers
        // are larger than 80x25*2.
        let _ = old_text; // old_text diffing is handled by VGA snapshotting; we render directly.
        self.render_text_mode();
    }

    fn graphics_tile_update(&mut self, _tile: &[u8], _x: u32, _y: u32) {
        // Text mode only for now
        tracing::trace!("TermGUI: Graphics tile update (not implemented)");
    }

    fn handle_events(&mut self) {
        // Read keyboard input non-blocking and queue scancodes
        use std::io::{stdin, Read};

        let mut stdin = stdin();
        let mut buffer = [0u8; 16];

        // Try to read available input (non-blocking)
        loop {
            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;

                // Use poll/select for non-blocking read on Unix
                let fd = stdin.as_raw_fd();
                let mut readfds: libc::fd_set = unsafe { std::mem::zeroed() };
                unsafe {
                    libc::FD_ZERO(&mut readfds);
                    libc::FD_SET(fd, &mut readfds);
                    let mut timeout = libc::timeval {
                        tv_sec: 0,
                        tv_usec: 0, // Immediate return
                    };
                    let result = libc::select(
                        fd + 1,
                        &mut readfds,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        &mut timeout,
                    );
                    if result <= 0 {
                        break; // No data available
                    }
                }
            }

            #[cfg(windows)]
            {
                use winapi::um::consoleapi::GetNumberOfConsoleInputEvents;
                use winapi::um::processenv::GetStdHandle;
                use winapi::um::winbase::STD_INPUT_HANDLE;

                unsafe {
                    let handle = GetStdHandle(STD_INPUT_HANDLE);
                    if handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
                        let mut num_events: u32 = 0;
                        if GetNumberOfConsoleInputEvents(handle, &mut num_events) == 0
                            || num_events == 0
                        {
                            break; // No input available
                        }
                    } else {
                        break;
                    }
                }
            }

            // Read available input
            match stdin.read(&mut buffer) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    // Process each byte as a character
                    for &byte in &buffer[..n] {
                        if let Some(ch) = char::from_u32(byte as u32) {
                            // Convert character to scancode sequence
                            let scancodes = char_to_scancode_sequence(ch);
                            for scancode in scancodes {
                                self.pending_scancodes.push_back(scancode);
                            }
                        }
                    }
                }
                Err(_) => break, // Error or no data
            }
        }
    }

    fn flush(&mut self) {
        // Render the text buffer to terminal
        // This is called periodically, but we also render on text_update if changed
        self.render_text_mode();
    }

    fn clear_screen(&mut self) {
        print!("\x1b[2J\x1b[H"); // Clear screen and move cursor to home
        self.text_buffer.fill(0);
    }

    fn palette_change(&mut self, _index: u8, _red: u8, _green: u8, _blue: u8) -> bool {
        // Text mode doesn't use palette
        true
    }

    fn dimension_update(&mut self, x: u32, y: u32, fheight: u32, fwidth: u32, _bpp: u32) {
        // Matching Bochs term.cc dimension_update():
        // text_cols = x / fwidth;  text_rows = y / fheight;
        if fheight > 0 && fwidth > 0 {
            self.screen_width = x / fwidth;
            self.screen_height = y / fheight;
        } else {
            // Graphics mode or invalid — keep current values
            self.screen_width = x;
            self.screen_height = y;
        }
        let buf_size = (self.screen_width * self.screen_height * 2) as usize;
        self.text_buffer.resize(buf_size, 0);
        tracing::debug!(
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
        // Not supported in text mode
        0
    }

    fn headerbar_bitmap(
        &mut self,
        _bmap_id: u32,
        _alignment: u32,
        _callback: Box<dyn Fn()>,
    ) -> u32 {
        // Not supported in text mode
        0
    }

    fn replace_bitmap(&mut self, _hbar_id: u32, _bmap_id: u32) {
        // No-op
    }

    fn show_headerbar(&mut self) {
        // No-op for text mode
    }

    fn get_clipboard_text(&mut self) -> Option<Vec<u8>> {
        None
    }

    fn set_clipboard_text(&mut self, _text: &str) -> bool {
        false
    }

    fn mouse_enabled_changed_specific(&mut self, _val: bool) {
        // No-op
    }

    fn exit(&mut self) {
        // Restore terminal mode
        self.restore_terminal_mode();
        // Restore terminal
        print!("\x1b[0m\x1b[2J\x1b[H"); // Reset colors, clear screen, home cursor
        tracing::info!("TermGUI: Exiting");
    }

    fn set_display_mode(&mut self, mode: DisplayMode) {
        self.display_mode = mode;
        tracing::debug!("TermGUI: Display mode changed to {:?}", mode);
    }

    fn show_ips(&mut self, ips_count: u32) {
        // Show IPS in status line (if we had one)
        tracing::trace!("TermGUI: IPS = {}", ips_count);
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
    /// Render text mode buffer to terminal
    /// Matching term.cc:551-608 - byte-by-byte comparison and rendering
    fn render_text_mode(&mut self) {
        use std::io::Write;

        // Move cursor to top-left to start drawing (similar to curses move(0,0))
        print!("\x1b[H");

        // Render each character (matching term.cc:567-592), but compute per-line
        // offsets using tm_info->line_offset like Bochs when available.
        let cols = self.screen_width as usize;
        let rows = self.screen_height as usize;
        let needed = cols * rows * 2;
        if self.text_buffer.len() != needed {
            self.text_buffer.resize(needed, 0);
        }

        if let Some(ref tm_info) = self.last_tm_info {
            // Build visible window buffer from raw VGA text using Bochs logic:
            // new_text starts at start_address and advances by line_offset each row.
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
            // No tm_info yet; best-effort sequential render.
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

        // Render each character from `self.text_buffer`
        for row in 0..self.screen_height {
            for col in 0..self.screen_width {
                let idx = ((row * self.screen_width + col) * 2) as usize;
                if idx + 1 < self.text_buffer.len() {
                    let ch_byte = self.text_buffer[idx];
                    let attr_byte = self.text_buffer[idx + 1];

                    // Extract colors from attribute byte (matching term.cc:474-479)
                    // VGA attribute: bits 0-3 = foreground, bits 4-6 = background, bit 7 = blink
                    let fg_color = attr_byte & 0x0F;
                    let bg_color = (attr_byte >> 4) & 0x07;
                    let bright = (fg_color & 0x08) != 0;
                    let blink = (attr_byte & 0x80) != 0;

                    // Map VGA colors to ANSI (matching term.cc:71-80)
                    let ansi_fg = match fg_color & 0x07 {
                        0 => 30, // Black
                        1 => 34, // Blue
                        2 => 32, // Green
                        3 => 36, // Cyan
                        4 => 31, // Red
                        5 => 35, // Magenta
                        6 => 33, // Yellow
                        7 => 37, // White
                        _ => 37,
                    };

                    let ansi_bg = match bg_color {
                        0 => 40, // Black
                        1 => 44, // Blue
                        2 => 42, // Green
                        3 => 46, // Cyan
                        4 => 41, // Red
                        5 => 45, // Magenta
                        6 => 43, // Yellow
                        7 => 47, // White
                        _ => 47,
                    };

                    // Build ANSI escape sequence
                    let mut ansi_seq = String::new();
                    if bright {
                        ansi_seq.push_str("\x1b[1m"); // Bold for bright
                    }
                    if blink {
                        ansi_seq.push_str("\x1b[5m"); // Blink
                    }
                    ansi_seq.push_str(&format!("\x1b[{};{}m", ansi_fg, ansi_bg));
                    print!("{}", ansi_seq);

                    // Convert character byte to printable character (matching term.cc:481-549)
                    // For now, handle basic ASCII and control characters
                    let ch_to_print = if ch_byte == 0 {
                        ' '
                    } else if ch_byte.is_ascii() && !ch_byte.is_ascii_control() {
                        ch_byte as char
                    } else if ch_byte == 0x0A || ch_byte == 0x0D {
                        // Skip newline/carriage return - handled by row structure
                        ' '
                    } else {
                        // Non-printable or extended - show as space
                        ' '
                    };

                    print!("{}", ch_to_print);
                }
            }
            // Newline after each row
            print!("\n");
        }

        // Reset colors
        print!("\x1b[0m");

        // Position cursor (matching term.cc:594-607)
        // Check cursor visibility based on cursor start/end registers
        if self.cursor_x < self.screen_width && self.cursor_y < self.screen_height {
            // Cursor is within visible area - position it (1-based for ANSI)
            print!("\x1b[{};{}H", self.cursor_y + 1, self.cursor_x + 1);
            // Show cursor (equivalent to curs_set(1) or curs_set(2) in curses)
            print!("\x1b[?25h");
        } else {
            // Cursor is outside visible area - hide it (equivalent to curs_set(0) in curses)
            print!("\x1b[?25l");
            // Move to bottom right to avoid interfering
            print!("\x1b[{};{}H", self.screen_height, self.screen_width);
        }

        // Flush output immediately to ensure it's visible
        let _ = std::io::stdout().flush();
    }
}
