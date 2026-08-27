//! A hypervisor partition, its guest-physical map, and its processors.
//!
//! The platform's own lifecycle is a two-state one — a partition accepts
//! properties until `WHvSetupPartition` and accepts memory and processors
//! after it — so this module models it as two types rather than one type with
//! a flag (R2). [`PartitionConfig`] is the first state and [`Partition`] the
//! second; there is no way to spell the mistake.
//!
//! ## Scope
//!
//! Processors are addressed by index on [`Partition`] rather than handed out
//! as owned `Vcpu` values. That is deliberate for this stage and not the final
//! shape: an owned processor wants the partition behind an `Arc` so several
//! can run at once, and how the guest-physical map is then shared depends on
//! answers this crate's probe exists to obtain — how much a remap costs and
//! how many mappings a partition tolerates. Choosing the sharing before
//! measuring is what the probe is meant to prevent.

use crate::error::{WhpError, WhpResult};
use crate::sys::{self, GpaPerms, GvaTranslation, PropertyCode, RawPartition};
use crate::vcpu::{
    Exit, InternalActivity, InterruptRequest, PendingInterruption, Reg, SegmentRegister,
};

/// Guest-physical pages are 4 KiB, and every WHP range must start and end on
/// one.
pub const PAGE_SIZE: usize = 4096;

/// Which local-APIC model the partition presents, if any.
///
/// The values are `WHV_X64_LOCAL_APIC_EMULATION_MODE` from the SDK.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LocalApicMode {
    /// No APIC. The host owns interrupt delivery end to end, which is what a
    /// machine whose guest predates the APIC needs.
    #[default]
    None,
    /// The hypervisor emulates an xAPIC.
    XApic,
    /// The hypervisor emulates an x2APIC.
    X2Apic,
}

impl LocalApicMode {
    const fn as_word(self) -> u64 {
        match self {
            Self::None => 0,
            Self::XApic => 1,
            Self::X2Apic => 2,
        }
    }
}

/// A page-aligned host allocation to back a guest-physical range.
///
/// WHP requires the host address it is handed to be page-aligned, and requires
/// it to stay valid for as long as the range is mapped. Both are structural
/// here: the allocation is over-sized and an aligned window taken inside it,
/// and a [`Partition`] takes ownership of the pages it maps, so host memory
/// cannot outlive or predecease the partition reading it.
#[derive(Debug)]
pub struct HostPages {
    /// Over-allocated so an aligned window of `len` always fits inside. Never
    /// grown after construction, so the heap buffer never moves and the
    /// address handed to the platform stays valid.
    storage: Vec<u8>,
    offset: usize,
    len: usize,
}

impl HostPages {
    /// Allocate `pages` zeroed guest pages.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::HostMemory`] if `pages` is zero or the aligned
    /// window cannot be placed.
    pub fn new(pages: usize) -> WhpResult<Self> {
        const CALL: &str = "HostPages::new";
        if pages == 0 {
            return Err(WhpError::host_memory(CALL));
        }
        let len = pages * PAGE_SIZE;
        let storage = vec![0u8; len + PAGE_SIZE];
        let offset = storage.as_ptr().align_offset(PAGE_SIZE);
        if offset > PAGE_SIZE || offset + len > storage.len() {
            return Err(WhpError::host_memory(CALL));
        }
        Ok(Self { storage, offset, len })
    }

    /// The guest-visible bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.storage[self.offset..self.offset + self.len]
    }

    /// The guest-visible bytes, writable by the host.
    #[must_use]
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        let (start, end) = (self.offset, self.offset + self.len);
        &mut self.storage[start..end]
    }

    /// How many bytes the guest sees.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the window is empty. Never true — [`HostPages::new`] refuses a
    /// zero-page request — but `clippy` asks for it beside `len`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// One mapped guest-physical range and the host pages behind it.
#[derive(Debug)]
struct Region {
    gpa: u64,
    perms: GpaPerms,
    pages: HostPages,
}

/// A partition handle whose destruction is the type's job, not a caller's.
#[derive(Debug)]
struct OwnedPartition(RawPartition);

impl Drop for OwnedPartition {
    fn drop(&mut self) {
        sys::delete_partition(self.0);
    }
}

