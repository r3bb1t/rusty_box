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
//! A [`Partition`] owns the guest-physical map; a [`Vcpu`] owns one processor.
//! [`Partition::take_vcpu`] hands a processor out once, and every verb that
//! reads or writes that processor's state lives on the handle rather than on
//! the partition. The platform requires a processor to be touched only from
//! the thread inside its `WHvRunVirtualProcessor` — `WHV_E_INVALID_VP_STATE`
//! is the error for the alternative — and a handle that is `Send` but not
//! `Sync` is that rule stated as a type. What another thread may do to a
//! running processor is then exactly the three copyable tokens
//! [`Canceller`], [`InterruptRequester`] and [`VpCounters`].

use crate::caps::{MsrExits, SyntheticFeatures};
use core::cell::Cell;
use core::marker::PhantomData;
use rusty_box_core::GpaPerms;

use crate::sys::{
    self, ApicStatePage, Exit, GvaTranslation, InternalActivity, InterruptRequest,
    PendingInterruption, PropertyCode, RawPartition, Reg, RegisterValue, SegmentRegister,
    TableRegister, VpStateType, WhpError, WhpResult,
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
    // UNSAFETY: `sys::delete_partition` asks its caller to own the uniqueness
    // a `Copy` handle cannot carry, and this type is that owner. Other types
    // here hold a `RawPartition` too — `Canceller` and `InterruptRequester`
    // carry one across threads — but this destructor is the crate's only path
    // to the deletion verb, and the value it deletes cannot be duplicated: the
    // field is private to this module and the type is neither `Copy` nor
    // `Clone`. So the handle is deleted exactly once, here, as the value dies.
    #[expect(
        unsafe_code,
        reason = "a destructor cannot carry the marker in its signature, so it states the guarantee here"
    )]
    fn drop(&mut self) {
        unsafe { sys::delete_partition(self.0) };
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

    /// Which processor features the guest may use, as the raw
    /// `WHV_PROCESSOR_FEATURES` word — normally exactly
    /// [`crate::Capabilities::processor_features`], the bank the host reports.
    ///
    /// A partition that never sets this gets the platform's own default set,
    /// which is narrower than what the host banks; a guest touching a feature
    /// the partition was not told about faults on hardware while working under
    /// an interpreter (a `wrmsr IA32_SPEC_CTRL` is the classic casualty).
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses the set, which it
    /// does when a feature beyond its own bank is asked for.
    pub fn processor_features(&mut self, features: u64) -> WhpResult<&mut Self> {
        sys::set_property(self.handle.0, PropertyCode::ProcessorFeatures, features)?;
        Ok(self)
    }

    /// Which Hyper-V enlightenments the guest may use.
    ///
    /// The payload is `WHV_SYNTHETIC_PROCESSOR_FEATURES_BANKS`: a bank count,
    /// a reserved word and the single bank the SDK declares. Normally
    /// [`SyntheticFeatures::OPENVMM_VTL0`] narrowed to what
    /// [`crate::Capabilities::synthetic_features`] reports, because a bank the
    /// host does not allow is refused whole rather than per flag.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses the set — which
    /// it does for any flag beyond its own bank, naming none of them, so a
    /// caller wanting to know WHICH flag offended must compare against the
    /// capability itself.
    pub fn synthetic_features(
        &mut self,
        bank0: SyntheticFeatures,
    ) -> WhpResult<&mut Self> {
        // BanksCount in the low half of the first word, Reserved0 in the high
        // half, then the bank — the structure's own little-endian layout.
        let mut payload = [0u8; 16];
        payload[..4].copy_from_slice(&1u32.to_le_bytes());
        payload[8..].copy_from_slice(&bank0.bits().to_le_bytes());
        sys::set_property_bytes(
            self.handle.0,
            PropertyCode::SyntheticProcessorFeaturesBanks,
            &payload,
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
        Ok(Partition {
            handle: self.handle,
            regions: Vec::new(),
            processors: Vec::new(),
            taken: Vec::new(),
        })
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
    /// The processors [`Partition::take_vcpu`] has already handed out, so that
    /// each is handed out at most once and one processor never has two owners.
    taken: Vec<u32>,
}

impl Drop for Partition {
    // UNSAFETY: `sys::delete_vp` asks its caller to own two things the handle
    // cannot carry — that the partition is still live, and that each processor
    // is released once. This body owns both. It runs before the handle's own
    // `Drop`, which follows because it belongs to a field, so the partition is
    // still live throughout; and `self.processors` is private to this module,
    // records exactly the indices `create_vp` succeeded for, and is read for
    // deletion nowhere else, so each index reaches the verb once.
    #[expect(
        unsafe_code,
        reason = "a destructor cannot carry the marker in its signature, so it states the guarantee here"
    )]
    fn drop(&mut self) {
        for index in &self.processors {
            unsafe { sys::delete_vp(self.handle.0, *index) };
        }
    }
}

