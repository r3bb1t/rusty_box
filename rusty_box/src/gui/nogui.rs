//! No GUI implementation
//!
//! This is a stub implementation that provides no visual output.
//! Useful for headless operation or testing.

use alloc::{boxed::Box, string::String, vec::Vec};

use super::gui_trait::{BxGui, DisplayMode, VgaTextModeInfo};

/// No GUI implementation - all methods are no-ops
pub struct NoGui {
    display_mode: DisplayMode,
}

impl NoGui {
    pub fn new() -> Self {
        Self {
            display_mode: DisplayMode::Sim,
        }
    }
}

impl Default for NoGui {
    fn default() -> Self {
        Self::new()
    }
}

impl BxGui for NoGui {
    fn specific_init(&mut self, _argc: i32, _argv: &[String], _header_bar_y: u32) {
        tracing::debug!("NoGUI: Initialized (no visual output)");
    }

    fn text_update(
        &mut self,
        _old_text: &[u8],
        _new_text: &[u8],
        _cursor_x: u32,
        _cursor_y: u32,
        _tm_info: &VgaTextModeInfo,
    ) {
        // No-op: no visual output
    }

    fn graphics_tile_update(&mut self, _tile: &[u8], _x: u32, _y: u32) {
        // No-op: no visual output
    }

    fn handle_events(&mut self) {
        // No-op: no events to handle
    }

    fn flush(&mut self) {
        // No-op: nothing to flush
    }

    fn clear_screen(&mut self) {
        // No-op: no screen to clear
    }

    fn palette_change(&mut self, _index: u8, _red: u8, _green: u8, _blue: u8) -> bool {
        // No-op: return true to indicate "handled"
        true
    }

    fn dimension_update(&mut self, _x: u32, _y: u32, _fheight: u32, _fwidth: u32, _bpp: u32) {
        // No-op
    }

    fn create_bitmap(&mut self, _bmap: &[u8], _xdim: u32, _ydim: u32) -> u32 {
        // Return dummy ID
        0
    }

    fn headerbar_bitmap(
        &mut self,
        _bmap_id: u32,
        _alignment: u32,
        _callback: Box<dyn Fn()>,
    ) -> u32 {
        // Return dummy ID
        0
    }

    fn replace_bitmap(&mut self, _hbar_id: u32, _bmap_id: u32) {
        // No-op
    }

    fn show_headerbar(&mut self) {
        // No-op
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
        tracing::debug!("NoGUI: Exiting");
    }

    fn set_display_mode(&mut self, mode: DisplayMode) {
        self.display_mode = mode;
        tracing::trace!("NoGUI: Display mode changed to {:?}", mode);
    }

    fn is_headless(&self) -> bool {
        true
    }
}
