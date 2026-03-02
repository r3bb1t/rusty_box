//! Shared display state between emulator thread and GUI thread.
//!
//! `SharedDisplay` holds the RGBA framebuffer, keyboard scancode queue,
//! and VGA text mode parameters. The emulator thread writes pixels via
//! `render_text_to_framebuffer()`, and the GUI thread reads the framebuffer
//! for texture upload and pushes scancodes for keyboard input.

use super::vga_font::{VGA_DEFAULT_PALETTE_16, VGA_FONT_8X16};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::AtomicBool;

/// Shared state between the emulator and GUI threads.
///
/// Protected by `Arc<Mutex<SharedDisplay>>` in both `BridgeGui` and `RustyBoxApp`.
pub struct SharedDisplay {
    /// RGBA pixel buffer (fb_width * fb_height * 4 bytes)
    pub framebuffer: Vec<u8>,
    /// Framebuffer width in pixels
    pub fb_width: u32,
    /// Framebuffer height in pixels
    pub fb_height: u32,
    /// True when framebuffer has been updated since last GUI read
    pub fb_dirty: bool,
    /// Keyboard scancodes from GUI to emulator (PS/2 set 2)
    pub pending_scancodes: Vec<u8>,
    /// Text mode columns (e.g. 80)
    pub screen_cols: u32,
    /// Text mode rows (e.g. 25)
    pub screen_rows: u32,
    /// Font cell width in pixels (8 or 9)
    pub font_width: u32,
    /// Font cell height in pixels (typically 16)
    pub font_height: u32,
    /// Whether the emulator is still running
    pub emu_running: bool,
    /// Current instructions per second for status display
    pub ips: u32,
    /// Custom palette (index → [R, G, B]), initially standard VGA 16-color
    pub palette: [[u8; 3]; 16],
    /// Set by GUI to request emulator restart; cleared by emulator thread when restart begins
    pub reset_requested: bool,
    /// Atomic flag polled by run_interactive to stop early (e.g. on reset); shared with GUI
    pub stop_flag: Arc<AtomicBool>,
}

