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

use crate::caps::MsrExits;
use crate::error::{WhpError, WhpResult};
use rusty_box_core::GpaPerms;

use crate::sys::{self, GvaTranslation, PropertyCode, RawPartition, RegisterValue};
use crate::vcpu::{
    Exit, InternalActivity, InterruptRequest, PendingInterruption, Reg, SegmentRegister,
    TableRegister,
};

/// Guest-physical pages are 4 KiB, and every WHP range must start and end on
/// one. Derived from core's [`rusty_box_core::GUEST_PAGE`] rather than restated,
/// so a window this crate accepts and a window a plan describes cannot disagree
/// about what a page is.
pub const PAGE_SIZE: usize = rusty_box_core::GUEST_PAGE as usize;

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

    /// Which model-specific register accesses should exit instead of being
    /// answered by the platform. Requires
    /// [`crate::caps::ExtendedVmExits::msr`], and without it that bit traps
    /// nothing at all — see [`MsrExits`].
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses.
    pub fn msr_exits(&mut self, exits: MsrExits) -> WhpResult<&mut Self> {
        sys::set_property(self.handle.0, PropertyCode::MsrExitBitmap, exits.as_word())?;
        Ok(self)
    }

    /// Which processor exceptions exit instead of being delivered to the
    /// guest, one bit per vector — bit 13 is `#GP`, bit 14 is `#PF`. Requires
    /// [`crate::caps::ExtendedVmExits::exception`], and without it this traps
    /// nothing, the same way the MSR bitmap needs its own extended exit.
    ///
    /// A partition that traps an exception owes the guest its delivery: the
    /// exit is the host's to service, and a host that neither re-injects nor
    /// emulates has stopped the guest mid-fault. For that reason this is a
    /// DIAGNOSTIC seam rather than something a running machine wants — a
    /// guest takes exceptions as part of working correctly, and a fault
    /// trapped here is one the guest's own handler never sees.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses the bitmap.
    pub fn exception_exits(&mut self, vectors: u64) -> WhpResult<&mut Self> {
        sys::set_property(self.handle.0, PropertyCode::ExceptionExitBitmap, vectors)?;
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

/// One intercept class: how often a processor left the hardware for it, and
/// how long the hypervisor spent servicing it.
///
/// The platform's `WHV_PROCESSOR_INTERCEPT_COUNTER`. Keeping the two together
/// is what lets a class that is rare but slow be told apart from one that is
/// frequent and cheap.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InterceptCounter {
    /// How many times the class was entered.
    pub count: u64,
    /// How long was spent in it, in 100-nanosecond units — the platform's own
    /// unit, kept rather than converted so nothing is lost to rounding.
    pub time_100ns: u64,
}

/// What a processor has been leaving the hardware for, counted by the
/// hypervisor.
///
/// The platform's `WHV_PROCESSOR_INTERCEPT_COUNTERS`, one class per field in
/// the order that structure declares them. That order is load-bearing: the
/// platform writes a flat run of words with no tags, so a field read one
/// position out would report halts as port I/O and every conclusion drawn from
/// it would be wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InterceptCounters {
    /// Guest TLB invalidations the hypervisor had to see.
    pub page_invalidations: InterceptCounter,
    /// Control-register reads and writes that trapped.
    pub control_register_accesses: InterceptCounter,
    /// `IN`, `OUT` and their string forms.
    pub io_instructions: InterceptCounter,
    /// Time spent on `HLT`. The COUNT of a halt that exits to the root
    /// partition lands in [`Self::other_intercepts`] instead, so this field's
    /// count stays zero for a guest whose halts this port services — measured
    /// on this platform, not assumed.
    pub halt_instructions: InterceptCounter,
    /// `CPUID` leaves the partition asked to trap.
    pub cpuid_instructions: InterceptCounter,
    /// `RDMSR` and `WRMSR` the partition's MSR bitmap traps.
    pub msr_accesses: InterceptCounter,
    /// Everything the platform does not class separately, which on this
    /// platform includes the count of every root-serviced `HLT`.
    pub other_intercepts: InterceptCounter,
    /// Interrupts held pending because the guest could not yet take them.
    pub pending_interrupts: InterceptCounter,
    /// Instructions the hypervisor's own emulator completed rather than
    /// re-entering the guest for.
    pub emulated_instructions: InterceptCounter,
    /// Debug-register accesses that trapped.
    pub debug_register_accesses: InterceptCounter,
    /// Guest page faults delivered to the host.
    pub page_fault_intercepts: InterceptCounter,
    /// Second-level (nested paging) faults, which is what an unmapped or
    /// permission-refused guest-physical access arrives as.
    pub nested_page_fault_intercepts: InterceptCounter,
    /// `VMCALL`-class instructions the guest issued.
    pub hypercalls: InterceptCounter,
    /// `RDPMC`.
    pub rdpmc_instructions: InterceptCounter,
}

