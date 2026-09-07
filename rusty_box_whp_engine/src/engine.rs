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

use rusty_box_whp::{
    DestinationMode, Exit, ExitReason, ExtendedVmExits, InterceptCounters, InterruptKind,
    InterruptRequest, LocalApicMode, MsrExits, Partition, PartitionConfig, RuntimeCounters,
    TriggerMode, Vcpu, WhpError,
};

use super::xsave;
use rusty_box::cpu::arch_state::VcpuArchState;
use rusty_box::cpu::{cpu::BxCpuC, instrumentation::Instrumentation, CpuError, Result};
use rusty_box::emulator::{
    DeliveryRoute, DeviceClock, Emulator, EventDelivery, PcIo, Processor, Progress, ProgressUnit,
    SliceEngine, SliceRequest,
};
use rusty_box::iodev::ioapic::IoApicDeliveryMode;
use rusty_box::iodev::irq::{IoApicDelivery, IoApicDestinationMode, IoApicTrigger};
use rusty_box_core::{EngineFault, EngineFaultKind};

use rusty_box::memory::plan::MemoryPlan;
use rusty_box::memory::BxMemC;
use rusty_box::GpaWindow;

/// The processor this engine runs. SMP under a hypervisor is its own unit; a
/// machine with more processors than this refuses to start rather than running
/// one and pretending.
const BOOT_VP: u32 = 0;

/// RFLAGS.TF, the trap flag. A guest with it set owes a single-step `#DB`
/// after every instruction — including one this engine finishes for it.
pub(crate) const RFLAGS_TF: u64 = 1 << 8;

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

/// `IA32_APIC_BASE` is among them, which is what an emulated APIC needs: a
/// guest relocating or software-disabling its local APIC writes that register,
/// and under an offloaded APIC the write is the only announcement this engine
/// gets. Stated as an assertion rather than a comment so widening `ALL` cannot
/// quietly take it away.
const _: () = assert!(TRAPPED_MSRS.apic_base_write);

/// The last few exits, kept so a fault can say what led to it.
///
/// A trapped exception reports the instant it happened and nothing about how
/// the guest got there, which for a stack that has drifted out of balance is
/// the only interesting part. Written on every exit and read only when one
/// cannot be serviced, so it costs a store and an increment.
///
/// DIAGNOSTIC, alongside `WHP_TRAP_EXCEPTIONS`, and inert without it.
pub(crate) struct ExitHistory {
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

