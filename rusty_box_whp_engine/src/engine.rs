//! The engine that runs a slice on the host's hypervisor.
//!
//! ## Why a shadow processor is not an optimisation
//!
//! Measured on this host (`docs/whp-platform-probe-2026-08-27.md`): a memory
//! exit reports `InstructionLength = 0`, does not advance `RIP`, and for a
//! write to a read-only window carries no instruction bytes either. There is
//! therefore no way to finish a trapped access by stepping over it, and no
//! decoder-free variant of this design to fall back on. Every exit the platform
//! cannot complete itself is completed by executing ONE instruction on this
//! port's own processor, through the same `ExecCtx` the interpreter uses — which
//! is also what keeps a trapped access bit-identical to what Bochs would do.
//!
//! ## Why time comes from the host
//!
//! The hypervisor retires instructions this port never sees, so the shadow's
//! `icount` stands still while the guest runs. What did pass is host time, so
//! the slice reports [`Progress::Ticks`], converted at the machine's own
//! instructions-per-second rate — the same rate the software engine's ticks are
//! denominated in, which is what lets a snapshot cross between them.

use core::time::Duration;

use rusty_box_whp::{
    Exit, ExitReason, ExtendedVmExits, InterceptCounters, InterruptionType, LocalApicMode,
    MsrExits, Partition, PartitionConfig, PendingInterruption, Reg, RuntimeCounters, VpContext,
    WhpError,
};

use super::alarm::Alarm;
use super::state::{self, VpRegisters};
use super::xsave::{self, XsaveArea};
use super::Vp;
use rusty_box::cpu::arch_state::VcpuArchState;
use rusty_box::cpu::{
    cpu::{BxCpuC, CpuActivityState},
    instrumentation::Instrumentation,
    CpuError, Result,
};
use rusty_box::emulator::{
    EventDelivery, PcIo, Progress, ProgressUnit, SliceEngine, SliceRequest,
};
use rusty_box::memory::plan::MemoryPlan;
use rusty_box::memory::BxMemC;
use rusty_box::GpaWindow;

/// The processor this engine runs. SMP under a hypervisor is its own unit; a
/// machine with more processors than this refuses to start rather than running
/// one and pretending.
const BOOT_VP: u32 = 0;

/// RFLAGS.TF, the trap flag. A guest with it set owes a single-step `#DB`
/// after every instruction — including one this engine finishes for it.
const RFLAGS_TF: u64 = 1 << 8;

/// Every `CPUID` leaf this engine takes away from the host.
///
/// A leaf that is not here executes on the host's own processor and answers
/// with the host's own values — the model, the stepping, the feature words,
/// and leaf 1's `HypervisorPresent` bit. That is the loudest divergence a
/// guest can hear, and the one that matters most to anything looking for an
/// emulator, so the list is generous rather than minimal: a leaf omitted by
/// oversight is a leaf the host answers.
///
/// The ranges are the architectural ones — the standard leaves, the
/// hypervisor range, the extended leaves and Centaur's — each taken well past
/// what this port implements, because an unimplemented leaf still has a
/// correct answer and it is this port's to give (Bochs `cpuid.cc` returns
/// zeros or the highest supported leaf, never the host's).
const TRAPPED_CPUID_LEAVES: &[u32] = &{
    let mut leaves = [0u32; 0x20 + 0x10 + 0x20 + 0x02];
    let mut at = 0;
    let mut leaf = 0x0000_0000u32;
    while leaf < 0x0000_0020 {
        leaves[at] = leaf;
        at += 1;
        leaf += 1;
    }
    leaf = 0x4000_0000;
    while leaf < 0x4000_0010 {
        leaves[at] = leaf;
        at += 1;
        leaf += 1;
    }
    leaf = 0x8000_0000;
    while leaf < 0x8000_0020 {
        leaves[at] = leaf;
        at += 1;
        leaf += 1;
    }
    leaves[at] = 0xC000_0000;
    leaves[at + 1] = 0xC000_0001;
    leaves
};

/// Which model-specific register accesses this engine takes away from the
/// platform.
///
/// Setting the MSR exit bit alone traps nothing: the platform answers a fixed
/// handful of MSRs itself and hands over only what this names — a fact that
/// cost a test which read `IA32_MISC_ENABLE` and got the host's `0x00851809`
/// while the interpreter answered zero.
///
/// Everything is taken EXCEPT the two time-stamp-counter entries, and that is
/// a decision rather than an oversight. This port derives its TSC from retired
/// instructions, and a shadow processor retires almost none while the guest
/// runs on hardware — so a trapped `RDMSR` of the TSC would answer from a
/// clock that barely moves, while `RDTSC`, which is a separate exit this
/// engine does not request, would keep answering from the host's. Two clocks
/// that disagree is worse for a calibrating guest than one clock that is not
/// this port's, so both stay with the hardware until the cross-engine TSC
/// bridge exists. That is the one MSR divergence this engine has, and it is
/// registered rather than hidden.
const TRAPPED_MSRS: MsrExits = MsrExits {
    tsc_read: false,
    tsc_write: false,
    ..MsrExits::ALL
};

/// The last few exits, kept so a fault can say what led to it.
///
/// A trapped exception reports the instant it happened and nothing about how
/// the guest got there, which for a stack that has drifted out of balance is
/// the only interesting part. Written on every exit and read only when one
/// cannot be serviced, so it costs a store and an increment.
///
/// DIAGNOSTIC, alongside `WHP_TRAP_EXCEPTIONS`, and inert without it.
struct ExitHistory {
    entries: [(u64, u8); Self::DEPTH],
    at: usize,
    seen: usize,
}

impl Default for ExitHistory {
    fn default() -> Self {
        Self { entries: [(0, 0); Self::DEPTH], at: 0, seen: 0 }
    }
}

impl ExitHistory {
    const DEPTH: usize = 64;

    fn record(&mut self, rip: u64, reason: u8) {
        self.entries[self.at] = (rip, reason);
        self.at = (self.at + 1) % Self::DEPTH;
        self.seen += 1;
    }

    /// Oldest first, so the report reads forwards in time.
    fn replay(&self) -> impl Iterator<Item = (u64, u8)> + '_ {
        let held = self.seen.min(Self::DEPTH);
        let first = if self.seen > Self::DEPTH { self.at } else { 0 };
        (0..held).map(move |step| self.entries[(first + step) % Self::DEPTH])
    }
}

/// Which processor exceptions this engine takes away from the guest, as a
/// bitmap of vectors — DIAGNOSTIC, and empty unless asked for.
///
/// `WHP_TRAP_EXCEPTIONS` names them in hexadecimal, so `2000` is `#GP` (vector
/// 13) alone and `6000` is `#GP` and `#PF` together.
///
/// A guest takes exceptions in the course of working: a page fault is how
/// demand paging happens, and Linux uses `#GP` deliberately in places. Trapping
/// one means its own handler never runs, so a boot under this will diverge —
/// which is the point. It answers the question the guest's own crash report
/// cannot: what the processor was doing at the instant of the fault, rather
/// than what the handler could still print once the damage was done.
/// The exceptions the platform hands back instead of delivering to the guest.
///
/// `#GP` always, because the platform answers a handful of model-specific
/// registers itself and refuses writes this port's own processor completes.
/// `IA32_FEATURE_CONTROL` is the one every boot reaches: the firmware sees VMX
/// in `CPUID`, writes the register to lock it, and the platform refuses —
/// in code that has no interrupt descriptor table yet, which is a triple fault
/// before the boot loader has run. Trapped, the write is finished on the
/// shadow instead, and the guest sees what the interpreter would have shown
/// it.
///
/// `WHP_TRAP_EXCEPTIONS` names a different set as a hex vector bitmask — bit
/// 13 is `#GP`, bit 14 is `#PF` — and asks for a full report at each one.
fn trapped_exceptions() -> u64 {
    /// Vector 13, the fault the platform's refusals arrive as.
    const GENERAL_PROTECTION: u64 = 1 << 13;

    match std::env::var("WHP_TRAP_EXCEPTIONS") {
        Ok(vectors) => u64::from_str_radix(vectors.trim_start_matches("0x"), 16)
            .unwrap_or(GENERAL_PROTECTION),
        Err(_) => GENERAL_PROTECTION,
    }
}

/// Whether each trapped fault is described in full as it is serviced.
///
/// Off unless asked for: servicing one is ordinary work, and a guest takes
/// exceptions as part of running correctly.
fn reports_each_fault() -> bool {
    static SETTING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SETTING.get_or_init(|| std::env::var_os("WHP_TRAP_EXCEPTIONS").is_some())
}

/// Whether every slice is run on the shadow — the diagnostic bisection.
///
/// Read once. It is asked on the slice path, where reading the environment
/// again per slice costs a lock, an allocation and a parse to learn something
/// that cannot have changed since the machine started.
fn everything_on_the_shadow() -> bool {
    static SETTING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SETTING.get_or_init(|| std::env::var_os("WHP_ALL_SHADOW").is_some())
}

/// Whether every port access is finished on the shadow rather than from the
/// exit. Read once, and asked once per port exit — the most frequent exit a
/// booting guest takes.
fn ports_on_the_shadow() -> bool {
    static SETTING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SETTING.get_or_init(|| std::env::var_os("WHP_PORTS_ON_SHADOW").is_some())
}

/// Guest code this port has not been asked to run on hardware yet, and which
/// the platform cannot finish alone. Each is a real exit that a complete engine
/// services; refusing by name is what keeps a half-serviced one from looking
/// like a working machine.
fn unserviced(what: &'static str, exit: &Exit) -> CpuError {
    // Where the guest was matters more than what the exit was called: an exit
    // this engine cannot service is a report about guest code, and the report
    // is useless without an address to look at.
    tracing::error!(
        "WHP exit not serviced by this engine: {what} at {:#x}:{:#x} (cs base {:#x}, rflags {:#x}, \
         execution state {:#06x} — interruption pending {}, interrupt shadow {})",
        exit.vp.cs.selector,
        exit.vp.rip,
        exit.vp.cs.base,
        exit.vp.rflags,
        exit.vp.execution_state,
        // `WHV_X64_VP_EXECUTION_STATE`: bit 6 says a delivery is already in
        // flight, bit 12 that the processor is in an interrupt shadow. Both
        // arrive free on every exit, and both are state this engine does NOT
        // carry across its seam — so a slice that rewrites the processor while
        // one is set can make the hardware deliver an event a second time.
        (exit.vp.execution_state >> 6) & 1,
        (exit.vp.execution_state >> 12) & 1,
    );
    // The reason travels in the error, not only in the log. A caller that shows
    // this to a person — the GUI puts it in a box on the window — otherwise
    // reports that something went wrong without saying what, and the one fact
    // that would identify it is sitting in a log they may not be reading.
    CpuError::UnsupportedCpuOperation { operation: what }
}

/// Report a processor state the platform would not take, and say what was in
/// it.
///
/// `WHvSetVirtualProcessorRegisters` answers `WHV_E_INVALID_VP_STATE` without
/// naming the register that offended it, and by the time anyone reads the
/// error the state is gone — the shadow has moved on and the partition kept
/// what it had. Here is the only place it still exists, so here is where it is
/// described. The fields are the ones a platform validates against each other:
/// the mode bits, the paging registers, and every segment, since a descriptor
/// this port can build is not always one the platform will accept.
fn refused_state(error: WhpError, state: &VcpuArchState) -> CpuError {
    tracing::error!(
        "the hypervisor refused a processor state: {error}; rip {:#x} rflags {:#x} \
         cr0 {:#x} cr2 {:#x} cr3 {:#x} cr4 {:#x} efer {:#x} xcr0 {:#x} apic_base {:#x}",
        state.rip,
        state.rflags,
        state.cr0,
        state.cr2,
        state.cr3,
        state.cr4,
        state.msrs.efer,
        state.xcr0,
        state.msrs.apic_base
    );
    const SEGMENTS: [&str; 6] = ["es", "cs", "ss", "ds", "fs", "gs"];
    for (name, seg) in SEGMENTS.iter().zip(&state.segments) {
        tracing::error!(
            "  {name} = {:#06x} base {:#x} limit {:#x} attr {:#x}",
            seg.selector,
            seg.base,
            seg.limit,
            seg.attributes.bits()
        );
    }
    platform_failed(error)
}

fn platform_failed(error: WhpError) -> CpuError {
    tracing::error!("WHP platform call failed: {error}");
    CpuError::UnsupportedCpuOperation { operation: "the hypervisor refused" }
}

/// What the injection decision must know about deliverability, kept current
/// without a single register read (QEMU `target/i386/whpx/whpx-all.c`
/// `whpx_vcpu_post_run` keeps the same cache for the same reason).
///
/// The invariant this type exists for: everything [`stage_injection`]
/// consults is already in hand the moment it decides, so consulting it costs
/// zero platform calls — where a register read would cost the very exchange
/// injection is meant to avoid.
///
/// ## The freshness contract
///
/// The decision runs at the tail of a loop iteration and at the head of a
/// slice; between an exit and its tail the engine may SERVICE the exit on the
/// shadow interpreter, and a serviced instruction can change `IF` (a `CLI`,
/// or a fault entering a gate) and leave the guest at a fresh boundary. So
/// the cache has two authorities, and which one is current depends on whether
/// the shadow is what the next entry runs:
///
/// - **No errand** (a raw port write, an interrupt-window exit): the exit
///   header describes the processor the tail will re-enter, so
///   [`Self::refresh_from`] — called once per iteration on the fresh header —
///   is authoritative. It decodes `in_flight`, `if_flag`, `cr8` AND the
///   interrupt-shadow bit.
/// - **An errand ran** (`finish_on_the_shadow`, `burst_on_the_shadow`): the
///   errand's `impose_the_shadow` has just made the partition identical to
///   the shadow, so the SHADOW is what the next VM entry sees and the header
///   is stale. Every errand path therefore ends with
///   [`Self::refresh_from_shadow`], which republishes `if_flag` from the
///   interpreter and PRESUMES the interrupt shadow live — the interpreter's
///   inhibit bookkeeping is invisible to this crate, and unknown means
///   blocked, never permitted.
/// - **A slice head** (`install_the_shadow`) is the errand case writ large:
///   whatever the cache holds predates everything the machine did between
///   slices — shadow-converted slices, deliveries, state the machine wrote —
///   or there was never a header at all. The install is an imposition, so it
///   carries the same republish, and the head's staging decision always runs
///   on the shadow's own `IF` with the inhibit presumed live.
///
/// The property both together guarantee: at the instant `stage_injection`
/// pops a vector — an irreversible acknowledge — the gate holds POSITIVE
/// evidence that delivery is permitted, never merely the absence of evidence
/// that it is blocked. Getting this wrong injects a maskable interrupt into
/// an `IF=0` or interrupt-shadowed context, which the VM-entry guest-state
/// checks reject with `WHV_E_INVALID_VP_STATE` and which loses the
/// acknowledged vector.
struct InjectState {
    /// `ExecutionState` bit 6, `InterruptionPending` — a delivery the platform
    /// has begun and not landed. From the exit header alone.
    in_flight: bool,
    /// Whether interrupts are enabled for the processor the next VM entry will
    /// run: `RFLAGS` bit 9 from the exit header on an errand-free iteration,
    /// or `cpu.interrupts_enabled()` republished by [`Self::refresh_from_shadow`]
    /// after an errand.
    if_flag: bool,
    /// The exit header's `Cr8` field, and the only fresh source there is: a
    /// guest `MOV CR8` retires on hardware without an exit, so no copy this
    /// engine keeps between exits can be trusted over the header's.
    cr8: u8,
    /// Whether the processor the next VM entry will run may be inside an
    /// interrupt shadow — an `STI` / `MOV SS` / `POP SS` window that blocks
    /// delivery for one instruction. `ExecutionState` bit 12 from the exit
    /// header on an errand-free iteration; PRESUMED true by
    /// [`Self::refresh_from_shadow`] after an errand, because the
    /// interpreter's inhibit state is invisible to this crate and the gate
    /// may only act on positive permission.
    shadowed: bool,
    /// A `DeliverabilityNotifications` request armed at this priority and not
    /// yet answered by a window exit. NOT touched by either refresh — no
    /// header or shadow reports it back. It is owned by the injection logic —
    /// [`stage_injection`] arms it, the `InterruptWindow` arm clears it.
    window: Option<u8>,
}

