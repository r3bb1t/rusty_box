//! A display adapter as a VGA core plus one extension.
//!
//! Bochs composes display adapters by inheritance: `bx_vgacore_c` is a complete
//! standard VGA, `bx_vga_c` derives from it to add the Bochs VBE extension, and
//! `bx_geforce_c` derives from it again to add an NV chip. Each subclass
//! overrides a handful of virtuals and calls the base implementation for
//! everything else — `bx_geforce_c::update()` literally calls
//! `bx_vgacore_c::update()` when the card is in a legacy mode.
//!
//! Rust composes by holding both and handing the core to the extension
//! explicitly. [`VgaCard`] owns a [`VgaCore`] and an `E: VgaExtension`; every
//! hook on that trait has a default that falls through to the core, so a card
//! implements only what it changes. That is what real extensions do — Cirrus
//! overrides 9 of the ~18 `bx_vgacore_c` virtuals, GeForce 10, plain VGA 8, and
//! always the same clump.
//!
//! **Every hook takes a context, never a parameter list.** `VgaExtension` is
//! unsealed — writing your own card is a supported use — so it cannot rely on
//! sealing to stay additive. Under RFC 1105 a defaulted method may be added in
//! a minor release, but *changing an existing method's parameters is a
//! permanently breaking change*. A context struct with private fields can grow
//! a method when a future card needs the DAC state or the current tick; a
//! parameter list cannot grow at all.
//!
//! Outcome enums go the other way and stay exhaustive (R5): a new way for a
//! refresh to end should break every card that inspects one.
//!
//! `VgaExtension` is deliberately not dyn-compatible — `vga_refresh` is generic
//! over the sink — and must stay that way (R8). In-tree cards are selected by
//! type, so a defaulted hook that falls through monomorphises to nothing.

use super::device_api::WindowOffset;
use super::display_sink::{DisplaySink, Refreshed};
use super::vga::{VgaCore, VgaWindow};

/// What an extension did with a memory write it was offered first.
///
/// Named rather than `Option<()>`: a write carries no value back, so there is
/// nothing for an `Option` to hold and the two states deserve their names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// The extension did not claim this write; the core performs it.
    FallThrough,
    /// The extension performed the write itself.
    Done,
}

/// Where a frame's changed regions are known from.
///
/// A software emulator sees every guest write, so it tracks its own dirty
/// tiles. A hypervisor does not: a window mapped as host RAM produces no exits,
/// and the only record of what changed is the page bitmap the hypervisor keeps
/// — `KVM_GET_DIRTY_LOG` on KVM, `WHvQueryGpaRangeDirtyBitmap` on WHP, both one
/// bit per page. This is the input that cannot be retrofitted: a refresh which
/// only ever consults its own tile bitmap has nowhere to receive one.
///
/// The two are a **union**, not a choice. Even under a hypervisor a device can
/// write its own framebuffer by paths the hardware never sees, so page-tracked
/// dirt supplements self-tracked dirt rather than replacing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dirt<'a> {
    /// The device observed every write. Every software-engine frame.
    SelfTracked,
    /// One bit per page of the direct-mapped window, least-significant bit of
    /// word zero being the window's first page.
    Pages {
        bitmap: &'a [u64],
        page_size: usize,
    },
}

impl Dirt<'_> {
    /// Whether the bytes `[start, end)` of the direct-mapped window may have
    /// changed.
    ///
    /// `SelfTracked` answers "ask the device instead" by reporting nothing
    /// extra; the caller unions this with its own tile bitmap, so a `false`
    /// here never suppresses a tile the device knows is dirty.
    pub(crate) fn covers(&self, start: u64, end: u64) -> bool {
        let Self::Pages { bitmap, page_size } = self else {
            return false;
        };
        let page_size = *page_size as u64;
        if page_size == 0 || end <= start {
            return false;
        }
        let first = start / page_size;
        let last = (end - 1) / page_size;
        (first..=last).any(|page| {
            let word = (page / 64) as usize;
            bitmap
                .get(word)
                .is_some_and(|bits| bits & (1u64 << (page % 64)) != 0)
        })
    }
}