    pub(crate) fn record(&mut self, rip: u64, reason: u8) {
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

/// The single character an exit leaves in the history.
///
/// One mapping (R5) for the two owners that write the history — the slice loop
/// and the vCPU thread — because the trail is one machine's and a reader who
/// had to know which of them wrote a character could not read it.
pub(crate) const fn history_mark(reason: ExitReason) -> u8 {
    match reason {
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
pub(crate) fn reports_each_fault() -> bool {
    static SETTING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SETTING.get_or_init(|| std::env::var_os("WHP_TRAP_EXCEPTIONS").is_some())
}

/// Whether every port access is finished on the shadow rather than from the
/// exit. Read once, and asked once per port exit — the most frequent exit a
/// booting guest takes.
pub(crate) fn ports_on_the_shadow() -> bool {
    static SETTING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SETTING.get_or_init(|| std::env::var_os("WHP_PORTS_ON_SHADOW").is_some())
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
pub(crate) fn refused_state(error: WhpError, state: &VcpuArchState) -> CpuError {
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

pub(crate) fn platform_failed(error: WhpError) -> CpuError {
    tracing::error!("WHP platform call failed: {error}");
    CpuError::UnsupportedCpuOperation { operation: "the hypervisor refused" }
}

/// A state the platform handed back that this port will not load — a segment
/// whose attributes describe no descriptor it can build.
///
/// The one place that refusal becomes a machine error (R5), so an import
/// through the whole-state path and one through a group mask report it the
/// same way.
pub(crate) fn refused_import(error: rusty_box::cpu::arch_state::ArchStateError) -> CpuError {
    tracing::error!("the hypervisor returned a state this port refuses: {error}");
    CpuError::UnsupportedCpuOperation { operation: "hypervisor state refused on import" }
}

/// `WHvRegisterInterruptState` as this engine exchanges it: bit 0 the
/// interrupt shadow, bit 1 the NMI mask (`WHV_X64_INTERRUPT_STATE_REGISTER`).
/// The one place the word is decoded and encoded (R5), so the read-back and
/// the imposition cannot disagree about which bit is which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InterruptStateWord {
    /// The guest stands inside an `STI` / `MOV SS` / `POP SS` window.
    pub(crate) shadow: bool,
    /// NMIs are masked — the hardware's own record of a guest inside an NMI
    /// handler, which no architectural register the exchange carries holds.
    pub(crate) nmi_masked: bool,
}

impl InterruptStateWord {
    /// A processor that has never run: it cannot be mid-instruction, and
    /// NMIs are unmasked at reset.
    pub(crate) const AT_RESET: Self = Self { shadow: false, nmi_masked: false };

    pub(crate) const fn decode(word: u64) -> Self {
        Self { shadow: word & 1 != 0, nmi_masked: word & 2 != 0 }
    }

    pub(crate) fn encode(self) -> u64 {
        u64::from(self.shadow) | (u64::from(self.nmi_masked) << 1)
    }
}

/// A partition that has been configured, given memory and given a processor.
///
/// FIELD ORDER IS LOAD-BEARING: `vcpu` is declared before `partition` because
/// Rust drops fields in declaration order, and it names this partition's
/// handle. Releasing it before the partition is destroyed is the whole of the
/// ordering obligation, and this is where it is discharged for the handle this
/// struct still holds.
struct Started {
    /// The processor, taken from the partition once — `None` once a vCPU
    /// thread has taken it in turn.
    ///
    /// Two owners can want this processor and only one may run it: the slice
    /// loop below, which reads and writes its registers between runs, and a
    /// [`crate::vcpu_thread::VcpuThread`], which owns it outright for the life
    /// of the thread. [`bring_up`] is the one place it leaves, and it leaves by
    /// being TAKEN, so a slice asked for afterwards refuses through
    /// [`Started::vcpu`] rather than racing a thread for the same registers.
    ///
    /// The ordering obligation leaves with it. While the handle is here it dies
    /// before the partition by field order; once a thread holds it, that
    /// thread's owner must join the thread before this machine drops — the
    /// obligation `rusty_box_whp::Vcpu` states, transferred to whoever called
    /// `bring_up`.
    vcpu: Option<Vcpu>,
    partition: Partition,
    /// Which local APIC this partition has, if any.
    ///
    /// The one fact that decides who owns the guest's interrupts, and what
    /// `owns_the_guests_local_apic` answers from. Under
    /// [`LocalApicMode::None`] this machine's own `cpu/apic.rs` is the guest's
    /// local APIC: the 8259's line reaches the processor as a level, and the
    /// controller is acknowledged only when the processor can take the vector.
    /// Under either emulated mode the partition arbitrates instead, and every
    /// delivery reaches it as a `WHvRequestInterrupt` through
    /// `route_ioapic_delivery` — an I/O APIC message as itself, and the 8259's
    /// line as the vector the machine resolved at its own boundary, because a
    /// local APIC takes numbers and the platform has no verb for asserting
    /// LINT0.
    ///
    /// Recorded rather than re-derived because it is what the platform
    /// ACCEPTED, not what was asked for: the ladder in [`start`] falls back,
    /// and a delivery path that guessed would guess wrong on a host that
    /// refused the first rung.
    apic_mode: LocalApicMode,
    /// The map the partition is currently holding.
    ///
    /// Kept because a borrowed mapping leaves no bookkeeping behind — the
    /// partition records the range only for memory it owns — so this is the
    /// only record of what is installed, and the only way to know which
    /// windows a new map removed rather than merely changed.
    installed: MemoryPlan,
}

impl Started {
    /// The processor, while this engine still holds it.
    ///
    /// # Errors
    /// [`CpuError::UnsupportedCpuOperation`] once [`bring_up`] has handed the
    /// processor to a thread.
    fn vcpu(&self) -> Result<&Vcpu> {
        still_held(&self.vcpu)
    }
}

/// Turn an absent processor into a refusal.
///
/// The one place that absence is named (R5), so no path can reach for a
/// processor a thread is running by unwrapping an `Option` — including the
/// paths that destructure [`Started`] whole and so cannot go through
/// [`Started::vcpu`].
///
/// # Errors
/// [`CpuError::UnsupportedCpuOperation`] once [`bring_up`] has handed the
/// processor to a thread.
fn still_held(vcpu: &Option<Vcpu>) -> Result<&Vcpu> {
    vcpu.as_ref().ok_or(CpuError::UnsupportedCpuOperation {
        operation: "the processor was taken by a thread",
    })
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
    /// A fault the partition handed back rather than delivering, which only
    /// happens when `WHP_TRAP_EXCEPTIONS` asked for it.
    pub exception: u64,
    /// The guest halted.
    pub halt: u64,
    /// The host asked for the processor back mid-run.
    pub canceled: u64,
    /// A device dispatch asked for the machine's boundary to be serviced.
    pub boundary: u64,
    /// An interrupt-window exit. The platform raises it only for a
    /// deliverability notification someone armed, and nothing in this crate
    /// arms one — the partition's own APIC holds every vector the guest is
    /// owed until it can take it — so a count here is a request this engine
    /// never made.
    pub window: u64,
    /// The guest ended an interrupt at a local APIC the hypervisor owns.
    pub apic_eoi: u64,
    /// The guest wrote a local-APIC register the partition asked to trap.
    pub apic_write: u64,
    /// An exit no arm of the service match answers, which is a fault and
    /// therefore at most one per processor.
    ///
    /// Its own bucket rather than silence, because "no exit of class X" is
    /// only assertable when every class has somewhere to be counted: a total
    /// that does not add up is how a reason falling through the tally is
    /// found.
    pub other: u64,
}

impl ExitCounts {
    /// Every exit counted here, whatever its class.
    ///
    /// The cross-check on the individual buckets: one VM entry and one VM exit
    /// per unit, so this is what a stretch of guest time cost in round trips.
    ///
    /// [`Self::boundary`] is deliberately left out. It counts a slice that
    /// ended because the machine had work to do, which is a decision taken
    /// AFTER an exit already counted elsewhere — adding it would count that
    /// exit twice and make the total disagree with the platform's own
    /// intercept count.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.port
            + self.memory
            + self.cpuid
            + self.msr
            + self.exception
            + self.halt
            + self.canceled
            + self.window
            + self.apic_eoi
            + self.apic_write
            + self.other
    }

    /// Add another processor's tally into this one, class by class.
    ///
    /// A machine's exits are the sum of its processors', because each
    /// processor's thread counts its own: there is no longer a single loop
    /// through which every exit passes, so a machine-wide total has to be
    /// built rather than read.
    pub fn absorb(&mut self, other: Self) {
        let Self {
            port,
            memory,
            cpuid,
            msr,
            exception,
            halt,
            canceled,
            boundary,
            window,
            apic_eoi,
            apic_write,
            other: other_class,
        } = other;
        // Named one by one rather than with `..`: a class added to this struct
        // has to be considered here instead of silently going uncounted, which
        // is the same rule the destructure above enforces for the source.
        self.port += port;
        self.memory += memory;
        self.cpuid += cpuid;
        self.msr += msr;
        self.exception += exception;
        self.halt += halt;
        self.canceled += canceled;
        self.boundary += boundary;
        self.window += window;
        self.apic_eoi += apic_eoi;
        self.apic_write += apic_write;
        self.other += other_class;
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
    /// Diagnostic; see [`ExitHistory`]. Lives here rather than in a slice
    /// because a guest's path to a fault crosses slice boundaries — a handler
    /// that traps for its own port I/O is several slices old by the time it
    /// faults, and a per-slice history shows only the fault itself.
    ///
    /// Shared with the vCPU thread, which records into it under the machine's
    /// lock: one history per machine, whichever of the two ran the guest, so a
    /// trail never breaks at the seam between them.
    pub(crate) history: ExitHistory,
}

impl WhpEngine {
    /// What the guest has been leaving the hardware for, since this machine
    /// was built. Reached through `Emulator::engine`.
    #[must_use]
    pub const fn exits(&self) -> ExitCounts {
        self.exits
    }

    /// The partition this engine started, once it has one.
    ///
    /// For the machine's driver, which suspends and resumes the partition's own
    /// clock around a pause: the guest's TSC and its platform timers are the
    /// hypervisor's, and a pause that left them running would hand the guest a
    /// jump in time it did not live through.
    pub(crate) fn partition(&self) -> Option<&Partition> {
        self.started.as_ref().map(|started| &started.partition)
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
        let counters = started.vcpu()?.counters();
        Ok(PlatformCounters {
            intercepts: counters.intercept_counters().map_err(platform_failed)?,
            runtime: counters.runtime_counters().map_err(platform_failed)?,
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

/// The `WHvRequestInterrupt` kind an I/O APIC delivery mode asks for, or
/// `None` when the platform has no kind for it.
///
/// Exhaustive on the I/O APIC's own closed set (R5). Two modes have no
/// counterpart and they are different absences: SMI is a gap in this port, and
/// the two reserved encodings are a guest programming error. ExtINT has one,
/// and its arm says why.
const fn requested_kind(mode: IoApicDeliveryMode) -> Option<InterruptKind> {
    match mode {
        IoApicDeliveryMode::Fixed => Some(InterruptKind::Fixed),
        IoApicDeliveryMode::LowPriority => Some(InterruptKind::LowestPriority),
        IoApicDeliveryMode::Nmi => Some(InterruptKind::Nmi),
        IoApicDeliveryMode::Init => Some(InterruptKind::Init),
        // ExtINT is `Fixed` by the time it reaches here, and the reason is that
        // the acknowledge has ALREADY happened. `BxIoApic::service` runs the
        // INTA against the 8259 for a mode-7 entry and puts the answered vector
        // in the message — Bochs `ioapic.cc service_ioapic` does exactly this
        // (`if (entry->delivery_mode() == 7) vector = DEV_pic_iac()`), and then
        // hands that vector to the destination APIC through the same path as a
        // Fixed one: `apic.cc bx_local_apic_c::deliver` case `APIC_DM_EXTINT`
        // calls `trigger_irq`, whose `bypass_irr_isr` argument skips only the
        // already-pending check. So there is nothing ExtINT-shaped left to
        // carry, and `WHvRequestInterrupt` with a resolved vector is precisely
        // what the model path would do. The entry's own vector field is not
        // used at all; a masked or idle 8259 answers with its spurious vector,
        // which is the delivery hardware makes too.
        IoApicDeliveryMode::ExtInt => Some(InterruptKind::Fixed),
        // `WHV_INTERRUPT_TYPE` has an SMI kind under none of its names, and an
        // SMI raised through an I/O APIC entry has no path to the shadow: the
        // `apic_smi_trap` arm answers a guest's APIC WRITE, not a device's
        // message. Refused rather than dropped, deliberately — a machine that
        // stops loudly is better than one that silently loses a
        // system-management interrupt some firmware is waiting on.
        //
        // A reserved encoding answers `None` too, and the call site tells the
        // two apart: one is a gap in this port, the other a guest programming
        // its own I/O APIC wrongly.
        IoApicDeliveryMode::Smi
        | IoApicDeliveryMode::Reserved3
        | IoApicDeliveryMode::Reserved6 => None,
    }
}

/// Ask the platform for the local APIC this machine's device clock implies,
/// and report the mode it accepted.
///
/// The clock is the selector because it is what says who turns the machine's
/// wheel. Under [`DeviceClock::Ticks`] the wheel advances only when the
/// scheduler has the processor back, so the guest's local APIC has to be this
/// machine's own: a halted processor under an emulated APIC never leaves
/// `WHvRunVirtualProcessor` (measured), and a timer that only turns between
/// slices would never fire to wake it. Under [`DeviceClock::HostTime`] a thread
/// of its own turns the wheel, the processor may stay inside the partition
/// indefinitely, and the partition's own APIC is what makes that possible.
///
/// `X2Apic` first because it is the mode this port measured on its own host,
/// and because the x2APIC register file is the one a modern guest programs
/// through MSRs — a guest that enables x2APIC under `XApic` would find its
/// `WRMSR`s unanswered. `XApic` is the fallback for a host that offers only it;
/// what a guest sees differs there only if it asks for x2APIC, which is a mode
/// the machine's own CPUID leaf controls.
///
/// # Errors
/// [`CpuError::EngineFault`] with [`EngineFaultKind::Unsupported`] when a
/// machine asked for the hypervisor's APIC and the platform offers neither
/// mode: running it with `None` instead would strand every interrupt this
/// engine then had no way to deliver.
fn choose_the_local_apic(
    config: &mut PartitionConfig,
    clock: DeviceClock,
) -> Result<LocalApicMode> {
    let wanted: &[LocalApicMode] = match clock {
        DeviceClock::Ticks => &[LocalApicMode::None],
        DeviceClock::HostTime => &[LocalApicMode::X2Apic, LocalApicMode::XApic],
    };
    let mut refused = None;
    for mode in wanted {
        match config.local_apic(*mode) {
            Ok(_) => {
                tracing::debug!("this partition's local APIC is the hypervisor's {mode:?}");
                return Ok(*mode);
            }
            Err(error) => {
                tracing::debug!("the platform refused {mode:?} as this partition's APIC: {error}");
                refused = Some(error);
            }
        }
    }
    match refused {
        Some(error) => {
            tracing::error!(
                "this machine's devices run on host time, which needs the hypervisor's own \
                 local APIC, and the platform offers neither x2APIC nor xAPIC: {error}"
            );
            Err(CpuError::EngineFault(EngineFault::with_code(
                EngineFaultKind::Unsupported,
                "no hypervisor local APIC on this host",
                error.hresult(),
            )))
        }
        // `wanted` is never empty, so a loop that fell through recorded a
        // refusal; this arm exists because the compiler cannot know that.
        None => Err(CpuError::EngineFault(EngineFault::new(
            EngineFaultKind::Unsupported,
            "no hypervisor local APIC on this host",
        ))),
    }
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
                .map_err(platform_failed)?;
            let apic_mode = choose_the_local_apic(&mut config, io.pc_system().device_clock())?;
            let hypervisor_apic = apic_mode != LocalApicMode::None;
            config
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
                    // A guest whose local APIC is the partition's programs
                    // LINT0 where this engine cannot see it, and the 8259's
                    // vector reaches that APIC as a `WHvRequestInterrupt`, which
                    // no LVT entry masks. The trap is what keeps the fabric's
                    // own copy current, and the fabric's copy is the only gate
                    // there is (`IrqFabric::lint0_admits_ext_int`).
                    apic_write_lint0_trap: hypervisor_apic,
                    // An SMI the partition's APIC would deliver is one no
                    // hypervisor can run: system-management mode belongs to the
                    // shadow, so the SMI comes back here instead of entering a
                    // handler on hardware.
                    apic_smi_trap: hypervisor_apic,
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

            let installed = install_the_machines_map(&mut partition, io.memory(), None)
                .map_err(CpuError::EngineFault)?;
            partition.create_processor(BOOT_VP).map_err(platform_failed)?;
            let vcpu = partition.take_vcpu(BOOT_VP).map_err(platform_failed)?;

            // The fresh processor's own area, read once here so every later
            // exchange patches the platform's bytes rather than inventing
            // them — the components this port does not model cross untouched
            // that way. A host that refuses the read cannot keep the two
            // register files agreed, so it refuses the engine.
            *started = Some(Started {
                vcpu: Some(vcpu),
                partition,
                apic_mode,
                installed,
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

/// Build the partition, map the machine into it, create the boot processor and
/// hand its handle out.
///
/// What [`SliceEngine::run_slice`] does on its first call, reachable without a
/// slice: a machine that will be run by a thread never asks for one, and the
/// partition still has to exist before the thread can enter it.
///
/// The processor LEAVES the engine here. Whoever takes it owns the obligation
/// `rusty_box_whp::Vcpu` states — the handle names a partition this machine
/// destroys when it drops, so the thread holding it must be joined first — and
/// the engine refuses every later slice rather than running a processor it no
/// longer holds.
///
/// # Errors
/// Whatever [`start`] refused, or
/// [`CpuError::UnsupportedCpuOperation`] if the processor has already been
/// taken: one processor has one runner.
pub(crate) fn bring_up<T: Instrumentation>(
    machine: &mut Emulator<T, WhpEngine>,
) -> Result<Vcpu> {
    let Processor { mut io, engine, .. } = machine.processor(BOOT_VP as usize);
    let started = start(&mut engine.started, &mut io)?;
    started.vcpu.take().ok_or(CpuError::UnsupportedCpuOperation {
        operation: "the processor was already taken by a thread",
    })
}

/// The machine's vocabulary for a mapping this engine could not apply.
///
/// Keeps what [`platform_failed`] cannot: the failing API and the platform's
/// own `HRESULT` both survive into the fault the machine reports, which is the
/// difference between a reader who can trace a refusal to the platform and one
/// who is told only that something was unsupported.
fn mapping_failed(error: &WhpError) -> EngineFault {
    tracing::error!("installing this machine's guest-physical map failed: {error}");
    EngineFault::with_code(EngineFaultKind::Memory, error.call(), error.hresult())
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
) -> core::result::Result<MemoryPlan, EngineFault> {
    let plan = MemoryPlan::derive(memory).map_err(|error| {
        tracing::error!("this machine has no stable guest-physical map: {error:?}");
        EngineFault::new(EngineFaultKind::Memory, "deriving a partially resident machine's map")
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
            .map_err(|error| mapping_failed(&error))?;
    }

    for window in plan.windows() {
        if old.contains(window) {
            continue;
        }
        let host = memory
            .allocation_slice(window.host.get(), window.len)
            .ok_or(EngineFault::new(
                EngineFaultKind::Memory,
                "a plan window fell outside the machine's allocation",
            ))?;
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
) -> core::result::Result<(), EngineFault> {
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
        EngineFault::with_code(EngineFaultKind::Memory, error.call(), error.hresult())
    })
}

impl<T: Instrumentation> SliceEngine<T> for WhpEngine {
    // The hardware retires the guest's instructions and this port never sees
    // them, so what a slice can report is the time it took. See `ticks_elapsed`.
    const PROGRESS_UNIT: ProgressUnit = ProgressUnit::Ticks;

    // The machine's between-batch delivery is for a processor the machine runs
    // itself, and none of this engine's are: the hardware runs them. So the
    // machine's loop takes nothing off the 8259 for this engine, and the line
    // is resolved at the machine's boundary instead — `route_the_legacy_line`
    // acknowledges the controller and hands the vector to
    // `route_ioapic_delivery`, which requests it of the partition's APIC.
    const EVENT_DELIVERY: EventDelivery = EventDelivery::Engine;

    fn memory_map_changed(
        &mut self,
        memory: &mut BxMemC,
    ) -> core::result::Result<(), EngineFault> {
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

    /// An I/O APIC message goes to whichever local APIC the guest is actually
    /// reading.
    ///
    /// While the partition has none, that is this machine's own model and the
    /// machine writes it. Once the hypervisor owns the APIC, the message must
    /// reach the partition instead — and it goes as a `WHvRequestInterrupt`,
    /// which addresses the partition rather than a stopped processor's
    /// registers and is therefore the one delivery route safe to take from a
    /// thread that is not the one running the guest.
    fn route_ioapic_delivery(&mut self, delivery: IoApicDelivery) -> DeliveryRoute {
        let Some(started) = self.started.as_ref() else {
            // Reset and restore reach here before any partition exists, and
            // the model is the truth until one does.
            return DeliveryRoute::Model;
        };
        if started.apic_mode == LocalApicMode::None {
            return DeliveryRoute::Model;
        }
        let Some(kind) = requested_kind(delivery.delivery_mode) else {
            return match delivery.delivery_mode {
                // A guest programmed its own I/O APIC with an encoding the
                // architecture does not define. Bochs leaves the entry stuck
                // and runs on, and so does this: a machine that stopped would
                // punish the guest's mistake by ending the guest.
                IoApicDeliveryMode::Reserved3 | IoApicDeliveryMode::Reserved6 => {
                    DeliveryRoute::Undelivered
                }
                // A gap in this port rather than the guest's error, so it is
                // loud. Dropping it silently would leave firmware waiting on a
                // system-management interrupt that never comes, which is the
                // harder failure to find of the two.
                _ => DeliveryRoute::Refused(EngineFault::new(
                    EngineFaultKind::Unsupported,
                    "an I/O APIC SMI has no path to the shadow",
                )),
            };
        };
        let request = InterruptRequest {
            kind,
            destination_mode: match delivery.dest_mode {
                IoApicDestinationMode::Physical => DestinationMode::Physical,
                IoApicDestinationMode::Logical => DestinationMode::Logical,
            },
            trigger_mode: match delivery.trigger_mode {
                IoApicTrigger::Edge => TriggerMode::Edge,
                IoApicTrigger::Level => TriggerMode::Level,
            },
            destination: delivery.dest,
            vector: u32::from(delivery.vector),
        };
        match started.partition.interrupt_requester().request(request) {
            Ok(()) => DeliveryRoute::Backend,
            Err(error) => {
                tracing::error!(
                    "the partition's APIC refused vector {:#04x}: {error}",
                    delivery.vector
                );
                DeliveryRoute::Refused(EngineFault::with_code(
                    EngineFaultKind::Vcpu,
                    "WHvRequestInterrupt",
                    error.hresult(),
                ))
            }
        }
    }

    fn owns_the_guests_local_apic(&self) -> bool {
        self.started
            .as_ref()
            .is_some_and(|started| started.apic_mode != LocalApicMode::None)
    }

    /// Refused: a machine on the hypervisor is driven by `FastMachine`.
    ///
    /// The slice model this used to implement is gone, and with it every
    /// reason to hand a hardware processor a bounded stretch of guest time.
    /// Measured over 300 seconds, it bought 2.19 million partition entries of
    /// which 85.4% produced no exit at all, and left under 1% of the wall
    /// clock actually executing guest instructions; its mean slice overran its
    /// budget 271-fold, which a guest sees as a timer that fires in bursts at
    /// slice boundaries instead of at its deadline.
    ///
    /// A machine on this engine is adopted by `FastMachine` instead, which
    /// gives each processor a thread that stays bound to it and gives the
    /// device wheel a thread of its own.
    ///
    /// This is the one place that says so (R5). The scheduler reaches an
    /// engine only through `Emulator::step`, which `FastMachine` never calls,
    /// so a caller arriving here is a caller that built a machine on this
    /// engine and forgot to adopt it — and the honest answer to that is a
    /// refusal naming the verb it wanted, not a guest run some other way.
    ///
    /// # Errors
    /// Always.
    fn run_slice(
        &mut self,
        _cpu: &mut BxCpuC<T>,
        _io: PcIo<'_>,
        _request: SliceRequest,
    ) -> Result<Progress> {
        Err(CpuError::UnsupportedCpuOperation {
            operation: "a machine on the hypervisor is driven by FastMachine, not by slices",
        })
    }
}

/// The shadow holds extended state the partition's area has no place for —
/// a defect in what this engine offered the guest, reported rather than
/// silently dropped on the floor.
pub(crate) fn uncarried(refused: xsave::UncarriedComponent) -> CpuError {
    tracing::error!(
        "the shadow holds extended-state component {} and the partition's area cannot carry it",
        refused.index
    );
    CpuError::UnsupportedCpuOperation {
        operation: "an extended-state component the partition cannot carry",
    }
}

/// `WHV_DELIVERABILITY_NOTIFICATIONS_REGISTER`'s `InterruptNotification`
/// bit, from the SDK's `WinHvPlatformDefs.h` (the AMD64 layout:
/// `NmiNotification` bit 0, `InterruptNotification` bit 1,
/// `InterruptPriority` bits 2..6). The windows-sys bindings collapse the
/// layout into one opaque bitfield, so the word is built by hand.
const INTERRUPT_NOTIFICATION: u64 = 1 << 1;

/// The deliverability notification belongs to a partition with NO APIC of its
/// own, and only to one.
///
/// Measured on this host, under both `XApic` and `X2Apic`: the register is
/// accepted, reads back exactly as written, and never produces an
/// interrupt-window exit — across sixteen thousand `STI`s in one run. Under
/// `LocalApicMode::None` the same word on the same processor produces one
/// window exit per ask (`docs/whp-interrupt-window-2026-09-06.md`, probe P10).
/// Nothing in this crate arms one, and this constant is named here so the
/// measurement and the register it was made against sit beside each other.
const _: () = assert!(INTERRUPT_NOTIFICATION == 2);

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
pub(crate) fn run_the_shadow_out_of_smm<T: Instrumentation>(
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
pub(crate) enum Trapped {
    /// A memory access, or a model-specific register. Whatever the shadow made
    /// of it is what the guest gets.
    Access,
    /// `CPUID` for this leaf.
    Cpuid { leaf: u32 },
}

/// Report the whole state the platform refused to run, so the register that
/// broke an architectural entry check can be found by inspection — the
/// refusal itself names nothing.
pub(crate) fn report_the_state_the_platform_refused(held: &VcpuArchState) {
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
///
/// Applied to the shadow processor that just retired the `CPUID`, not to a
/// copy of its answer: the shadow is what the next imposition installs, so
/// the withheld bit has to be gone from the register the guest reads on
/// every processor that will ever describe it.
pub(crate) fn withhold_virtualisation_from<T: Instrumentation>(cpu: &mut BxCpuC<T>, leaf: u32) {
    /// `CPUID.1:ECX[5]`, Intel's VMX.
    const VMX: u64 = 1 << 5;
    /// `CPUID.80000001:ECX[2]`, AMD's SVM.
    const SVM: u64 = 1 << 2;

    let withheld = match leaf {
        1 => VMX,
        0x8000_0001 => SVM,
        _ => return,
    };
    // `CPUID` leaves its answer in RCX.
    let rcx = cpu.rcx();
    cpu.set_rcx(rcx & !withheld);
}

/// The report itself, over a shadow that already holds the faulting state.
///
/// Split from [`report_the_fault`] because the two engines reach this point by
/// different routes and only one of them owes a read-back: the slice loop must
/// pull the whole processor into the shadow first, while the vCPU thread has
/// already imported exactly what an [`crate::exchange::ExitClass::Exception`]
/// needs. The description is one body either way, so the two cannot drift into
/// reporting different things about the same fault.
pub(crate) fn describe_the_fault<T: Instrumentation>(
    vcpu: &Vcpu,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    exit: &Exit,
    history: &ExitHistory,
) {
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
        match vcpu.translate_gva(linear) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine whose partition has no local APIC of its own keeps this
    /// machine's model APIC, and says so.
    ///
    /// The answer decides who acknowledges the 8259: an engine that owns the
    /// guest's APIC takes the legacy line as a resolved vector, and one that
    /// does not leaves it to `set_legacy_intr_level` and the deferred
    /// acknowledge. An engine that has not started a partition at all owns
    /// nothing.
    #[test]
    fn an_engine_with_no_partition_owns_no_local_apic() {
        let engine = WhpEngine::default();
        assert!(
            !<WhpEngine as SliceEngine<()>>::owns_the_guests_local_apic(&engine),
            "an engine that has started nothing cannot be the guest's APIC"
        );
    }

    /// `WHvRegisterInterruptState` round-trips through the one decoder and
    /// encoder: bit 0 is the interrupt shadow, bit 1 the NMI mask, and the
    /// bits above them are not this engine's to carry.
    #[test]
    fn the_interrupt_state_word_round_trips_its_two_bits() {
        assert_eq!(InterruptStateWord::decode(0), InterruptStateWord::AT_RESET);
        for (word, shadow, nmi_masked) in
            [(0b00u64, false, false), (0b01, true, false), (0b10, false, true), (0b11, true, true)]
        {
            let decoded = InterruptStateWord::decode(word);
            assert_eq!(decoded, InterruptStateWord { shadow, nmi_masked });
            assert_eq!(decoded.encode(), word);
        }
        assert_eq!(
            InterruptStateWord::decode(0xFFFF_FFFF_FFFF_FFFC),
            InterruptStateWord::AT_RESET,
            "only the two low bits are read"
        );
    }
}
