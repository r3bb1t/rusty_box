#![allow(unused_assignments, dead_code)]
//! GUI trait definition
//!
//! Based on bx_gui_c class from gui/gui.h
//! This trait defines the interface that all GUI implementations must provide.

use alloc::{boxed::Box, vec::Vec};

use rusty_box_devices::display::sink::{CursorPos, Dimensions, DisplaySink, Redraw, Rgb, TilePos};

/// Display mode for the GUI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    /// Configuration interface mode
    Config,
    /// Simulation mode
    Sim,
}

/// Facade: [`BxGui::text_update`] takes one of these, so a front end
/// implementing this trait must be able to name it without depending on the
/// device crate directly.
pub use rusty_box_devices::display::vga::VgaTextModeInfo;

/// GUI trait - all GUI implementations must provide these methods
///
/// Based on bx_gui_c class from cpp_orig/bochs/gui/gui.h
pub trait BxGui: Send + Sync {
    /// Initialize the GUI with specific parameters
    fn specific_init(&mut self, argc: i32, argv: &[&str], header_bar_y: u32);

    /// Update text mode display
    fn text_update(
        &mut self,
        old_text: &[u8],
        new_text: &[u8],
        cursor_x: u32,
        cursor_y: u32,
        tm_info: &VgaTextModeInfo,
    );

    /// Install one of the two guest character generators.
    ///
    /// Bochs `bx_gui_c::set_text_charmap(int map, Bit8u *fmap)` (gui.cc), called
    /// from `bx_vgacore_c::update_charmap()`. `data` is 256 glyphs x 32 bytes of
    /// raw VGA bitmap, each byte MSB-first (bit 7 = leftmost pixel). Default
    /// no-op so text-only and headless GUIs need not implement it.
    #[allow(unused_variables)]
    fn set_text_charmap(&mut self, map: usize, data: &[u8]) {}

    /// Update a graphics tile
    fn graphics_tile_update(&mut self, tile: &[u8], x: u32, y: u32);

    /// Update a graphics tile with explicit RGBA dimensions. The default
    /// forwards to the fixed-tile-size path, which reads the dimensions from
    /// the tile geometry the device declared at init.
    #[allow(unused_variables)]
    fn graphics_tile_update_rgba(&mut self, tile: &[u8], x: u32, y: u32, width: u32, height: u32) {
        self.graphics_tile_update(tile, x, y);
    }

    /// Handle GUI events (keyboard, mouse, etc.)
    fn handle_events(&mut self);

    /// Flush display updates to screen
    fn flush(&mut self);

    /// Clear the screen
    fn clear_screen(&mut self);

    /// Change palette color
    fn palette_change(&mut self, index: u8, red: u8, green: u8, blue: u8) -> bool;

    /// Update display dimensions
    fn dimension_update(&mut self, x: u32, y: u32, fheight: u32, fwidth: u32, bpp: u32);

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
    fn register_statusitem(&mut self, _text: &str, _auto_off: bool) -> i32 {
        // Default: return -1 (not supported)
        -1
    }

    /// Unregister a status bar item
    fn unregister_statusitem(&mut self, _id: i32) {
        // Default: no-op
    }

    /// Set status bar item state
    fn statusbar_setitem(&mut self, _element: i32, _active: bool, _w: bool) {
        // Default: no-op
    }

    /// Initialize signal handlers
    fn init_signal_handlers(&mut self) {
        // Default: no-op
    }

    /// Show instructions per second
    fn show_ips(&mut self, _ips_count: u32) {
        // Default: no-op
    }

    /// Get signal handler mask (which signals the GUI handles)
    fn get_sighandler_mask(&self) -> u32 {
        // Default: no signals handled
        0
    }

    /// Returns true if this is a headless (no display) GUI.
    /// Used to skip real-time HLT synchronization in headless mode.
    fn is_headless(&self) -> bool {
        false
    }

    /// Handle a signal
    fn sighandler(&mut self, _sig: i32) {
        // Default: no-op
    }

    /// Set display mode
    fn set_display_mode(&mut self, _mode: DisplayMode) {
        // Default: no-op
    }

    /// Get pending keyboard scancodes
    /// Returns a vector of scancode bytes that should be sent to the keyboard device
    fn get_pending_scancodes(&mut self) -> Vec<u8> {
        // Default: empty (GUIs that don't support keyboard input)
        Vec::new()
    }

    /// Get pending guest key press/release events.
    ///
    /// Preferred over [`BxGui::get_pending_scancodes`]: the keyboard controller
    /// renders these through the guest's active scancode set, so selecting set 1
    /// or 3 works (Bochs keyboard.cc `gen_scancode`).
    fn get_pending_keys(&mut self) -> Vec<(crate::iodev::scancodes::BxKey, bool)> {
        Vec::new()
    }

    /// Get pending relative mouse events to forward to the PS/2 aux device.
    fn get_pending_mouse(&mut self) -> Vec<crate::gui::host_input::HostMouseEvent> {
        // Default: empty (GUIs without mouse forwarding)
        Vec::new()
    }

    /// Get pending serial input bytes (ASCII chars to inject into serial port RX)
    /// Used when console=ttyS0 — keyboard input needs to go to serial, not PS/2
    fn get_pending_serial_input(&mut self) -> Vec<u8> {
        Vec::new()
    }

    /// Append text to the serial console log (for display in GUI)
    fn append_serial_log(&self, _text: &str) {
        // Default: no-op
    }
}

/// Presents a [`BxGui`] as the [`DisplaySink`] a display adapter pushes to.
///
/// The two vocabularies are the same one — both are `bx_gui_c`'s — so this only
/// unpacks the named types and restores the sentinel `BxGui` implementations
/// expect for an absent cursor.
pub struct GuiSink<'a> {
    gui: &'a mut dyn BxGui,
}

impl<'a> GuiSink<'a> {
    #[inline]
    pub fn new(gui: &'a mut dyn BxGui) -> Self {
        Self { gui }
    }
}

impl DisplaySink for GuiSink<'_> {
    fn dimension_update(&mut self, dims: Dimensions) {
        self.gui.dimension_update(
            dims.width,
            dims.height,
            dims.font_height,
            dims.font_width,
            u32::from(dims.bits_per_pixel),
        );
    }

    fn text_update(
        &mut self,
        previous: &[u8],
        current: &[u8],
        cursor: Option<CursorPos>,
        info: &VgaTextModeInfo,
    ) {
        // Bochs marks "no cursor" with an out-of-range cell rather than an
        // absent one, and every `BxGui` implementation tests for it.
        let (cursor_x, cursor_y) = match cursor {
            Some(at) => (at.col, at.row),
            None => (0xffff, 0xffff),
        };
        self.gui
            .text_update(previous, current, cursor_x, cursor_y, info);
    }

    fn graphics_tile_update(&mut self, rgba: &[u8], at: TilePos) {
        self.gui
            .graphics_tile_update_rgba(rgba, at.x, at.y, at.width, at.height);
    }

    fn palette_change(&mut self, index: u8, colour: Rgb) -> Redraw {
        if self
            .gui
            .palette_change(index, colour.red, colour.green, colour.blue)
        {
            Redraw::Full
        } else {
            Redraw::NotNeeded
        }
    }

    fn set_text_charmap(&mut self, map: usize, glyphs: &[u8]) {
        self.gui.set_text_charmap(map, glyphs);
    }

    fn clear_screen(&mut self) {
        self.gui.clear_screen();
    }

    fn flush(&mut self) {
        self.gui.flush();
    }
}
