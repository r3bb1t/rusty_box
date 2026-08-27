//! The vocabulary a machine and its execution engine share.
//!
//! An engine is whatever actually runs the guest — this port's own interpreter,
//! or a hardware hypervisor. What both ends must agree on before either can be
//! written is the *data*: which processor, where guest-physical memory lives in
//! the host, what a window permits, and how a failure is reported. Those types
//! are here; the traits that pass them come with the first engine that needs
//! them.
//!
//! Nothing in this module knows what an x86 is. A guest-physical address is a
//! number, a permission is a permission, and the arrangement of a PC's memory
//! is the machine's business, not this crate's.

use core::fmt;

/// Bytes in a guest-physical page.
///
/// Every hypervisor this seam is meant to reach maps at page granularity, so a
/// window whose base or length is not a multiple of this cannot be expressed.
/// The constant lives here rather than in each backend so the two cannot
/// disagree about it.
pub const GUEST_PAGE: u64 = 4096;

/// Which virtual processor.
///
/// A newtype because an index into the processors and an index into anything
/// else are different things, and passing one where the other belongs is the
/// mistake worth making impossible (R4).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(transparent)]
pub struct VpIndex(u32);

impl VpIndex {
    /// The processor a machine starts on.
    pub const BOOT: Self = Self(0);

    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A byte offset into the host allocation that backs guest memory.
///
/// Deliberately not a pointer and deliberately not a guest-physical address
/// (R4). The machine owns the allocation and may move it; what survives is the
/// distance from its start, which is also the only form an engine in another
/// process or another crate can be handed safely.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[repr(transparent)]
pub struct HostOffset(u64);

impl HostOffset {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(offset: u64) -> Self {
        Self(offset)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advance by `bytes`, or `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, bytes: u64) -> Option<Self> {
        match self.0.checked_add(bytes) {
            Some(sum) => Some(Self(sum)),
            None => None,
        }
    }
}

/// What a guest may do with one mapped window.
///
/// An access a window does not permit does not fault the guest — it leaves the
/// engine, so the machine can service it. That is what makes a read-only
/// window the right shape for a shadowed ROM, and an unmapped range the right
/// shape for device MMIO.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GpaPerms {
    /// Guest reads are served from host memory.
    pub read: bool,
    /// Guest writes are served from host memory. Clear with `read` set means a
    /// write leaves the engine instead of landing.
    pub write: bool,
    /// Guest instruction fetches are served from host memory.
    pub execute: bool,
    /// The engine records which pages the guest wrote, for an incremental
    /// snapshot. Not every engine can, so asking is not the same as getting —
    /// see the caps an engine reports.
    pub track_dirty: bool,
}

impl GpaPerms {
    /// Nothing is served from host memory; every access leaves the engine.
    /// This is how device MMIO and the unbacked parts of the address space are
    /// expressed — as the absence of a window, or a window that permits
    /// nothing.
    pub const NONE: Self =
        Self { read: false, write: false, execute: false, track_dirty: false };
    /// Ordinary guest RAM.
    pub const RWX: Self = Self { read: true, write: true, execute: true, track_dirty: false };
    /// A shadowed ROM, or a chipset region whose writes the machine must see:
    /// reads and fetches run at full speed, writes leave the engine.
    pub const RX: Self = Self { read: true, write: false, execute: true, track_dirty: false };
    /// Guest RAM whose writes are being tracked.
    pub const RWX_TRACKED: Self = Self { track_dirty: true, ..Self::RWX };

    /// Whether any access at all is served from host memory. A window that
    /// permits nothing is indistinguishable from no window, and a plan should
    /// omit it rather than map it.
    #[must_use]
    pub const fn serves_anything(self) -> bool {
        self.read || self.write || self.execute
    }
}

/// One contiguous window of guest-physical space, and where it lives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GpaWindow {
    /// First guest-physical byte. A multiple of [`GUEST_PAGE`].
    pub gpa: u64,
    /// Length in bytes. A non-zero multiple of [`GUEST_PAGE`].
    pub len: u64,
    /// Where the window's first byte lives in the host allocation.
    pub host: HostOffset,
    pub perms: GpaPerms,
}

impl GpaWindow {
    /// One past the last guest-physical byte, or `None` if the window runs off
    /// the end of the address space.
    #[must_use]
    pub const fn end(&self) -> Option<u64> {
        self.gpa.checked_add(self.len)
    }

    /// Whether `gpa` falls inside this window.
    #[must_use]
    pub const fn contains(&self, gpa: u64) -> bool {
        match self.end() {
            Some(end) => self.gpa <= gpa && gpa < end,
            None => self.gpa <= gpa,
        }
    }

    /// Whether the window's base and length are both page-aligned and the
    /// length is non-zero — the shape every engine requires.
    #[must_use]
    pub const fn is_page_shaped(&self) -> bool {
        self.len != 0
            && self.gpa % GUEST_PAGE == 0
            && self.len % GUEST_PAGE == 0
            && self.host.get() % GUEST_PAGE == 0
    }

    /// Whether two windows claim any of the same guest-physical byte.
    #[must_use]
    pub const fn overlaps(&self, other: &Self) -> bool {
        let (Some(mine), Some(theirs)) = (self.end(), other.end()) else {
            // A window running off the end of the address space overlaps
            // anything at or above its base; treating it as overlapping is the
            // conservative answer and such a window is rejected anyway.
            return true;
        };
        self.gpa < theirs && other.gpa < mine
    }
}

/// Two windows of a plan that claim the same guest-physical bytes, by position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GpaOverlap {
    pub first: usize,
    pub second: usize,
}

/// Why a set of windows is not a usable guest-physical map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GpaPlanError {
    /// A window's base, length or host offset is not page-shaped.
    NotPageShaped { at: usize },
    /// A window permits nothing, which is the same as not mapping it.
    PermitsNothing { at: usize },
    /// Two windows claim the same guest-physical bytes.
    Overlap(GpaOverlap),
}

/// The complete guest-physical map a machine wants its engine to install.
///
/// Borrowed rather than owned: the machine derives it from its own memory
/// topology into a buffer it keeps, and the engine only reads it. That is also
/// what keeps this type usable without an allocator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GpaPlan<'a> {
    windows: &'a [GpaWindow],
}

impl<'a> GpaPlan<'a> {
    /// Accept a set of windows as a plan, checking the invariants an engine
    /// would otherwise discover one failed mapping at a time.
    ///
    /// # Errors
    /// [`GpaPlanError`] naming the offending window by position.
    pub fn new(windows: &'a [GpaWindow]) -> Result<Self, GpaPlanError> {
        for (at, window) in windows.iter().enumerate() {
            if !window.is_page_shaped() {
                return Err(GpaPlanError::NotPageShaped { at });
            }
            if !window.perms.serves_anything() {
                return Err(GpaPlanError::PermitsNothing { at });
            }
        }
        // Quadratic, over a map with a handful of windows on any machine this
        // serves — and a plan is derived once per topology change, not per
        // access.
        for (first, window) in windows.iter().enumerate() {
            for (offset, other) in windows[first + 1..].iter().enumerate() {
                if window.overlaps(other) {
                    return Err(GpaPlanError::Overlap(GpaOverlap {
                        first,
                        second: first + 1 + offset,
                    }));
                }
            }
        }
        Ok(Self { windows })
    }

    /// The windows, in the order the machine derived them.
    #[must_use]
    pub const fn windows(&self) -> &'a [GpaWindow] {
        self.windows
    }

    /// How many windows the plan installs.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether the plan maps nothing at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Total guest-physical bytes the plan makes directly accessible, or `None`
    /// on overflow.
    #[must_use]
    pub fn total_bytes(&self) -> Option<u64> {
        self.windows.iter().try_fold(0u64, |sum, window| sum.checked_add(window.len))
    }

    /// The window covering `gpa`, if the plan maps it. Everything else is an
    /// access the machine must service itself.
    #[must_use]
    pub fn window_at(&self, gpa: u64) -> Option<&'a GpaWindow> {
        self.windows.iter().find(|window| window.contains(gpa))
    }
}

/// What an engine can and cannot do, asked once rather than assumed.
///
/// `#[non_exhaustive]` at birth: this is the type most likely to grow as
/// engines are added, and growing it must stay a minor change.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[non_exhaustive]
pub struct EngineCaps {
    /// The engine honours the A20 gate. No hardware hypervisor does, so a
    /// machine that must mask addresses has to know.
    pub a20_gate: bool,
    /// The engine can enter system-management mode.
    pub smm: bool,
    /// The engine counts retired instructions, so progress can be expressed as
    /// a count rather than only as elapsed time.
    pub counts_instructions: bool,
    /// The engine can report which guest pages were written.
    pub tracks_dirty_pages: bool,
}

impl EngineCaps {
    /// Everything a software interpreter of this port can do.
    #[must_use]
    pub const fn software() -> Self {
        Self { a20_gate: true, smm: true, counts_instructions: true, tracks_dirty_pages: false }
    }
}

/// The coarse cause of an engine failure.
///
/// `#[non_exhaustive]` because it crosses a crate boundary: an engine this
/// crate has never heard of may fail in a way none of these name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum EngineFaultKind {
    /// The engine does not exist on this host or in this build. The one kind a
    /// caller is expected to handle by choosing a different engine.
    Unsupported,
    /// A guest-physical mapping could not be applied.
    Memory,
    /// A processor could not be created, run or stopped.
    Vcpu,
    /// Architectural state could not be moved into or out of a processor.
    State,
    /// The engine's host backing failed in a way it cannot classify further.
    Host,
}

/// An engine refused or failed.
///
/// Private fields so context can be added later without breaking callers, and
/// so the platform code an engine wants to preserve has somewhere to live.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EngineFault {
    kind: EngineFaultKind,
    /// The backend's own error number, or zero when it has none. Kept because
    /// a classified error a reader cannot trace back to the platform is worth
    /// less than one they can.
    code: i32,
    /// The operation that failed, for a message a reader can act on.
    at: &'static str,
}

impl EngineFault {
    #[must_use]
    pub const fn new(kind: EngineFaultKind, at: &'static str) -> Self {
        Self { kind, code: 0, at }
    }