/// A memory access the extension is offered before the core sees it.
pub struct MemCtx<'a> {
    core: &'a mut VgaCore,
    window: VgaWindow,
    at: WindowOffset,
}

impl<'a> MemCtx<'a> {
    pub(crate) fn new(core: &'a mut VgaCore, window: VgaWindow, at: WindowOffset) -> Self {
        Self { core, window, at }
    }

    /// Which of the card's windows the access landed in.
    pub fn window(&self) -> VgaWindow {
        self.window
    }

    /// How far into that window it landed.
    pub fn offset(&self) -> WindowOffset {
        self.at
    }

    /// The standard VGA underneath, for a card that needs to consult or drive
    /// it while handling an access itself.
    pub fn core(&mut self) -> &mut VgaCore {
        self.core
    }
}

/// The pieces of a refresh, borrowed disjointly.
///
/// Handing these out together is what lets a card read the core and write the
/// sink in the same expression; taking them one accessor at a time would
/// borrow the whole context twice. Same shape as `memory/residency.rs`'s
/// `resident_parts`.
pub struct RefreshParts<'a, S: DisplaySink> {
    pub core: &'a mut VgaCore,
    pub sink: &'a mut S,
    pub dirt: Dirt<'a>,
}

/// A frame the extension is offered before the core draws one.
pub struct RefreshCtx<'a, S: DisplaySink> {
    core: &'a mut VgaCore,
    sink: &'a mut S,
    dirt: Dirt<'a>,
}

impl<'a, S: DisplaySink> RefreshCtx<'a, S> {
    pub(crate) fn new(core: &'a mut VgaCore, sink: &'a mut S, dirt: Dirt<'a>) -> Self {
        Self { core, sink, dirt }
    }

    /// Where this frame's changed regions are known from.
    pub fn dirt(&self) -> Dirt<'_> {
        self.dirt
    }

    pub fn core(&mut self) -> &mut VgaCore {
        self.core
    }

    pub fn sink(&mut self) -> &mut S {
        self.sink
    }

    /// Core, sink and dirt at once, for the common case of drawing from one
    /// into the other.
    pub fn parts(&mut self) -> RefreshParts<'_, S> {
        RefreshParts {
            core: self.core,
            sink: self.sink,
            dirt: self.dirt,
        }
    }
}

/// A vertical retrace the extension is offered.
pub struct TimingCtx<'a> {
    core: &'a mut VgaCore,
    icount: u64,
}

impl<'a> TimingCtx<'a> {
    pub(crate) fn new(core: &'a mut VgaCore, icount: u64) -> Self {
        Self { core, icount }
    }

    pub fn core(&mut self) -> &mut VgaCore {
        self.core
    }

    /// Retired instructions at the retrace — the machine's clock as this port
    /// currently supplies it to the card. Becomes a rate-carrying `VmClock`
    /// when `set_icount_sync` goes, which is a snapshot-format change and so
    /// belongs to its own unit.
    pub fn icount(&self) -> u64 {
        self.icount
    }
}

/// A reset the extension is offered after the core has reset itself.
pub struct ResetCtx<'a> {
    core: &'a mut VgaCore,
}

impl<'a> ResetCtx<'a> {
    pub(crate) fn new(core: &'a mut VgaCore) -> Self {
        Self { core }
    }

    pub fn core(&mut self) -> &mut VgaCore {
        self.core
    }
}

/// What a card adds to a standard VGA.
///
/// Every hook falls through to the core by default, so `impl VgaExtension for
/// MyCard {}` is a complete, working standard VGA.
pub trait VgaExtension {
    /// How much video memory the card has. The core owns the storage and
    /// allocates it once from this size — upstream inverts that, letting each
    /// subclass allocate into the base's pointer, which is where its
    /// double-free class of bug lives.
    fn vga_vram_bytes(&self) -> usize {
        VgaCore::LEGACY_VRAM_BYTES
    }