/// A partition before `WHvSetupPartition`: the state in which properties may
/// be set and nothing else may happen.
#[derive(Debug)]
pub struct PartitionConfig {
    handle: OwnedPartition,
}

impl PartitionConfig {
    /// Create a partition and put it in its configurable state.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Unsupported`] on a host without the platform,
    /// otherwise [`crate::WhpErrorKind::Platform`].
    pub fn new() -> WhpResult<Self> {
        Ok(Self { handle: OwnedPartition(sys::create_partition()?) })
    }

    /// How many virtual processors the partition will have. Required: the
    /// platform refuses `WHvSetupPartition` without it.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses the count.
    pub fn processor_count(&mut self, count: u32) -> WhpResult<&mut Self> {
        sys::set_property(self.handle.0, PropertyCode::ProcessorCount, u64::from(count))?;
        Ok(self)
    }

    /// Which of the optional exits the partition wants. Asking for one the
    /// host does not advertise in [`crate::capabilities`] is refused.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host will not provide them.
    pub fn extended_vm_exits(
        &mut self,
        exits: crate::caps::ExtendedVmExits,
    ) -> WhpResult<&mut Self> {
        sys::set_property(self.handle.0, PropertyCode::ExtendedVmExits, exits.as_word())?;
        Ok(self)
    }

    /// The `CPUID` leaves that should exit instead of executing. Requires
    /// [`crate::caps::ExtendedVmExits::cpuid`].
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses the list.
    pub fn cpuid_exit_list(&mut self, leaves: &[u32]) -> WhpResult<&mut Self> {
        sys::set_cpuid_exit_list(self.handle.0, leaves)?;
        Ok(self)
    }

    /// Put the partition in its own security domain. Documented to make guest
    /// exits cheaper at the cost of a side-channel mitigation, so it is
    /// exposed rather than assumed.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses.
    pub fn separate_security_domain(&mut self, separate: bool) -> WhpResult<&mut Self> {
        sys::set_property(
            self.handle.0,
            PropertyCode::SeparateSecurityDomain,
            u64::from(separate),
        )?;
        Ok(self)
    }

    /// Which local APIC, if any, the hypervisor emulates.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses, which it does
    /// when [`crate::caps::Features::local_apic_emulation`] is clear.
    pub fn local_apic(&mut self, mode: LocalApicMode) -> WhpResult<&mut Self> {
        sys::set_property(
            self.handle.0,
            PropertyCode::LocalApicEmulationMode,
            mode.as_word(),
        )?;
        Ok(self)
    }

    /// Finish configuration. Everything the platform accepts only before setup
    /// has been offered by now; what comes back accepts memory and processors.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`], most often because no processor
    /// count was set.
    pub fn setup(self) -> WhpResult<Partition> {
        sys::setup(self.handle.0)?;
        Ok(Partition { handle: self.handle, regions: Vec::new(), processors: Vec::new() })
    }
}

/// A configured partition: its guest-physical map and its processors.
#[derive(Debug)]
pub struct Partition {
    handle: OwnedPartition,
    regions: Vec<Region>,
    processors: Vec<u32>,
}

impl Drop for Partition {
    fn drop(&mut self) {
        // Before the handle's own `Drop`, which runs after this body because
        // it belongs to a field.
        for index in &self.processors {
            sys::delete_vp(self.handle.0, *index);
        }
    }
}

impl Partition {
    /// Map host pages at a guest-physical address, taking ownership of them.
    ///
    /// A mapping REPLACES any prior one covering the same range rather than
    /// requiring an unmap first, which is what makes a chipset's shadow-RAM
    /// flip a single call.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if `gpa` is not page-aligned,
    /// otherwise [`crate::WhpErrorKind::Platform`].
    pub fn map(&mut self, gpa: u64, mut pages: HostPages, perms: GpaPerms) -> WhpResult<()> {
        const CALL: &str = "Partition::map";
        if gpa % PAGE_SIZE as u64 != 0 {
            return Err(WhpError::contract(CALL));
        }
        sys::map_gpa(self.handle.0, pages.bytes_mut(), gpa, perms)?;
        self.regions.push(Region { gpa, perms, pages });
        Ok(())
    }

