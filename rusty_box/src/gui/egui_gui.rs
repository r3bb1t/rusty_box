//! egui-based GUI implementation
//!
//! A modern GUI using egui for rendering VGA text mode output.
//! Based on egui framework for cross-platform GUI support.
//!
//! Note: egui requires its own event loop, so this GUI works differently
//! from the terminal GUI. It should be run in a separate thread or
//! integrated with the emulator's main loop using egui's request_repaint mechanism.

#[cfg(feature = "gui-egui")]
mod egui_impl {
    use super::super::gui_trait::{BxGui, DisplayMode, VgaTextModeInfo};
    use alloc::collections::VecDeque;
    use alloc::vec::Vec;
    use alloc::string::String;

    /// egui-based GUI implementation
    /// 
    /// This GUI uses egui which has its own event loop.
    /// For integration with the emulator, you need to call `update_egui()`
    /// from within an egui app, or run this in a separate thread.
    pub struct EguiGui {
        display_mode: DisplayMode,
        screen_width: u32,
        screen_height: u32,
        text_buffer: Vec<u8>,
        cursor_x: u32,
        cursor_y: u32,
        pending_scancodes: VecDeque<u8>,
        egui_ctx: Option<egui::Context>,
    }

    impl EguiGui {
        pub fn new() -> Self {
            Self {
                display_mode: DisplayMode::Sim,
                screen_width: 80,
                screen_height: 25,
                text_buffer: vec![0; 80 * 25 * 2],
                cursor_x: 0,
                cursor_y: 0,
                pending_scancodes: VecDeque::new(),
                egui_ctx: None,
            }
        }

        /// Set the egui context (called from eframe app)
        pub fn set_ctx(&mut self, ctx: egui::Context) {
            self.egui_ctx = Some(ctx);
        }

        /// Update egui (call this from your eframe App::update method)
        pub fn update_egui(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            // Handle keyboard input
            use super::super::keymap::char_to_scancode_sequence;
            
            ctx.input(|i| {
                for event in &i.events {
                    if let egui::Event::Text(text) = event {
                        for ch in text.chars() {
                            let scancodes = char_to_scancode_sequence(ch);
                            for scancode in scancodes {
                                self.pending_scancodes.push_back(scancode);
                            }
                        }
                    }
                }
            });

            // Draw VGA text mode
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("VGA Text Mode Display (80x25)");

                egui::ScrollArea::vertical()
                    .max_height(600.0)
                    .show(ui, |ui| {
                        // Render text buffer - simplified approach using TextEdit for monospace
                        for row in 0..self.screen_height {
                            let mut line_text = String::new();
                            for col in 0..self.screen_width {
                                let idx = ((row * self.screen_width + col) * 2) as usize;
                                if idx + 1 < self.text_buffer.len() {
                                    let ch = self.text_buffer[idx] as char;
                                    let ch_to_display = if ch == '\0' {
                                        ' '
                                    } else if ch.is_ascii() && !ch.is_control() {
                                        ch
                                    } else {
                                        ' '
                                    };
                                    line_text.push(ch_to_display);
                                }
                            }
                            // Highlight cursor line
                            if row == self.cursor_y {
                                ui.colored_label(egui::Color32::YELLOW, &line_text);
                            } else {
                                ui.monospace(&line_text);
                            }
                        }
                    });
            });
        }
    }

    impl Default for EguiGui {
        fn default() -> Self {
            Self::new()
        }
    }

    impl BxGui for EguiGui {
        fn specific_init(&mut self, _argc: i32, _argv: &[String], _header_bar_y: u32) {
            tracing::info!("EguiGUI: Initialized");
        }

        fn text_update(
            &mut self,
            old_text: &[u8],
            new_text: &[u8],
            cursor_x: u32,
            cursor_y: u32,
            _tm_info: &VgaTextModeInfo,
        ) {
            if new_text.len() == self.text_buffer.len() {
                self.text_buffer.copy_from_slice(new_text);
            }
            self.cursor_x = cursor_x;
            self.cursor_y = cursor_y;
        }

        fn graphics_tile_update(&mut self, _tile: &[u8], _x: u32, _y: u32) {
            tracing::trace!("EguiGUI: Graphics tile update (not implemented)");
        }

        fn handle_events(&mut self) {
            // Keyboard input is handled in the egui update loop
            // This method is called from the emulator's event loop
        }

        fn flush(&mut self) {
            // egui handles rendering automatically
        }

        fn clear_screen(&mut self) {
            self.text_buffer.fill(0);
        }

        fn palette_change(&mut self, _index: u8, _red: u8, _green: u8, _blue: u8) -> bool {
            true
        }

        fn dimension_update(
            &mut self,
            x: u32,
            y: u32,
            _fheight: u32,
            _fwidth: u32,
            _bpp: u32,
        ) {
            self.screen_width = x;
            self.screen_height = y;
            self.text_buffer.resize((x * y * 2) as usize, 0);
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
            tracing::info!("EguiGUI: Exiting");
        }

        fn set_display_mode(&mut self, mode: DisplayMode) {
            self.display_mode = mode;
        }

        fn show_ips(&mut self, ips_count: u32) {
            tracing::trace!("EguiGUI: IPS = {}", ips_count);
        }

        fn get_pending_scancodes(&mut self) -> Vec<u8> {
            let mut result = Vec::new();
            while let Some(scancode) = self.pending_scancodes.pop_front() {
                result.push(scancode);
            }
            result
        }
    }

}

#[cfg(feature = "gui-egui")]
pub use egui_impl::EguiGui;

#[cfg(not(feature = "gui-egui"))]
/// Placeholder when gui-egui feature is not enabled
pub struct EguiGui;

#[cfg(not(feature = "gui-egui"))]
impl EguiGui {
    pub fn new() -> Self {
        Self
    }
    
    pub fn run(self) -> Result<(), String> {
        Err("egui GUI requires 'gui-egui' feature to be enabled".to_string())
    }
}

#[cfg(not(feature = "gui-egui"))]
impl Default for EguiGui {
    fn default() -> Self {
        Self::new()
    }
}