impl SharedDisplay {
    /// Create a new SharedDisplay with default 80x25 text mode (720x400 px).
    pub fn new() -> Self {
        let cols = 80u32;
        let rows = 25u32;
        let fw = 9u32; // 9-pixel wide cells (8 + 1 for line graphics)
        let fh = 16u32;
        let w = cols * fw;
        let h = rows * fh;
        Self {
            framebuffer: vec![0u8; (w * h * 4) as usize],
            fb_width: w,
            fb_height: h,
            fb_dirty: false,
            pending_scancodes: Vec::new(),
            screen_cols: cols,
            screen_rows: rows,
            font_width: fw,
            font_height: fh,
            emu_running: true,
            ips: 0,
            palette: VGA_DEFAULT_PALETTE_16,
            reset_requested: false,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Resize the framebuffer for new text mode dimensions.
    pub fn resize(&mut self, cols: u32, rows: u32, font_width: u32, font_height: u32) {
        self.screen_cols = cols;
        self.screen_rows = rows;
        self.font_width = if font_width == 0 { 9 } else { font_width };
        self.font_height = if font_height == 0 { 16 } else { font_height };
        self.fb_width = self.screen_cols * self.font_width;
        self.fb_height = self.screen_rows * self.font_height;
        let size = (self.fb_width * self.fb_height * 4) as usize;
        self.framebuffer.resize(size, 0);
        self.framebuffer.fill(0);
    }

    /// Render VGA text buffer (char+attr pairs) into the RGBA framebuffer.
    ///
    /// Algorithm matches Bochs `draw_char_common()` from gui.cc:1202-1244.
    ///
    /// # Parameters
    /// - `text`: VGA text buffer — 2 bytes per cell (char, attr), row-major
    /// - `cursor_x`, `cursor_y`: cursor position (column, row)
    /// - `cs_start`, `cs_end`: cursor scanline start/end (0..font_height)
    /// - `line_graphics`: if true, chars 0xC0-0xDF duplicate bit 0 to 9th pixel
    /// - `start_address`: CRTC start address (byte offset into text buffer)
    /// - `line_offset`: CRTC line offset (bytes per row in VGA memory)
    pub fn render_text_to_framebuffer(
        &mut self,
        text: &[u8],
        cursor_x: u32,
        cursor_y: u32,
        cs_start: u8,
        cs_end: u8,
        line_graphics: bool,
        start_address: u32,
        line_offset: u32,
    ) {
        let cols = self.screen_cols;
        let rows = self.screen_rows;
        let fw = self.font_width;
        let fh = self.font_height;
        let stride = self.fb_width * 4;
        let palette = self.palette; // Copy for parallel access

        // Helper closure: render a single character cell into framebuffer slice
        let render_cell =
            |fb: &mut [u8], row: u32, col: u32, ch: usize, attr: u8, is_cursor: bool| {
                let fg_idx = (attr & 0x0F) as usize;
                let bg_idx = ((attr >> 4) & 0x07) as usize;
                let fg = if fg_idx < 16 {
                    palette[fg_idx]
                } else {
                    [0xFF, 0xFF, 0xFF]
                };
                let bg = if bg_idx < 16 {
                    palette[bg_idx]
                } else {
                    [0x00, 0x00, 0x00]
                };

                let px = col * fw;
                let py = row * fh;

                for scanline in 0..fh {
                    let font_byte = if (scanline as usize) < 16 {
                        VGA_FONT_8X16[ch][scanline as usize]
                    } else {
                        0
                    };
                    let cursor_invert = is_cursor
                        && cs_start <= cs_end
                        && scanline as u8 >= cs_start
                        && scanline as u8 <= cs_end;

                    for bit in 0..8u32 {
                        let pixel_on = (font_byte >> bit) & 1 != 0;
                        let color = if cursor_invert {
                            if pixel_on { bg } else { fg }
                        } else {
                            if pixel_on { fg } else { bg }
                        };
                        let fb_x = px + bit;
                        let fb_y = py + scanline;
                        let offset = (fb_y * stride + fb_x * 4) as usize;
                        if offset + 3 < fb.len() {
                            fb[offset] = color[0];
                            fb[offset + 1] = color[1];
                            fb[offset + 2] = color[2];
                            fb[offset + 3] = 0xFF;
                        }
                    }

                    if fw >= 9 {
                        let ninth_on = if line_graphics && (0xC0..=0xDF).contains(&ch) {
                            (font_byte >> 7) & 1 != 0
                        } else {
                            false
                        };
                        let color = if cursor_invert {
                            if ninth_on { bg } else { fg }
                        } else {
                            if ninth_on { fg } else { bg }
                        };
                        let fb_x = px + 8;
                        let fb_y = py + scanline;
                        let offset = (fb_y * stride + fb_x * 4) as usize;
                        if offset + 3 < fb.len() {
                            fb[offset] = color[0];
                            fb[offset + 1] = color[1];
                            fb[offset + 2] = color[2];
                            fb[offset + 3] = 0xFF;
                        }
                    }
                }
            };

        {
            let fb = &mut self.framebuffer;
            for row in 0..rows {
                for col in 0..cols {
                    // Use CRTC start_address and line_offset, matching Bochs gui.cc:1311-1314
                    let text_idx = (start_address + row * line_offset + col * 2) as usize;
                    if text_idx + 1 >= text.len() {
                        continue;
                    }
                    let ch = text[text_idx] as usize;
                    let attr = text[text_idx + 1];
                    let is_cursor = col == cursor_x && row == cursor_y;
                    render_cell(fb, row, col, ch, attr, is_cursor);
                }
            }
        }

        self.fb_dirty = true;
    }
}

impl Default for SharedDisplay {
    fn default() -> Self {
        Self::new()
    }
}