    /// Change the permissions of an already-mapped range without disturbing
    /// its contents — the chipset's PAM flip, and the verb whose cost decides
    /// whether a shadowed BIOS is affordable.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if nothing is mapped at `gpa`,
    /// otherwise [`crate::WhpErrorKind::Platform`].
    pub fn remap(&mut self, gpa: u64, perms: GpaPerms) -> WhpResult<()> {
        const CALL: &str = "Partition::remap";
        let handle = self.handle.0;
        let region = self
            .regions
            .iter_mut()
            .find(|region| region.gpa == gpa)
            .ok_or(WhpError::contract(CALL))?;
        sys::map_gpa(handle, region.pages.bytes_mut(), gpa, perms)?;
        region.perms = perms;
        Ok(())
    }

    /// Unmap a range and give its host pages back.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if nothing is mapped at `gpa`,
    /// otherwise [`crate::WhpErrorKind::Platform`].
    pub fn unmap(&mut self, gpa: u64) -> WhpResult<HostPages> {
        const CALL: &str = "Partition::unmap";
        let at = self
            .regions
            .iter()
            .position(|region| region.gpa == gpa)
            .ok_or(WhpError::contract(CALL))?;
        let len = self.regions[at].pages.len() as u64;
        sys::unmap_gpa(self.handle.0, gpa, len)?;
        Ok(self.regions.swap_remove(at).pages)
    }

    /// Unmap part of a mapped range, leaving the host pages owned and the
    /// range's bookkeeping intact.
    ///
    /// Requires [`crate::Features::partial_unmap`]; a host without it refuses.
    /// The recorded region still describes the whole range, so a later
    /// [`Partition::remap`] of it re-establishes every page — which is the
    /// intended way to undo this.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`], including when the host cannot do
    /// partial unmaps at all.
    pub fn unmap_subrange(&mut self, gpa: u64, len: u64) -> WhpResult<()> {
        sys::unmap_gpa(self.handle.0, gpa, len)
    }

    /// How many ranges are mapped.
    #[must_use]
    pub fn mapped_regions(&self) -> usize {
        self.regions.len()
    }

    /// A handle that can interrupt a running processor from another thread.
    ///
    /// The platform documents `WHvCancelRunVirtualProcessor` as callable from
    /// a thread other than the one inside `WHvRunVirtualProcessor`, and this
    /// is the only capability of this crate that crosses threads.
    ///
    /// The returned value is owned rather than borrowed, so that it can be
    /// moved into a thread while this partition is being run — which is the
    /// whole point, and impossible for a borrow, since running takes `&mut`.
    /// It is therefore the caller's job to keep the canceller's life inside
    /// the partition's; `std::thread::scope` is how to say that. A canceller
    /// outliving its partition names a handle the platform has reclaimed, and
    /// gets a refusal or, worse, a partition that was created since.
    #[must_use]
    pub fn canceller(&self, index: u32) -> Canceller {
        Canceller { handle: self.handle.0, index }
    }

    /// A handle that can deliver an interrupt through the emulated APIC from
    /// another thread while a processor is running.
    ///
    /// Carries the same lifetime obligation as [`Partition::canceller`]. See
    /// [`InterruptRequester`] for why this capability has to exist separately
    /// from [`Partition::inject`].
    #[must_use]
    pub fn interrupt_requester(&self) -> InterruptRequester {
        InterruptRequester { handle: self.handle.0 }
    }

    /// The permissions a mapped range currently carries.
    #[must_use]
    pub fn perms_at(&self, gpa: u64) -> Option<GpaPerms> {
        self.regions.iter().find(|region| region.gpa == gpa).map(|region| region.perms)
    }

    /// Host-side access to a mapped range's bytes.
    #[must_use]
    pub fn bytes_at(&self, gpa: u64) -> Option<&[u8]> {
        self.regions.iter().find(|region| region.gpa == gpa).map(|region| region.pages.bytes())
    }

    /// Host-side write access to a mapped range's bytes, for loading a guest
    /// image or reading back what the guest stored.
    #[must_use]
    pub fn bytes_at_mut(&mut self, gpa: u64) -> Option<&mut [u8]> {
        self.regions
            .iter_mut()
            .find(|region| region.gpa == gpa)
            .map(|region| region.pages.bytes_mut())
    }