impl InjectState {
    /// A processor that has never run has no exit header to have said
    /// anything: nothing in flight, `IF` clear — the architectural reset
    /// value — not shadowed, `CR8` zero, and no window armed.
    const fn at_reset() -> Self {
        Self { in_flight: false, if_flag: false, cr8: 0, shadowed: false, window: None }
    }

    /// Cache what an exit header says — the authority for an iteration in
    /// which no shadow errand ran.
    ///
    /// Sets `in_flight`, `if_flag`, `cr8` and `shadowed`; `window` is
    /// deliberately untouched, because it records a notification this engine
    /// armed and no header reports one back.
    fn refresh_from(&mut self, vp: &VpContext) {
        // `WHV_X64_VP_EXECUTION_STATE`: `InterruptionPending` is bit 6,
        // `InterruptShadow` is bit 12 (the `unserviced` diagnostic reads the
        // same two bits).
        self.in_flight = (vp.execution_state >> 6) & 1 == 1;
        self.shadowed = (vp.execution_state >> 12) & 1 == 1;
        // `RFLAGS.IF` is bit 9.
        self.if_flag = (vp.rflags >> 9) & 1 == 1;
        self.cr8 = vp.cr8;
    }

    /// Republish deliverability from the shadow — the authority whenever the
    /// shadow is what the next VM entry runs.
    ///
    /// Called at the end of every path that emulates before the injection
    /// tail, once `impose_the_shadow` has made the partition identical to the
    /// interpreter, and by `install_the_shadow` at every slice head — always
    /// with the shadow's live `IF` (`cpu.interrupts_enabled()`).
    ///
    /// `shadowed` is set TRUE — unknown, therefore blocked. The
    /// interpreter's one-instruction interrupt inhibit (`STI` / `MOV SS` /
    /// `POP SS`) is crate-private bookkeeping this crate cannot read, and an
    /// errand CAN stop with one live: the batch loop breaks on its
    /// instruction budget at the loop head, before the async-event handling
    /// that would lapse an inhibit (rusty_box `cpu.rs` `cpu_loop_n_impl`,
    /// `inhibit_interrupts`), so a stretch whose last retired instruction is
    /// a shadower returns mid-window, and a single trapped `MOV SS`/`POP SS`
    /// with a device-memory operand does the same in one step. Claiming "not
    /// shadowed" there would hand the gate positive permission it does not
    /// have, and the gate acknowledges on it — an irreversible INTA — before
    /// injecting into a live shadow. So the gate is told BLOCKED, the vector
    /// is deferred to a window, and the NEXT exit's header bit 12 —
    /// authoritative and free — answers truthfully. The cost of the
    /// presumption is one deferred injection resolved by one window exit;
    /// nothing irreversible happens on the unknown.
    ///
    /// `in_flight` and `cr8` are not republished: nothing an errand does
    /// begins a platform delivery, and the header's `CR8` still stands.
    ///
    /// Takes the flag rather than the processor so the freshness rule is
    /// unit-testable without a constructed `BxCpuC`; the call site reads it
    /// from the shadow.
    fn refresh_from_shadow(&mut self, shadow_if: bool) {
        self.if_flag = shadow_if;
        self.shadowed = true;
    }
}

/// A partition that has been configured, given memory and given a processor.
///
/// FIELD ORDER IS LOad-BEARING: `alarm` is declared before `partition` because
/// Rust drops fields in declaration order, and the alarm's thread holds a
/// canceller naming this partition. Joining that thread before the partition
/// is destroyed is the whole of the ordering obligation, and this is where it
/// is discharged.
struct Started {
    alarm: Alarm,
    partition: Partition,
    /// Reused across slices so a state exchange allocates nothing per exit.
    state: VcpuArchState,
    /// The map the partition is currently holding.
    ///
    /// Kept because a borrowed mapping leaves no bookkeeping behind — the
    /// partition records the range only for memory it owns — so this is the
    /// only record of what is installed, and the only way to know which
    /// windows a new map removed rather than merely changed.
    installed: MemoryPlan,
    /// Whether the guest was inside an interrupt shadow when the hardware last
    /// handed it back.
    ///
    /// x86 blocks interrupts for exactly one instruction after `MOV SS`,
    /// `POP SS` and `STI`, so that a stack switch written as `MOV SS, x` /
    /// `MOV ESP, y` cannot be interrupted between its two halves. The
    /// interpreter tracks this itself and never delivers inside one; this
    /// engine cannot, because the instruction retired on the hardware and the
    /// fact lives in the partition's own interrupt state rather than in any
    /// architectural register the exchange carries.
    ///
    /// Delivering anyway pushes the interrupt frame with the NEW `SS` and the
    /// OLD `ESP` — a frame at an address the handler never agreed to — and the
    /// `IRET` that ends the handler pops whatever happened to be there. That
    /// is a `general protection: 0000` on `IRET`, which is exactly how a DLX
    /// boot died once IDE interrupts started flowing.
    shadowed: bool,
    /// Whether the guest had NMIs masked when the hardware last handed it
    /// back — bit 1 of `WHvRegisterInterruptState`, read beside
    /// [`Self::shadowed`].
    ///
    /// Kept so [`impose_the_shadow`] can write that register's inhibit bit
    /// without clobbering this one: the mask is the hardware's own record of
    /// a guest inside an NMI handler, no architectural register the exchange
    /// carries holds it, and a blind zero would unmask NMIs mid-handler.
    /// Between the read-back that fills it and any imposition that writes the
    /// register the partition never runs, so the copy is current at every
    /// write.
    nmi_masked: bool,
    /// What the platform's registers hold, as far as this engine knows.
    ///
    /// Written at exactly the two moments the answer is certain: after reading
    /// the processor, and after writing it. Every other platform register write
    /// this engine makes happens inside a slice, before the read-back that ends
    /// it, so this is accurate by the time the next slice consults it.
    ///
    /// `None` until the first read, which is the honest reading of "not known".
    held: Option<VcpuArchState>,
    /// How many memory exits in a row this partition has taken.
    ///
    /// A guest that touched device memory once produces one; a guest clearing
    /// the VGA planar aperture produces tens of thousands, one per `mov`. The
    /// two want opposite treatment, and the run length is what tells them
    /// apart — see [`BURST_AFTER`].
    consecutive_mmio: u32,
    /// The processor's extended-state area — the x87 and vector file the
    /// named-register exchange cannot carry. Refreshed at every read-back and
    /// patched at every write-back, alongside `state` and under the same
    /// `held` skip, so the two register files a guest can reach never
    /// diverge across the seam.
    xsave: XsaveArea,
    /// What the last exit's header said about deliverability.
    ///
    /// Refreshed before anything else reads a fresh exit, so it is current
    /// whenever any arm — or a slice's end — consults it.
    inject: InjectState,
}

/// What the guest has been leaving the hardware for.
///
/// An exit is the unit of cost of this whole design — measured at roughly four
/// microseconds on this host — so how many of each a guest takes is the first
/// thing worth knowing about a machine that is slow, and the first thing worth
/// looking at when one is stuck: a boot that has taken no port exits has not
/// reached its firmware, whatever else it has been doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExitCounts {
    /// Port I/O, answered from the machine's device set.
    pub port: u64,
    /// An access the partition's map did not serve, finished on the shadow.
    pub memory: u64,
    /// `CPUID`, answered from this port's own model.
    pub cpuid: u64,
    /// An MSR access, answered from this port's own register file.
    pub msr: u64,
    /// The guest halted.
    pub halt: u64,
    /// The host asked for the processor back mid-run.
    pub canceled: u64,
    /// A device dispatch asked for the machine's boundary to be serviced.
    pub boundary: u64,
    /// An interrupt window the engine armed opened. The platform raises this
    /// exit only when asked, and the engine asks only when delivery was
    /// blocked — an interrupt shadow, or IF clear — at the moment it wanted
    /// to inject; a guest whose delivery is open at that moment never pays
    /// one. Read beside [`InjectCensus::windows_armed`], which counts the
    /// asks this counts the answers to.
    pub window: u64,
}

/// How the guest's time divided into slices, and what ended each one.
///
/// [`ExitCounts`] says what the guest asked the hardware for; it cannot say how
/// those asks were divided into slices, and the division is what a slice costs.
/// A slice holding one exit bought a VM entry and a VM exit — roughly four
/// microseconds on this host — and ran the guest for whatever fits between
/// them, which is approximately nothing. So the shape of the histogram is the
/// shape of the problem, and the split of [`Yielded::Boundary`] says which of
/// three different fixes the shape calls for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SliceCensus {
    /// Slices that ended in something the machine could act on.
    ///
    /// A slice that ended in an error is not one of them: the machine is handed
    /// a fault rather than a processor, and counting it as a slice would put a
    /// row in the histogram for a stretch the guest never got.
    pub slices: u64,
    /// How many exits each slice held, bucketed by [`Self::bucket_of`] and
    /// named by [`Self::BUCKET_LABELS`].
    pub exits_per_slice: [u64; Self::BUCKETS],
    /// Slices the guest ended by halting.
    pub ended_halted: u64,
    /// Slices the host ended by asking for the processor back mid-run.
    pub ended_canceled: u64,
    /// Slices that ran for the whole stretch the machine asked for.
    pub ended_budget: u64,
    /// Slices ended because the processor itself asked for a machine boundary.
    pub ended_wants_machine_boundary: u64,
    /// Slices ended because a device had latched work only the machine can do.
    pub ended_needs_boundary: u64,
    /// Slices ended because an event had become deliverable that the slice
    /// could not deliver itself: on the hardware, an NMI, an SMI or an INIT —
    /// a maskable external vector is injected mid-slice and never ends one —
    /// and on a shadow-converted slice, whatever its batch left standing at
    /// its boundary.
    pub ended_event_to_deliver: u64,
}

impl SliceCensus {
    /// How many buckets [`Self::exits_per_slice`] holds.
    pub const BUCKETS: usize = 6;

    /// What each bucket of [`Self::exits_per_slice`] covers, in order.
    ///
    /// Fine at the bottom and coarse at the top, because that is where the
    /// question lives: one exit per slice and two exits per slice are different
    /// diagnoses, while forty and eighty are the same one.
    pub const BUCKET_LABELS: [&'static str; Self::BUCKETS] =
        ["0", "1", "2", "3-4", "5-8", "9+"];

    /// Which bucket a slice holding `exits` exits belongs in.
    ///
    /// The one place the edges are written down, so the histogram a reader
    /// prints and the histogram this records cannot describe different ranges.
    #[must_use]
    pub const fn bucket_of(exits: u64) -> usize {
        match exits {
            0 => 0,
            1 => 1,
            2 => 2,
            3..=4 => 3,
            5..=8 => 4,
            _ => 5,
        }
    }

    /// Record one slice: how many exits it held, and what ended it.
    ///
    /// The single place a slice is counted, so the histogram and the ending
    /// tallies cannot disagree about how many slices there were — they are
    /// two readings of the same population and a report subtracts one from the
    /// other.
    fn record(&mut self, exits: u64, yielded: &Yielded) {
        self.slices += 1;
        self.exits_per_slice[Self::bucket_of(exits)] += 1;
        match yielded {
            Yielded::Halted => self.ended_halted += 1,
            Yielded::Canceled => self.ended_canceled += 1,
            Yielded::Budget => self.ended_budget += 1,
            Yielded::Boundary(BoundaryReason::ProcessorAsked) => {
                self.ended_wants_machine_boundary += 1;
            }
            Yielded::Boundary(BoundaryReason::DeviceLatched) => self.ended_needs_boundary += 1,
            Yielded::Boundary(BoundaryReason::EventToDeliver) => {
                self.ended_event_to_deliver += 1;
            }
        }
    }
}

/// What this engine has put into the partition's pending-event slot, and how
/// often it had to arm a window and wait for the guest to become able to
/// take it.
///
/// [`ExitCounts`] and [`SliceCensus`] measure what the guest asked the
/// hardware for; this measures what the machine pushed in. Every field here
/// is one half of a comparison whose other half is kept by an independent
/// party — the interrupt fabric, or the platform's own exit stream — because
/// the defect class these counters exist to catch is a vector acknowledged
/// on one side and lost or doubled on the other, and a counter shows that
/// only by disagreeing with an account that does not share its bugs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InjectCensus {
    /// Interrupts injected into the partition.
    ///
    /// Must equal the interrupt fabric's `acknowledge_count` delta
    /// attributable to injection: every injection acknowledges exactly one
    /// vector at the fabric, so a shortfall here is a vector acknowledged
    /// and lost, and an excess is one delivered twice.
    pub injected: u64,
    /// Interrupt windows armed because delivery was blocked — an interrupt
    /// shadow, or IF clear — at the moment the engine wanted to inject.
    ///
    /// Paired with [`ExitCounts::window`]: every armed window must
    /// eventually be answered by a window exit, so armed-with-no-exits is
    /// not a quiet guest but a wedged one, holding a vector it will never
    /// take.
    pub windows_armed: u64,
    /// [`Self::injected`], split by vector.
    ///
    /// The aggregate can balance while two vectors trade places; matching
    /// this against the fabric's `vectors_acknowledged` histogram is the
    /// comparison that catches a swap.
    pub injected_per_vector: [u32; 256],
}

/// Everything zero: nothing injected, no window armed. Written by hand
/// because this toolchain derives `Default` for arrays only up to 32
/// elements, and the per-vector histogram holds 256.
impl Default for InjectCensus {
    fn default() -> Self {
        Self { injected: 0, windows_armed: 0, injected_per_vector: [0; 256] }
    }
}

/// Runs guest code on the Windows Hypervisor Platform.
///
/// Starts unconfigured because a machine constructs its engine before it has
/// memory to map — the partition is built on the first slice, which is the
/// first moment the engine is handed [`PcIo`] and can see the machine's own
/// guest-physical map.
#[derive(Default)]
pub struct WhpEngine {
    started: Option<Started>,
    exits: ExitCounts,
    census: SliceCensus,
    inject_census: InjectCensus,
    /// Diagnostic; see [`ExitHistory`]. Lives here rather than in a slice
    /// because a guest's path to a fault crosses slice boundaries — a handler
    /// that traps for its own port I/O is several slices old by the time it
    /// faults, and a per-slice history shows only the fault itself.
    history: ExitHistory,
}

impl WhpEngine {
    /// What the guest has been leaving the hardware for, since this machine
    /// was built. Reached through `Emulator::engine`.
    #[must_use]
    pub const fn exits(&self) -> ExitCounts {
        self.exits
    }

    /// How the guest's time has been divided into slices, and what ended each.
    ///
    /// A slice holding one exit means the engine bought a VM entry and a VM
    /// exit and ran the guest for nothing in between; the shape of this
    /// histogram is therefore the shape of the problem.
    #[must_use]
    pub const fn census(&self) -> SliceCensus {
        self.census
    }

    /// What this engine has put into the partition's pending-event slot, and
    /// how often it had to arm a window and wait.
    ///
    /// Returned by reference rather than by value: the per-vector histogram
    /// makes a copy a kilobyte, not a pair of words.
    #[must_use]
    pub const fn inject_census(&self) -> &InjectCensus {
        &self.inject_census
    }

