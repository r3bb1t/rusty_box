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
    /// Whether this carries no report of its own.
    ///
    /// The whole-frame early-out needs this: a card that bails out on its own
    /// clean tile bitmap would never look at a page bitmap at all, which is
    /// the retrofit hazard in miniature. `SelfTracked` reports nothing because
    /// the device's own bitmap already is the answer.
    pub(crate) fn reports_nothing(&self) -> bool {
        match self {
            Self::SelfTracked => true,
            Self::Pages { bitmap, .. } => bitmap.iter().all(|word| *word == 0),
        }
    }

    /// Whether the bytes `[start, end)` of the direct-mapped window may have
    /// changed.
    ///
    /// `SelfTracked` answers "ask the device instead" by reporting nothing
    /// extra; the caller unions this with its own tile bitmap, so a `false`
    /// here never suppresses a tile the device knows is dirty.
    #[cfg_attr(
        not(feature = "alloc"),
        allow(
            dead_code,
            reason = "the only in-tree consumer is the framebuffer tile loop,                       which needs a conversion buffer and so an allocator"
        )
    )]
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
    #[cfg_attr(
        not(feature = "alloc"),
        allow(dead_code, reason = "built by `VgaCard::refresh`, whose callers are front ends")
    )]
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

/// A display adapter: a standard VGA core, plus whatever one card adds to it.
///
/// This is the type a machine holds in its display slot. Bochs's `bx_vga_c`
/// and `bx_geforce_c` are both `VgaCard<E>` for different `E`, the way they are
/// both `bx_vgacore_c` subclasses upstream.
#[derive(Debug, Default)]
pub struct VgaCard<E: VgaExtension = StdVga> {
    core: VgaCore,
    ext: E,
}

impl<E: VgaExtension> VgaCard<E> {
    /// Build a card around a given extension.
    pub(crate) fn with_extension(ext: E) -> Self {
        Self {
            core: VgaCore::new(),
            ext,
        }
    }

    /// The standard VGA underneath.
    ///
    /// Exposed because diagnostics and the snapshot writer read core registers
    /// directly; a card's *behaviour* reaches the core through the hook
    /// contexts instead.
    pub(crate) fn core(&self) -> &VgaCore {
        &self.core
    }

    #[cfg_attr(
        not(feature = "alloc"),
        allow(dead_code, reason = "consulted by the snapshot rebuild, which needs std")
    )]
    pub(crate) fn core_mut(&mut self) -> &mut VgaCore {
        &mut self.core
    }

    /// Draw one frame — the core's pump, with the extension offered it first.
    ///
    /// `dirt` says where this frame's changed regions are known from. Under the
    /// software engine that is always [`Dirt::SelfTracked`]; a hypervisor
    /// engine supplies the page bitmap for the windows it mapped as host RAM.
    #[cfg_attr(
        not(feature = "alloc"),
        allow(dead_code, reason = "called by the front ends, which need an allocator")
    )]
    pub(crate) fn refresh<S: DisplaySink>(&mut self, sink: &mut S, dirt: Dirt<'_>) -> Refreshed {
        let mut cx = RefreshCtx::new(&mut self.core, sink, dirt);
        if let Some(drawn) = self.ext.vga_refresh(&mut cx) {
            return drawn;
        }
        self.core.refresh(sink, dirt)
    }

    /// Reset the card — core first, exactly as a C++ destructor-ordered base
    /// call would, then the extension over the state that leaves.
    pub(crate) fn reset(&mut self) {
        self.core.reset();
        self.ext.vga_reset(&mut ResetCtx::new(&mut self.core));
    }

    /// One vertical retrace. Bochs `bx_vgacore_c::vertical_timer` latches the
    /// frame's start address and re-anchors the retrace waveform; a card is
    /// offered the tick afterwards.
    pub(crate) fn vertical_timer(&mut self, now_usec: u64, icount: u64) -> bool {
        let retrace = self.core.vertical_timer(now_usec);
        self.ext
            .vga_vertical_timer(&mut TimingCtx::new(&mut self.core, icount));
        retrace
    }
}