    /// The same, carrying the backend's own error number.
    #[must_use]
    pub const fn with_code(kind: EngineFaultKind, at: &'static str, code: i32) -> Self {
        Self { kind, code, at }
    }

    /// No engine of this sort is available here.
    #[must_use]
    pub const fn unsupported(at: &'static str) -> Self {
        Self::new(EngineFaultKind::Unsupported, at)
    }

    #[must_use]
    pub const fn kind(self) -> EngineFaultKind {
        self.kind
    }

    /// The backend's error number, or zero when it reported none.
    #[must_use]
    pub const fn code(self) -> i32 {
        self.code
    }

    /// The operation that failed.
    #[must_use]
    pub const fn at(self) -> &'static str {
        self.at
    }
}

impl fmt::Display for EngineFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            EngineFaultKind::Unsupported => "no such engine on this host",
            EngineFaultKind::Memory => "a guest-physical mapping failed",
            EngineFaultKind::Vcpu => "a processor operation failed",
            EngineFaultKind::State => "architectural state could not be moved",
            EngineFaultKind::Host => "the engine's host backing failed",
        };
        write!(f, "{}: {what}", self.at)?;
        if self.code != 0 {
            write!(f, " (code {:#010x})", self.code as u32)?;
        }
        Ok(())
    }
}

