//! Bridge GUI — connects emulator to an eframe/egui window via shared state.
//!
//! `BridgeGui` implements `BxGui` and communicates with the eframe `RustyBoxApp`
//! through an `Arc<Mutex<SharedDisplay>>`. The emulator thread calls `text_update()`
//! which renders VGA text to pixels in the shared framebuffer. The GUI thread
//! reads the framebuffer for texture upload and pushes keyboard scancodes.

#[cfg(feature = "gui-egui")]
mod bridge_impl {
    use super::super::gui_trait::{BxGui, DisplayMode, VgaTextModeInfo};
    use super::super::shared_display::SharedDisplay;
    use alloc::boxed::Box;
    use alloc::collections::VecDeque;
    use alloc::string::String;
    use alloc::vec::Vec;

    extern crate std;
    use std::sync::{Arc, Mutex};

    /// Bridge GUI that connects the emulator to an eframe/egui window.
    ///
    /// Holds an `Arc<Mutex<SharedDisplay>>` shared with `RustyBoxApp`.
    /// - `text_update()`: renders VGA text → pixels in shared framebuffer
    /// - `handle_events()`: drains scancodes from shared → local queue
    /// - `get_pending_scancodes()`: returns local queue contents
    pub struct BridgeGui {
        shared: Arc<Mutex<SharedDisplay>>,
        local_scancodes: VecDeque<u8>,
        display_mode: DisplayMode,
    }

    impl BridgeGui {
        /// Create a new BridgeGui with the given shared display.
        pub fn new(shared: Arc<Mutex<SharedDisplay>>) -> Self {
            Self {
                shared,
                local_scancodes: VecDeque::new(),
                display_mode: DisplayMode::Sim,
            }
        }
    }

    impl BxGui for BridgeGui {
        fn specific_init(&mut self, _argc: i32, _argv: &[String], _header_bar_y: u32) {
            tracing::info!("BridgeGui: Initialized");
        }

        fn text_update(
            &mut self,
            _old_text: &[u8],
            new_text: &[u8],
            cursor_x: u32,
            cursor_y: u32,
            tm_info: &VgaTextModeInfo,
        ) {
            if let Ok(mut display) = self.shared.lock() {
                display.render_text_to_framebuffer(
                    new_text,
                    cursor_x,
                    cursor_y,
                    tm_info.cs_start,
                    tm_info.cs_end,
                    tm_info.line_graphics,
                    tm_info.start_address as u32,
                    tm_info.line_offset as u32,
                );
            }
        }

        fn graphics_tile_update(&mut self, _tile: &[u8], _x: u32, _y: u32) {
            // Graphics mode not yet supported
        }

        fn handle_events(&mut self) {
            // Drain scancodes from shared display into local queue
            if let Ok(mut display) = self.shared.lock() {
                for scancode in display.pending_scancodes.drain(..) {
                    self.local_scancodes.push_back(scancode);
                }
            }
        }

        fn flush(&mut self) {
            // Rendering happens in text_update; nothing extra needed
        }

        fn clear_screen(&mut self) {
            if let Ok(mut display) = self.shared.lock() {
                display.framebuffer.fill(0);
                display.fb_dirty = true;
            }
        }

        fn palette_change(&mut self, index: u8, red: u8, green: u8, blue: u8) -> bool {
            if let Ok(mut display) = self.shared.lock() {
                if (index as usize) < 16 {
                    display.palette[index as usize] = [red, green, blue];
                }
            }
            true
        }

        fn dimension_update(&mut self, x: u32, y: u32, fheight: u32, fwidth: u32, _bpp: u32) {
            if let Ok(mut display) = self.shared.lock() {
                let cols = if fwidth > 0 { x / fwidth } else { x };
                let rows = if fheight > 0 { y / fheight } else { y };
                display.resize(cols, rows, fwidth, fheight);
            }
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
            if let Ok(mut display) = self.shared.lock() {
                display.emu_running = false;
            }
            tracing::info!("BridgeGui: Exiting");
        }

        fn set_display_mode(&mut self, mode: DisplayMode) {
            self.display_mode = mode;
        }

        fn show_ips(&mut self, ips_count: u32) {
            if let Ok(mut display) = self.shared.lock() {
                display.ips = ips_count;
            }
        }

        fn get_pending_scancodes(&mut self) -> Vec<u8> {
            self.local_scancodes.drain(..).collect()
        }
    }
}

#[cfg(feature = "gui-egui")]
pub use bridge_impl::BridgeGui;

/// Type alias for backward compatibility
#[cfg(feature = "gui-egui")]
pub type EguiGui = BridgeGui;

#[cfg(not(feature = "gui-egui"))]
/// Placeholder when gui-egui feature is not enabled
pub struct EguiGui;

#[cfg(not(feature = "gui-egui"))]
impl EguiGui {
    pub fn new() -> Self {
        Self
    }

    pub fn run(self) -> Result<(), alloc::string::String> {
        Err(alloc::string::String::from(
            "egui GUI requires 'gui-egui' feature to be enabled",
        ))
    }
}

#[cfg(not(feature = "gui-egui"))]
impl Default for EguiGui {
    fn default() -> Self {
        Self::new()
    }
}
