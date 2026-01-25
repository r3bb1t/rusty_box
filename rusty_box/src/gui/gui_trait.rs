//! GUI trait definition
//!
//! Based on bx_gui_c class from gui/gui.h
//! This trait defines the interface that all GUI implementations must provide.

use alloc::{boxed::Box, string::String, vec::Vec};

/// Display mode for the GUI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    /// Configuration interface mode
    Config,
    /// Simulation mode
    Sim,
}

/// VGA text mode information
#[derive(Debug, Clone)]
pub struct VgaTextModeInfo {
    pub start_address: u16,
    pub cs_start: u8,
    pub cs_end: u8,
    pub line_offset: u16,
    pub line_compare: u16,
    pub h_panning: u8,
    pub v_panning: u8,
    pub line_graphics: bool,
    pub split_hpanning: bool,
    pub blink_flags: u8,
    pub actl_palette: [u8; 16],
}

/// GUI trait - all GUI implementations must provide these methods
///
/// Based on bx_gui_c class from cpp_orig/bochs/gui/gui.h
pub trait BxGui: Send + Sync {
    /// Initialize the GUI with specific parameters
    fn specific_init(&mut self, argc: i32, argv: &[String], header_bar_y: u32);

    /// Update text mode display
    fn text_update(
        &mut self,
        old_text: &[u8],
        new_text: &[u8],
        cursor_x: u32,
        cursor_y: u32,
        tm_info: &VgaTextModeInfo,
    );

    /// Update a graphics tile
    fn graphics_tile_update(&mut self, tile: &[u8], x: u32, y: u32);

    /// Handle GUI events (keyboard, mouse, etc.)
    fn handle_events(&mut self);

    /// Flush display updates to screen
    fn flush(&mut self);

    /// Clear the screen
    fn clear_screen(&mut self);

    /// Change palette color
    fn palette_change(&mut self, index: u8, red: u8, green: u8, blue: u8) -> bool;

    /// Update display dimensions
    fn dimension_update(
        &mut self,
        x: u32,
        y: u32,
        fheight: u32,
        fwidth: u32,
        bpp: u32,
    );

    /// Create a bitmap
    fn create_bitmap(&mut self, bmap: &[u8], xdim: u32, ydim: u32) -> u32;

    /// Add bitmap to header bar
    fn headerbar_bitmap(&mut self, bmap_id: u32, alignment: u32, callback: Box<dyn Fn()>) -> u32;

    /// Replace bitmap in header bar
    fn replace_bitmap(&mut self, hbar_id: u32, bmap_id: u32);

    /// Show header bar
    fn show_headerbar(&mut self);

    /// Get clipboard text
    fn get_clipboard_text(&mut self) -> Option<Vec<u8>>;

    /// Set clipboard text
    fn set_clipboard_text(&mut self, text: &str) -> bool;

    /// Mouse enabled state changed
    fn mouse_enabled_changed_specific(&mut self, val: bool);

    /// Exit the GUI
    fn exit(&mut self);

    // Optional methods with default implementations

    /// Update drive status buttons
    fn update_drive_status_buttons(&mut self) {
        // Default: no-op
    }

    /// Register a status bar item
    fn register_statusitem(&mut self, text: &str, auto_off: bool) -> i32 {
        // Default: return -1 (not supported)
        -1
    }

    /// Unregister a status bar item
    fn unregister_statusitem(&mut self, id: i32) {
        // Default: no-op
    }

    /// Set status bar item state
    fn statusbar_setitem(&mut self, element: i32, active: bool, w: bool) {
        // Default: no-op
    }

    /// Initialize signal handlers
    fn init_signal_handlers(&mut self) {
        // Default: no-op
    }

    /// Show instructions per second
    fn show_ips(&mut self, ips_count: u32) {
        // Default: no-op
    }

    /// Get signal handler mask (which signals the GUI handles)
    fn get_sighandler_mask(&self) -> u32 {
        // Default: no signals handled
        0
    }

    /// Handle a signal
    fn sighandler(&mut self, sig: i32) {
        // Default: no-op
    }

    /// Set display mode
    fn set_display_mode(&mut self, mode: DisplayMode) {
        // Default: no-op
    }

    /// Get pending keyboard scancodes
    /// Returns a vector of scancode bytes that should be sent to the keyboard device
    fn get_pending_scancodes(&mut self) -> Vec<u8> {
        // Default: empty (GUIs that don't support keyboard input)
        Vec::new()
    }
}