    /// What the hypervisor itself charged this guest, beside what this engine
    /// believes it did.
    ///
    /// The independent check on [`Self::exits`] and [`Self::census`]: those are
    /// this port's own account of its own behaviour, and an account cannot
    /// audit itself. A disagreement means one of the two is measuring
    /// something other than what it claims, and which one is wrong is a
    /// question worth answering before designing against either.
    ///
    /// The two sets come back together because they describe one instant: read
    /// separately, a slice can run between them and the runtime no longer
    /// belongs to the intercepts.
    ///
    /// The mapping between these counters and this engine's own tallies is not
    /// field-for-field obvious — notably a halt serviced by the machine leaves
    /// `halt_instructions.count` at zero and is booked under
    /// `other_intercepts`. See `docs/superpowers/plans/2026-08-29-whp-fast-path.md`.
    ///
    /// # Errors
    /// [`CpuError::UnsupportedCpuOperation`] if no partition has started, since
    /// an engine that has not run has no hardware to have charged anything, or
    /// if the platform refuses the query.
    pub fn platform_counters(&self) -> Result<PlatformCounters> {
        let started = self.started.as_ref().ok_or(CpuError::UnsupportedCpuOperation {
            operation: "the partition did not start",
        })?;
        Ok(PlatformCounters {
            intercepts: started.partition.intercept_counters(BOOT_VP).map_err(platform_failed)?,
            runtime: started.partition.runtime_counters(BOOT_VP).map_err(platform_failed)?,
        })
    }
}

/// The hypervisor's own accounting for a processor, both sets at one instant.
///
/// Named rather than a pair because the two are read together and mean
/// different things: one says what the guest left the hardware for and how
/// long each class took, the other says how much of the processor's whole life
/// went to the hypervisor rather than to the guest. Neither answers the
/// other's question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlatformCounters {
    /// Per-class count and time.
    pub intercepts: InterceptCounters,
    /// Total versus hypervisor-attributed runtime.
    pub runtime: RuntimeCounters,
}

/// Build the partition, map the machine's memory into it and create the
/// processor — once, on the first slice.
///
/// Takes the slot rather than the engine so a caller can hold the engine's
/// other fields at the same time: a running slice counts its exits while the
/// partition is borrowed.
fn start<'a>(started: &'a mut Option<Started>, io: &mut PcIo<'_>) -> Result<&'a mut Started> {
    {
        if started.is_none() {
            // The guest is offered exactly the processor features the host
            // banks. A partition that is never told falls back to the
            // platform's default set, which is narrower than the host's: a
            // guest touching a feature the host has but the partition was not
            // told about faults on hardware while working under the
            // interpreter — a `wrmsr IA32_SPEC_CTRL` #GP that no shadow run
            // ever shows.
            let banked = rusty_box_whp::capabilities()
                .map_err(platform_failed)?
                .processor_features;
            let mut config = PartitionConfig::new().map_err(platform_failed)?;
            config
                .processor_count(1)
                .map_err(platform_failed)?
                .processor_features(banked)
                .map_err(platform_failed)?
                // No APIC in the partition: this port's own `cpu/apic.rs` is
                // the guest's local APIC, its 8259 pair stays in host memory,
                // and injection is therefore ours end to end. Measured
                // (probe finding 10): under `XApic` a halted processor never
                // leaves `WHvRunVirtualProcessor`, which would strand the
                // timer wheel on a thread that never returns.
                .local_apic(LocalApicMode::None)
                .map_err(platform_failed)?
                // What a guest asks ABOUT the processor is this port's to
                // answer, not the host's. Both are serviced on the shadow, so
                // a guest reading CPUID or an MSR under a hypervisor gets the
                // same bytes it would get with none — which is the whole of
                // what parity means here.
                .extended_vm_exits(ExtendedVmExits {
                    cpuid: true,
                    msr: true,
                    // Diagnostic only, and off unless asked for: a guest takes
                    // exceptions as part of running correctly, and one trapped
                    // here is one its own handler never sees. What it buys is
                    // the fault ITSELF rather than the guest's report of it —
                    // the faulting address and the processor's state at the
                    // instant, instead of whatever the guest managed to print
                    // afterwards from a state the fault had already disturbed.
                    exception: trapped_exceptions() != 0,
                    ..ExtendedVmExits::default()
                })
                .map_err(platform_failed)?
                .cpuid_exit_list(TRAPPED_CPUID_LEAVES)
                .map_err(platform_failed)?
                .msr_exits(TRAPPED_MSRS)
                .map_err(platform_failed)?
                .exception_exits(trapped_exceptions())
                .map_err(platform_failed)?;
            let mut partition = config.setup().map_err(platform_failed)?;

            let installed = install_the_machines_map(&mut partition, io.memory(), None)?;
            partition.create_processor(BOOT_VP).map_err(platform_failed)?;

            // The fresh processor's own area, read once here so every later
            // exchange patches the platform's bytes rather than inventing
            // them — the components this port does not model cross untouched
            // that way. A host that refuses the read cannot keep the two
            // register files agreed, so it refuses the engine.
            let xsave = XsaveArea::read_from(
                &partition,
                BOOT_VP,
                xsave::HostComponents::of_this_host(),
            )
            .map_err(platform_failed)?;

            let alarm = Alarm::watching(partition.canceller(BOOT_VP));
            *started = Some(Started {
                alarm,
                partition,
                state: VcpuArchState::default(),
                installed,
                // A processor that has never run cannot be mid-instruction,
                // and NMIs are unmasked at reset.
                shadowed: false,
                nmi_masked: false,
                consecutive_mmio: 0,
                // Nothing has been read from this processor yet.
                held: None,
                xsave,
                // No exit header has said anything yet.
                inject: InjectState::at_reset(),
            });
        }
    }
    // Established just now, or by an earlier slice. Reported rather than
    // asserted: a library says what it cannot do instead of ending the
    // host's process to say it.
    started.as_mut().ok_or(CpuError::UnsupportedCpuOperation {
        operation: "the partition did not start",
    })
}

/// Why a processor came back to the machine.
///
/// Both endings hand the machine work only it can do, which is why neither is
/// an error and why they are told apart: a halt changes what the machine knows
/// about the processor, a cancellation does not.
enum Yielded {
    /// The guest executed `HLT`. The machine's own fast-forward advances time
    /// to the next device deadline from here and decides when the processor
    /// may run again.
    Halted,
    /// The host asked for the processor back mid-run.
    Canceled,
    /// A device dispatch asked for the machine's boundary to be serviced.
    ///
    /// The guest must not run on until it has been. A chipset write that
    /// re-routes guest-physical space — a PAM flip, an SMRAM open, a relocated
    /// BAR — only takes effect when the machine applies it, and this engine
    /// then has a new map to install; a guest that kept running would be
    /// running against the layout the write was replacing.
    Boundary(BoundaryReason),
    /// The stretch the machine asked for is over.
    ///
    /// The machine bounds a slice by the time to its next device deadline, and
    /// a device timer fires only when the machine has the processor back. An
    /// engine that ran until the guest happened to stop would starve every
    /// timer in the machine: the first guest to find out is the BIOS, whose
    /// keyboard self-test polls the 8042 a bounded number of times and panics
    /// when the controller — waiting on a timer that never fires — does not
    /// answer.
    Budget,
}

/// Which of the three questions ended a slice at [`Yielded::Boundary`].
///
/// Carried as a state rather than tallied at each `return`, because the three
/// call for different fixes and a single `Boundary` count cannot tell them
/// apart: a device that latched work wants the drain moved, a processor that
/// asked for a boundary wants the machine to run, and a deliverable event
/// ending slices wants to be rarer than the events themselves — a maskable
/// vector is injected mid-slice and never ends one.
enum BoundaryReason {
    /// The processor itself asked for a machine boundary.
    ProcessorAsked,
    /// A device latched work only the machine can do.
    DeviceLatched,
    /// An event injection cannot carry — an NMI, an SMI, an INIT — became
    /// deliverable, and those are the shadow's to deliver at the head of the
    /// next slice.
    EventToDeliver,
}

/// Install the machine's guest-physical map into the partition, replacing
/// whatever `installed` describes, and report what is now installed.
///
/// The map is derived from the machine's own memory, so the hypervisor and
/// this port's interpreter serve the same bytes at the same addresses — which
/// is what makes an exit serviced by the shadow processor land where the guest
/// expects it.
///
/// Two things make the replacement more than a loop of maps. A mapping
/// REPLACES any prior one over the same range, so a window that merely changed
/// its permissions or its backing needs no unmap — but a window the new map
/// DROPS has to be taken out explicitly, or a range the chipset just turned
/// into device space would keep being served from RAM and never exit. And a
/// borrowed mapping leaves the partition no bookkeeping to consult, so what
/// the old map was has to be remembered rather than asked for.
///
/// Windows that did not change are left alone. That is not only for the cost
/// of the call: re-mapping a range discards the second-level translations the
/// hypervisor built for it, and a BIOS flipping one PAM area has no business
/// making the guest fault its way back through all of RAM.
///
/// # Errors
/// A machine whose memory has no stable map, a window outside the allocation,
/// or a platform call that refused.
fn install_the_machines_map(
    partition: &mut Partition,
    memory: &mut BxMemC,
    installed: Option<&MemoryPlan>,
) -> Result<MemoryPlan> {
    let plan = MemoryPlan::derive(memory).map_err(|error| {
        tracing::error!("this machine has no stable guest-physical map: {error:?}");
        CpuError::UnsupportedCpuOperation {
            operation: "a partially resident machine has no map to install",
        }
    })?;
    let old: &[GpaWindow] = installed.map_or(&[], MemoryPlan::windows);
    if old == plan.windows() {
        return Ok(plan);
    }

    for window in old {
        if plan.windows().contains(window) {
            continue;
        }
        partition
            .unmap_subrange(window.gpa, window.len)
            .map_err(platform_failed)?;
    }

    for window in plan.windows() {
        if old.contains(window) {
            continue;
        }
        let host = memory
            .allocation_slice(window.host.get(), window.len)
            .ok_or(CpuError::UnsupportedCpuOperation {
                operation: "a plan window fell outside the machine's allocation",
            })?;
        // The machine owns this allocation in a boxed slice that never moves,
        // and owns this engine beside it, so the mapping cannot outlive the
        // memory — which is the obligation `map_borrowed` names. Discharging it
        // is why this call is `unsafe` and this crate is not: see the wrapper
        // in `rusty_box_whp`, whose signature carries the contract.
        map_window(partition, window.gpa, host, window.perms)?;
    }
    Ok(plan)
}

/// Install one window, discharging the contract `map_borrowed` names.
///
/// The single place this crate reaches for `unsafe`, and it holds no unsafe
/// operation of its own — what it does is assert an ownership fact the type
/// system cannot: the machine owns this allocation, keeps it in a boxed slice
/// that never moves for its lifetime, and owns the engine holding this
/// partition alongside it. The mapping therefore cannot outlive the memory it
/// points at, and the partition is torn down with the machine that made it.
fn map_window(
    partition: &mut Partition,
    gpa: u64,
    host: &mut [u8],
    perms: rusty_box_whp::GpaPerms,
) -> Result<()> {
    // SAFETY: as above — the mapped bytes belong to the machine that owns this
    // partition, and neither the allocation nor its address changes while the
    // machine lives.
    unsafe { partition.map_borrowed(gpa, host, perms) }.map_err(|error| {
        // Mapping is where the platform materialises its backing partition,
        // and it names that partition after the process — so a second machine
        // on this engine in one process is refused HERE, with
        // `ERROR_VID_PARTITION_ALREADY_EXISTS`, and not at any earlier call.
        // Measured in `rusty_box_whp`'s `a_process_holds_one_partition_at_a_time`.
        tracing::error!(
            "installing guest memory at {gpa:#x} failed: {error}. One process runs one \
             machine on this engine; a second needs a second process."
        );
        CpuError::UnsupportedCpuOperation {
            operation: "the hypervisor would not take this machine's memory",
        }
    })
}

impl<T: Instrumentation> SliceEngine<T> for WhpEngine {
    // The hardware retires the guest's instructions and this port never sees
    // them, so what a slice can report is the time it took. See `ticks_elapsed`.
    const PROGRESS_UNIT: ProgressUnit = ProgressUnit::Ticks;

    // Delivery is this machine's, not the partition's: the partition has no
    // local APIC of its own and the hardware knows nothing of this machine's
    // 8259 pair, so the vector, the acknowledge, the priority and the EOI all
    // belong to this machine's own controllers wherever the guest happens to
    // be executing. Only the final push crosses the seam — a maskable vector
    // as `stage_injection`'s register write, at a slice head or an exit tail
    // alike; an NMI, SMI or INIT as the shadow's own interpreted delivery at
    // a head.
    const EVENT_DELIVERY: EventDelivery = EventDelivery::Engine;

    fn memory_map_changed(&mut self, memory: &mut BxMemC) -> Result<()> {
        // Before the first slice there is no partition and nothing installed;
        // the map this would have installed is the one `start` derives, so a
        // change now is not a change to anything.
        let Some(started) = self.started.as_mut() else {
            return Ok(());
        };
        started.installed =
            install_the_machines_map(&mut started.partition, memory, Some(&started.installed))?;
        Ok(())
    }