    /// Which pages of a range the guest has written since the last query.
    /// Requires the range to have been mapped with
    /// [`GpaPerms::track_dirty`].
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses, which it does
    /// when the range is not tracked.
    pub fn dirty_pages(&mut self, gpa: u64, len: u64, out: &mut [u64]) -> WhpResult<()> {
        sys::dirty_bitmap(self.handle.0, gpa, len, out)
    }

    /// Bring a virtual processor into existence.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the index is beyond the
    /// configured processor count or the processor already exists.
    pub fn create_processor(&mut self, index: u32) -> WhpResult<()> {
        sys::create_vp(self.handle.0, index)?;
        self.processors.push(index);
        Ok(())
    }

    /// Run a processor until it exits.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the run itself is refused; an
    /// exit is a success, whatever its reason.
    pub fn run(&mut self, index: u32) -> WhpResult<Exit> {
        sys::run_vp(self.handle.0, index)
    }

    /// Ask a running processor to exit, from any thread.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses.
    pub fn cancel_run(&self, index: u32) -> WhpResult<()> {
        sys::cancel_vp(self.handle.0, index)
    }

    /// Read word-shaped registers.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if the two slices disagree in
    /// length, otherwise [`crate::WhpErrorKind::Platform`].
    pub fn read_regs(&self, index: u32, regs: &[Reg], out: &mut [u64]) -> WhpResult<()> {
        sys::get_words(self.handle.0, index, regs, out)
    }

    /// Read one word-shaped register.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn read_reg(&self, index: u32, reg: Reg) -> WhpResult<u64> {
        let mut out = [0u64; 1];
        sys::get_words(self.handle.0, index, &[reg], &mut out)?;
        Ok(out[0])
    }

    /// Write word-shaped registers.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn write_regs(&self, index: u32, regs: &[Reg], words: &[u64]) -> WhpResult<()> {
        sys::set_words(self.handle.0, index, regs, words)
    }

    /// Write one word-shaped register.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn write_reg(&self, index: u32, reg: Reg, word: u64) -> WhpResult<()> {
        sys::set_words(self.handle.0, index, &[reg], &[word])
    }

    /// Write segment registers.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn write_segments(
        &self,
        index: u32,
        regs: &[Reg],
        segments: &[SegmentRegister],
    ) -> WhpResult<()> {
        sys::set_segments(self.handle.0, index, regs, segments)
    }

    /// Hand a processor an interrupt, NMI or exception to take at its next
    /// opportunity, bypassing any emulated APIC.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn inject(&self, index: u32, event: PendingInterruption) -> WhpResult<()> {
        self.write_reg(index, Reg::PendingInterruption, event.as_word())
    }

    /// Hand the partition's emulated APIC an interrupt to arbitrate and
    /// deliver, rather than injecting it into one processor.
    ///
    /// Only meaningful when the partition was configured with an APIC; with
    /// [`LocalApicMode::None`] there is nothing to accept the request.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses.
    pub fn request_interrupt(&self, request: InterruptRequest) -> WhpResult<()> {
        sys::request_interrupt(self.handle.0, request)
    }

    /// Why a processor is not executing.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn internal_activity(&self, index: u32) -> WhpResult<InternalActivity> {
        Ok(InternalActivity::from_word(
            self.read_reg(index, Reg::InternalActivityState)?,
        ))
    }

    /// Set a processor's activity state — clearing a halt suspend, for
    /// instance.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn set_internal_activity(
        &self,
        index: u32,
        activity: InternalActivity,
    ) -> WhpResult<()> {
        self.write_reg(index, Reg::InternalActivityState, activity.as_word())
    }

    /// Translate a guest linear address through the guest's own paging.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the call itself is refused; a
    /// failed translation is a success carrying a non-zero result code.
    pub fn translate_gva(&self, index: u32, gva: u64) -> WhpResult<GvaTranslation> {
        sys::translate_gva(self.handle.0, index, gva)
    }

    /// Set a single-word partition property after setup.
    ///
    /// Most properties are configuration and belong on [`PartitionConfig`],
    /// where the type system puts them; the platform is nonetheless the
    /// authority on which of them it will still accept here. This verb exists
    /// so a caller can ask it, and so the properties that are legal late stay
    /// reachable.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] carrying the platform's own refusal,
    /// which is the answer a caller asking the question wants.
    pub fn set_property_late(&mut self, code: LateProperty, value: u64) -> WhpResult<()> {
        sys::set_property(self.handle.0, code.into_code(), value)
    }
}