impl InterceptCounters {
    /// How many `u64`s the platform's structure is: two per class.
    const WORDS: usize = 14 * 2;

    /// Read the platform's flat run of words, or refuse a run too short to be
    /// the structure it claims to be.
    fn from_words(words: &[u64]) -> Option<Self> {
        let words: &[u64; Self::WORDS] = words.get(..Self::WORDS)?.try_into().ok()?;
        Some(Self {
            page_invalidations: counter_at(words, 0),
            control_register_accesses: counter_at(words, 1),
            io_instructions: counter_at(words, 2),
            halt_instructions: counter_at(words, 3),
            cpuid_instructions: counter_at(words, 4),
            msr_accesses: counter_at(words, 5),
            other_intercepts: counter_at(words, 6),
            pending_interrupts: counter_at(words, 7),
            emulated_instructions: counter_at(words, 8),
            debug_register_accesses: counter_at(words, 9),
            page_fault_intercepts: counter_at(words, 10),
            nested_page_fault_intercepts: counter_at(words, 11),
            hypercalls: counter_at(words, 12),
            rdpmc_instructions: counter_at(words, 13),
        })
    }
}

/// The class at position `class` of the platform's structure.
///
/// Each `WHV_PROCESSOR_INTERCEPT_COUNTER` is a count followed by a time, so a
/// class occupies two consecutive words and its position is its index in the
/// declaration order of `WHV_PROCESSOR_INTERCEPT_COUNTERS`.
const fn counter_at(
    words: &[u64; InterceptCounters::WORDS],
    class: usize,
) -> InterceptCounter {
    InterceptCounter { count: words[class * 2], time_100ns: words[class * 2 + 1] }
}

/// How many whole `u64`s of a `capacity`-word buffer the platform filled.
///
/// Capped at the buffer, so a byte count larger than what was offered — which
/// the platform's contract forbids — yields a short read that the caller
/// refuses, rather than an index past the end.
const fn filled_words(written_bytes: usize, capacity: usize) -> usize {
    let whole = written_bytes / core::mem::size_of::<u64>();
    if whole < capacity {
        whole
    } else {
        capacity
    }
}

/// How long a processor has existed, and how much of that the hypervisor spent
/// on its behalf rather than running its guest.
///
/// The platform's `WHV_PROCESSOR_RUNTIME_COUNTERS`. The difference between the
/// two is guest time; the hypervisor's share is what an engine leaving the
/// partition too often is paying for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RuntimeCounters {
    /// Total processor runtime, in 100-nanosecond units.
    pub total_100ns: u64,
    /// The part of it attributed to the hypervisor, in the same units.
    pub hypervisor_100ns: u64,
}

impl RuntimeCounters {
    /// The structure is two words, in this order.
    const WORDS: usize = 2;