impl fmt::Debug for EngineFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for EngineFault {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ram(gpa: u64, len: u64, host: u64) -> GpaWindow {
        GpaWindow { gpa, len, host: HostOffset::new(host), perms: GpaPerms::RWX }
    }

    #[test]
    fn a_plan_refuses_windows_that_claim_the_same_guest_bytes() {
        let windows = [ram(0, 0x2000, 0), ram(0x1000, 0x2000, 0x2000)];
        assert_eq!(
            GpaPlan::new(&windows),
            Err(GpaPlanError::Overlap(GpaOverlap { first: 0, second: 1 }))
        );
    }

    /// Two windows that merely touch are the normal case — guest RAM split at
    /// the PCI hole is exactly this — so the overlap check must not reject it.
    #[test]
    fn windows_that_abut_are_not_an_overlap() {
        let windows = [ram(0, 0x1000, 0), ram(0x1000, 0x1000, 0x1000)];
        assert!(GpaPlan::new(&windows).is_ok());
    }

    #[test]
    fn a_plan_refuses_a_window_that_is_not_page_shaped() {
        for bad in [
            ram(0x800, GUEST_PAGE, 0),
            ram(0, GUEST_PAGE + 1, 0),
            ram(0, GUEST_PAGE, 0x800),
            ram(0, 0, 0),
        ] {
            assert_eq!(
                GpaPlan::new(&[bad]),
                Err(GpaPlanError::NotPageShaped { at: 0 }),
                "{bad:?}"
            );
        }
    }