/// The crate's single door onto the platform's one retaining verb.
///
/// `sys::map_gpa` leaves the hypervisor holding `host`'s address after the
/// borrow ends, so every mapping this crate performs answers the same question
/// — who keeps those bytes alive — and answering it in one place is what makes
/// the three answers comparable (R5).
// UNSAFETY: the obligation `sys::map_gpa` states is discharged by each of the
// three callers below, and this is the only code in the crate that reaches the
// verb:
//   - `Partition::map` moves its `HostPages` into `self.regions` on success, so
//     the partition owns the allocation it just mapped and cannot drop it while
//     the mapping stands;
//   - `Partition::remap` re-maps pages `self.regions` already owns, covered by
//     that same ownership;
//   - `Partition::map_borrowed` owns nothing and forwards the obligation to its
//     own caller, which is exactly what its `unsafe` signature says.
#[expect(
    unsafe_code,
    reason = "the crate's one call to the retaining verb, discharged by each of its three callers"
)]
fn map_range(
    handle: RawPartition,
    host: &mut [u8],
    gpa: u64,
    perms: GpaPerms,
) -> WhpResult<()> {
    unsafe { sys::map_gpa(handle, host, gpa, perms) }
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
        map_range(self.handle.0, pages.bytes_mut(), gpa, perms)?;
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
    // UNSAFETY: a signature rather than a block — this function performs no
    // unsafe operation itself, and reaches the seam through `map_range` like
    // every other mapping. It is marked so because it is the one mapping the
    // partition does not own the memory for: it hands the hypervisor host
    // addresses that outlive the borrow, which is an obligation only a caller
    // can discharge.
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
        map_range(self.handle.0, host, gpa, perms)
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
        map_range(handle, region.pages.bytes_mut(), gpa, perms)?;
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
    /// a thread other than the one inside `WHvRunVirtualProcessor`. It is one
    /// of the three capabilities of this crate that cross threads, beside
    /// [`InterruptRequester`] and [`VpCounters`] — and they are the whole set,
    /// because a [`Vcpu`] itself is `!Sync` and cannot be shared.
    ///
    /// The returned value is owned rather than borrowed, so that it can be
    /// moved into a thread while the processor it names is running on
    /// another — which is the whole point, and impossible for a borrow, since
    /// the [`Vcpu`] that runs has itself moved onto that thread. It is
    /// therefore the caller's job to keep the canceller's life inside the
    /// partition's; `std::thread::scope` is how to say that. A canceller
    /// outliving its partition names a handle the platform has reclaimed, and
    /// gets a refusal or, worse, a partition that was created since.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] for a processor this partition never
    /// created — the same answer [`Partition::vp_counters`] gives, and for the
    /// same reason: a capability naming a processor that does not exist is a
    /// caller error the partition can detect from its own record, without
    /// asking the platform.
    pub fn canceller(&self, index: u32) -> WhpResult<Canceller> {
        const CALL: &str = "Partition::canceller";
        if !self.processors.contains(&index) {
            return Err(WhpError::contract(CALL));
        }
        Ok(Canceller { handle: self.handle.0, index })
    }

    /// A handle that can deliver an interrupt through the emulated APIC from
    /// another thread while a processor is running.
    ///
    /// Carries the same lifetime obligation as [`Partition::canceller`]. See
    /// [`InterruptRequester`] for why this capability has to exist separately
    /// from [`Vcpu::inject`].
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

    /// Hand out a processor, once.
    ///
    /// The returned [`Vcpu`] carries every verb that reads or writes that
    /// processor's state; the partition keeps the map. Handing it out once is
    /// what makes "one processor, one owner" a fact rather than a rule a caller
    /// is asked to observe.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if `index` names a processor
    /// [`Partition::create_processor`] never brought into existence, or one
    /// this partition has already handed out. Both are answered from the
    /// partition's own record, without asking the platform, because the
    /// partition is the authority on both.
    pub fn take_vcpu(&mut self, index: u32) -> WhpResult<Vcpu> {
        const CALL: &str = "Partition::take_vcpu";
        if !self.processors.contains(&index) || self.taken.contains(&index) {
            return Err(WhpError::contract(CALL));
        }
        self.taken.push(index);
        Ok(Vcpu { handle: self.handle.0, index, one_thread: PhantomData })
    }

    /// A copyable token for a processor's two counter reads, usable from any
    /// thread.
    ///
    /// The counters are the hypervisor's own accounting rather than processor
    /// state, so reading them does not need the processor stopped and does not
    /// need the handle that runs it — which is what lets a supervising thread
    /// watch a guest that is running.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if `index` names a processor that was
    /// never created.
    pub fn vp_counters(&self, index: u32) -> WhpResult<VpCounters> {
        const CALL: &str = "Partition::vp_counters";
        if !self.processors.contains(&index) {
            return Err(WhpError::contract(CALL));
        }
        Ok(VpCounters { handle: self.handle.0, index })
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

    /// The partition's reference time, in the platform's own 100-nanosecond
    /// units.
    ///
    /// This is the clock the guest reads through the platform's enlightened
    /// time sources, and the one [`Partition::suspend_time`] stops — so it is
    /// also the only way to observe that a suspend took effect.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses the read.
    pub fn reference_time_100ns(&self) -> WhpResult<u64> {
        sys::get_property_word(self.handle.0, PropertyCode::ReferenceTime)
    }

    /// Stop the partition's reference clock.
    ///
    /// What lets a host hold a guest's sense of time still while the host is
    /// doing something the guest must not see the duration of. Paired with
    /// [`Partition::resume_time`]; the platform, not this type, owns the count
    /// of outstanding suspends.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses.
    pub fn suspend_time(&self) -> WhpResult<()> {
        sys::suspend_time(self.handle.0)
    }

    /// Start it again, from where [`Partition::suspend_time`] stopped it.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses — which it does
    /// for a resume with no matching suspend.
    pub fn resume_time(&self) -> WhpResult<()> {
        sys::resume_time(self.handle.0)
    }
}

/// One virtual processor of a partition, owned by the thread that runs it.
///
/// Not `Copy` and not `Sync`, on purpose: every verb that reads or writes a
/// processor's state is here and nowhere else, so "only the vCPU thread
/// touches VP state" — `WHV_E_INVALID_VP_STATE` is what the platform answers
/// the alternative with — is a property of the type rather than a convention.
/// A handle therefore MOVES into the thread that runs the processor and
/// cannot be shared with a second one. What other threads may do to a running
/// processor is exactly the three copyable tokens: [`Canceller`],
/// [`InterruptRequester`] and [`VpCounters`].
///
/// # Lifetime obligation
///
/// A `Vcpu` must not outlive its [`Partition`], for the reason
/// [`Partition::canceller`] gives: it names a handle the platform reclaims
/// when the partition is deleted, and a verb issued after that is refused or,
/// worse, reaches a partition created since. No borrow can say so — a handle
/// moved into a thread outlives every borrow it could have carried — so an
/// owner that moves one onto a thread discharges the obligation by JOINING that
/// thread before the partition drops.
///
/// Nothing in this workspace moves one onto a thread today. The engine crate's
/// `Started` holds its `Vcpu` and its `Partition` in that order, and a struct
/// drops its fields in declaration order, so the handle dies before the
/// partition it names without anyone having to remember. An owner that does
/// hand a `Vcpu` to a thread takes the joining obligation on with it.
///
/// A processor moves to the thread that runs it:
/// ```
/// # use rusty_box_whp::{Exit, Vcpu, WhpResult};
/// fn onto_the_vcpu_thread(vcpu: Vcpu) -> std::thread::JoinHandle<WhpResult<Exit>> {
///     std::thread::spawn(move || vcpu.run())
/// }
/// ```
///
/// and cannot be shared with a second one:
/// ```compile_fail
/// # use rusty_box_whp::Vcpu;
/// fn share(vcpu: &Vcpu) {
///     std::thread::scope(|scope| {
///         scope.spawn(|| vcpu.index());
///     });
/// }
/// ```
#[derive(Debug)]
pub struct Vcpu {
    handle: RawPartition,
    index: u32,
    /// `Cell<T>` is `Send` and not `Sync`, which is exactly the pair of
    /// answers this handle needs, and a zero-sized marker is how a type
    /// borrows them without borrowing the cell.
    one_thread: PhantomData<Cell<()>>,
}

impl Vcpu {
    /// Which processor of its partition this is.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// A handle that can interrupt this processor from another thread.
    ///
    /// Carries the lifetime obligation [`Partition::canceller`] states.
    #[must_use]
    pub const fn canceller(&self) -> Canceller {
        Canceller { handle: self.handle, index: self.index }
    }

    /// A copyable token for this processor's counter reads, usable from any
    /// thread while the processor runs.
    #[must_use]
    pub const fn counters(&self) -> VpCounters {
        VpCounters { handle: self.handle, index: self.index }
    }

    /// Run the processor until it exits.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the run itself is refused; an
    /// exit is a success, whatever its reason.
    pub fn run(&self) -> WhpResult<Exit> {
        sys::run_vp(self.handle, self.index)
    }

    /// Ask the processor to leave a run it is not currently in — a cancel the
    /// owning thread issues for itself, which the platform makes sticky and
    /// the next run then observes.
    ///
    /// [`Canceller`] is how ANOTHER thread ends a run in progress; this thread
    /// cannot be both inside [`Vcpu::run`] and here.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses.
    pub fn cancel_run(&self) -> WhpResult<()> {
        sys::cancel_vp(self.handle, self.index)
    }

    /// Read word-shaped registers.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if the two slices disagree in
    /// length, otherwise [`crate::WhpErrorKind::Platform`].
    pub fn read_regs(&self, regs: &[Reg], out: &mut [u64]) -> WhpResult<()> {
        sys::get_words(self.handle, self.index, regs, out)
    }

    /// Read one word-shaped register.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn read_reg(&self, reg: Reg) -> WhpResult<u64> {
        let mut out = [0u64; 1];
        sys::get_words(self.handle, self.index, &[reg], &mut out)?;
        Ok(out[0])
    }

    /// Write word-shaped registers.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn write_regs(&self, regs: &[Reg], words: &[u64]) -> WhpResult<()> {
        sys::set_words(self.handle, self.index, regs, words)
    }

    /// Write one word-shaped register.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn write_reg(&self, reg: Reg, word: u64) -> WhpResult<()> {
        sys::set_words(self.handle, self.index, &[reg], &[word])
    }

    /// Read the processor's whole extended-state area — the x87 and vector
    /// file as the architecture's own XSAVE layout — answering how many bytes
    /// the platform wrote.
    ///
    /// The one shape the platform offers that file in: its register names stop
    /// at the XMM halves, so the YMM and ZMM state crosses here or not at all.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses — a buffer
    /// too small for the area is one such refusal.
    pub fn read_xsave(&self, out: &mut [u8]) -> WhpResult<usize> {
        sys::get_xsave(self.handle, self.index, out)
    }

    /// Write the processor's whole extended-state area — the counterpart of
    /// [`Vcpu::read_xsave`], taking the same layout back.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses the area.
    pub fn write_xsave(&self, area: &[u8]) -> WhpResult<()> {
        sys::set_xsave(self.handle, self.index, area)
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
    /// As [`Vcpu::read_regs`], and a contract error if the slices disagree in
    /// length or exceed one call's worth.
    pub fn read_registers(&self, regs: &[Reg], out: &mut [RegisterValue]) -> WhpResult<()> {
        sys::get_registers(self.handle, self.index, regs, out)
    }

    /// Write registers of mixed shape in a single call. The counterpart of
    /// [`Vcpu::read_registers`].
    ///
    /// # Errors
    /// As [`Vcpu::read_registers`].
    pub fn write_registers(&self, regs: &[Reg], values: &[RegisterValue]) -> WhpResult<()> {
        sys::set_registers(self.handle, self.index, regs, values)
    }

    /// Read segment registers.
    ///
    /// Separate from [`Vcpu::read_regs`] because the platform keeps a segment
    /// in a different member of its value union: reading one as a word yields
    /// its base and silently drops the limit, selector and attributes.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn read_segments(&self, regs: &[Reg], out: &mut [SegmentRegister]) -> WhpResult<()> {
        sys::get_segments(self.handle, self.index, regs, out)
    }

    /// Write segment registers.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn write_segments(
        &self,
        regs: &[Reg],
        segments: &[SegmentRegister],
    ) -> WhpResult<()> {
        sys::set_segments(self.handle, self.index, regs, segments)
    }

    /// Read the descriptor-table registers, `GDTR` and `IDTR`.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn read_tables(&self, regs: &[Reg], out: &mut [TableRegister]) -> WhpResult<()> {
        sys::get_tables(self.handle, self.index, regs, out)
    }

    /// Write the descriptor-table registers.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn write_tables(&self, regs: &[Reg], tables: &[TableRegister]) -> WhpResult<()> {
        sys::set_tables(self.handle, self.index, regs, tables)
    }

    /// Hand the processor an interrupt, NMI or exception to take at its next
    /// opportunity, bypassing any emulated APIC.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn inject(&self, event: PendingInterruption) -> WhpResult<()> {
        self.write_reg(Reg::PendingInterruption, event.as_word())
    }

    /// Why the processor is not executing.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn internal_activity(&self) -> WhpResult<InternalActivity> {
        Ok(InternalActivity::from_word(
            self.read_reg(Reg::InternalActivityState)?,
        ))
    }

    /// Set the processor's activity state — clearing a halt suspend, for
    /// instance.
    ///
    /// # Errors
    /// As [`Vcpu::read_regs`].
    pub fn set_internal_activity(&self, activity: InternalActivity) -> WhpResult<()> {
        self.write_reg(Reg::InternalActivityState, activity.as_word())
    }

    /// Translate a guest linear address through the guest's own paging.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the call itself is refused; a
    /// failed translation is a success carrying a non-zero result code.
    pub fn translate_gva(&self, gva: u64) -> WhpResult<GvaTranslation> {
        sys::translate_gva(self.handle, self.index, gva)
    }

    /// Read the processor's local-APIC state into `page`.
    ///
    /// The whole model in one blob, which is the only shape the platform
    /// offers it in: an offloaded APIC has no register names in
    /// `WHV_REGISTER_NAME`, so its state crosses here or not at all. See
    /// [`ApicStatePage`] for how a register's offset maps into the page.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses — which it does
    /// when the partition has no APIC to have state.
    pub fn read_apic_state(&self, page: &mut ApicStatePage) -> WhpResult<()> {
        const CALL: &str = "WHvGetVirtualProcessorState(InterruptControllerState2)";
        let written = sys::get_vp_state(
            self.handle,
            self.index,
            VpStateType::InterruptControllerState2,
            page.0.as_mut_slice(),
        )?;
        // A short answer would leave the tail of the page holding whatever the
        // caller's buffer held before, which a later write would hand back to
        // the platform as if it were state the platform itself produced.
        if written != ApicStatePage::BYTES {
            return Err(WhpError::contract(CALL));
        }
        Ok(())
    }

    /// Write the processor's local-APIC state back — the counterpart of
    /// [`Vcpu::read_apic_state`], taking the same page at the same size.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the host refuses the page.
    pub fn write_apic_state(&self, page: &ApicStatePage) -> WhpResult<()> {
        sys::set_vp_state(
            self.handle,
            self.index,
            VpStateType::InterruptControllerState2,
            page.0.as_slice(),
        )
    }

    /// Read a 128-bit register — today, [`crate::Reg::PendingEvent`].
    ///
    /// Separate from [`Vcpu::read_reg`] because the platform keeps the whole
    /// sixteen bytes and a word read would return the low half and drop the
    /// rest silently. Which registers those are is [`crate::shape_of`]'s
    /// answer, not the caller's, so asking for a word-shaped register here is
    /// refused rather than misread.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if `reg` is not 128 bits wide,
    /// otherwise [`crate::WhpErrorKind::Platform`].
    pub fn read_words128(&self, reg: Reg) -> WhpResult<[u64; 2]> {
        const CALL: &str = "WHvGetVirtualProcessorRegisters(128-bit)";
        let mut out = [RegisterValue::Words128([0; 2])];
        sys::get_registers(self.handle, self.index, &[reg], &mut out)?;
        match out[0] {
            RegisterValue::Words128(words) => Ok(words),
            RegisterValue::Word(_) | RegisterValue::Segment(_) | RegisterValue::Table(_) => {
                Err(WhpError::contract(CALL))
            }
        }
    }

    /// Write a 128-bit register — the counterpart of [`Vcpu::read_words128`].
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Contract`] if `reg` is not 128 bits wide,
    /// otherwise [`crate::WhpErrorKind::Platform`].
    pub fn write_words128(&self, reg: Reg, words: [u64; 2]) -> WhpResult<()> {
        const CALL: &str = "WHvSetVirtualProcessorRegisters(128-bit)";
        if !reg.is_words128() {
            return Err(WhpError::contract(CALL));
        }
        sys::set_registers(
            self.handle,
            self.index,
            &[reg],
            &[RegisterValue::Words128(words)],
        )
    }
}

/// The hypervisor's own accounting for one processor, readable from any
/// thread.
///
/// Copyable and `Sync` because neither read touches processor state: they ask
/// the hypervisor what it has charged the processor, which is a question a
/// supervising thread may ask while the guest is running — and the only
/// question about a running processor that this crate answers off its own
/// thread.
///
/// Carries the lifetime obligation [`Partition::canceller`] states.
#[derive(Clone, Copy, Debug)]
pub struct VpCounters {
    handle: RawPartition,
    index: u32,
}

impl VpCounters {
    /// Which processor these count.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
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
    pub fn intercept_counters(&self) -> WhpResult<InterceptCounters> {
        const CALL: &str = "WHvGetVirtualProcessorCounters(Intercepts)";
        let mut words = [0u64; InterceptCounters::WORDS];
        let written =
            sys::get_counters(self.handle, self.index, sys::CounterSet::Intercepts, &mut words)?;
        InterceptCounters::from_words(&words[..filled_words(written, words.len())])
            .ok_or(WhpError::contract(CALL))
    }

    /// How long the processor has run, and how much of that went to the
    /// hypervisor rather than to the guest.
    ///
    /// # Errors
    /// As [`VpCounters::intercept_counters`].
    pub fn runtime_counters(&self) -> WhpResult<RuntimeCounters> {
        const CALL: &str = "WHvGetVirtualProcessorCounters(Runtime)";
        let mut words = [0u64; RuntimeCounters::WORDS];
        let written =
            sys::get_counters(self.handle, self.index, sys::CounterSet::Runtime, &mut words)?;
        RuntimeCounters::from_words(&words[..filled_words(written, words.len())])
            .ok_or(WhpError::contract(CALL))
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
/// thread — unlike [`Vcpu::inject`], whose register write requires the
/// processor to be stopped, and so belongs to the thread that owns it.
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

    /// A partition and its one processor, the processor pointed at a single
    /// `HLT`.
    ///
    /// Both halves, because the map lives on one and the register file on the
    /// other and a measurement guest needs each.
    #[cfg(test)]
    #[derive(Debug)]
    struct HaltingGuest {
        vcpu: Vcpu,
        partition: Partition,
    }

    /// A partition holding one processor pointed at a single `HLT`.
    ///
    /// The smallest thing that proves a partition is live, and the memory is
    /// mapped because mapping is where the platform materialises the partition
    /// underneath — a fact the test below exists to record.
    #[cfg(test)]
    fn a_halting_guest() -> WhpResult<HaltingGuest> {
        let mut config = PartitionConfig::new()?;
        config.processor_count(1)?.local_apic(LocalApicMode::None)?;
        let mut partition = config.setup()?;

        let mut pages = HostPages::new(2)?;
        pages.bytes_mut()[GUEST_IP as usize] = 0xF4;
        partition.map(RESET_CS_BASE, pages, GpaPerms::RWX)?;
        partition.create_processor(0)?;
        let vcpu = partition.take_vcpu(0)?;
        vcpu.write_reg(Reg::Rip, GUEST_IP)?;
        Ok(HaltingGuest { vcpu, partition })
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

    /// Put `code` where the processor is pointed, and point it there again.
    ///
    /// The map is the partition's and the entry point is the processor's, so
    /// loading a guest needs both halves — which is the whole of why this is
    /// a function rather than a method on either.
    #[cfg(test)]
    fn load_real_mode_code(
        partition: &mut Partition,
        vcpu: &Vcpu,
        code: &[u8],
    ) -> WhpResult<()> {
        let start = GUEST_IP as usize;
        let memory = partition.bytes_at_mut(RESET_CS_BASE).expect("the mapped guest page");
        memory[start..start + code.len()].copy_from_slice(code);
        vcpu.write_reg(Reg::Rip, GUEST_IP)
    }

    /// Put `code` where the processor is pointed and run until the guest halts.
    ///
    /// Port accesses are stepped over rather than serviced: no device is behind
    /// them, and what a caller here is measuring is the platform's own account
    /// of the access. The platform leaves `RIP` on the trapping instruction for
    /// an I/O exit, so advancing it by the exit's instruction length is what
    /// lets the guest continue.
    #[cfg(test)]
    fn run_until_halt_with(guest: &mut HaltingGuest, code: &[u8]) -> WhpResult<()> {
        load_real_mode_code(&mut guest.partition, &guest.vcpu, code)?;

        for _ in 0..EXIT_CEILING {
            let exit = guest.vcpu.run()?;
            match exit.reason {
                crate::ExitReason::Halt => return Ok(()),
                crate::ExitReason::IoPortAccess(_) => {
                    guest.vcpu.write_reg(Reg::Rip, exit.rip_after_instruction())?;
                }
                other => panic!("the measurement guest took an unexpected exit: {other:?}"),
            }
        }
        panic!("the measurement guest ran past {EXIT_CEILING} exits without halting");
    }

    /// How long the clock tests hold real time still while watching the
    /// partition's own. Five milliseconds is fifty thousand of the clock's
    /// 100-nanosecond units, so a clock that is running cannot be mistaken for
    /// one that is held, whatever the host's scheduling does to the delay.
    #[cfg(test)]
    const CLOCK_WINDOW: std::time::Duration = std::time::Duration::from_millis(5);

    /// Let real time pass without yielding the processor.
    ///
    /// A sleep would hand the thread back to the host scheduler, which is what
    /// this measurement is trying to hold constant; the point is to observe the
    /// PARTITION's clock across a known stretch of real time.
    #[cfg(test)]
    fn while_real_time_passes() {
        let until = std::time::Instant::now();
        while until.elapsed() < CLOCK_WINDOW {
            std::hint::spin_loop();
        }
    }

    /// The partition's reference clock starts when a processor first runs, and
    /// a suspend holds it still.
    ///
    /// The guest-visible property the two verbs exist for (R9): a host that
    /// suspends partition time and then does something slow must not have the
    /// guest see the delay. Reading the clock through a partition property is
    /// also the only observation of it a host outside the guest has.
    ///
    /// The first assertion is a platform fact worth pinning on its own — the
    /// clock counts the partition's OWN time, so it reads zero until a
    /// processor has run, and a host that anchored a guest's timebase to it
    /// before the first run would anchor to nothing.
    #[test]
    fn the_partitions_reference_clock_starts_with_the_guest_and_a_suspend_holds_it() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let guest = a_halting_guest().expect("a partition");
        let partition = &guest.partition;

        assert_eq!(
            partition.reference_time_100ns().expect("the reference clock is readable"),
            0,
            "a partition whose processor has never run has accumulated no time"
        );
        let exit = guest.vcpu.run().expect("the guest runs");
        assert!(matches!(exit.reason, crate::ExitReason::Halt), "{:?}", exit.reason);

        let started = partition.reference_time_100ns().expect("readable after the run");
        assert!(started > 0, "running the guest started the partition's clock");
        while_real_time_passes();
        let running = partition.reference_time_100ns().expect("readable again");
        assert!(
            running > started,
            "the clock advances with real time once started: {started} then {running}"
        );

        partition.suspend_time().expect("partition time suspends");
        let held = partition.reference_time_100ns().expect("readable while suspended");
        while_real_time_passes();
        assert_eq!(
            partition.reference_time_100ns().expect("readable while suspended"),
            held,
            "a suspended partition's clock does not advance, however long the host takes"
        );

        partition.resume_time().expect("partition time resumes");
        while_real_time_passes();
        let resumed = partition.reference_time_100ns().expect("readable after the resume");
        assert!(
            resumed > held,
            "the clock picks up from where it was held: {held} then {resumed}"
        );
    }

    /// The host names the enlightenments it will grant, and a partition that
    /// asks for exactly those is accepted.
    ///
    /// Both halves matter: the capability read is banked rather than a word, so
    /// a wrong parse would report the bank COUNT as a feature set, and the
    /// property is a 16-byte structure whose first word is that same count.
    #[test]
    fn a_partition_may_ask_for_the_synthetic_features_the_host_allows() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let caps = crate::capabilities().expect("the host's capabilities");
        let allowed = caps.synthetic_features;
        assert!(
            allowed.contains(crate::SyntheticFeatures::HYPERVISOR_PRESENT),
            "a host running WHP allows a partition to report a hypervisor: {allowed:?}"
        );
        assert!(
            caps.processor_clock_hz > 0,
            "the platform reports how fast its processor clock runs"
        );
        assert!(
            caps.interrupt_clock_hz > 0,
            "and how fast its interrupt clock runs"
        );
        println!(
            "P2: synthetic bank 0 = {:#x}, unnamed bits {:#x}; processor clock {} Hz, \
             interrupt clock {} Hz, TSC deadline timer {}",
            allowed.bits(),
            allowed.bits() & !crate::SyntheticFeatures::all().bits(),
            caps.processor_clock_hz,
            caps.interrupt_clock_hz,
            caps.tsc_deadline_timer,
        );

        let wanted = crate::SyntheticFeatures::OPENVMM_VTL0.intersection(allowed);
        let mut config = PartitionConfig::new().expect("a partition");
        config
            .processor_count(1)
            .expect("one processor")
            .synthetic_features(wanted)
            .expect("the host accepts the bank it just said it allows");
    }

    /// The 128-bit pending-event slot takes an ExtINT and gives it back.
    ///
    /// The register the design needs and the word exchange cannot carry: read
    /// as a word it would return its low half, which happens to contain the
    /// whole encoding and so would look right until a field moved above bit 64.
    #[test]
    fn the_pending_event_slot_carries_an_ext_int_whole() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let guest = a_halting_guest().expect("a partition");
        assert_eq!(
            guest.vcpu.read_words128(Reg::PendingEvent).expect("the slot is readable"),
            [0; 2],
            "a processor that has never run holds no pending event"
        );

        let event = crate::PendingExtIntEvent { vector: 0x20 };
        guest
            .vcpu
            .write_words128(Reg::PendingEvent, event.as_words())
            .expect("the platform accepts an ExtINT in the pending-event slot");
        let read_back =
            guest.vcpu.read_words128(Reg::PendingEvent).expect("readable after the write");
        assert_eq!(
            crate::PendingExtIntEvent::from_words(read_back),
            Some(event),
            "the vector and the event type survive the platform: {read_back:?}"
        );

        // A word-shaped verb must refuse the register rather than truncate it.
        assert_eq!(
            guest
                .vcpu
                .write_words128(Reg::Rax, [0; 2])
                .expect_err("RAX is not 128 bits wide")
                .kind(),
            crate::WhpErrorKind::Contract,
        );
    }

    /// The state page round-trips through the platform unchanged, at its exact
    /// size — and its layout is what [`ApicStatePage`] claims.
    ///
    /// The layout cannot be checked against the SDK: `WinHvPlatformDefs.h`
    /// declares no x64 structure for this state type at all. So it is checked
    /// against the platform, which is the stronger oracle anyway. A fresh
    /// processor's APIC is at its architectural reset state, and three of those
    /// values are distinctive enough to pin the field order on their own — a
    /// version word reporting six LVT entries, a destination format of all
    /// ones, a spurious vector of 0xFF — with the six masked LVT entries then
    /// landing exactly where three eight-word bitmaps and the ICR put them.
    /// Under any other stride those bytes would fall somewhere else.
    #[test]
    fn the_apic_state_page_round_trips_through_the_platform() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut config = PartitionConfig::new().expect("a partition");
        config
            .processor_count(1)
            .expect("one processor")
            .local_apic(LocalApicMode::X2Apic)
            .expect("an x2APIC");
        let mut partition = config.setup().expect("setup");
        partition.create_processor(0).expect("a processor");
        let vcpu = partition.take_vcpu(0).expect("the processor");
        let mut page = ApicStatePage::zeroed();
        vcpu.read_apic_state(&mut page)
            .expect("a fresh processor's APIC page is readable");

        let version = page.register(crate::ApicRegister::Version);
        assert_ne!(
            version, 0,
            "a fresh VP's version register is populated; the page is not a zero buffer"
        );
        // Bits 0..8 are the version and 16..24 the highest LVT index, so a
        // hypervisor APIC with the six architectural entries reports 5 here.
        let lvt_entries = (version >> 16 & 0xFF) + 1;
        assert_eq!(
            page.register(crate::ApicRegister::Dfr),
            u32::MAX,
            "the destination format register resets to all ones"
        );
        assert_eq!(
            page.register(crate::ApicRegister::Spurious),
            0xFF,
            "the spurious vector resets to 0xFF with the APIC software-disabled"
        );
        // Each LVT entry resets masked, and there are exactly as many of them
        // as the version register just said — which is only true if the three
        // interrupt bitmaps sit between the spurious vector and the ICR.
        for lvt in [
            crate::ApicRegister::LvtTimer,
            crate::ApicRegister::LvtThermal,
            crate::ApicRegister::LvtPerfmon,
            crate::ApicRegister::LvtLint0,
            crate::ApicRegister::LvtLint1,
            crate::ApicRegister::LvtError,
        ] {
            assert_eq!(
                page.register(lvt),
                1 << 16,
                "{lvt:?} resets masked and nothing else"
            );
        }
        assert_eq!(lvt_entries, 6, "the version register agrees with the six entries above");
        assert_eq!(
            page.vector(crate::ApicVector::Request),
            [0; 8],
            "nothing is pending on a processor that has never run"
        );
        // Recorded for the probe document, not asserted: the version a host's
        // offloaded APIC reports is the host's to choose.
        println!("P2: hypervisor LAPIC version register = {version:#x}");

        vcpu.write_apic_state(&page)
            .expect("the same page is accepted back at its exact size");
        let mut back = ApicStatePage::zeroed();
        vcpu.read_apic_state(&mut back).expect("readable again");
        assert_eq!(
            page.0[..1024],
            back.0[..1024],
            "the first KiB — every register the model owns — survives the round trip"
        );
    }

    /// Where the request bitmap sits, measured rather than transcribed.
    ///
    /// The reset-state evidence pins words 0, 1, 3, 4 and 32..38 by value and
    /// forces words 5..32 to be a 27-word run, but says nothing about what is
    /// *inside* that run: the three bitmaps could be in any order and the
    /// `Esr`/`IcrHigh`/`IcrLow` group could have its two ICR halves either way
    /// round. This test makes the platform write one bit into the middle of the
    /// run and checks it lands where [`ApicVector::Request`] says, which pins
    /// that bitmap by observation and leaves the other two only their two
    /// remaining slots.
    ///
    /// The vector is deliberately 0x41 rather than something under 32: word
    /// `0x41 / 32` is 2, so the bit lands in the third word of the bitmap and a
    /// reading whose words ran the other way would put it somewhere else.
    #[test]
    fn a_requested_vector_appears_in_the_pages_request_bitmap() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut config = PartitionConfig::new().expect("a partition");
        config
            .processor_count(1)
            .expect("one processor")
            .local_apic(LocalApicMode::X2Apic)
            .expect("an x2APIC");
        let mut partition = config.setup().expect("setup");
        // Mapping is where the platform materialises the partition underneath,
        // so an APIC request before it has nothing to arbitrate against.
        let pages = HostPages::new(1).expect("one page");
        partition.map(RESET_CS_BASE, pages, GpaPerms::RWX).expect("a mapped page");
        partition.create_processor(0).expect("a processor");
        let vcpu = partition.take_vcpu(0).expect("the processor");

        // An APIC resets software-disabled — SVR is 0xFF, with the enable bit 8
        // clear — and a disabled APIC drops a fixed vector instead of latching
        // it. `whp_probe`'s wake measurement records the same thing from the
        // guest's side: a real-mode guest never enables its APIC and never sees
        // the vector, while an NMI still arrives. So the page has to be written
        // back software-enabled before the request has anywhere to land.
        let mut page = ApicStatePage::zeroed();
        vcpu.read_apic_state(&mut page).expect("the APIC page is readable");
        page.set_register(crate::ApicRegister::Spurious, 0x1FF);
        vcpu.write_apic_state(&page).expect("the APIC accepts being enabled");

        const VECTOR: u32 = 0x41;
        partition
            .request_interrupt(InterruptRequest {
                kind: crate::InterruptKind::Fixed,
                destination_mode: crate::DestinationMode::Physical,
                trigger_mode: crate::TriggerMode::Edge,
                destination: 0,
                vector: VECTOR,
            })
            .expect("the emulated APIC accepts a fixed vector for processor 0");

        vcpu.read_apic_state(&mut page).expect("the APIC page is readable again");

        // The processor has never run, so nothing has moved the vector out of
        // the request bitmap and into the in-service one.
        let mut expected = [0u32; 8];
        expected[VECTOR as usize / 32] = 1 << (VECTOR % 32);
        assert_eq!(
            page.vector(crate::ApicVector::Request),
            expected,
            "the requested vector sits at bit {} of word {} of the request bitmap",
            VECTOR % 32,
            VECTOR / 32
        );
        assert_eq!(
            page.vector(crate::ApicVector::InService),
            [0; 8],
            "a processor that has never run has accepted nothing"
        );
        assert_eq!(
            page.vector(crate::ApicVector::TriggerMode),
            [0; 8],
            "the request was edge-triggered, so no trigger-mode bit is set"
        );
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
        let first = a_halting_guest().expect("the first partition");
        let refused = a_halting_guest().expect_err("a second live partition");
        assert_eq!(
            refused.kind(),
            crate::WhpErrorKind::Platform,
            "the second partition must be refused BY THE PLATFORM, and at the map that \
             materialises it: {refused}"
        );

        // Dropping the first releases the name, so partitions are serially
        // reusable within one process even though they do not coexist.
        drop(first);
        let again = a_halting_guest().expect("a partition after the first is gone");
        let exit = again.vcpu.run().expect("the guest runs");
        assert!(
            matches!(exit.reason, crate::ExitReason::Halt),
            "the guest must reach its HLT, not {:?}",
            exit.reason
        );
    }

    /// The extended-state area round-trips through the platform, and the
    /// legacy region sits where the architecture says it does.
    ///
    /// This is the contract the engine's state exchange rests on: the area a
    /// `read_xsave` returns can be patched and handed back through
    /// `write_xsave`, and a patch to the legacy `XMM0` slot (offset 160, the
    /// same in the standard and the compacted layout) is a write to that
    /// register. The header prints rather than asserts — whether this host
    /// hands out the standard or the compacted form is measured here, not
    /// assumed anywhere.
    #[test]
    fn the_extended_state_area_round_trips_and_a_legacy_patch_is_a_register_write() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let guest = a_halting_guest().expect("a partition");

        let mut area = [0u8; 4096];
        let written = guest.vcpu.read_xsave(&mut area).expect("the area reads");
        assert!(
            written >= 576,
            "an XSAVE area is at least the legacy region plus its header, got {written}"
        );

        let xstate_bv = u64::from_le_bytes(area[512..520].try_into().expect("eight bytes"));
        let xcomp_bv = u64::from_le_bytes(area[520..528].try_into().expect("eight bytes"));
        eprintln!(
            "measured: {written} bytes, XSTATE_BV {xstate_bv:#x}, XCOMP_BV {xcomp_bv:#x} \
             ({} form)",
            if xcomp_bv >> 63 != 0 { "compacted" } else { "standard" }
        );

        // Identical put-back first: the exchange's common case.
        guest.vcpu.write_xsave(&area[..written]).expect("the unchanged area writes");

        // Now the patch: XMM0's sixteen legacy bytes, with the SSE component
        // marked live so the platform treats them as state rather than init.
        const XMM0: usize = 160;
        const SSE_LIVE: u64 = 1 << 1;
        let pattern: [u8; 16] = *b"\xA5\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0A\x0B\x0C\x0D\x0E\x5A";
        area[XMM0..XMM0 + 16].copy_from_slice(&pattern);
        let marked = (xstate_bv | SSE_LIVE).to_le_bytes();
        area[512..520].copy_from_slice(&marked);
        guest.vcpu.write_xsave(&area[..written]).expect("the patched area writes");

        let mut back = [0u8; 4096];
        let again = guest.vcpu.read_xsave(&mut back).expect("the area reads back");
        assert!(again >= 576);
        assert_eq!(
            back[XMM0..XMM0 + 16],
            pattern,
            "the patched XMM0 must come back as written, or the legacy region is not \
             where the exchange thinks it is"
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
        let mut guest = a_halting_guest().expect("a partition");
        let counters = guest.vcpu.counters();

        let before = counters
            .intercept_counters()
            .expect("a created processor reports its counters");

        // out 0xE9, al ; out 0xE9, al ; out 0xE9, al ; hlt
        run_until_halt_with(&mut guest, &[0xE6, 0xE9, 0xE6, 0xE9, 0xE6, 0xE9, 0xF4])
            .expect("the guest runs");

        let after = counters
            .intercept_counters()
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
        let mut guest = a_halting_guest().expect("a partition");
        run_until_halt_with(&mut guest, &[0xE6, 0xE9, 0xF4]).expect("the guest runs");

        let runtime = guest
            .partition
            .vp_counters(0)
            .expect("processor 0 exists")
            .runtime_counters()
            .expect("a processor that has run");
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

    /// The host banks processor features, and a partition told exactly that
    /// bank sets up and runs a guest.
    ///
    /// The single-word `ProcessorFeatures` property is the form under test:
    /// the platform also offers a multi-bank form, and this asserts the
    /// one-word one is accepted end to end — property set, `setup`, and a
    /// guest reaching its `HLT` on the hardware. The bank is printed rather
    /// than asserted bit by bit because which features a host has is a fact
    /// about the host, not about this port.
    #[test]
    fn a_partition_accepts_the_processor_features_the_host_banks() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();

        let features = crate::capabilities()
            .expect("a host with a hypervisor answers its capability queries")
            .processor_features;
        assert_ne!(
            features, 0,
            "a host with a hypervisor banks at least one processor feature"
        );
        eprintln!("measured: WHV_PROCESSOR_FEATURES {features:#018x}");

        let mut config = PartitionConfig::new().expect("a configurable partition");
        config
            .processor_count(1)
            .expect("the processor count")
            .processor_features(features)
            .expect("the single-word ProcessorFeatures property, set to the host's own bank")
            .local_apic(LocalApicMode::None)
            .expect("no emulated APIC");
        let mut partition =
            config.setup().expect("setup succeeds with the banked features offered");

        let mut pages = HostPages::new(2).expect("guest pages");
        pages.bytes_mut()[GUEST_IP as usize] = 0xF4;
        partition.map(RESET_CS_BASE, pages, GpaPerms::RWX).expect("the guest map");
        partition.create_processor(0).expect("the processor");
        let vcpu = partition.take_vcpu(0).expect("the processor is handed out");
        vcpu.write_reg(Reg::Rip, GUEST_IP).expect("the entry point");
        let exit = vcpu.run().expect("the guest runs");
        assert!(
            matches!(exit.reason, crate::ExitReason::Halt),
            "the guest must reach its HLT under the widened feature set, not {:?}",
            exit.reason
        );
    }

    /// A processor runs from a thread that is not the one holding the
    /// partition, and is handed out exactly once.
    ///
    /// The guest-visible property the split exists for (R9): the map stays
    /// reachable here while the guest executes elsewhere, which is what a VMM
    /// shape needs and what a partition-shaped `run` cannot offer. The refusals
    /// are the other half — a second handle to one processor is what
    /// `WHV_E_INVALID_VP_STATE` exists to punish, and a handle to a processor
    /// that was never created names nothing at all.
    #[test]
    fn a_vcpu_runs_on_another_thread_while_the_partition_is_held_here() {
        if !crate::hypervisor_present().unwrap_or(false) {
            eprintln!("skipped: this host has no Windows Hypervisor Platform");
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let HaltingGuest { vcpu, mut partition } = a_halting_guest().expect("a partition");
        load_real_mode_code(&mut partition, &vcpu, &[0xF4]).expect("hlt at the entry point");

        let ran = std::thread::spawn(move || vcpu.run())
            .join()
            .expect("the vCPU thread finished")
            .expect("the guest ran");
        assert!(
            matches!(ran.reason, crate::ExitReason::Halt),
            "the guest halted on the other thread: {ran:?}"
        );
        assert_eq!(
            partition.mapped_regions(),
            1,
            "the partition was usable here throughout"
        );
        assert_eq!(
            partition.take_vcpu(0).expect_err("a processor is handed out once").kind(),
            crate::WhpErrorKind::Contract,
            "a second handle to one processor is refused by this crate, not by the platform"
        );
        assert_eq!(
            partition.take_vcpu(1).expect_err("processor 1 was never created").kind(),
            crate::WhpErrorKind::Contract,
            "a handle to a processor that does not exist is refused before the platform \
             is asked"
        );
        // The two cross-thread capabilities the partition hands out answer the
        // same way, so a caller cannot name a processor that does not exist by
        // going around `take_vcpu`.
        assert_eq!(
            partition.canceller(1).expect_err("processor 1 was never created").kind(),
            crate::WhpErrorKind::Contract,
        );
        assert_eq!(
            partition.vp_counters(1).expect_err("processor 1 was never created").kind(),
            crate::WhpErrorKind::Contract,
        );
        assert!(
            partition.canceller(0).is_ok(),
            "a created processor still yields a canceller after its handle is taken"
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
