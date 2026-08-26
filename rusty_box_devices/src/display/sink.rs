//! Where a display adapter pushes a frame.
//!
//! Bochs `bx_vgacore_c::update()` calls `bx_gui->text_update()`,
//! `graphics_tile_update()`, `dimension_update()`, `palette_change()`,
//! `set_text_charmap()` and `clear_screen()` directly, from inside the refresh.
//! This port could not: the device had no way to reach a front end, so it
//! returned the frame as a value and stashed the rest in side queues that the
//! run loop drained and forwarded. That cost two 32 KiB arrays copied out per
//! text frame, a `Vec` of `Vec`s per graphics frame, and two hand-maintained
//! copies of the same forwarding code — one for the `BxGui` trait, one for the
//! shared framebuffer.
//!
//! Passing the sink *into* the refresh restores the upstream shape. Every
//! method takes slices, so a frame crosses this boundary without being copied
//! and without allocating, which is what lets the same call work in the
//! no-alloc build.
//!
//! The sink is deliberately Rust-internal. Foreign-language bindings get the
//! pull side instead — `DisplaySource` and a render-into-your-buffer call —
//! because a foreign trait implementation cannot receive borrowed slices, and
//! copying every tile across an FFI boundary each frame would be the wrong
//! shape even where it is possible.

use crate::display::vga::VgaTextModeInfo;

/// The geometry a mode change announces — Bochs `dimension_update`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
    /// Character cell size. Zero in graphics modes, where there are no cells —
    /// Bochs passes 0 for both and front ends read `bpp` instead.
    pub font_width: u32,
    pub font_height: u32,
    pub bits_per_pixel: u8,
}

/// Where a rectangle of pixels goes, and how big it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TilePos {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where the text cursor is, in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPos {
    pub col: u32,
    pub row: u32,
}

/// One DAC entry — Bochs `palette_change`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// Whether a palette change invalidates what is already on screen.
///
/// Bochs returns a bare `bool` from `palette_change` meaning "the front end
/// needs a full redraw". Named because the caller has to act on it and a bare
/// bool at a trait boundary says nothing about which way is which (R0/R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redraw {
    NotNeeded,
    Full,
}

/// What a refresh did, so a caller can tell a frame from an idle tick.
///
/// Exhaustive on purpose (R5): a new outcome must break every site that
/// decides whether to present a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refreshed {
    /// Nothing had changed; the sink was not touched.
    Unchanged,
    /// The sink received a frame and should present it.
    Frame,
}

/// A front end a display adapter can push a frame to.
///
/// The method set is `bx_gui_c`'s, so a card ported from Bochs calls the same
/// things in the same order.
pub trait DisplaySink {
    /// The mode changed. Sent before any pixels of the new mode.
    fn dimension_update(&mut self, dims: Dimensions);

    /// A text frame: the previous cell contents, the current ones, and where
    /// the cursor sits. Both planes are character/attribute pairs; the front
    /// end diffs them to find what to repaint, which is why the old plane is
    /// passed rather than kept by the front end.
    ///
    /// `cursor` is `None` when the cursor is disabled or parked outside the
    /// visible page — Bochs signals that with an out-of-range address, which
    /// stops at this boundary rather than travelling on as a magic number.
    fn text_update(
        &mut self,
        previous: &[u8],
        current: &[u8],
        cursor: Option<CursorPos>,
        info: &VgaTextModeInfo,
    );

    /// One rectangle of RGBA pixels, row-major, four bytes per pixel.
    fn graphics_tile_update(&mut self, rgba: &[u8], at: TilePos);

    /// One DAC entry changed.
    fn palette_change(&mut self, index: u8, colour: Rgb) -> Redraw;

    /// A guest character generator changed. `glyphs` is the whole map.
    fn set_text_charmap(&mut self, map: usize, glyphs: &[u8]);

    /// The sequencer asked for a blank screen.
    fn clear_screen(&mut self);

    /// The frame is complete.
    fn flush(&mut self);
}

/// A sink that drops everything, for a machine with no front end attached.
///
/// Not a stub: a headless machine genuinely has nowhere to put a frame, and the
/// card must still run its refresh so the state a guest reads back — the tile
/// dirty bitmap, the text snapshot, the retrace timing — advances exactly as it
/// does with a display present.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullDisplay;

impl DisplaySink for NullDisplay {
    fn dimension_update(&mut self, _dims: Dimensions) {}
    fn text_update(
        &mut self,
        _previous: &[u8],
        _current: &[u8],
        _cursor: Option<CursorPos>,
        _info: &VgaTextModeInfo,
    ) {
    }
    fn graphics_tile_update(&mut self, _rgba: &[u8], _at: TilePos) {}
    fn palette_change(&mut self, _index: u8, _colour: Rgb) -> Redraw {
        Redraw::NotNeeded
    }
    fn set_text_charmap(&mut self, _map: usize, _glyphs: &[u8]) {}
    fn clear_screen(&mut self) {}
    fn flush(&mut self) {}
}