    /// As [`InterceptCounters::from_words`].
    fn from_words(words: &[u64]) -> Option<Self> {
        let words: &[u64; Self::WORDS] = words.get(..Self::WORDS)?.try_into().ok()?;
        Some(Self { total_100ns: words[0], hypervisor_100ns: words[1] })
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

    /// Map memory the caller owns, without taking it.
    ///
    /// A machine that already has guest RAM — allocated, resident, and reached
    /// by its own interpreter through the same bytes — cannot hand that
    /// allocation over, so this maps it in place. The partition records the
    /// range but not the storage, and [`Partition::unmap_subrange`] is how it
    /// is taken back.
    ///
    /// # Safety
    /// The caller keeps `host` alive, at the same address, and unmoved, for as
    /// long as the mapping stands or the partition lives — whichever ends
    /// first. The hypervisor holds the host addresses directly; freeing or
    /// reallocating mapped memory leaves the guest running against pages the
    /// host no longer owns, and no borrow here can say so, because a partition
    /// stored beside the memory it maps cannot borrow its sibling.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if `gpa` or the buffer is not
    /// page-shaped, otherwise [`crate::WhpErrorKind::Platform`].
    // UNSAFETY: the one `unsafe` outside `sys/`, and it is a signature rather
    // than a block — this function performs no unsafe operation itself. It is
    // marked so because it hands the hypervisor host addresses that outlive the
    // borrow, which is an obligation only a caller can discharge.
    #[expect(
        unsafe_code,
        reason = "the mapping outlives the borrow, so the contract belongs in the signature"
    )]
    pub unsafe fn map_borrowed(
        &mut self,
        gpa: u64,
        host: &mut [u8],
        perms: GpaPerms,
    ) -> WhpResult<()> {
        const CALL: &str = "Partition::map_borrowed";
        if gpa % PAGE_SIZE as u64 != 0 || host.is_empty() || host.len() % PAGE_SIZE != 0 {
            return Err(WhpError::contract(CALL));
        }
        sys::map_gpa(self.handle.0, host, gpa, perms)
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

    /// Read registers of mixed shape — words, segments and descriptor tables —
    /// in a single call.
    ///
    /// The whole architectural state of a processor is one transfer rather
    /// than several. That matters because the cost of a transfer is the call
    /// and not the registers in it, and a machine running a guest makes one per
    /// slice: on a DLX boot, a hundred thousand of them. Each value arrives in
    /// the shape its register names, decided here rather than by the caller.
    ///
    /// # Errors
    /// As [`Partition::read_regs`], and a contract error if the slices
    /// disagree in length or exceed one call's worth.
    pub fn read_registers(
        &self,
        index: u32,
        regs: &[Reg],
        out: &mut [RegisterValue],
    ) -> WhpResult<()> {
        sys::get_registers(self.handle.0, index, regs, out)
    }

    /// Write registers of mixed shape in a single call. The counterpart of
    /// [`Partition::read_registers`].
    ///
    /// # Errors
    /// As [`Partition::read_registers`].
    pub fn write_registers(
        &self,
        index: u32,
        regs: &[Reg],
        values: &[RegisterValue],
    ) -> WhpResult<()> {
        sys::set_registers(self.handle.0, index, regs, values)
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

    /// Read segment registers.
    ///
    /// Separate from [`Partition::read_regs`] because the platform keeps a
    /// segment in a different member of its value union: reading one as a word
    /// yields its base and silently drops the limit, selector and attributes.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn read_segments(
        &self,
        index: u32,
        regs: &[Reg],
        out: &mut [SegmentRegister],
    ) -> WhpResult<()> {
        sys::get_segments(self.handle.0, index, regs, out)
    }

    /// Read the descriptor-table registers, `GDTR` and `IDTR`.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn read_tables(
        &self,
        index: u32,
        regs: &[Reg],
        out: &mut [TableRegister],
    ) -> WhpResult<()> {
        sys::get_tables(self.handle.0, index, regs, out)
    }

    /// Write the descriptor-table registers.
    ///
    /// # Errors
    /// As [`Partition::read_regs`].
    pub fn write_tables(
        &self,
        index: u32,
        regs: &[Reg],
        tables: &[TableRegister],
    ) -> WhpResult<()> {
        sys::set_tables(self.handle.0, index, regs, tables)
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

    /// What the guest has been leaving the hardware for, counted by the
    /// hypervisor rather than by this port.
    ///
    /// The count AND the time per class, so a class that is rare but slow is
    /// distinguishable from one that is frequent and cheap — which no tally
    /// this port keeps can tell apart. Being the platform's own accounting is
    /// the point: a disagreement between it and an engine's census means one of
    /// the two is measuring something other than what it claims.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses,
    /// [`crate::WhpErrorKind::Contract`] if it answers with fewer counters than
    /// its own structure holds.
    pub fn intercept_counters(&self, index: u32) -> WhpResult<InterceptCounters> {
        const CALL: &str = "WHvGetVirtualProcessorCounters(Intercepts)";
        let mut words = [0u64; InterceptCounters::WORDS];
        let written =
            sys::get_counters(self.handle.0, index, sys::CounterSet::Intercepts, &mut words)?;
        InterceptCounters::from_words(&words[..filled_words(written, words.len())])
            .ok_or(WhpError::contract(CALL))
    }

    /// How long a processor has run, and how much of that went to the
    /// hypervisor rather than to the guest.
    ///
    /// # Errors
    /// As [`Partition::intercept_counters`].
    pub fn runtime_counters(&self, index: u32) -> WhpResult<RuntimeCounters> {
        const CALL: &str = "WHvGetVirtualProcessorCounters(Runtime)";
        let mut words = [0u64; RuntimeCounters::WORDS];
        let written =
            sys::get_counters(self.handle.0, index, sys::CounterSet::Runtime, &mut words)?;
        RuntimeCounters::from_words(&words[..filled_words(written, words.len())])
            .ok_or(WhpError::contract(CALL))
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

    /// Where a fresh processor's `CS` is based, and so where a guest reached
    /// without touching `CS` must live.
    ///
    /// A processor the platform has just created is at the architectural reset
    /// state, `CS` included, so `RIP` alone does not say where execution goes.
    /// Mapping here and leaving `CS` as found keeps the measurement about the
    /// partition rather than about segment loading.
    #[cfg(test)]
    const RESET_CS_BASE: u64 = 0xF_0000;
    /// Offset into that segment where the measurement guest's `HLT` sits.
    #[cfg(test)]
    const GUEST_IP: u64 = 0x1000;

    /// A partition holding one processor pointed at a single `HLT`.
    ///
    /// The smallest thing that proves a partition is live, and the memory is
    /// mapped because mapping is where the platform materialises the partition
    /// underneath — a fact the test below exists to record.
    #[cfg(test)]
    fn halting_partition() -> WhpResult<Partition> {
        let mut config = PartitionConfig::new()?;
        config.processor_count(1)?.local_apic(LocalApicMode::None)?;
        let mut partition = config.setup()?;

        let mut pages = HostPages::new(2)?;
        pages.bytes_mut()[GUEST_IP as usize] = 0xF4;
        partition.map(RESET_CS_BASE, pages, GpaPerms::RWX)?;
        partition.create_processor(0)?;
        partition.write_reg(0, Reg::Rip, GUEST_IP)?;
        Ok(partition)
    }

    /// The turn a test takes before putting a partition on the hardware.
    ///
    /// A process holds one partition at a time, which
    /// [`a_process_holds_one_partition_at_a_time`] measures. Libtest runs tests
    /// on several threads, so without this two of them would hold partitions at
    /// once and the platform would refuse one — a fact about the platform
    /// rather than about anything under test.
    #[cfg(test)]
    fn a_turn_on_the_hardware() -> std::sync::MutexGuard<'static, ()> {
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        // A test that panicked while holding the turn poisoned nothing: the
        // guard protects an ordering, not a value.
        TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// How many exits one measurement guest may take before the loop calls it
    /// a runaway. Far above what any code these tests run can need, and finite
    /// so a guest that never halts fails rather than hangs.
    #[cfg(test)]
    const EXIT_CEILING: usize = 64;

    /// Put `code` where the processor is pointed and run until the guest halts.
    ///
    /// Port accesses are stepped over rather than serviced: no device is behind
    /// them, and what a caller here is measuring is the platform's own account
    /// of the access. The platform leaves `RIP` on the trapping instruction for
    /// an I/O exit, so advancing it by the exit's instruction length is what
    /// lets the guest continue.
    #[cfg(test)]
    fn run_until_halt_with(partition: &mut Partition, code: &[u8]) -> WhpResult<()> {
        let start = GUEST_IP as usize;
        let memory = partition.bytes_at_mut(RESET_CS_BASE).expect("the mapped guest page");
        memory[start..start + code.len()].copy_from_slice(code);
        partition.write_reg(0, Reg::Rip, GUEST_IP)?;

        for _ in 0..EXIT_CEILING {
            let exit = partition.run(0)?;
            match exit.reason {
                crate::ExitReason::Halt => return Ok(()),
                crate::ExitReason::IoPortAccess(_) => {
                    partition.write_reg(0, Reg::Rip, exit.rip_after_instruction())?;
                }
                other => panic!("the measurement guest took an unexpected exit: {other:?}"),
            }
        }
        panic!("the measurement guest ran past {EXIT_CEILING} exits without halting");
    }

    /// **A process holds one partition at a time.** Measured, not read.
    ///
    /// The second partition's first `WHvMapGpaRange` fails with
    /// `ERROR_VID_PARTITION_ALREADY_EXISTS` (0xC0370008) — the platform does
    /// not materialise its backing partition until memory is mapped, and the
    /// name it uses is the process's, so the collision surfaces at the map
    /// rather than at `WHvCreatePartition` or `WHvSetupPartition`. Both of
    /// those succeed for the second partition, which is why a caller cannot
    /// learn this any earlier.
    ///
    /// What it means for anything built on this crate: a fleet of machines on
    /// this engine is a fleet of PROCESSES. One hypervisor-backed machine per
    /// process, alongside as many software-backed machines as the host will
    /// hold. Two hypervisor-backed machines in one process need the surrogate
    /// process that other hypervisor front ends use, which this port does not
    /// have.
    #[test]
    fn a_process_holds_one_partition_at_a_time() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let first = halting_partition().expect("the first partition");
        let refused = halting_partition().expect_err("a second live partition");
        assert_eq!(
            refused.kind(),
            crate::WhpErrorKind::Platform,
            "the second partition must be refused BY THE PLATFORM, and at the map that \
             materialises it: {refused}"
        );

        // Dropping the first releases the name, so partitions are serially
        // reusable within one process even though they do not coexist.
        drop(first);
        let mut again = halting_partition().expect("a partition after the first is gone");
        let exit = again.run(0).expect("the guest runs");
        assert!(
            matches!(exit.reason, crate::ExitReason::Halt),
            "the guest must reach its HLT, not {:?}",
            exit.reason
        );
    }

    /// The platform counts a guest's port writes, and says so without being
    /// asked.
    ///
    /// This is an engine's independent witness: the hypervisor's own
    /// accounting, not this port's, so a disagreement between it and an
    /// engine's census means one of the two is measuring something other than
    /// what it claims.
    ///
    /// The guest writes THREE times because the structure is a flat, untagged
    /// run of words and a reader one position out would silently attribute a
    /// class to its neighbour. A delta of three cannot come from the single
    /// halt on either side of the I/O class, so the count pins the position
    /// rather than merely agreeing with it — and the neighbours are asserted
    /// still, so the shift is refused in both directions.
    ///
    /// The halt itself is the platform's own oddity, measured here rather than
    /// assumed: a `HLT` that exits to the root partition is COUNTED under
    /// `OtherIntercepts`, while its time is charged to `HaltInstructions`. An
    /// engine reading `halt_instructions.count` to find its halts would find
    /// none.
    #[test]
    fn the_platform_counts_the_guests_port_writes_apart_from_its_halts() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut partition = halting_partition().expect("a partition");

        let before = partition
            .intercept_counters(0)
            .expect("a created processor reports its counters");

        // out 0xE9, al ; out 0xE9, al ; out 0xE9, al ; hlt
        run_until_halt_with(&mut partition, &[0xE6, 0xE9, 0xE6, 0xE9, 0xE6, 0xE9, 0xF4])
            .expect("the guest runs");

        let after = partition
            .intercept_counters(0)
            .expect("a processor that has run reports its counters");

        assert_eq!(
            after.io_instructions.count - before.io_instructions.count,
            3,
            "three OUTs executed, so the platform must report exactly three I/O \
             intercepts; before {before:?}, after {after:?}"
        );
        assert!(
            after.io_instructions.time_100ns > before.io_instructions.time_100ns,
            "an intercept that took no time at all would mean the count and the time of \
             a class are being read the wrong way round; before {before:?}, after {after:?}"
        );
        assert_eq!(
            after.control_register_accesses.count, before.control_register_accesses.count,
            "the class before I/O must not have absorbed the port writes; \
             before {before:?}, after {after:?}"
        );
        assert_eq!(
            after.halt_instructions.count, before.halt_instructions.count,
            "the class after I/O must not have absorbed the port writes, and the halt \
             does not move this counter either: the platform books a root-serviced HLT \
             under OtherIntercepts and charges only its time here; \
             before {before:?}, after {after:?}"
        );
        assert_eq!(
            after.other_intercepts.count - before.other_intercepts.count,
            1,
            "the one halt is where the platform's count of it lands; \
             before {before:?}, after {after:?}"
        );
    }

    /// A processor's runtime is accounted for, and the hypervisor's share of it
    /// is a part rather than the whole.
    ///
    /// The two words are the engine's measure of what leaving the partition
    /// costs: guest time is the difference between them, so a run whose
    /// hypervisor share exceeded its total would mean the pair is being read
    /// the wrong way round.
    #[test]
    fn a_processor_that_has_run_accounts_for_its_time() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut partition = halting_partition().expect("a partition");
        run_until_halt_with(&mut partition, &[0xE6, 0xE9, 0xF4]).expect("the guest runs");

        let runtime = partition.runtime_counters(0).expect("a processor that has run");
        assert!(
            runtime.total_100ns > 0,
            "a processor that executed instructions has spent time: {runtime:?}"
        );
        assert!(
            runtime.hypervisor_100ns <= runtime.total_100ns,
            "the hypervisor's share cannot exceed the total it is a share of: {runtime:?}"
        );
    }

    /// A counter buffer shorter than the platform's structure is refused, not
    /// silently padded with zeroes.
    ///
    /// A short answer means the platform reported fewer classes than this port
    /// knows how to name, and reading it anyway would report an absent class as
    /// one that never fired.
    #[test]
    fn a_short_counter_answer_is_refused_rather_than_read() {
        assert_eq!(InterceptCounters::from_words(&[0; 27]), None);
        assert_eq!(RuntimeCounters::from_words(&[0; 1]), None);
        assert_eq!(
            RuntimeCounters::from_words(&[3, 1]),
            Some(RuntimeCounters { total_100ns: 3, hypervisor_100ns: 1 })
        );
    }

    #[test]
    fn the_local_apic_modes_carry_the_platform_s_own_numbering() {
        assert_eq!(LocalApicMode::None.as_word(), 0);
        assert_eq!(LocalApicMode::XApic.as_word(), 1);
        assert_eq!(LocalApicMode::X2Apic.as_word(), 2);
        assert_eq!(LocalApicMode::default(), LocalApicMode::None);
    }
}