    fn run_slice(
        &mut self,
        cpu: &mut BxCpuC<T>,
        mut io: PcIo<'_>,
        request: SliceRequest,
    ) -> Result<Progress> {
        let ips = io.pc_system.ips();

        let progress = (|| {

        // A sleeping shadow is in no interrupt shadow, whatever the
        // partition's register says. The sleep was entered by an instruction
        // that retired after any `MOV SS` could have shadowed it, so the
        // interpreter's own inhibit has lapsed; and the bit the read-back
        // returned after a shadow-retired halt is the one `impose_the_shadow`
        // writes after every errand — unknown-means-blocked — not a shadow
        // the guest is in. Read as one it would hold off every wake below and
        // let the partition run past the halt, so it is consumed here, ahead
        // of every path that reads it.
        let asleep = !matches!(cpu.activity_state, CpuActivityState::Active);
        if asleep {
            if let Some(started) = self.started.as_mut() {
                started.shadowed = false;
            }
        }

        // DIAGNOSTIC BISECTION, not a shipping mode. `WHP_ALL_SHADOW=1` keeps
        // every part of this engine except the hardware: the same slice
        // budgeting, the same delivery-at-slice-entry policy, the same device
        // servicing through `PcIo` — but the guest's instructions are retired
        // on the shadow instead of the partition, so no state ever crosses to
        // the platform and back.
        //
        // It splits the remaining search space in half. If a DLX boot survives
        // this, everything machine-side is sound and the fault lives in
        // hardware execution or the state handoff around it. If it still dies
        // with the same `general protection` on `IRET`, the fault is in this
        // engine's own slice and delivery logic, which the interpreter's loop
        // does differently.
        if everything_on_the_shadow() {
            let shadowed = match self.started.as_mut() {
                Some(started) => core::mem::replace(&mut started.shadowed, false),
                None => false,
            };
            return run_slice_on_the_shadow(cpu, &mut io, request, &mut self.census, shadowed);
        }

        // A slice the hardware cannot be interrupted within belongs to the
        // shadow, which can end one exactly.
        //
        // The alarm that bounds a hardware slice is a thread waiting on a
        // condition variable, so its resolution is a thread wake — tens of
        // microseconds. A machine whose next device deadline is nearer than
        // that asks for a budget the hardware will overrun no matter what,
        // because the floor in `host_time_for` is already larger than the
        // request: measured on an Alpine boot, the mean slice overran its
        // budget 271-fold and the worst by over a million.
        //
        // What the guest sees when that happens is a timer that fires in
        // bursts at slice boundaries instead of at its deadline. The pulses
        // and the brief windows in which a kernel has its I/O APIC pin
        // unmasked stop overlapping, so `check_timer` finds no tick and Linux
        // panics with "IO-APIC + timer doesn't work" before it ever reaches
        // userspace.
        //
        // The interpreter has no such floor: it retires instructions and stops
        // on the one the budget names. So the engine runs on the processor
        // that can honour the request, and the guest cannot tell which — the
        // shadow is this port's own interpreter against this machine's own
        // devices.
        if host_time_exactly(request.instructions(), ips) < shortest_hardware_slice() {
            // Taken, not merely read: the slice below retires the shadowing
            // instruction itself, so the flag must lapse with it — a stale
            // `true` would hold the next slice's delivery off for an
            // instruction the guest has already executed.
            let shadowed = match self.started.as_mut() {
                Some(started) => core::mem::replace(&mut started.shadowed, false),
                None => false,
            };
            return run_slice_on_the_shadow(cpu, &mut io, request, &mut self.census, shadowed);
        }

        // What injection cannot carry — an NMI, an SMI, an INIT, a shutdown,
        // a processor asleep in a wait state — is delivered HERE, on the
        // shadow, before the hardware is given the processor. The partition
        // was created with no local APIC of its own precisely so that delivery
        // stays on this side (REPLAN decision 6), and running these through
        // the interpreter means the guest's frame is what it would be with no
        // hypervisor in the picture.
        //
        // A deliverable external vector is NOT among them: at the head
        // exactly as at an exit's tail, it is [`stage_injection`]'s — staged
        // below, once the shadow is installed, and it crosses to the
        // partition as a register write instead of an interpreted delivery.
        // Make the processor's view of the bus current BEFORE asking whether
        // it has anything to deliver. The 8259's interrupt line is a level,
        // and what the processor holds is a latched copy of it: a line the
        // guest's own handler dropped while this engine was elsewhere leaves
        // that copy asserted, and the next slice delivers a vector the PIC no
        // longer has. The interpreter never has this problem because it syncs
        // inside its own loop, at every instruction boundary.
        io.sync_io_events(cpu);
        // An interrupt shadow the hardware is still holding forbids the
        // interpreter delivering here, however ready the machine's own
        // controller is. The shadowing instruction retires the moment the
        // guest runs again, so the event waits exactly one slice — which is
        // what the architecture asks for and what the interpreter does with
        // its own inhibit mask.
        let shadowed = self.started.as_ref().is_some_and(|started| started.shadowed);
        if shadowed {
            tracing::debug!(target: "irq", "CPU: shadow delivery held off — the guest is in an interrupt shadow");
        }
        // A processor in a wait state wakes through the interpreter whatever
        // the pending event: `handle_wait_for_event` (rusty_box cpu/event.rs)
        // is the one place the wake rules live, and wake-then-deliver at the
        // loop head is Bochs's own halt sequence. A guest that was asleep was
        // idle — it is not paying the mid-run interruption cost injection
        // exists to remove — so its delivery keeps the interpreted path.
        if !shadowed
            && (cpu.has_non_ext_int_event() || (asleep && cpu.has_an_event_to_deliver()))
        {
            // Both RIPs, because a delivery that happened shows as a jump to a
            // handler and one that did not shows as an ordinary instruction.
            let before = cpu.rip();
            let rsp_before = {
                let mut state = VcpuArchState::default();
                cpu.export_arch_state(&mut state);
                state.gprs[4]
            };
            io.emulate_one(cpu)?;
            tracing::debug!(
                target: "irq",
                "CPU: had an event at slice entry; rip {:#x} -> {:#x}, IF={} rsp {:#x} -> {:#x}",
                before,
                cpu.rip(),
                cpu.interrupts_enabled(),
                rsp_before,
                {
                    let mut after = VcpuArchState::default();
                    cpu.export_arch_state(&mut after);
                    after.gprs[4]
                }
            );
            // Delivery acknowledged the interrupt, which changes the line.
            io.sync_io_events(cpu);
        }
        // A sleeping shadow the wake above did not wake goes no further. The
        // scheduler's runnable test is deliberately wider than the wake set
        // (divergence D1), so an empty slice can reach here; the partition
        // holds the instruction after the halt, and running it would carry
        // the guest past a halt it executed. The slice ends as one, and the
        // machine's fast-forward brings the event that ends the wait.
        if !matches!(cpu.activity_state, CpuActivityState::Active) {
            self.census.record(0, &Yielded::Halted);
            return Ok(Progress::Ticks(0));
        }
        // An SMI the shadow just took puts the processor in a mode the
        // hardware has no equivalent for, so the handler runs to completion
        // here rather than being handed over half-entered.
        run_the_shadow_out_of_smm(cpu, &mut io)?;

        let Self { started, exits, census, history, inject_census } = self;
        let started = start(started, &mut io)?;

        // The shadow describes the processor; the platform runs it.
        install_the_shadow(started, cpu)?;

        // A deliverable external vector is offered to the hardware, not to
        // the interpreter — staged only now, because the state must be on the
        // partition before an injection is staged against it, and judged by
        // the same gate as at an exit's tail. `install_the_shadow` has just
        // republished deliverability from the shadow, so the gate holds the
        // honest answer a head can give: `IF` is the shadow's own, fresh and
        // positive, while the inhibit is UNKNOWN — the interpreter's
        // one-instruction inhibit is invisible to this crate, and
        // `started.shadowed` above is only as fresh as the last read-back,
        // which predates any errand's retirement — so the gate is told
        // BLOCKED and a head-pending vector defers to a window the next
        // exit's header answers. The deferral costs one window exit; nothing
        // irreversible happens on the unknown.
        //
        // The guard mirrors the exit tail's: an NMI, SMI or INIT outranks any
        // maskable vector (the order Bochs event.cc keeps at its own
        // boundary) and is the shadow's to deliver, so no external interrupt
        // is staged over one.
        if !cpu.has_non_ext_int_event() {
            match stage_injection(started, cpu, &mut io, inject_census)? {
                Staged::Injected(vector) => {
                    tracing::debug!(
                        target: "irq",
                        "CPU: vector {vector:#04x} injected at a slice head"
                    );
                    // The acknowledge changed the controllers' lines; the
                    // processor's latched copy follows before anything asks.
                    io.sync_io_events(cpu);
                }
                // Blocked or nothing to carry: the run below is the next
                // word — a window exit if one was armed, and the exit tail
                // re-asks every gate with a fresh header.
                Staged::Windowed | Staged::Nothing => {}
            }
        }

        let outcome = run_until_the_machine_is_needed(
            started,
            cpu,
            &mut io,
            deadline(request, ips),
            request.instructions(),
            ips,
            exits,
            inject_census,
            history,
        );

        // Whatever ended the run, the shadow must describe the processor again
        // before the machine looks at it: the scheduler reads activity state,
        // the interrupt fabric reads IF, and a snapshot reads all of it.
        read_back_into_the_shadow(started, cpu, io.pc_system.time_ticks())?;

        let SliceOutcome { yielded, ran, exits: exits_held } = outcome?;
        census.record(exits_held, &yielded);
        match yielded {
            // Halting is not architectural state, so it does not arrive in the
            // exchange above; the platform reports it as the reason the run
            // ended, and the shadow is where the machine reads it. A shadow
            // that entered a sleep state itself — an errand retired the `HLT`,
            // or an `MWAIT` — already says so, in the state it chose, and a
            // plain halt must not overwrite it: `MwaitIf` wakes on an
            // interrupt with IF clear, which `Hlt` never does.
            Yielded::Halted => {
                if matches!(cpu.activity_state, CpuActivityState::Active) {
                    cpu.record_halt();
                }
            }
            // A boundary request is already on the processor, where the
            // scheduler takes it the moment this slice returns; ending the
            // slice is the whole of what this engine owes it. The other two
            // leave the processor exactly as the hardware left it.
            Yielded::Canceled | Yielded::Boundary(_) | Yielded::Budget => {}
        }
        // What the guest earned, whole.
        //
        // NOT clamped to what the machine asked for, which was tried and
        // measured: a slice overshoots its budget by however much guest time
        // the last run bought, and discarding that overshoot rather than
        // banking it drops the machine's clock 836-fold behind the guest that
        // is driving it. The boot went 539 times slower and reached less of
        // the kernel than before. The overshoot is fixed by ending the slice
        // sooner — the budget check above — never by under-reporting a slice
        // that has already run.
        Ok(Progress::Ticks(ticks_elapsed(ran, ips)))

        })();
        if let Err(error) = &progress {
            // The trail a dead slice leaves. A guest fault the shadow could
            // not deliver surfaces as this error, and by then the guest's own
            // report — if it ever manages one — describes the wreckage, not
            // the approach. What led here is only readable now: the last
            // exits the hardware took (`o`/`i` port writes and reads, `S` a
            // string or repeated one, `m` memory, `c`/`r` CPUID and MSR, `h`
            // a halt, `x` a cancellation, `w` an interrupt window), and the
            // processor the shadow was left holding.
            let trail: std::vec::Vec<std::string::String> = self
                .history
                .replay()
                .map(|(rip, kind)| std::format!("{}@{rip:#x}", kind as char))
                .collect();
            let mut state = VcpuArchState::default();
            cpu.export_arch_state(&mut state);
            let cs = &state.segments[1];
            tracing::error!(
                "a slice died: {error:?}; the shadow held rip={:#x} efer={:#x} cr0={:#x} \
                 cr4={:#x} cs sel={:#06x} base={:#x} limit={:#x} (scaled {:#x}) attr={:#06x}",
                state.rip,
                state.msrs.efer,
                state.cr0,
                state.cr4,
                cs.selector,
                cs.base,
                cs.limit,
                cs.scaled_limit(),
                cs.attributes.bits()
            );
            tracing::error!("  exits leading here (oldest first): {}", trail.join(" "));
        }
        progress
    }
}

/// Run a slice entirely on the shadow, touching no partition.
///
/// The other half of the `WHP_ALL_SHADOW` bisection described at its call
/// site. Everything this engine does around the guest is kept — the delivery
/// decision at slice entry, the SMM drain, the boundary questions, the budget
/// denominated in the machine's own ticks — and only the executor changes.
///
/// One instruction per step rather than a batch, because that is the verb
/// `PcIo` offers and this path is a diagnostic: it is slower than the
/// interpreter's own loop and does not need not to be.
///
/// It keeps the same census as the hardware path, in the same
/// [`SliceCensus`], because a bisection that could not say how its slices ended
/// would be comparing a measured run against an unmeasured one. Every slice
/// here lands in the histogram's first bucket by construction: the guest's
/// instructions retire on the shadow, so no exit crosses this path at all.
///
/// # Errors
/// Whatever the guest raised that the shadow could not take.
fn run_slice_on_the_shadow<T: Instrumentation>(
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    request: SliceRequest,
    census: &mut SliceCensus,
    shadowed: bool,
) -> Result<Progress> {
    io.sync_io_events(cpu);
    // An interrupt shadow the hardware left behind: `MOV SS`, `POP SS` and
    // `STI` block delivery for exactly one instruction, and that fact lives
    // in the partition's interrupt state, not in any register the exchange
    // carries — the interpreter's own inhibit mask did not cross the seam.
    // So the shadowing instruction retires here first, with delivery held
    // off for its length, exactly as the architecture asks. Delivering
    // instead would push the interrupt frame with the new `SS` and the old
    // `ESP` — the same wrong frame the hardware path's guard exists to
    // prevent.
    if shadowed {
        io.finish_the_instruction(cpu)?;
        io.sync_io_events(cpu);
    }
    if cpu.has_an_event_to_deliver() {
        io.emulate_one(cpu)?;
        io.sync_io_events(cpu);
    }
    run_the_shadow_out_of_smm(cpu, io)?;

    let budget = request.instructions();
    let mut retired = 0u64;
    let yielded = loop {
        if retired >= budget {
            break Yielded::Budget;
        }
        // In bulk, not one at a time. Building an execution context per
        // instruction cost this path six times the interpreter's own speed on
        // a DLX boot — 92 seconds against 15 — which is a property of how it
        // asked rather than of what it was running.
        let ran = io.emulate_batch(cpu, budget - retired)?;
        retired += ran;
        if cpu.wants_a_machine_boundary() {
            break Yielded::Boundary(BoundaryReason::ProcessorAsked);
        }
        if io.needs_boundary() {
            break Yielded::Boundary(BoundaryReason::DeviceLatched);
        }
        if cpu.has_an_event_to_deliver() {
            break Yielded::Boundary(BoundaryReason::EventToDeliver);
        }
        if ran == 0 {
            // A batch that retired nothing while no question above holds is a
            // processor that is no longer executing — the shadow's own `HLT`
            // handler has entered the sleep state the machine reads. An active
            // processor that still retired nothing has no more to give this
            // slice either, and the machine gets it back the same way.
            break if matches!(cpu.activity_state, CpuActivityState::Active) {
                Yielded::Budget
            } else {
                Yielded::Halted
            };
        }
    };
    census.record(0, &yielded);
    // One instruction is one tick here, which is the software engine's own
    // denomination — this path retires instructions and can count them.
    Ok(Progress::Ticks(retired))
}

/// Describe the platform's processor from the shadow.
///
/// One of the two halves of the state exchange, named because both halves run
/// in two places now — around a whole slice, and around each exit the shadow
/// has to finish. Writing them out twice is how the two could drift.
///
/// Also republishes deliverability from the shadow, for the same reason
/// [`impose_after_errand`] does (R5): the head's staging decision runs next,
/// and the processor the next VM entry runs is the shadow being installed
/// here — not whatever exit header the cache still holds, which at a head may
/// be a slice old or may never have existed. `IF` and `CR8` come fresh from
/// the export below; the inhibit is presumed live, because between the last
/// read-back and this head the shadow may have retired instructions whose
/// inhibit bookkeeping this crate cannot read — see [`InjectState`]'s
/// freshness contract. Republished on the unchanged early return too: an
/// untouched shadow is still what the entry runs.
///
/// # Errors
/// A register the platform refused to take.
fn install_the_shadow<T: Instrumentation>(
    started: &mut Started,
    cpu: &BxCpuC<T>,
) -> Result<()> {
    // Named rather than elided, so a field added to `Started` has to be
    // considered here instead of silently ignored.
    let Started {
        alarm: _,
        partition,
        state,
        installed: _,
        shadowed: _,
        nmi_masked,
        consecutive_mmio: _,
        held,
        xsave,
        // Republished from the shadow just below; the state exchange itself
        // does not change it again.
        inject,
    } = started;
    cpu.export_arch_state(state);
    inject.refresh_from_shadow(cpu.interrupts_enabled());
    // At a head the shadow's `CR8` is the freshest there is — the partition
    // has not run since the read-back that produced it, and any `MOV CR8` a
    // shadow stretch retired since landed in the shadow alone — so the header
    // copy is superseded here, unlike at an errand tail where the header
    // still stands.
    inject.cr8 = (state.cr8 & 0xF) as u8;
    // A slice that ended with the guest on hardware left the processor holding
    // exactly what the read-back then copied into the shadow, so installing it
    // again writes every register back unchanged. That is the common case —
    // most slices deliver nothing and emulate nothing — and it costs a platform
    // round trip per register group to say nothing.
    //
    // Compared rather than tracked with a flag, because the engine is not the
    // only writer: the machine owns this processor between slices and may set a
    // register itself, and a flag the engine maintains could not see that. The
    // comparison can.
    // Every field except the time-stamp counter, because that is the one field
    // `state::import` does not write: the hardware owns it, and the shadow's
    // own answer for it is not what the partition holds. Comparing it asks
    // whether a register that is never sent has changed, which it always has —
    // and the skip below never fired.
    //
    // Swapped in place rather than compared field by field, so a field added
    // to the state joins the comparison without being remembered here.
    if let Some(previous) = held.as_ref() {
        let exported = state.msrs.tsc;
        state.msrs.tsc = previous.msrs.tsc;
        let unchanged = previous == state;
        state.msrs.tsc = exported;
        if unchanged {
            return Ok(());
        }
    }
    impose_the_shadow(partition, state, xsave, held.as_ref(), *nmi_masked)?;
    *held = Some(state.clone());
    Ok(())
}

