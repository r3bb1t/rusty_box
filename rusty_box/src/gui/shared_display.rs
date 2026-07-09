//! Shared display state between emulator thread and GUI thread.
//!
//! `SharedDisplay` holds the RGBA framebuffer, keyboard scancode queue,
//! and VGA text mode parameters. The emulator thread writes pixels via
//! `render_text_to_framebuffer()`, and the GUI thread reads the framebuffer
//! for texture upload and pushes scancodes for keyboard input.

use super::vga_font::{VGA_DEFAULT_PALETTE_16, VGA_FONT_8X16};
use alloc::string::String;
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
    /// Relative PS/2 mouse events from GUI to emulator, drained by the pump.
    pub pending_mouse: Vec<super::host_input::HostMouseEvent>,
    /// Whether the GUI currently captures the mouse for the guest (forwarding
    /// motion/buttons and routing chords past egui). Toggled by the frontend.
    pub mouse_captured: bool,
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
    /// True after the GUI has requested launch but before the emulator reports running
    pub start_pending: bool,
    /// Current instructions per second for status display
    pub ips: u32,
    /// Custom palette (index → [R, G, B]), initially standard VGA 16-color
    pub palette: [[u8; 3]; 16],
    /// Set by GUI to request emulator restart; cleared by emulator thread when restart begins
    pub reset_requested: bool,
    /// Last emulator startup/runtime error reported by the worker thread
    pub runtime_error: Option<String>,
    /// Atomic flag polled by run_interactive to stop early (e.g. on reset); shared with GUI
    pub stop_flag: Arc<AtomicBool>,
    /// Serial console output text (accumulated from serial port TX)
    pub serial_log: String,
    /// ASCII bytes from GUI to inject into serial port RX (for console input)
    pub pending_serial_input: Vec<u8>,
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
            pending_mouse: Vec::new(),
            mouse_captured: false,
            screen_cols: cols,
            screen_rows: rows,
            font_width: fw,
            font_height: fh,
            emu_running: false,
            start_pending: false,
            ips: 0,
            palette: VGA_DEFAULT_PALETTE_16,
            reset_requested: false,
            runtime_error: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
            serial_log: String::new(),
            pending_serial_input: Vec::new(),
        }
    }

    /// Queue a line of ASCII text for serial RX injection.
    pub fn queue_serial_input_line(&mut self, input: &str) {
        self.pending_serial_input
            .extend_from_slice(input.as_bytes());
        self.pending_serial_input.push(b'\n');
    }

    /// Drain pending serial RX bytes.
    pub fn drain_serial_input(&mut self) -> Vec<u8> {
        self.pending_serial_input.drain(..).collect()
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

    /// Resize the framebuffer for packed RGBA graphics dimensions.
    pub fn resize_pixels(&mut self, width: u32, height: u32) {
        self.screen_cols = width;
        self.screen_rows = height;
        self.font_width = 1;
        self.font_height = 1;
        self.fb_width = width;
        self.fb_height = height;
        let size = (width * height * 4) as usize;
        self.framebuffer.resize(size, 0);
        self.framebuffer.fill(0);
        self.fb_dirty = true;
    }

    /// Copy row-major RGBA pixels into the framebuffer, clipping at the edges.
    pub fn blit_rgba_tile(&mut self, x: u32, y: u32, width: u32, height: u32, rgba: &[u8]) {
        if x >= self.fb_width || y >= self.fb_height || width == 0 || height == 0 {
            return;
        }

        let copy_width = width.min(self.fb_width - x);
        let copy_height = height.min(self.fb_height - y);
        let mut copied = false;

        for row in 0..copy_height {
            for col in 0..copy_width {
                let src = ((row * width + col) * 4) as usize;
                if src + 4 > rgba.len() {
                    break;
                }
                let dst = (((y + row) * self.fb_width + x + col) * 4) as usize;
                self.framebuffer[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
                copied = true;
            }
        }

        if copied {
            self.fb_dirty = true;
        }
    }

    /// Render VGA text buffer (char+attr pairs) into the RGBA framebuffer.
    ///
    /// Algorithm matches Bochs `draw_char_common()` from gui.cc.
    ///
    /// # Parameters
    /// - `text`: VGA text buffer — 2 bytes per cell (char, attr), row-major
    /// - `cursor_x`, `cursor_y`: cursor position (column, row)
    /// - `cs_start`, `cs_end`: cursor scanline start/end (0..font_height)
    /// - `line_graphics`: if true, chars 0xC0-0xDF duplicate bit 0 to 9th pixel
    /// - `start_address`: CRTC start address (byte offset into text buffer)
    /// - `line_offset`: CRTC line offset (bytes per row in VGA memory)
    #[allow(clippy::too_many_arguments)]
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
        actl_palette: &[u8; 16],
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
                // ACTL palette indirection (Bochs gui.cc)
                let fg_idx = actl_palette[(attr & 0x0F) as usize] as usize;
                let bg_idx = actl_palette[((attr >> 4) & 0x07) as usize] as usize;
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
                        // Font data (VGA_FONT_8X16 from Bochs bx_vgafont) is LSB-first:
                        // bit 0 = leftmost pixel. Matches Bochs DrawBitmap (rfb.cc).
                        let pixel_on = (font_byte >> bit) & 1 != 0;
                        let color = if cursor_invert {
                            if pixel_on {
                                bg
                            } else {
                                fg
                            }
                        } else {
                            if pixel_on {
                                fg
                            } else {
                                bg
                            }
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
                            if ninth_on {
                                bg
                            } else {
                                fg
                            }
                        } else {
                            if ninth_on {
                                fg
                            } else {
                                bg
                            }
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
            let text_len = text.len();
            for row in 0..rows {
                for col in 0..cols {
                    // Use CRTC start_address and line_offset, matching Bochs gui.cc
                    // Wrap within text buffer (VGA text memory is 32KB, kernel scrolls by
                    // advancing start_address and wraps around)
                    let text_idx =
                        ((start_address + row * line_offset + col * 2) as usize) % text_len;
                    if text_idx + 1 >= text_len {
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

    /// Render a VGA text update using the core VGA metadata type.
    ///
    /// This keeps `VgaTextModeInfo` internals private while allowing external
    /// GUI frontends to implement `BxGui::text_update`.
    pub fn render_vga_text_update(
        &mut self,
        text: &[u8],
        cursor_x: u32,
        cursor_y: u32,
        tm_info: &crate::iodev::vga::VgaTextModeInfo,
    ) {
        self.render_text_to_framebuffer(
            text,
            cursor_x,
            cursor_y,
            tm_info.cs_start,
            tm_info.cs_end,
            tm_info.line_graphics,
            tm_info.start_address as u32,
            tm_info.line_offset as u32,
            &tm_info.actl_palette,
        );
    }
}

impl Default for SharedDisplay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::SharedDisplay;
    use core::sync::atomic::Ordering;

    #[test]
    fn shared_display_starts_stopped() {
        let shared = SharedDisplay::new();

        assert!(!shared.emu_running);
        assert!(!shared.reset_requested);
        assert!(!shared.start_pending);
        assert!(shared.runtime_error.is_none());
        assert!(!shared.stop_flag.load(Ordering::Relaxed));
    }

    #[test]
    fn serial_input_queue_appends_newline_and_drains() {
        let mut shared = SharedDisplay::new();

        shared.queue_serial_input_line("boot");

        assert_eq!(shared.drain_serial_input(), b"boot\n");
        assert!(shared.drain_serial_input().is_empty());
    }

    #[test]
    fn resize_pixels_sets_exact_graphics_dimensions() {
        let mut shared = SharedDisplay::new();

        shared.resize_pixels(3, 2);

        assert_eq!(shared.fb_width, 3);
        assert_eq!(shared.fb_height, 2);
        assert_eq!(shared.framebuffer.len(), 24);
        assert!(shared.fb_dirty);
        assert_eq!(shared.font_width, 1);
        assert_eq!(shared.font_height, 1);
    }

    #[test]
    fn blit_rgba_tile_clips_to_framebuffer() {
        let mut shared = SharedDisplay::new();
        shared.resize_pixels(3, 2);

        shared.blit_rgba_tile(
            2,
            1,
            2,
            2,
            &[
                0xAA, 0xBB, 0xCC, 0xFF, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A,
                0x0B, 0x0C,
            ],
        );

        let pixel = ((1 * shared.fb_width + 2) * 4) as usize;
        assert_eq!(
            &shared.framebuffer[pixel..pixel + 4],
            &[0xAA, 0xBB, 0xCC, 0xFF]
        );
    }

    #[test]
    fn render_vga_text_update_marks_framebuffer_dirty() {
        let mut shared = SharedDisplay::new();
        let mut text = vec![0u8; (shared.screen_cols * shared.screen_rows * 2) as usize];
        text[0] = b'A';
        text[1] = 0x07;
        let tm_info = crate::iodev::vga::VgaTextModeInfo {
            start_address: 0,
            cs_start: 14,
            cs_end: 15,
            line_offset: (shared.screen_cols * 2) as u16,
            line_compare: 0,
            h_panning: 0,
            v_panning: 0,
            line_graphics: false,
            split_hpanning: false,
            blink_flags: 0,
            actl_palette: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        };

        shared.render_vga_text_update(&text, 0, 0, &tm_info);

        assert!(shared.fb_dirty);
    }
}