    /// Offered a read before the core serves it. `None` falls through.
    fn vga_mem_read(&mut self, cx: &mut MemCtx<'_>) -> Option<u8> {
        let _ = cx;
        None
    }

    /// Offered a write before the core performs it.
    fn vga_mem_write(&mut self, cx: &mut MemCtx<'_>, value: u8) -> Written {
        let _ = (cx, value);
        Written::FallThrough
    }

    /// Offered the frame before the core draws one. `None` falls through —
    /// which is what a card in a legacy mode does, exactly as
    /// `bx_geforce_c::update()` calls its base when `crtc28 == 0`.
    fn vga_refresh<S: DisplaySink>(&mut self, cx: &mut RefreshCtx<'_, S>) -> Option<Refreshed> {
        let _ = cx;
        None
    }

    /// Offered each vertical retrace, after the core has latched its own frame
    /// state.
    fn vga_vertical_timer(&mut self, cx: &mut TimingCtx<'_>) {
        let _ = cx;
    }

    /// Offered a reset after the core has performed its own.
    fn vga_reset(&mut self, cx: &mut ResetCtx<'_>) {
        let _ = cx;
    }
}

/// A plain VGA with the Bochs VBE extension — Bochs `bx_vga_c`.
///
/// Every hook falls through today because the VBE state still lives in the
/// core, where this port's merged `BxVgaC` kept it. Moving it here is the next
/// step of this unit and is what makes the name true; until then this is the
/// card identity the machine names, and the composition it names it through is
/// already the final one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StdVga;

impl VgaExtension for StdVga {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page-tracked window reports a tile dirty when any page overlapping it
    /// is set, and only then. This is the arithmetic a hypervisor frame rests
    /// on: the device sees none of those writes, so an off-by-one here loses
    /// pixels with nothing else to notice.
    #[test]
    fn page_dirt_covers_exactly_the_pages_a_range_touches() {
        // Page 0 and page 3 dirty, in a 4 KiB-page window.
        let bitmap = [0b1001u64];
        let dirt = Dirt::Pages {
            bitmap: &bitmap,
            page_size: 4096,
        };

        assert!(dirt.covers(0, 1), "the first byte of page 0");
        assert!(dirt.covers(4095, 4096), "the last byte of page 0");
        assert!(!dirt.covers(4096, 8192), "page 1 is clean");
        assert!(!dirt.covers(8192, 12288), "page 2 is clean");
        assert!(dirt.covers(12288, 12289), "page 3 is dirty");
        assert!(
            dirt.covers(4096, 12289),
            "a range spanning clean pages into a dirty one is dirty"
        );
        assert!(
            !dirt.covers(4096, 12288),
            "and the same range stopping one byte short is not"
        );
    }

    /// An empty range covers nothing, whatever the bitmap says — a zero-width
    /// tile has no pixels to be dirty.
    #[test]
    fn an_empty_range_is_never_dirty() {
        let bitmap = [u64::MAX];
        let dirt = Dirt::Pages {
            bitmap: &bitmap,
            page_size: 4096,
        };
        assert!(!dirt.covers(0, 0));
        assert!(!dirt.covers(8192, 4096));
    }

    /// Past the end of the bitmap is clean rather than a panic: the hypervisor
    /// sizes the bitmap for the window it tracks, and a query beyond it is
    /// asking about memory nothing reported on.
    #[test]
    fn a_range_past_the_bitmap_is_clean() {
        let bitmap = [0u64];
        let dirt = Dirt::Pages {
            bitmap: &bitmap,
            page_size: 4096,
        };
        assert!(!dirt.covers(4096 * 1000, 4096 * 1000 + 1));
    }

    /// Self-tracked dirt adds nothing of its own — the device's tile bitmap is
    /// the whole answer, and this must never subtract from it.
    #[test]
    fn self_tracked_dirt_reports_nothing_extra() {
        assert!(!Dirt::SelfTracked.covers(0, u64::MAX));
    }
}