/// Write the shadow processor into the partition — the named registers, the
/// interruptibility the exchange cannot carry and, when it moved, the vector
/// file.
///
/// `previous` is the last state the two sides agreed on. The named registers
/// go every time — the caller already knows they changed — but the x87 and
/// vector file is a whole-area transfer, and most exchanges move a `RIP` and
/// a flag word while that file sat still, so it goes only when it differs.
/// With no agreement to compare against, everything goes.
///
/// `WHvRegisterInterruptState` goes every time, and the inhibit bit goes as
/// LIVE. An imposition means the shadow retired instructions — or the machine
/// wrote state — since the read-back the partition's own bit dates from, so
/// that bit is stale in both directions: a stretch may have retired the
/// instruction a set bit was protecting (a partition still refusing delivery
/// would reject an injection at entry with `WHV_E_INVALID_VP_STATE` the
/// moment the gate correctly said yes), or retired a `MOV SS`/`POP SS`/`STI`
/// as its LAST instruction and left an inhibit this crate cannot read (a
/// clear bit would open an armed window at entry and the fresh header would
/// pass the gate — an injection into the shadowing window, the new-`SS`
/// old-`ESP` frame [`Started::shadowed`]'s doc records). Unknown means
/// blocked, so the partition is told what the gate is told: inhibited, for
/// the one instruction the architecture gives an inhibit, after which the
/// hardware clears the bit itself and every later header answers truthfully.
/// The NMI mask rides along unchanged from the last read-back
/// ([`Started::nmi_masked`]); zeroing it would unmask NMIs mid-handler.
///
/// The one direction where "always live" could have been optimistic rather
/// than pessimistic is debug exceptions: VMX's MOV-SS-type blocking also
/// SUPPRESSES the single-step `#DB` after the next instruction, and a
/// suppressed step is lost, not deferred. Measured on this platform
/// (`a_single_step_trap_survives_the_imposed_inhibit`): the instruction that
/// consumes the imposed bit still delivers its single-step trap, in order —
/// the write is transparent to `TF` stepping here, and the test stands guard
/// on that answer.
///
/// What a consumer that CAN read its interpreter's inhibit writes instead is
/// the real value: VirtualBox's NEM backend exports
/// `CPUMIsInInterruptShadow` into this register, anchors the shadow to the
/// `RIP` fetched beside it on import so a shadow never outlives its
/// instruction, and skips the write when the previous and current values are
/// both clear (VirtualBox `NEMAllNativeTemplate-win.cpp.h`
/// `nemHCWinCopyStateToHyperV` / `nemHCWinCopyStateFromHyperV`). That
/// accessor is exactly what rusty_box keeps crate-private, so the honest
/// value available on this side of the seam is the presumption above.
fn impose_the_shadow(
    partition: &Partition,
    state: &VcpuArchState,
    xsave: &mut XsaveArea,
    previous: Option<&VcpuArchState>,
    nmi_masked: bool,
) -> Result<()> {
    let vp = Vp::new(partition, BOOT_VP);
    state::import(&vp, state).map_err(|error| refused_state(error, state))?;
    // Bit 0 is the interrupt shadow, bit 1 the NMI mask.
    let interrupt_state = 1u64 | (u64::from(nmi_masked) << 1);
    vp.write_words(&[Reg::InterruptState], &[interrupt_state])
        .map_err(platform_failed)?;
    if previous.is_none_or(|held| xsave::vector_file_differs(state, held)) {
        xsave.patch(state).map_err(uncarried)?;
        xsave.write_to(partition, BOOT_VP).map_err(platform_failed)?;
    }
    Ok(())
}

/// The shadow holds extended state the partition's area has no place for —
/// a defect in what this engine offered the guest, reported rather than
/// silently dropped on the floor.
fn uncarried(refused: xsave::UncarriedComponent) -> CpuError {
    tracing::error!(
        "the shadow holds extended-state component {} and the partition's area cannot carry it",
        refused.index
    );
    CpuError::UnsupportedCpuOperation {
        operation: "an extended-state component the partition cannot carry",
    }
}

/// Describe the shadow from the platform's processor.
///
/// # Errors
/// A register the platform refused to hand over, or a state this port will not
/// import — a segment whose attributes describe no descriptor it can build.
fn read_back_into_the_shadow<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    machine_ticks: u64,
) -> Result<()> {
    let Started {
        alarm: _,
        partition,
        state,
        installed: _,
        shadowed,
        nmi_masked,
        consecutive_mmio: _,
        held,
        xsave,
        // What the last exit header said; a state exchange does not change it.
        inject: _,
    } = started;
    let vp = Vp::new(partition, BOOT_VP);
    state::export(&vp, state).map_err(platform_failed)?;
    // The x87 and vector file arrives beside the named registers, so the
    // shadow starts from everything the hardware left behind — not just what
    // has a register name. A stretch of guest code that lands here mid
    // string routine reads the very bytes the hardware was holding.
    xsave.refresh_from(partition, BOOT_VP).map_err(platform_failed)?;
    xsave.fill(state);
    // The one moment the two are known to agree, because the shadow was just
    // copied from the processor.
    *held = Some(state.clone());

    // Read alongside the architectural state, because it is not part of it:
    // the interrupt shadow is a property of where the guest stopped, and the
    // only place it exists is the partition. Bit 0 of
    // `WHvRegisterInterruptState` is the shadow; bit 1 is the NMI mask.
    let mut interrupt_state = [0u64; 1];
    vp.read_words(&[rusty_box_whp::Reg::InterruptState], &mut interrupt_state)
        .map_err(platform_failed)?;
    *shadowed = interrupt_state[0] & 1 != 0;
    *nmi_masked = interrupt_state[0] & 2 != 0;

    cpu.import_arch_state(state).map_err(|error| {
        tracing::error!("the hypervisor returned a state this port refuses: {error:?}");
        CpuError::UnsupportedCpuOperation { operation: "hypervisor state refused on import" }
    })?;
    // The hardware may have written any page of the shared memory while it
    // held the guest, and no write of its makes it into the interpreter's
    // self-modifying-code stamps. Whatever the shadow decoded before this
    // hand-back may therefore describe bytes that are gone — a guest kernel
    // patching its own text leaves a transient `INT3` at the patch site, and
    // a cached trace must not deliver it after the patch retired.
    cpu.discard_decoded_traces();

    // One counter, whichever processor the guest asks. `RDTSC` is not a trapped
    // instruction here, so on the hardware it answers from the partition's own
    // counter; the shadow would otherwise answer from this machine's tick
    // clock, which has neither the same epoch nor the same rate. A guest that
    // reads it on both sides subtracts one from the other, and the subtraction
    // is unsigned: Linux's `delay_with_tsc` takes `start` on one processor and
    // `now` on the other, sees a difference larger than the whole delay it was
    // waiting out, and returns at once. `check_timer` then measures no timer
    // ticks in a window that never happened and panics with `IO-APIC + timer
    // doesn't work` before userspace.
    //
    // So the shadow is set to the counter the guest last saw, at the tick the
    // machine is standing on. Rebased at every read-back rather than converted
    // once, because the two run at different rates and only the handoffs are
    // points where they are known to agree.
    cpu.set_tsc(state.msrs.tsc, machine_ticks);

    Ok(())
}

/// How a slice ended, and how much of its wall-clock time the guest was
/// actually executing for.
///
/// The two are not the same number, and treating them as one is what let a
/// guest outrun its own devices. A slice spends its wall clock on guest
/// execution AND on this engine's own work — the architectural state exchange
/// around a trapped instruction, a device answering a port, the shadow
/// finishing what the hardware handed back undone. None of that second part is
/// the guest running, so none of it may become guest time.
struct SliceOutcome {
    /// Why the processor came back.
    yielded: Yielded,
    /// Host time spent inside the platform's run call, and nowhere else.
    ran: Duration,
    /// How many times the hardware handed the guest back during this slice.
    ///
    /// The per-slice figure rather than the running total in [`ExitCounts`]:
    /// what a slice cost is a VM entry plus this many VM exits, and a total
    /// divided by a slice count would only give the mean of a distribution
    /// whose shape is the thing in question.
    exits: u64,
}

/// Run the processor, servicing what the platform cannot, until something the
/// machine owns has to happen.
fn run_until_the_machine_is_needed<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    deadline: Duration,
    budget: u64,
    ips: u64,
    counts: &mut ExitCounts,
    inject_census: &mut InjectCensus,
    history: &mut ExitHistory,
) -> Result<SliceOutcome> {
    let mut ran = Duration::ZERO;
    let mut exits = 0u64;
    let yielded = run_the_exit_loop(
        started, cpu, io, deadline, budget, ips, counts, inject_census, &mut ran, &mut exits,
        history,
    );
    // The alarm is armed per run inside the loop, so an error path can leave
    // one armed against a processor nobody is running. Disarming here is what
    // makes that impossible.
    started.alarm.disarm();
    yielded.map(|yielded| SliceOutcome { yielded, ran, exits })
}