/// Everything the machine drives the card with, forwarded to the core.
///
/// These are the verbs a *machine* uses — lifecycle, PCI bar bookkeeping,
/// scanout timing, snapshot mapping targets. None of them is a place a card
/// changes behaviour, so none is a hook; a card that wants a say in one gets it
/// through the hooks on [`VgaExtension`], which these call.
impl<E: VgaExtension> VgaCard<E> {
    pub(crate) fn init(
        &mut self,
        io: &mut super::BxDevicesC,
        mem: &mut crate::memory::BxMemC,
    ) -> crate::Result<()> {
        self.core.init(io, mem)
    }

    pub(crate) fn set_preferred_mode(&mut self, xres: u16, yres: u16, bpp: u16) {
        self.core.set_preferred_mode(xres, yres, bpp);
    }

    pub(crate) fn set_icount_sync(&mut self, ips: u64) {
        self.core.set_icount_sync(ips);
    }

    #[cfg_attr(
        not(feature = "alloc"),
        allow(dead_code, reason = "the interactive loop's redraw pre-check, which needs std")
    )]
    pub(crate) fn is_text_dirty(&self) -> bool {
        self.core.is_text_dirty()
    }

    #[cfg_attr(
        not(feature = "alloc"),
        allow(dead_code, reason = "reached through the `Display` handle, which needs an allocator")
    )]
    pub(crate) fn force_initial_update(&mut self) {
        self.core.force_initial_update();
    }

    pub(crate) fn vertical_period_usec(&self) -> u32 {
        self.core.vertical_period_usec()
    }

    pub(crate) fn enable_pci(&mut self) {
        self.core.enable_pci();
    }

    pub(crate) fn lfb_size(&self) -> u32 {
        self.core.lfb_size()
    }

    pub(crate) fn peek_pending_lfb_relocate(&self) -> Option<(u32, u32)> {
        self.core.peek_pending_lfb_relocate()
    }

    pub(crate) fn commit_pending_lfb_relocate(&mut self) -> Option<(u32, u32)> {
        self.core.commit_pending_lfb_relocate()
    }

    pub(crate) fn peek_pending_mmio_relocate(&self) -> Option<(u32, u32)> {
        self.core.peek_pending_mmio_relocate()
    }

    pub(crate) fn commit_pending_mmio_relocate(&mut self) -> Option<(u32, u32)> {
        self.core.commit_pending_mmio_relocate()
    }

    #[cfg_attr(
        not(feature = "alloc"),
        allow(dead_code, reason = "reached through the `Display` handle, which needs an allocator")
    )]
    pub(crate) fn init_text_mode3(&mut self) {
        self.core.init_text_mode3();
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn get_all_text_rows(&self) -> alloc::vec::Vec<alloc::string::String> {
        self.core.get_all_text_rows()
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn scan_all_text_memory(&self) -> alloc::string::String {
        self.core.scan_all_text_memory()
    }

    #[cfg(feature = "std")]
    pub(crate) fn snapshot_v3_mapping_target(&self) -> super::vga::VgaSnapshotRestoreTarget {
        self.core.snapshot_v3_mapping_target()
    }

    #[cfg(feature = "std")]
    pub(crate) fn snapshot_v3_committed_mapping_target(
        &self,
    ) -> super::vga::VgaSnapshotRestoreTarget {
        self.core.snapshot_v3_committed_mapping_target()
    }

    #[cfg(feature = "std")]
    pub(crate) fn commit_snapshot_v3_mapping_target(
        &mut self,
        target: super::vga::VgaSnapshotRestoreTarget,
    ) {
        self.core.commit_snapshot_v3_mapping_target(target);
    }
}

impl<E: VgaExtension> super::device_api::PioDevice for VgaCard<E> {
    fn pio_read(
        &mut self,
        port: u16,
        len: super::device_api::IoLen,
        ctx: &mut super::device_api::DeviceCtx<'_>,
    ) -> u32 {
        super::device_api::PioDevice::pio_read(&mut self.core, port, len, ctx)
    }

    fn pio_write(
        &mut self,
        port: u16,
        value: u32,
        len: super::device_api::IoLen,
        ctx: &mut super::device_api::DeviceCtx<'_>,
    ) {
        super::device_api::PioDevice::pio_write(&mut self.core, port, value, len, ctx);
    }
}

/// The card's memory windows, with the extension offered each access first.
///
/// Bochs `bx_geforce_c::mem_read`/`mem_write` decide whether an address belongs
/// to the NV chip and otherwise call `bx_vgacore_c`'s. Here the extension is
/// asked and says so by falling through, which keeps the decision in the card
/// that knows it rather than in a dispatch that has to know about every card.
///
/// **Video memory is offered a byte at a time; a register block is not.**
/// `bx_vgacore_c::mem_read` takes one address and returns one byte, because a
/// card claims individual addresses and a multi-byte access can straddle the
/// boundary. The BAR2 register window is the opposite: `vbe_mmio_read` decodes
/// a register and formats the result to the access width, so splitting it into
/// bytes would read the register once per byte and return nonsense. A card that
/// wants registers of its own declares its own window, as the real GeForce does
/// with its NV MMIO BAR.
impl<E: VgaExtension> super::device_api::MmioDevice for VgaCard<E> {
    fn mmio_read(
        &mut self,
        window: super::device_api::WindowId,
        at: super::device_api::WindowOffset,
        len: u32,
        data: &mut [u8],
        ctx: &mut super::device_api::DeviceCtx<'_>,
    ) {
        match VgaWindow::from_id(window) {
            Some(vga_window @ (VgaWindow::Legacy | VgaWindow::Lfb)) => {
                for (index, byte) in data.iter_mut().enumerate().take(len as usize) {
                    let offset = at.get() + index as u64;
                    let mut offered =
                        MemCtx::new(&mut self.core, vga_window, WindowOffset(offset));
                    *byte = match self.ext.vga_mem_read(&mut offered) {
                        Some(value) => value,
                        None => self.core.window_read_byte(vga_window, offset),
                    };
                }
            }
            _ => super::device_api::MmioDevice::mmio_read(
                &mut self.core,
                window,
                at,
                len,
                data,
                ctx,
            ),
        }
    }

    fn mmio_write(
        &mut self,
        window: super::device_api::WindowId,
        at: super::device_api::WindowOffset,
        len: u32,
        data: &[u8],
        ctx: &mut super::device_api::DeviceCtx<'_>,
    ) {
        match VgaWindow::from_id(window) {
            Some(vga_window @ (VgaWindow::Legacy | VgaWindow::Lfb)) => {
                for (index, &value) in data.iter().enumerate().take(len as usize) {
                    let offset = at.get() + index as u64;
                    let mut offered =
                        MemCtx::new(&mut self.core, vga_window, WindowOffset(offset));
                    if self.ext.vga_mem_write(&mut offered, value) == Written::FallThrough {
                        self.core.window_write_byte(vga_window, offset, value);
                    }
                }
            }
            _ => super::device_api::MmioDevice::mmio_write(
                &mut self.core,
                window,
                at,
                len,
                data,
                ctx,
            ),
        }
    }
}

impl<E: VgaExtension> crate::emulator::DisplaySource for VgaCard<E> {
    fn resolution(&self) -> crate::emulator::Resolution {
        self.core.resolution()
    }

    fn text_grid(&self) -> Option<crate::emulator::TextGrid> {
        self.core.text_grid()
    }

    fn text_char(&self, pos: crate::emulator::TextPos) -> char {
        self.core.text_char(pos)
    }
}

impl<E: VgaExtension> super::pci::PciDevice for VgaCard<E> {
    const DEVFUNC: u8 = <VgaCore as super::pci::PciDevice>::DEVFUNC;
    type WriteEffects = <VgaCore as super::pci::PciDevice>::WriteEffects;

    fn pci_read(&self, address: u8, io_len: u8) -> u32 {
        self.core.pci_read(address, io_len)
    }

    fn pci_write(&mut self, address: u8, value: u32, io_len: u8) -> Self::WriteEffects {
        self.core.pci_write(address, value, io_len)
    }
}

#[cfg(feature = "std")]
impl<E: VgaExtension> crate::snapshot::SnapshotSection for VgaCard<E> {
    const TAG: u32 = <VgaCore as crate::snapshot::SnapshotSection>::TAG;
    type Restored = <VgaCore as crate::snapshot::SnapshotSection>::Restored;

    fn snapshot_v3_len(&self) -> std::io::Result<u64> {
        self.core.snapshot_v3_len()
    }

    fn save_snapshot_v3<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.core.save_snapshot_v3(writer)
    }

    fn restore_snapshot_v3<R: std::io::Read>(
        &mut self,
        reader: &mut crate::snapshot::SnapshotReader<R>,
    ) -> std::io::Result<Self::Restored> {
        self.core.restore_snapshot_v3(reader)
    }
}