/// One of the two things this crate lets another thread do to a running
/// processor.
///
/// See [`Partition::canceller`] for the lifetime obligation that comes with
/// holding one; it applies equally to [`InterruptRequester`].
#[derive(Clone, Copy, Debug)]
pub struct Canceller {
    handle: RawPartition,
    index: u32,
}

impl Canceller {
    /// Ask the processor to leave `WHvRunVirtualProcessor`, which it does with
    /// [`crate::ExitReason::Canceled`].
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses.
    pub fn cancel(&self) -> WhpResult<()> {
        sys::cancel_vp(self.handle, self.index)
    }
}

/// The other: hand the partition's emulated APIC an interrupt while a
/// processor is running.
///
/// This exists because it is the ONLY delivery route available when the
/// hypervisor emulates the APIC. In that mode a halted processor does not
/// leave `WHvRunVirtualProcessor` at all, so there is no moment at which the
/// running thread could stop and inject; the vector has to arrive from
/// somewhere else while the run is in progress.
///
/// `WHvRequestInterrupt` addresses the partition rather than a stopped
/// processor's registers, which is what makes it safe to call from another
/// thread — unlike [`Partition::inject`], whose register write requires the
/// processor to be stopped.
#[derive(Clone, Copy, Debug)]
pub struct InterruptRequester {
    handle: RawPartition,
}

impl InterruptRequester {
    /// Deliver an interrupt through the emulated APIC.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses — which it
    /// does with `ERROR_HV_OPERATION_DENIED` when the partition has no APIC.
    pub fn request(&self, request: InterruptRequest) -> WhpResult<()> {
        sys::request_interrupt(self.handle, request)
    }
}

/// The properties [`Partition::set_property_late`] will attempt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LateProperty {
    ProcessorCount,
    ExtendedVmExits,
    SeparateSecurityDomain,
    LocalApicEmulationMode,
}

impl LateProperty {
    const fn into_code(self) -> PropertyCode {
        match self {
            Self::ProcessorCount => PropertyCode::ProcessorCount,
            Self::ExtendedVmExits => PropertyCode::ExtendedVmExits,
            Self::SeparateSecurityDomain => PropertyCode::SeparateSecurityDomain,
            Self::LocalApicEmulationMode => PropertyCode::LocalApicEmulationMode,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_pages_hand_out_a_page_aligned_window_of_exactly_the_size_asked_for() {
        let mut pages = HostPages::new(3).expect("three pages");
        assert_eq!(pages.len(), 3 * PAGE_SIZE);
        assert_eq!(pages.bytes().len(), 3 * PAGE_SIZE);
        assert_eq!(pages.bytes_mut().as_ptr() as usize % PAGE_SIZE, 0);
        assert!(pages.bytes().iter().all(|byte| *byte == 0));
    }

    /// The host address handed to the platform stays valid only because the
    /// window never moves; a `HostPages` that reallocated would invalidate a
    /// live mapping silently.
    #[test]
    fn the_window_address_survives_moving_the_owner() {
        let mut pages = HostPages::new(1).expect("one page");
        let before = pages.bytes_mut().as_ptr() as usize;
        let mut moved = pages;
        assert_eq!(moved.bytes_mut().as_ptr() as usize, before);
    }

    #[test]
    fn a_zero_page_allocation_is_refused_rather_than_producing_an_empty_window() {
        let refused = HostPages::new(0).expect_err("zero pages");
        assert_eq!(refused.kind(), crate::WhpErrorKind::HostMemory);
    }

    #[test]
    fn the_local_apic_modes_carry_the_platform_s_own_numbering() {
        assert_eq!(LocalApicMode::None.as_word(), 0);
        assert_eq!(LocalApicMode::XApic.as_word(), 1);
        assert_eq!(LocalApicMode::X2Apic.as_word(), 2);
        assert_eq!(LocalApicMode::default(), LocalApicMode::None);
    }
}