/// The exit loop proper, wrapped so the alarm is disarmed on every path out —
/// including an error.
fn run_the_exit_loop<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    deadline: Duration,
    budget: u64,
    ips: u64,
    counts: &mut ExitCounts,
    inject_census: &mut InjectCensus,
    ran: &mut Duration,
    exits: &mut u64,
    history: &mut ExitHistory,
) -> Result<Yielded> {
    // Whether the platform holds an interruption it has begun — or been
    // handed — and not yet delivered. While it does, the slice may not end —
    // see the `Canceled` arm — so the budget checks below stand down for
    // exactly one more entry. Seeded from the engine's own record rather than
    // from `false`, so a delivery staged at the slice head is owed its entry
    // by the very first iteration.
    let mut delivery_in_flight = started.inject.in_flight;
    loop {
        // Checked before running rather than after, so a slice whose budget is
        // already spent hands the machine back without another exit's worth of
        // guest time on top of it.
        //
        // Spent against the time the GUEST ran, not against the wall clock. A
        // slice dominated by exits — the IDE probe is nothing else — spends
        // most of its wall time in this engine rather than in the guest, and
        // charging that to the guest's budget ends the slice having executed
        // almost nothing while the machine's clock advances as though a full
        // slice had passed. That is how a disk seek modelled as three thousand
        // ticks was answered before its driver could poll once.
        //
        // NOT re-derived from the machine's countdown each time round, which
        // was tried and measured: that countdown does not decrement while a
        // slice runs — the machine is not ticking — so a small value at slice
        // entry stays small and clamps every iteration to the floor below,
        // turning every exit into its own 50-microsecond slice with a whole
        // architectural state exchange each way. It made the boot thirty times
        // slower and fixed nothing.
        // Spent in the unit the machine asked in. An exit is the one moment
        // this engine can stop the guest for free — it is already stopped —
        // and during any stretch that matters for device timing the guest is
        // exiting constantly, because polling a device IS a port exit. So the
        // budget is checked here, against guest time earned, and the alarm
        // below is left as the backstop for a guest that exits for nothing.
        //
        // Checking host time here instead is what let a slice overshoot by
        // three orders of magnitude: 45,000 ticks is 4.7 microseconds of host
        // time at this machine's rate, the floor rounds that to 50, and the
        // alarm — a condition variable, on an operating system whose timed
        // waits are granular to the millisecond — could not tell either from
        // four milliseconds. Every slice ran 836 times its budget, so every
        // device deadline inside it was serviced late, together, at the end.
        if !delivery_in_flight && ticks_elapsed(*ran, ips) >= budget {
            return Ok(Yielded::Budget);
        }
        let left = match deadline.checked_sub(*ran).filter(|left| !left.is_zero()) {
            Some(left) => left,
            // Past the deadline with a delivery in flight: the slice cannot
            // end yet, so the processor gets one more entry — long enough for
            // an injection to land, bounded by the shortest stretch the alarm
            // can measure rather than by a budget already spent.
            None if delivery_in_flight => shortest_hardware_slice(),
            None => return Ok(Yielded::Budget),
        };
        delivery_in_flight = false;
        // The budget needs a voice that does not depend on the guest exiting:
        // a guest in a loop that touches no device and no unmapped page is
        // inside the platform's run call and never reaches the check above.
        // Armed per run rather than once around the loop, because what remains
        // of a budget denominated in guest time is known only here — an alarm
        // set once against the wall clock would fire while this engine was
        // servicing an exit, which is time the budget never bought.
        started.alarm.arm(std::time::Instant::now() + left);
        let entered = std::time::Instant::now();
        let exit = started.partition.run(BOOT_VP);
        *ran += entered.elapsed();
        // Disarmed before the error is propagated, so no path leaves an alarm
        // pointed at a processor that is no longer running. A cancellation
        // that lands between the run returning and this call is sticky and
        // would otherwise end the next run before it began.
        started.alarm.disarm();
        let exit = exit.map_err(platform_failed)?;
        // The first thing done with a fresh exit: cache what its header says
        // about deliverability, so everything below — the tallies, the
        // history, every arm — runs with the cache already current.
        started.inject.refresh_from(&exit.vp);
        // Every exit this slice took, whatever its reason. Counted apart from
        // the per-class tally below, whose match leaves some reasons out: what
        // a slice cost is one VM entry plus this many VM exits, and a reason
        // this engine does not classify cost exactly as much as one it does.
        *exits += 1;
        // A delivery the platform has begun owns the processor until it lands
        // (`execution_state` bit 6, `InterruptionPending`, now cached). This
        // is the one choke point for that hazard (R5): before this engine
        // injected, no exit could carry the bit and it was only a tripwire;
        // now every serviced exit — a memory access, a `CPUID`, an MSR —
        // flows into a shadow errand that rewrites the whole VP state, and
        // doing that underneath a delivery in flight corrupts it. So the
        // processor goes straight back until the delivery lands, exactly the
        // standdown a `Canceled` exit takes, and only then is the re-exit —
        // now without the bit — serviced. The delivery lands on re-entry
        // because its IDT and stack pushes stay in RAM on this machine and
        // never themselves exit. (QEMU `target/i386/whpx/whpx-all.c`
        // `whpx_vcpu_pre_run` refuses to stage over an `InterruptionPending`
        // for the same reason.)
        if started.inject.in_flight {
            tracing::debug!(
                target: "irq",
                "exit at {:#x} arrived mid-delivery; re-entering until it lands",
                exit.vp.rip
            );
            delivery_in_flight = true;
            continue;
        }
        // How long the current run of memory exits is, maintained in one place
        // (R5) so the two readings — extending a run and ending one — cannot
        // disagree. Any exit that is not a memory access ends the run: the
        // guest asking a device a question is not a guest looping over device
        // memory, and the burst is for the second.
        started.consecutive_mmio = match exit.reason {
            ExitReason::MemoryAccess(_) => started.consecutive_mmio.saturating_add(1),
            _ => 0,
        };
        // Counted before it is serviced, so the tally describes what the guest
        // asked for even when servicing it fails.
        match exit.reason {
            ExitReason::IoPortAccess(_) => counts.port += 1,
            ExitReason::MemoryAccess(_) => counts.memory += 1,
            ExitReason::Cpuid(_) => counts.cpuid += 1,
            ExitReason::MsrAccess(_) => counts.msr += 1,
            ExitReason::Halt => counts.halt += 1,
            ExitReason::Canceled { .. } => counts.canceled += 1,
            _ => {}
        }
        history.record(
            exit.vp.rip,
            match exit.reason {
                ExitReason::IoPortAccess(access) => {
                    if access.string_op || access.rep_prefix {
                        b'S'
                    } else if access.is_write {
                        b'o'
                    } else {
                        b'i'
                    }
                }
                ExitReason::MemoryAccess(_) => b'm',
                ExitReason::Cpuid(_) => b'c',
                ExitReason::MsrAccess(_) => b'r',
                ExitReason::Halt => b'h',
                ExitReason::Canceled { .. } => b'x',
                ExitReason::InterruptWindow => b'w',
                _ => b'?',
            },
        );
        match exit.reason {
            // The one exit the platform pre-decodes: port, width, direction and
            // RAX all arrive in the exit, so no decoder is involved and the
            // machine's own device dispatch answers it directly.
            ExitReason::IoPortAccess(access) => {
                if access.string_op
                    || access.rep_prefix
                    || ports_on_the_shadow()
                    || exit.vp.rflags & RFLAGS_TF != 0
                {
                    // A string or repeated port access moves memory as well as
                    // a register, and the exit describes only the register.
                    // Finishing it from the exit alone would transfer one item
                    // and step over the rest — an IDE `REP INSW` reading a
                    // 512-byte sector would deliver two bytes and the guest
                    // would never know. The shadow runs the instruction whole,
                    // through the machine's own dispatch, exactly as the
                    // interpreter would.
                    //
                    // A guest single-stepping owes a `#DB` after this
                    // instruction as well, and finishing it from the exit —
                    // RIP arithmetic, no shadow instruction — leaves nobody
                    // to deliver it. The shadow retires it and its tail takes
                    // the trap. TF is read from the exit header, already
                    // here, so a guest that is not stepping pays nothing.
                    finish_on_the_shadow(started, cpu, io, Trapped::Access)?;
                } else {
                    service_port_access(started, io, &exit, access)?;
                }
                // A device answering that write may have latched something on
                // the bus — an interrupt line, a hold request, a machine
                // boundary to service. The interpreter drains those at its own
                // next instruction boundary; this is that boundary here.
                io.sync_io_events(cpu);
            }
            // A halted processor is the machine's business: its HLT
            // fast-forward advances time to the next deadline and decides when
            // the guest may run again.
            ExitReason::Halt => return Ok(Yielded::Halted),
            // Someone asked for the processor back. Which someone matters:
            // past the deadline it is this slice's own alarm, which is the
            // budget running out and not an interruption.
            ExitReason::Canceled { .. } => {
                // A cancel that arrived with a delivery in flight was already
                // handled by the standdown above (the `in_flight` check after
                // the refresh), which is why this arm no longer tests bit 6:
                // a text-poke `#BP` held pending across a cancel and delivered
                // after the shadow had moved the guest on showed as `Oops:
                // int3`, so the delivery must land before the slice ends, and
                // it now does for every exit reason rather than this one alone.
                // What remains here is the ordinary cancel: past the deadline
                // it is this slice's own alarm — the budget running out — and
                // otherwise the host asking for the processor back.
                return Ok(if *ran >= deadline {
                    Yielded::Budget
                } else {
                    Yielded::Canceled
                });
            }
            // The platform documents this as one a run never returns.
            ExitReason::None => return Err(unserviced("the platform's own \"no reason\"", &exit)),
            // An access the partition's map does not answer: a device window,
            // a page the plan left out, or a write to a range mapped read-only.
            // The shadow finishes it, because nothing else can — see below.
            ExitReason::MemoryAccess(access) => {
                tracing::trace!(
                    "servicing a {:?} at gpa {:#x} on the shadow processor",
                    access.access,
                    access.gpa
                );
                if started.consecutive_mmio >= BURST_AFTER {
                    burst_on_the_shadow(started, cpu, io)?;
                } else {
                    finish_on_the_shadow(started, cpu, io, Trapped::Access)?;
                }
            }
            // What the guest asked about the processor. Answered by executing
            // the instruction on the shadow, so the answer is this port's
            // model rather than the host's silicon — see the exit list above.
            ExitReason::Cpuid(access) => {
                let leaf = access.rax as u32;
                finish_on_the_shadow(started, cpu, io, Trapped::Cpuid { leaf })?;
            }
            ExitReason::MsrAccess(_) => {
                finish_on_the_shadow(started, cpu, io, Trapped::Access)?;
            }
            // The platform's answer to a notification `stage_injection`
            // armed: delivery has just become possible. No injection happens
            // in this arm — QEMU's whpx accelerator keeps the window exit a
            // pure notification too (QEMU whpx-all.c `whpx_vcpu_post_run`
            // only clears its `window_registered` flag) — because the
            // staging below runs with this exit's own header and re-asks
            // every gate rather than trusting a stale answer.
            ExitReason::InterruptWindow => {
                counts.window += 1;
                started.inject.window = None;
            }
            ExitReason::Exception => {
                // The platform hands a trapped fault back instead of
                // delivering it, so the processor still stands on the
                // instruction that raised it and the shadow can finish that
                // instruction itself. It either completes what the platform
                // refused — the firmware's write to `IA32_FEATURE_CONTROL` is
                // the one every boot reaches — or raises the same fault and
                // enters the guest's own handler. Either way the guest sees
                // what the interpreter would have shown it, which is the same
                // treatment an MMIO access, a `CPUID` and a port access get.
                if reports_each_fault() {
                    report_the_fault(started, cpu, io, &exit, history)?;
                }
                finish_on_the_shadow(started, cpu, io, Trapped::Access)?;
            }
            ExitReason::Rdtsc => return Err(unserviced("RDTSC", &exit)),
            ExitReason::UnrecoverableException => {
                return Err(unserviced("unrecoverable exception", &exit))
            }
            ExitReason::InvalidVpRegisterValue => {
                // The platform refuses to run the processor it holds, and
                // does not say which register offends. The last state this
                // engine wrote is the prime suspect, so it is reported whole
                // — segments first, because the architecture's entry checks
                // put most of their rules on segments.
                if let Some(held) = started.held.as_ref() {
                    report_the_state_the_platform_refused(held);
                }
                return Err(unserviced("invalid processor register", &exit));
            }
            ExitReason::UnsupportedFeature { .. } => return Err(unserviced("unsupported feature", &exit)),
            ExitReason::ApicEoi { .. }
            | ExitReason::ApicSmiTrap
            | ExitReason::ApicInitSipiTrap
            | ExitReason::ApicWriteTrap
            | ExitReason::SynicSintDeliverable
            | ExitReason::Hypercall => return Err(unserviced("a partition-APIC exit", &exit)),
            ExitReason::Unrecognized(code) => {
                tracing::error!("the platform reported exit reason {code}, which is newer than this port");
                return Err(unserviced("an exit reason newer than this port", &exit));
            }
        }

        // Servicing that exit may have left the machine work only the machine
        // can do — most importantly a device timer, armed while the device was
        // answering, which raises the interrupt the guest is waiting for and
        // cannot fire until the machine has the processor back. Asked after
        // every serviced exit rather than only after a port write, because a
        // device reached through the shadow arms timers the same way.
        if cpu.wants_a_machine_boundary() {
            counts.boundary += 1;
            return Ok(Yielded::Boundary(BoundaryReason::ProcessorAsked));
        }
        if io.needs_boundary() {
            counts.boundary += 1;
            return Ok(Yielded::Boundary(BoundaryReason::DeviceLatched));
        }

        // Bring the bus's levels onto the processor for every exit, not only
        // for the port write that has its own drain above: a device reached
        // through the shadow raises interrupt lines the same way, and a line
        // nobody moved onto the processor is a line the guest never sees.
        io.sync_io_events(cpu);

        // An errand that put the shadow to sleep — a burst that reached
        // `HLT`, or the instruction a `MOV SS` shadowed being one — ends the
        // slice exactly as a hardware `Halt` exit does. The partition now
        // holds the state after the `HLT`, and running it would carry the
        // guest past a halt it executed. The shadow's own activity state is
        // what the machine reads, its fast-forward decides when the guest
        // may run again, and the wake goes through the slice head's
        // interpreted delivery — so a trap the halting instruction owes
        // lands where Bochs lands it, after the wait ends and before anything
        // else (event.cc handleAsyncEvent: handleWaitForEvent, then
        // Priority 4). Nothing is staged into a halted processor.
        if !matches!(cpu.activity_state, CpuActivityState::Active) {
            return Ok(Yielded::Halted);
        }

        // An NMI, SMI or INIT outranks any maskable vector (the order Bochs
        // event.cc keeps at its own boundary) and is the shadow's to
        // deliver, so no external interrupt is staged over one: the slice
        // ends below and the head delivers in the architecture's own order.
        if !cpu.has_non_ext_int_event() {
            match stage_injection(started, cpu, io, inject_census)? {
                Staged::Injected(vector) => {
                    tracing::debug!(
                        target: "irq",
                        "CPU: vector {vector:#04x} injected into the partition at {:#x}",
                        exit.vp.rip
                    );
                    // The acknowledge changed the controllers' lines; the
                    // processor's latched copy follows before anything asks.
                    io.sync_io_events(cpu);
                    // The injected event is the platform's until it lands, so
                    // the slice may not end before the guest runs again — the
                    // same standdown an in-flight cancellation gets.
                    delivery_in_flight = true;
                    continue;
                }
                // Delivery is blocked and a window stands armed: straight
                // back to the hardware, whose window exit is the next word
                // on the subject. The budget checks at the loop's head still
                // bound the wait.
                Staged::Windowed => continue,
                Staged::Nothing => {}
            }
        }

        // An event still deliverable HERE ends the slice: everything
        // injection cannot carry — an NMI, an SMI, an INIT, each the
        // shadow's to deliver at the head of a slice — and any external
        // vector staging had to leave with the machine. Ending costs one
        // architectural state exchange; the deliverable maskable vector,
        // the common case, was injected above and never pays it.
        if cpu.has_an_event_to_deliver() {
            counts.boundary += 1;
            return Ok(Yielded::Boundary(BoundaryReason::EventToDeliver));
        }
    }
}

/// What staging an injection decided — a named outcome, so a caller cannot
/// mistake "armed a window and the vector waits" for "there was nothing to
/// do", which a bool or a bare `Option` would let it.
enum Staged {
    /// Nothing was handed to the partition: no deliverable external vector
    /// exists, a delivery the platform has begun is still in flight, or the
    /// acknowledge found only a stale line to reconcile.
    Nothing,
    /// Delivery is blocked right now — an interrupt shadow, or `IF` clear —
    /// so a deliverability notification stands armed and the vector stays
    /// with the machine's controllers until the window opens.
    Windowed,
    /// This vector was acknowledged at the machine's controllers and written
    /// into the partition's pending-event slot.
    Injected(u8),
}

/// `WHV_DELIVERABILITY_NOTIFICATIONS_REGISTER`'s `InterruptNotification`
/// bit, from the SDK's `WinHvPlatformDefs.h` (the AMD64 layout:
/// `NmiNotification` bit 0, `InterruptNotification` bit 1,
/// `InterruptPriority` bits 2..6). The windows-sys bindings collapse the
/// layout into one opaque bitfield, so the word is built by hand.
const INTERRUPT_NOTIFICATION: u64 = 1 << 1;

/// Hand a deliverable external interrupt to the partition as a register
/// write, or arm a window to be told the moment the guest could take one.
///
/// The gate is QEMU's, transcribed from `target/i386/whpx/whpx-all.c`
/// `whpx_vcpu_pre_run` (userspace-irqchip path): inject only when no
/// delivery is already in flight, the guest is not in an interrupt shadow,
/// and `IF` is set — and acknowledge at the machine's own controllers only
/// after every gate has passed, because the pop IS the INTA moment and a
/// vector acknowledged cannot be put back.
///
/// The ONE place this engine writes `WHvRegisterPendingInterruption` (R5).
/// A second injection site would be a second acknowledge path, and
/// [`InjectCensus::injected`] could no longer equal the interrupt fabric's
/// own acknowledge count.
///
/// External interrupts only: `has_deliverable_ext_int` answers for the
/// maskable external vectors alone, so an NMI, SMI or INIT never reaches
/// the acknowledge below — those end the slice and the shadow delivers
/// them at its head.
fn stage_injection<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    census: &mut InjectCensus,
) -> Result<Staged> {
    // A delivery the platform has begun is the partition's until it lands;
    // staging over it would replace an event the guest was owed.
    if started.inject.in_flight {
        return Ok(Staged::Nothing);
    }
    // The only fresh TPR there is: a guest `MOV CR8` retires on hardware
    // without an exit, so the shadow's copy is stale until the header's
    // nibble is written back — and the acknowledge below prioritises
    // against the TPR.
    cpu.set_lapic_tpr_from_cr8(started.inject.cr8);
    if !cpu.has_deliverable_ext_int() {
        return Ok(Staged::Nothing);
    }
    // Blocked right now — an interrupt shadow, or IF clear. Both are read
    // from the freshness cache, which an errand this iteration will have
    // republished from the shadow (see `InjectState`): the question is
    // whether the processor the NEXT VM entry runs can take a delivery, not
    // whether the one at the last exit could. Ask the platform to exit the
    // moment delivery becomes possible, and leave the vector with the
    // machine's controllers: nothing is acknowledged for a delivery that
    // cannot happen yet.
    if started.inject.shadowed || !started.inject.if_flag {
        // Priority 0: notify on ANY deliverable interrupt. QEMU derives
        // `irr >> 4` by reading its own APIC's request register without
        // acknowledging; this machine's local APIC and 8259 export no
        // acknowledge-free read of the pending vector across this seam —
        // the popper below IS the INTA — so the widest ask is armed
        // instead. A priority lower than the pending vector's only makes
        // the notification arrive sooner than a filtered one would; it can
        // never miss a deliverable vector.
        const ANY_PRIORITY: u8 = 0;
        // QEMU's dedup, same function: a window already armed at (or above)
        // this priority is not re-armed. Only priority 0 is ever armed
        // here, so any recorded window already covers this ask.
        if started.inject.window.is_none() {
            let word = INTERRUPT_NOTIFICATION | (u64::from(ANY_PRIORITY & 0xF) << 2);
            Vp::new(&started.partition, BOOT_VP)
                .write_words(&[Reg::DeliverabilityNotifications], &[word])
                .map_err(platform_failed)?;
            census.windows_armed += 1;
            started.inject.window = Some(ANY_PRIORITY);
        }
        return Ok(Staged::Windowed);
    }
    // THE INTA MOMENT — reached only with every gate passed. LAPIC before
    // 8259, the fabric's counted acknowledge, spurious vectors and the
    // deasserted-pin reconcile included: the same body the interpreter
    // acknowledges through, which is what keeps the two engines
    // acknowledging in one order.
    let Some(vector) = io.pop_deliverable_vector(cpu) else {
        return Ok(Staged::Nothing);
    };
    started
        .partition
        .inject(
            BOOT_VP,
            PendingInterruption {
                kind: InterruptionType::Interrupt,
                vector: u16::from(vector),
                error_code: None,
            },
        )
        .map_err(platform_failed)?;
    // The platform holds a delivery now. The next exit's header will say so
    // itself; until one arrives, this is the record.
    started.inject.in_flight = true;
    census.injected += 1;
    census.injected_per_vector[usize::from(vector)] =
        census.injected_per_vector[usize::from(vector)].saturating_add(1);
    Ok(Staged::Injected(vector))
}

/// How many instructions a system-management handler may take before this
/// engine stops believing it is one.
///
/// The chipset's own handler is a few hundred instructions; a bound three
/// orders of magnitude above that costs a correct machine nothing and stops an
/// incorrect one from spinning inside a slice forever, where no timer of the
/// machine's could ever fire to end it.
const SMM_HANDLER_CEILING: u64 = 1_000_000;