    /// A window permitting nothing is the same as no window, and mapping one
    /// would tell an engine to install a range every access then leaves again.
    #[test]
    fn a_plan_refuses_a_window_that_permits_nothing() {
        let dead = GpaWindow { perms: GpaPerms::NONE, ..ram(0, GUEST_PAGE, 0) };
        assert_eq!(GpaPlan::new(&[dead]), Err(GpaPlanError::PermitsNothing { at: 0 }));
        assert!(!GpaPerms::NONE.serves_anything());
        assert!(GpaPerms::RX.serves_anything());
    }

    #[test]
    fn a_plan_answers_which_window_covers_an_address() {
        let windows = [ram(0, 0x2000, 0), ram(0x1_0000_0000, 0x1000, 0x2000)];
        let plan = GpaPlan::new(&windows).expect("a valid plan");
        assert_eq!(plan.window_at(0x1FFF), Some(&windows[0]));
        assert_eq!(plan.window_at(0x2000), None, "the gap between windows is unmapped");
        assert_eq!(plan.window_at(0x1_0000_0000), Some(&windows[1]));
        assert_eq!(plan.total_bytes(), Some(0x3000));
        assert_eq!(plan.len(), 2);
        assert!(!plan.is_empty());
    }

    #[test]
    fn a_read_only_window_is_distinguishable_from_ram_and_from_nothing() {
        assert!(GpaPerms::RX.read && !GpaPerms::RX.write && GpaPerms::RX.execute);
        assert!(GpaPerms::RWX_TRACKED.track_dirty && GpaPerms::RWX_TRACKED.write);
        assert_ne!(GpaPerms::RX, GpaPerms::RWX);
    }

    #[test]
    fn a_fault_keeps_the_backend_s_own_error_number() {
        let fault = EngineFault::with_code(EngineFaultKind::Memory, "map", 0x8007_0057_u32 as i32);
        assert_eq!(fault.kind(), EngineFaultKind::Memory);
        assert_eq!(fault.code(), 0x8007_0057_u32 as i32);
        assert_eq!(fault.at(), "map");
        assert_eq!(EngineFault::unsupported("x").code(), 0);
    }

    #[test]
    fn an_offset_and_an_index_are_different_types_with_their_own_zero() {
        assert_eq!(HostOffset::ZERO.get(), 0);
        assert_eq!(VpIndex::BOOT.get(), 0);
        assert_eq!(HostOffset::new(u64::MAX).checked_add(1), None);
        assert_eq!(HostOffset::new(1).checked_add(1), Some(HostOffset::new(2)));
    }
}