/// Run the shadow until it leaves system-management mode.
///
/// SMM is the one mode this engine cannot hand to the hardware. No hypervisor
/// offers it, the state a processor saves on entry lives in SMRAM in a layout
/// the hardware would not produce, and `RSM` outside SMM is an invalid opcode —
/// so a partition given a half-entered handler executes it as ordinary code
/// and triple-faults, which is exactly what a Bochs BIOS does at
/// `SMBASE + 0x8000` once its chipset enables the SMI.
///
/// Running it on the shadow instead is not a workaround but the parity answer:
/// the handler executes on this port's own processor, against this port's own
/// SMRAM, exactly as it would with no hypervisor present. The guest is handed
/// back at `RSM`, in the state the handler left, and cannot tell.
///
/// # Errors
/// A fault the shadow could not take, or a handler that never returns.
fn run_the_shadow_out_of_smm<T: Instrumentation>(
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
) -> Result<()> {
    let mut executed = 0u64;
    while cpu.is_in_smm() {
        if executed >= SMM_HANDLER_CEILING {
            tracing::error!(
                "a system-management handler has run {SMM_HANDLER_CEILING} instructions \
                 without returning; the machine cannot be handed back mid-handler"
            );
            return Err(CpuError::UnsupportedCpuOperation {
                operation: "a system-management handler did not return",
            });
        }
        io.emulate_one(cpu)?;
        executed += 1;
    }
    Ok(())
}

/// What the guest was doing when the hardware handed it back.
///
/// Only `CPUID` needs telling apart: its answer is the one thing this engine
/// adjusts on the way out, because it is the one thing that describes what the
/// machine can do rather than reporting what it did.
#[derive(Clone, Copy, Debug)]
enum Trapped {
    /// A memory access, or a model-specific register. Whatever the shadow made
    /// of it is what the guest gets.
    Access,
    /// `CPUID` for this leaf.
    Cpuid { leaf: u32 },
}

/// Report the whole state the platform refused to run, so the register that
/// broke an architectural entry check can be found by inspection — the
/// refusal itself names nothing.
fn report_the_state_the_platform_refused(held: &VcpuArchState) {
    tracing::error!("the platform refuses this processor; the state last written:");
    const NAMES: [&str; 6] = ["es", "cs", "ss", "ds", "fs", "gs"];
    for (name, seg) in NAMES.iter().zip(held.segments.iter()) {
        tracing::error!(
            "  {name}: sel={:#06x} base={:#x} limit={:#x} (scaled {:#x}) attr={:#06x}",
            seg.selector,
            seg.base,
            seg.limit,
            seg.scaled_limit(),
            seg.attributes.bits()
        );
    }
    tracing::error!(
        "  ldtr sel={:#06x} base={:#x} limit={:#x} attr={:#06x}; tr sel={:#06x} base={:#x} \
         limit={:#x} attr={:#06x}",
        held.ldtr.selector,
        held.ldtr.base,
        held.ldtr.limit,
        held.ldtr.attributes.bits(),
        held.tr.selector,
        held.tr.base,
        held.tr.limit,
        held.tr.attributes.bits()
    );
    tracing::error!(
        "  rip={:#x} rflags={:#x} cr0={:#x} cr3={:#x} cr4={:#x} cr8={:#x} efer={:#x} \
         xcr0={:#x} apic_base={:#x}",
        held.rip,
        held.rflags,
        held.cr0,
        held.cr3,
        held.cr4,
        held.cr8,
        held.msrs.efer,
        held.xcr0,
        held.msrs.apic_base
    );
    tracing::error!(
        "  dr6={:#x} dr7={:#x} gdtr={:#x}/{:#x} idtr={:#x}/{:#x} pat={:#x} star={:#x} \
         lstar={:#x} sfmask={:#x} kgsbase={:#x}",
        held.dr6,
        held.dr7,
        held.gdtr.base,
        held.gdtr.limit,
        held.idtr.base,
        held.idtr.limit,
        held.msrs.pat,
        held.msrs.star,
        held.msrs.lstar,
        held.msrs.sfmask,
        held.msrs.kernel_gs_base
    );
}

/// Take the virtualization features out of an answer this engine cannot honour.
///
/// A guest on this engine cannot run a hypervisor of its own. The shadow
/// processor's nested-virtualization state — the VMCS and VMCB caches — has no
/// place in a `VcpuArchState` and cannot cross the seam, and the hardware would
/// refuse the instructions in any case. So the guest is not told it has them:
/// a machine that offers a feature it cannot honour is worse than one that
/// does not offer it.
///
/// Told otherwise, the first guest to notice is the firmware, and it notices
/// immediately. Bochs's own BIOS reads `CPUID.1:ECX[5]`, believes it, and
/// writes `IA32_FEATURE_CONTROL` to enable VMX — a write the platform refuses,
/// in firmware that has no interrupt descriptor table yet, which is a triple
/// fault before the boot loader has run a single instruction. That is how this
/// was found, at `0xE1E80` in `rombios32`.
///
/// Registered as divergence H3.
fn withhold_virtualisation_from(state: &mut VcpuArchState, leaf: u32) {
    /// `CPUID` leaves its answers in RAX, RBX, RCX, RDX; the state carries the
    /// register file in the processor's own order, where RCX is second.
    const RCX: usize = 1;
    /// `CPUID.1:ECX[5]`, Intel's VMX.
    const VMX: u64 = 1 << 5;
    /// `CPUID.80000001:ECX[2]`, AMD's SVM.
    const SVM: u64 = 1 << 2;

    match leaf {
        1 => state.gprs[RCX] &= !VMX,
        0x8000_0001 => state.gprs[RCX] &= !SVM,
        _ => {}
    }
}

/// Describe a trapped fault while the processor is still standing on it.
///
/// A guest's own crash dump is written after its handler has already run over
/// half of this, so the registers, the segments, the bytes at the faulting
/// instruction and the stack it was about to act on are only readable here.
/// Asked for by name with `WHP_TRAP_EXCEPTIONS`, because a guest takes
/// exceptions as part of running correctly and every one of them would
/// otherwise be reported.
fn report_the_fault<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    exit: &Exit,
    history: &ExitHistory,
) -> Result<()> {
    read_back_into_the_shadow(started, cpu, io.pc_system.time_ticks())?;
    let mut state = VcpuArchState::default();
    cpu.export_arch_state(&mut state);
    tracing::error!(
        "trapped fault at {:#x}:{:#x} (cs base {:#x})",
        exit.vp.cs.selector,
        exit.vp.rip,
        exit.vp.cs.base
    );
    // What the guest was doing on the way here. `o`/`i` are port writes and
    // reads, `S` a string or repeated one, `m` memory, `h` a halt, `w` an
    // interrupt window.
    let trail: std::vec::Vec<std::string::String> =
        history.replay().map(|(rip, kind)| std::format!("{}@{rip:#x}", kind as char)).collect();
    tracing::error!("  exits leading here (oldest first): {}", trail.join(" "));
    const NAMES: [&str; 16] = [
        "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12",
        "r13", "r14", "r15",
    ];
    for (name, value) in NAMES.iter().zip(state.gprs.iter()) {
        tracing::error!("  {name} = {value:#x}");
    }
    const SEGMENTS: [&str; 6] = ["es", "cs", "ss", "ds", "fs", "gs"];
    for (name, seg) in SEGMENTS.iter().zip(state.segments.iter()) {
        tracing::error!(
            "  {name} = {:#06x} base {:#x} limit {:#x} attr {:#x}",
            seg.selector,
            seg.base,
            seg.limit,
            seg.attributes.bits()
        );
    }
    tracing::error!(
        "  rip {:#x} rflags {:#x} cr0 {:#x} cr2 {:#x} cr3 {:#x} cr4 {:#x}",
        state.rip,
        state.rflags,
        state.cr0,
        state.cr2,
        state.cr3,
        state.cr4
    );

    // The bytes at the faulting instruction, and the stack it is about to act
    // on. A fault on `IRET` is a claim about the frame under `SS:ESP`, and
    // this is the only place that frame can still be read as the processor
    // sees it.
    let stack = state.segments[2].base.wrapping_add(state.gprs[4]);
    let code = state.segments[1].base.wrapping_add(state.rip);
    // A window BEFORE the faulting instruction as well as at it: the handler
    // that led here is the interesting part, and it is the bytes immediately
    // preceding the `IRET`.
    let before = code.wrapping_sub(0x60);
    for (what, linear, len) in
        [("before", before, 0x60u64), ("code", code, 16u64), ("stack", stack, 24u64)]
    {
        match started.partition.translate_gva(BOOT_VP, linear) {
            Ok(translation) if translation.result_code == 0 => {
                // Straight out of the allocation at the guest-physical
                // address: RAM below the PCI hole is identity-mapped, which
                // `MemoryPlan`'s own equivalence test asserts page by page. A
                // diagnostic may lean on that; a data path may not.
                match io.memory.allocation_slice(translation.gpa, len) {
                    Some(bytes) => tracing::error!(
                        "  {what} at {linear:#x} (gpa {:#x}): {:02x?}",
                        translation.gpa,
                        bytes
                    ),
                    None => tracing::error!(
                        "  {what} at {linear:#x} (gpa {:#x}) is outside the allocation",
                        translation.gpa
                    ),
                }
            }
            Ok(translation) => tracing::error!(
                "  {what} at {linear:#x} did not translate (code {})",
                translation.result_code
            ),
            Err(error) => {
                tracing::error!("  {what} at {linear:#x} translation failed: {error}")
            }
        }
    }
    Ok(())
}

/// Execute the trapped instruction on the shadow processor.
///
/// The reason a shadow processor is mandatory rather than an optimisation, and
/// the measurement is in `docs/whp-platform-probe-2026-08-27.md`: a memory
/// exit reports `InstructionLength = 0` and does NOT advance `RIP`, so there is
/// no stepping over it; and for a write to a read-only window it carries no
/// instruction bytes either, so there is nothing to decode from the exit. The
/// only thing that can finish it is a processor that reads the instruction out
/// of guest memory and executes it — which is this port's interpreter, running
/// one instruction against the very same machine parts.
///
/// That is also what makes the result right rather than merely possible, and
/// it is why the same treatment serves an access, a `CPUID` and an MSR alike.
/// The access lands through the machine's own routing, so a device window
/// answers exactly as it would under the interpreter; the `CPUID` answers out
/// of this port's own model rather than the host's silicon; the MSR reads and
/// writes this port's own register file. Bit-identical, which is the property
/// the whole design rests on.
///
/// The exchange around it is the whole architectural state each way. A trapped
/// instruction may read any register, and the shadow must be the processor,
/// not an approximation of it.
///
/// # Errors
/// A state the exchange refused, or a fault the shadow could not take.
/// Memory exits in a row before this engine stops handing them back one at a
/// time.
///
/// High enough that a guest touching a device in passing is never bursted, low
/// enough that a loop is caught almost immediately. A guest in an MMIO loop
/// produces thousands in a row; eight is not an accident.
const BURST_AFTER: u32 = 8;

/// Instructions the shadow runs once a burst is called for.
///
/// The trade is one-sided by three orders of magnitude. An exit costs ~136 µs
/// here; the shadow interprets at ~75 M instructions/s, so this burst costs
/// ~55 µs even if the guest leaves device memory immediately and every
/// instruction of it was wasted. One additional exit avoided pays for the whole
/// burst twice over, and a VGA clear avoids thousands.
const BURST_INSTRUCTIONS: u64 = 4096;

/// Run the guest on the shadow processor for a stretch rather than for one
/// instruction, because it is going to trap again immediately.
///
/// The measured case is the kernel clearing the VGA planar aperture: 65,536
/// exits over eight pages, four plane passes of 2,048 word writes, every one a
/// single `mov` that leaves the hardware and pays a 52-register exchange in
/// each direction. On that stretch the interpreter is simply the better engine
/// — it serves device memory without leaving anything — and this is how the
/// engine reaches for it.
///
/// Nothing about correctness changes with the burst: the shadow is this port's
/// own interpreter running against this machine's own devices, so a guest
/// bursted through a stretch sees exactly what a guest interpreted through it
/// sees. Only who executed it differs, and no guest can ask.
fn burst_on_the_shadow<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
) -> Result<()> {
    read_back_into_the_shadow(started, cpu, io.pc_system.time_ticks())?;
    // An interrupt shadow crossing into the burst: the read-back just said
    // the guest stands inside one, and `emulate_batch` below delivers freely
    // at every instruction boundary — so the shadowing instruction retires
    // first with delivery held off, and the flag lapses with it, the same
    // rule the converted-slice path follows.
    if core::mem::replace(&mut started.shadowed, false) {
        io.finish_the_instruction(cpu)?;
    }
    // Not `finish_the_instruction` for the stretch itself: that verb holds
    // off the external interrupt for the length of a single trapped access,
    // which is right when finishing one instruction and wrong for a stretch.
    // Across a burst the interpreter delivers on its own terms, exactly as it
    // does when it owns the machine.
    io.emulate_batch(cpu, BURST_INSTRUCTIONS)?;
    impose_after_errand(started, cpu, io, Trapped::Access)
}

fn finish_on_the_shadow<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    trapped: Trapped,
) -> Result<()> {
    read_back_into_the_shadow(started, cpu, io.pc_system.time_ticks())?;
    // The trapped instruction, whole, on the machine's own dispatch path.
    // Whatever it does — completes the access, moves a sector, or raises a
    // fault and enters a handler — the processor it leaves behind is the one
    // the platform must continue from.
    io.finish_the_instruction(cpu)?;
    impose_after_errand(started, cpu, io, trapped)
}

/// The end every errand shares: deliver the trap the errand owes, republish
/// deliverability from the shadow, then write the shadow into the partition.
///
/// One function rather than a tail repeated in `finish_on_the_shadow` and
/// `burst_on_the_shadow`, because both obligations are safety obligations
/// (R5): the injection tail runs next, and a gate that reads the pre-errand
/// header after the interpreter has moved `IF` acknowledges vectors it must
/// not. With the obligations living here, no errand can skip one without
/// every errand losing it — which is a rewrite, not a slip.
fn impose_after_errand<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    trapped: Trapped,
) -> Result<()> {
    // The trap the retired instruction owes, first. A single-step `#DB` is
    // delivered at the head of the NEXT instruction, and that head belongs
    // to the hardware, which arms a step for its own instruction and knows
    // nothing of the shadow's — so the shadow takes the boundary before it
    // is written into the partition, and what the partition receives is the
    // handler's entry (or, for a shadow that went to sleep, the halt with its
    // trap kept for the wake — the exit loop ends the slice on it). Here
    // rather than in `finish_the_instruction` because every errand ends
    // here: a one-instruction finish and a burst's last instruction owe the
    // same step.
    io.deliver_the_trap_owed(cpu)?;

    // The errand has retired on the shadow, and `impose_the_shadow` below
    // makes the partition identical to it — so the shadow's live `IF`, not
    // the pre-errand exit header, is what the injection tail must judge
    // against, and the inhibit the errand may have armed is presumed live.
    // See `InjectState`'s freshness contract.
    started.inject.refresh_from_shadow(cpu.interrupts_enabled());

    let Started {
        alarm: _,
        partition,
        state,
        installed: _,
        shadowed: _,
        nmi_masked,
        consecutive_mmio: _,
        held,
        xsave,
        // Republished from the shadow just above; the state exchange below
        // does not change it again.
        inject: _,
    } = started;
    cpu.export_arch_state(state);
    if let Trapped::Cpuid { leaf } = trapped {
        withhold_virtualisation_from(state, leaf);
    }
    impose_the_shadow(partition, state, xsave, held.as_ref(), *nmi_masked)?;
    *held = Some(state.clone());
    Ok(())
}

/// Answer a port access out of the machine's own device set, then step the
/// processor past the instruction that caused it.
fn service_port_access(
    started: &mut Started,
    io: &mut PcIo<'_>,
    exit: &Exit,
    access: rusty_box_whp::IoPortAccess,
) -> Result<()> {
    let ticks = io.pc_system.time_ticks();
    let port = access.port;
    let width = access.access_size;

    // The platform hands back RAX WHOLE, whatever the access width, so both
    // directions have to narrow it themselves. An `OUT DX, AL` that handed a
    // device the other three bytes of RAX would be telling it something the
    // guest never wrote, and the interpreter — whose handler passes `AL`,
    // `AX` or `EAX` and nothing else — would tell it something different for
    // the same guest instruction.
    let mask: u64 = match width {
        1 => 0xFF,
        2 => 0xFFFF,
        _ => 0xFFFF_FFFF,
    };

    let rax = if access.is_write {
        io.devices.outp(
            port,
            (access.rax & mask) as u32,
            width,
            ticks,
            io.pc_system,
            io.device_manager,
            io.memory,
        );
        access.rax
    } else {
        let value =
            io.devices
                .inp(port, width, ticks, io.pc_system, io.device_manager);
        // A narrower `IN` leaves the bytes above its width as the guest had
        // them — the same rule the interpreter's own `port_in` follows.
        (access.rax & !mask) | (u64::from(value) & mask)
    };

    let vp = Vp::new(&started.partition, BOOT_VP);
    // Unlike a memory exit, a port exit DOES report its instruction length and
    // does not advance RIP itself, so finishing it is arithmetic rather than a
    // decode (probe finding 2).
    let resume = exit.vp.rip + u64::from(exit.vp.instruction_length);
    vp.write_words(
        &[rusty_box_whp::Reg::Rip, rusty_box_whp::Reg::Rax],
        &[resume, rax],
    )
    .map_err(platform_failed)
}

/// How long a slice may run on the host, from what the machine asked for.
///
/// The machine sizes a slice by the ticks remaining to its next device
/// deadline. An interpreter honours that by counting instructions; this engine
/// counts none, so it honours the same number as a span of host time at the
/// machine's own rate — the same conversion `ticks_elapsed` performs in
/// reverse, and the reason both engines' timers fire at the same guest time.
///
/// A slice that ran past it would starve every timer in the machine, because a
/// device timer fires only when the machine has the processor back.
fn deadline(request: SliceRequest, ips: u64) -> Duration {
    host_time_for(request.instructions(), ips)
}

/// How long the hardware takes to cover `ticks` of this machine's guest time.
///
/// The inverse of [`ticks_elapsed`], and they must stay inverses: a slice that
/// ran longer than the time it reports lets the guest outrun its own devices,
/// and one that ran shorter leaves the machine waiting for time that has
/// already passed.
fn host_time_for(ticks: u64, ips: u64) -> Duration {
    host_time_exactly(ticks, ips).max(SLICE_RESOLUTION)
}

/// The same conversion without the floor — what the machine actually asked
/// for, rather than the least this engine can deliver.
///
/// The two differ precisely when a device deadline is nearer than the hardware
/// can be interrupted, and telling them apart is what
/// [`SliceEngine::run_slice`] uses to decide which processor should run the
/// slice at all.
fn host_time_exactly(ticks: u64, ips: u64) -> Duration {
    if ips == 0 {
        // A machine with no rate cannot say how long a tick is; give the
        // slice this engine's own resolution and let the machine decide.
        return SLICE_RESOLUTION;
    }
    let rate = u128::from(ips).saturating_mul(u128::from(hardware_speed()));
    let nanos = u128::from(ticks).saturating_mul(1_000_000_000) / rate;
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// The shortest stretch this engine can actually run.
///
/// An exit costs about four microseconds on this host, so a slice asked to
/// last less than that cannot run one instruction, let alone one exit's worth
/// — it would return having advanced no guest time at all, and a machine whose
/// budget is denominated in guest time would ask again, forever. That is not
/// hypothetical: a machine asks for a one-tick slice whenever a device
/// deadline is already due, and one tick at 300 MHz is three nanoseconds.
///
/// So the floor is this engine stating its resolution rather than pretending
/// to a precision it does not have. A slice never runs SHORTER than the
/// machine asked for by more than the machine can measure, and never returns
/// claiming that no time passed when it ran.
const SLICE_RESOLUTION: Duration = Duration::from_micros(50);

/// The shortest stretch this engine will hand to the hardware.
///
/// Below it the shadow runs the slice, because the alarm that bounds a
/// hardware slice is a thread wake and cannot end one sooner than
/// [`SLICE_RESOLUTION`]. A machine asking for less gets a slice that overruns —
/// measured on an Alpine boot, 271-fold on average — and a guest whose timer
/// pulses arrive in bursts at slice boundaries rather than at their deadlines.
///
/// The default is low, because letting the hardware run is the point of this
/// engine: DLX boots in 4.3 seconds here against 28 at the alarm's own
/// resolution, and against the interpreter's 10.6. The overrun a short slice
/// takes is real and a guest that tolerates a late timer never notices it.
///
/// A guest that does notice hangs during interrupt setup — Linux says
/// `IO-APIC + timer doesn't work` — and wants `WHP_MIN_SLICE_US=50`, which is
/// the alarm's resolution and the only value that makes the overrun impossible
/// rather than merely small. Note what that costs: at 50 microseconds a
/// machine whose deadlines are closer than that runs entirely on the shadow,
/// so the guest is interpreted and the hardware never sees it. Alpine boots
/// that way today.
fn shortest_hardware_slice() -> Duration {
    static SETTING: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *SETTING.get_or_init(|| {
        match std::env::var("WHP_MIN_SLICE_US").ok().and_then(|us| us.parse().ok()) {
            Some(us) => Duration::from_micros(us),
            None => Duration::from_micros(3),
        }
    })
}


/// How much faster the host's own processor is than the rate this machine
/// nominally runs at.
///
/// A machine states its speed as instructions per second, and a guest on a
/// hypervisor runs on silicon retiring several billion a second. Equating a
/// host second with `ips` ticks — which is what a plain conversion does —
/// therefore pins the guest to real time: a `step` asking for 66 ms of guest
/// time has to burn 66 ms of wall clock to report it, however little the
/// guest had to do, and a boot takes exactly as long as a real machine's boot
/// no matter how fast the host is. Measured before this factor existed: 355
/// guest-seconds in 360 host-seconds, dead on 1:1.
///
/// This is the one number that says the two engines keep time differently, and
/// they may: an interpreter's tick is an instruction it retired, and there is
/// nothing for a hardware engine to count, so it converts the time it spent
/// instead and says by how much the hardware outruns the nominal rate.
///
/// Deliberately an under-estimate. Claiming less than the truth costs only
/// speed; claiming more would let guest time run ahead of the work the guest
/// actually did, and a guest polling a device would see it answer late.
const HARDWARE_SPEED: u64 = default_hardware_speed();

/// How much this engine claims the hardware outruns the machine's nominal rate.
///
/// One means honest time: a second of host time is a second of guest time, so
/// the guest's clock tracks the wall clock the way a VMware or KVM guest's
/// does. A real-time wait — a boot loader's countdown, a device's settling
/// delay — then takes exactly as long as it would on the metal, which is what
/// a person watching the machine expects.
///
/// Above one it is a fast-forward: the guest's clock runs that many times fast,
/// so those waits pass sooner in wall time. It costs two things. The guest sees
/// itself running slower than it is, which is a timing fingerprint; and every
/// device deadline is divided by the same factor in HOST time, which is what
/// pushed them below the alarm's resolution and made timer pulses arrive in
/// bursts rather than at their deadlines.
///
/// `WHP_FAST_FORWARD` sets it for a caller who wants a boot over with and can
/// live with both costs.
const fn default_hardware_speed() -> u64 {
    32
}

/// The fast-forward factor in force, honest time unless asked otherwise.
fn hardware_speed() -> u64 {
    static SETTING: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *SETTING.get_or_init(|| {
        match std::env::var("WHP_FAST_FORWARD").ok().and_then(|n| n.parse().ok()) {
            Some(n) if n >= 1 => n,
            _ => HARDWARE_SPEED,
        }
    })
}

/// Host nanoseconds as machine ticks.
///
/// Both engines denominate time in ticks at `ips`, so a device deadline armed
/// under one is the same deadline under the other and a snapshot crosses
/// between them unchanged (REPLAN decision 1). What differs is how a tick is
/// earned — see [`HARDWARE_SPEED`].
fn ticks_elapsed(elapsed: Duration, ips: u64) -> u64 {
    let nanos = u128::from(elapsed.as_nanos().min(u128::from(u64::MAX)));
    let rate = u128::from(ips).saturating_mul(u128::from(hardware_speed()));
    let ticks = nanos.saturating_mul(rate) / 1_000_000_000;
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_whp::SegmentRegister;

    /// The cache decodes an exit header and nothing else: `InterruptionPending`
    /// is `ExecutionState` bit 6, `InterruptShadow` is bit 12, `IF` is `RFLAGS`
    /// bit 9, `CR8` arrives as the header's own field, and the armed-window
    /// record survives every refresh because no header reports it. Pure struct
    /// decoding — no hypervisor, so it runs on any host.
    #[test]
    fn the_inject_cache_decodes_exit_headers_alone() {
        let header = |execution_state: u16, rflags: u64, cr8: u8| VpContext {
            rip: 0xFFF0,
            rflags,
            cs: SegmentRegister::default(),
            instruction_length: 0,
            cr8,
            execution_state,
        };
        let mut inject = InjectState::at_reset();
        assert!(!inject.in_flight);
        assert!(!inject.if_flag);
        assert!(!inject.shadowed);
        assert_eq!(inject.cr8, 0);
        assert_eq!(inject.window, None);
        inject.window = Some(3);

        // Bits 6 and 12 together, IF set.
        inject.refresh_from(&header(0x1040, 0x202, 0x5));
        assert!(inject.in_flight, "ExecutionState bit 6 is InterruptionPending");
        assert!(inject.shadowed, "ExecutionState bit 12 is InterruptShadow");
        assert!(inject.if_flag, "RFLAGS bit 9 is IF");
        assert_eq!(inject.cr8, 0x5, "the header's CR8 nibble round-trips");
        assert_eq!(inject.window, Some(3), "no header reports an armed window");

        inject.refresh_from(&header(0x0000, 0x2, 0x0));
        assert!(!inject.in_flight, "ExecutionState 0 means nothing in flight");
        assert!(!inject.shadowed, "ExecutionState 0 means no interrupt shadow");
        assert!(!inject.if_flag, "RFLAGS 0x2 has IF clear");
        assert_eq!(inject.cr8, 0);
        assert_eq!(inject.window, Some(3), "a refresh never touches the window");
    }

    /// The freshness contract: after an errand, the shadow's `IF` overrides
    /// whatever the exit header said, and the interrupt shadow is presumed
    /// LIVE — unknown-therefore-blocked. The interpreter's one-instruction
    /// inhibit is bookkeeping this crate cannot read, and an errand can stop
    /// with one armed, so the gate gets no claim that delivery is permitted;
    /// the next exit's header answers truthfully and for free. This is the
    /// property the injection gate turns on: a `CLI` retired on the shadow
    /// between the exit and the staging decision must be seen, and an
    /// invisible `STI`/`MOV SS` window must never be injected into.
    #[test]
    fn a_shadow_errand_republishes_if_and_presumes_the_inhibit() {
        let mut inject = InjectState::at_reset();
        inject.window = Some(0);
        // Header carries IF=1 and NO interrupt shadow — the processor as it
        // was at the exit, BEFORE the errand ran.
        inject.refresh_from(&VpContext {
            rip: 0,
            rflags: 0x202,
            cs: SegmentRegister::default(),
            instruction_length: 0,
            cr8: 0,
            execution_state: 0x0000,
        });
        assert!(inject.if_flag, "the header's IF is set");
        assert!(!inject.shadowed, "the header reports no interrupt shadow");

        // The errand retired a `CLI` (or entered a fault gate): the shadow's
        // live IF is now clear — and whether its last instruction armed an
        // inhibit is unknowable from this crate.
        inject.refresh_from_shadow(false);
        assert!(
            !inject.if_flag,
            "the shadow's IF must override the header's — the gate injects on this"
        );
        assert!(
            inject.shadowed,
            "an errand's inhibit state is unknown, so the gate must be told \
             BLOCKED — never that delivery is permitted without evidence"
        );
        // The window record and in-flight state are the errand's to leave
        // alone: no header or shadow reports the armed notification, and
        // nothing an errand does begins a platform delivery.
        assert_eq!(inject.window, Some(0), "an errand never touches the window");
        assert!(!inject.in_flight, "an errand begins no platform delivery");

        // The next exit's header is authoritative again: bit 12 clear lifts
        // the presumption, which is how a deferred injection resolves after
        // exactly one more exit.
        inject.refresh_from(&VpContext {
            rip: 0,
            rflags: 0x202,
            cs: SegmentRegister::default(),
            instruction_length: 0,
            cr8: 0,
            execution_state: 0x0000,
        });
        assert!(!inject.shadowed, "the next header's truth lifts the presumption");
        assert!(inject.if_flag);
    }

    /// A tick is a unit of guest time at the machine's own rate, so a second of
    /// host time is `ips` ticks — the same number the software engine would
    /// have retired in that second by construction.
    #[test]
    fn host_time_becomes_ticks_at_the_machines_own_rate() {
        let per_second = 300_000_000 * HARDWARE_SPEED;
        assert_eq!(ticks_elapsed(Duration::from_secs(1), 300_000_000), per_second);
        assert_eq!(
            ticks_elapsed(Duration::from_millis(1), 300_000_000),
            per_second / 1_000
        );
        assert_eq!(ticks_elapsed(Duration::from_secs(0), 300_000_000), 0);
    }

    /// A slice shorter than one tick is no ticks, not a rounded-up one: time
    /// the guest did not have must never be credited to it, or a device
    /// deadline arrives early.
    #[test]
    fn a_stretch_too_short_to_be_a_tick_is_no_ticks() {
        // Chosen so a tick is a whole number of nanoseconds and the assertion
        // is about the rounding rule rather than about the example.
        let rate = 250_000;
        let nanos_per_tick = 1_000_000_000 / (rate * HARDWARE_SPEED);
        assert_eq!(nanos_per_tick, 125, "the example must divide exactly");
        assert_eq!(ticks_elapsed(Duration::from_nanos(1), rate), 0);
        assert_eq!(ticks_elapsed(Duration::from_nanos(nanos_per_tick - 1), rate), 0);
        assert_eq!(ticks_elapsed(Duration::from_nanos(nanos_per_tick), rate), 1);
    }

    /// A slice runs for the time it will then report, and never longer.
    ///
    /// The two conversions are inverses up to the nanosecond both truncate to,
    /// and the direction of that truncation is the load-bearing part: a slice
    /// that ran LONGER than the time it reports would let the guest outrun its
    /// own devices, so a whole nanosecond is dropped rather than rounded up.
    /// The shortfall is bounded by what one nanosecond is worth.
    #[test]
    fn a_slice_runs_for_the_time_it_will_report_and_never_longer() {
        let ips = 300_000_000;
        let per_nanosecond = ips * HARDWARE_SPEED / 1_000_000_000 + 1;
        for asked in [1_000_000u64, 20_000_000, 100_000_000] {
            let reported = ticks_elapsed(host_time_for(asked, ips), ips);
            assert!(
                reported <= asked,
                "a slice reported {reported} ticks for a span asked to be {asked}"
            );
            assert!(
                asked - reported <= per_nanosecond,
                "a slice asked for {asked} ticks reported {reported}, short by more \
                 than the nanosecond both conversions truncate to"
            );
        }
    }

    /// An implausibly long stretch saturates rather than wrapping a counter the
    /// whole machine's clock is derived from.
    #[test]
    fn an_absurd_stretch_saturates_rather_than_wrapping() {
        assert_eq!(ticks_elapsed(Duration::from_secs(u64::MAX), u64::MAX), u64::MAX);
    }
}
