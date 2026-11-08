//! The Performance Monitors Extension (FEAT_PMUv3): the cycle counter, the
//! event counters, their controls and traps, and the overflow interrupt
//! request, PMUIRQ.
//!
//! Provenance (R7): the Arm ARM (DDI 0487M.d) chapter D13 and the event
//! descriptions of chapter D14; the shared pseudocode of the A64 ISA XML
//! (release 2026-09): `CountPMUEvents`, `AArch64_PMUCycle`,
//! `AArch64_IncrementEventCounter`, `AArch64_IncrementCycleCounter`,
//! `CheckPMUOverflowCondition`, `CheckForPMUOverflow`, `PMUSwIncrement`,
//! `GetPMUCounterRange`, `GetPMUReadMask`, `GetPMUWriteMask` and the
//! `Write_PM*` functions; AARCHMRS `Registers.json` for each register's
//! fields and the order its accessor checks its traps in; and, for the
//! Cortex-A57, its TRM (DDI 0488H): chapter 11 and the choices appendix
//! B.2 makes where Arm leaves one open.
//!
//! **Time.** The processor's clock, which the cycle counter and
//! `CPU_CYCLES` count, is the instruction clock the machine's time is
//! counted in: one cycle for each instruction the processor executes, or
//! that takes an exception in its place, at `ips` cycles a second. The
//! clock stops while the processor waits (`WFI`, `WFE`, their timed forms
//! and PSCI `CPU_SUSPEND`), which Arm leaves to the implementation (D14.4);
//! the generic timer's count goes on through the wait, so the two part
//! there. In every cycle counted one instruction is sent for execution, so
//! no cycle stalls: `STALL_FRONTEND`, `STALL_BACKEND` and `STALL` count
//! nothing.
//!
//! **Counting costs nothing per instruction.** A counter's value is kept
//! as of an instruction count, [`PmuCounts::settled`], and brought up to
//! the present only when something reads it or changes what it counts: a
//! register access, an exception taken or returned from (which changes the
//! level the filters read), and the machine's next overflow, which the
//! machine runs to ([`Cpu::pmu_outputs`]). Between two of those, every
//! instruction counts alike, so the counters advance by the instructions
//! retired between them, an instruction that took a synchronous exception
//! other than a call (`SVC`, `HVC`) being a cycle but no instruction
//! executed.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::el2::{
    MDCR_HCCD, MDCR_HLP, MDCR_HPMD, MDCR_HPME, MDCR_HPMFZO, MDCR_HPMN, MDCR_TPM, MDCR_TPMCR,
};
use super::features::{Feature, Features};
use super::sysreg::{trap_to_el1, trap_to_el2, SysRegError, SysRegKey, WriteEffect};
use super::{Cpu, El, Levels};

/// The most event counters a processor here has: the Cortex-A57's six
/// (DDI 0488H 11.2.3), which the Armv9.2 model has too.
pub(crate) const MAX_EVENT_COUNTERS: usize = 6;

/// How many event counters a PMU implements (Arm `NUM_PMU_COUNTERS`,
/// `PMCR_EL0.N`): at most [`MAX_EVENT_COUNTERS`], which the arrays of
/// [`PmuControls`] and [`PmuCounts`] hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct EventCounters(u8);

impl EventCounters {
    /// `count` event counters, checked against [`MAX_EVENT_COUNTERS`] where
    /// a shape is built.
    pub(crate) const fn new(count: u8) -> Self {
        assert!(
            count as usize <= MAX_EVENT_COUNTERS,
            "more event counters than a PMU here holds"
        );
        Self(count)
    }

    #[inline]
    pub(crate) const fn get(self) -> usize {
        self.0 as usize
    }
}

/// A common event (Arm ARM D14.3) this PMU counts, each one counted as its
/// description says.
///
/// Closed (R5): an event number no variant names is one no processor here
/// implements, and a counter set to it counts nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PmuEvent {
    /// 0x0000 `SW_INCR`: each write of the counter's bit to `PMSWINC_EL0`.
    SwIncr,
    /// 0x0008 `INST_RETIRED`: each instruction architecturally executed.
    InstRetired,
    /// 0x0009 `EXC_TAKEN`: each exception taken, at the level it is taken
    /// from.
    ExcTaken,
    /// 0x000A `EXC_RETURN`: each exception return instruction executed.
    ExcReturn,
    /// 0x000B `CID_WRITE_RETIRED`: each `MSR` to `CONTEXTIDR_EL1`.
    CidWriteRetired,
    /// 0x0011 `CPU_CYCLES`: each cycle.
    CpuCycles,
    /// 0x001C `TTBR_WRITE_RETIRED`: each `MSR` to `TTBR0_EL1` or
    /// `TTBR1_EL1`.
    TtbrWriteRetired,
    /// 0x001E `CHAIN`: on an odd counter, each overflow of bits 31:0 of the
    /// even counter below it.
    Chain,
    /// 0x0023 `STALL_FRONTEND`: no cycle stalls here, so nothing.
    StallFrontend,
    /// 0x0024 `STALL_BACKEND`: likewise.
    StallBackend,
    /// 0x003C `STALL`: likewise.
    Stall,
}

impl PmuEvent {
    /// Every event, each at the index of its bit in an [`EventSet`].
    pub(crate) const ALL: [Self; 11] = [
        Self::SwIncr,
        Self::InstRetired,
        Self::ExcTaken,
        Self::ExcReturn,
        Self::CidWriteRetired,
        Self::CpuCycles,
        Self::TtbrWriteRetired,
        Self::Chain,
        Self::StallFrontend,
        Self::StallBackend,
        Self::Stall,
    ];

    /// The event's number, as `PMEVTYPER<n>_EL0.evtCount` names it.
    pub(crate) const fn number(self) -> u16 {
        match self {
            Self::SwIncr => 0x0000,
            Self::InstRetired => 0x0008,
            Self::ExcTaken => 0x0009,
            Self::ExcReturn => 0x000A,
            Self::CidWriteRetired => 0x000B,
            Self::CpuCycles => 0x0011,
            Self::TtbrWriteRetired => 0x001C,
            Self::Chain => 0x001E,
            Self::StallFrontend => 0x0023,
            Self::StallBackend => 0x0024,
            Self::Stall => 0x003C,
        }
    }

    /// Whether the event occurs once in every cycle the processor executes
    /// an instruction in: `CPU_CYCLES`, and `INST_RETIRED` but in a cycle
    /// whose instruction took a synchronous exception in its place.
    const fn per_cycle(self, cycle: Cycle) -> bool {
        match self {
            Self::CpuCycles => true,
            Self::InstRetired => matches!(cycle, Cycle::Executed),
            Self::SwIncr
            | Self::ExcTaken
            | Self::ExcReturn
            | Self::CidWriteRetired
            | Self::TtbrWriteRetired
            | Self::Chain
            | Self::StallFrontend
            | Self::StallBackend
            | Self::Stall => false,
        }
    }

    /// The event's index in [`PmuEvent::ALL`].
    const fn index(self) -> usize {
        self as usize
    }
}

const _: () = {
    let mut i = 0;
    while i < PmuEvent::ALL.len() {
        assert!(
            PmuEvent::ALL[i] as usize == i,
            "PmuEvent::ALL is out of declaration order"
        );
        i += 1;
    }
};

/// What a cycle's instruction did, for the events that count per cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cycle {
    /// It executed: it retired, or it was a call (`SVC`, `HVC`) whose
    /// exception completes it.
    Executed,
    /// It took a synchronous exception in its place.
    Faulted,
}

/// The events a PMU implements, a bit each in [`PmuEvent::ALL`] order:
/// what `PMCEID0_EL0` and `PMCEID1_EL0` advertise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct EventSet(u16);

impl EventSet {
    /// The set of `events`.
    pub(crate) const fn of(events: &[PmuEvent]) -> Self {
        let mut bits = 0;
        let mut i = 0;
        while i < events.len() {
            bits |= 1 << events[i].index();
            i += 1;
        }
        Self(bits)
    }

    pub(crate) const fn has(self, event: PmuEvent) -> bool {
        self.0 & (1 << event.index()) != 0
    }

    /// The event numbered `number` (an `evtCount`), if the set has it.
    pub(crate) fn numbered(self, number: u64) -> Option<PmuEvent> {
        PmuEvent::ALL
            .into_iter()
            .find(|&event| u64::from(event.number()) == number && self.has(event))
    }

    /// `PMCEID0_EL0` (`Ceid::Low`) or `PMCEID1_EL0` (`Ceid::High`): bit `e`
    /// of the first is event `e` (0x00 to 0x1F), of the second event `0x20 +
    /// e` (0x20 to 0x3F); their bits 63:32 name the events 0x4000 to 0x403F,
    /// none of which a processor here implements.
    pub(crate) const fn ceid(self, register: Ceid) -> u64 {
        let base = match register {
            Ceid::Low => 0x00,
            Ceid::High => 0x20,
        };
        let mut value = 0;
        let mut i = 0;
        while i < PmuEvent::ALL.len() {
            let event = PmuEvent::ALL[i];
            let number = event.number();
            if self.has(event) && number >= base && number < base + 0x20 {
                value |= 1 << (number - base);
            }
            i += 1;
        }
        value
    }
}

/// One of the two common event identification registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ceid {
    /// `PMCEID0_EL0`.
    Low,
    /// `PMCEID1_EL0`.
    High,
}

/// Whether `PMCR_EL0.X`, the export of events to an external agent, is
/// implemented (an IMPLEMENTATION DEFINED choice, `ImpDefBool` in Arm's
/// `PMCR_EL0` fieldset). It exports nothing on this board, which has no
/// trace unit or event bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EventExport {
    /// `X` keeps what is written.
    Implemented,
    /// `X` is RAZ/WI.
    Absent,
}

/// What an access to an event counter beyond those the accessor can reach
/// does — one at or above `PMCR_EL0.N`, or with EL2 at or above
/// `MDCR_EL2.HPMN` from EL0 or EL1 — which Arm leaves CONSTRAINED
/// UNPREDICTABLE without FEAT_FGT (K1.2.6.2,
/// `Unpredictable_PMUEVENTCOUNTER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CounterBeyondReach {
    /// UNDEFINED, the choice Arm's own refinement of
    /// `ConstrainUnpredictableProcedure` makes.
    Undefined,
    /// The Cortex-A57's (DDI 0488H B.2.22 to B.2.25): the register is RES0,
    /// reading as zero and ignoring writes, but for a direct
    /// `PMEVCNTR<n>_EL0` or `PMEVTYPER<n>_EL0` of a counter the processor
    /// does not implement, which is UNALLOCATED.
    Res0,
}

/// What `ID_AA64DFR0_EL1.MTPMU` reports of a processor without FEAT_MTPMU,
/// whose `PMEVTYPER<n>_EL0.MT` is RES0 either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MtpmuReport {
    /// 0b0000, which before Armv8.6 may stand for an `MT` bit of an
    /// IMPLEMENTATION DEFINED kind: the Cortex-A57's (DDI 0488H 4.3.19).
    Unreported,
    /// 0b1111, which Armv8.6 and later require of a processor with a PMU
    /// (`(v8Ap6 && FEAT_PMUv3 && FEAT_AA64EL1) ==> UInt(MTPMU) != 0`).
    Res0,
}

impl MtpmuReport {
    /// The field's value.
    pub(crate) const fn field(self) -> u64 {
        match self {
            Self::Unreported => 0,
            Self::Res0 => 0xF,
        }
    }
}

/// A PMU's IMPLEMENTATION DEFINED choices, beside the revision its
/// features name (FEAT_PMUv3 to FEAT_PMUv3p7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PmuShape {
    /// The event counters, `PMCR_EL0.N`.
    pub(crate) counters: EventCounters,
    /// `PMCR_EL0.IMP` and `IDCODE` in their place, bits 31:16.
    pub(crate) identification: u64,
    pub(crate) export: EventExport,
    /// The events counted, which `PMCEIDn_EL0` advertise.
    pub(crate) events: EventSet,
    pub(crate) beyond_reach: CounterBeyondReach,
    pub(crate) mtpmu: MtpmuReport,
}

/// The events both processors count: every common event whose occurrences
/// the model knows exactly. Of the Cortex-A57's (DDI 0488H Table 11-5),
/// the cache, TLB, branch prediction, bus, memory error and speculative
/// ones are left out: the model has no cache, TLB timing, branch predictor
/// or bus to count them by.
const COUNTED: [PmuEvent; 8] = [
    PmuEvent::SwIncr,
    PmuEvent::InstRetired,
    PmuEvent::ExcTaken,
    PmuEvent::ExcReturn,
    PmuEvent::CidWriteRetired,
    PmuEvent::CpuCycles,
    PmuEvent::TtbrWriteRetired,
    PmuEvent::Chain,
];

impl PmuShape {
    /// The Cortex-A57's (DDI 0488H chapter 11): six event counters,
    /// `PMCR_EL0.IMP` 0x41 (Arm) and `IDCODE` 0x01 (Cortex-A57), `X`
    /// implemented, [`COUNTED`] — `PMCEID0_EL0` 0x5002_0F01, the TRM's
    /// 0x7FFF_0F3F less the events the model cannot count, and
    /// `PMCEID1_EL0` 0 — and the TRM's choices for the counters out of
    /// reach.
    pub(crate) const CORTEX_A57: Self = Self {
        counters: EventCounters::new(6),
        identification: 0x4101 << 16,
        export: EventExport::Implemented,
        events: EventSet::of(&COUNTED),
        beyond_reach: CounterBeyondReach::Res0,
        mtpmu: MtpmuReport::Unreported,
    };

    /// The Armv9.2 model's, the board's own: six event counters, as Arm's
    /// Armv9 cores have; `PMCR_EL0.IMP` reading as zero, as FEAT_PMUv3p7
    /// makes it, and `IDCODE` with it; no event export; [`COUNTED`] and the
    /// stall events FEAT_PMUv3p1 and FEAT_PMUv3p4 require (D14.6), which
    /// count nothing here — `PMCEID0_EL0` 0x5002_0F01, `PMCEID1_EL0`
    /// 0x1000_0018; Arm's own choice for the counters out of reach; and
    /// `MTPMU` 0b1111.
    pub(crate) const ARMV9P2: Self = Self {
        counters: EventCounters::new(6),
        identification: 0,
        export: EventExport::Absent,
        events: EventSet::of(&[
            PmuEvent::SwIncr,
            PmuEvent::InstRetired,
            PmuEvent::ExcTaken,
            PmuEvent::ExcReturn,
            PmuEvent::CidWriteRetired,
            PmuEvent::CpuCycles,
            PmuEvent::TtbrWriteRetired,
            PmuEvent::Chain,
            PmuEvent::StallFrontend,
            PmuEvent::StallBackend,
            PmuEvent::Stall,
        ]),
        beyond_reach: CounterBeyondReach::Undefined,
        mtpmu: MtpmuReport::Res0,
    };
}

// ----- register fields -----

/// `PMCR_EL0` fields.
const PMCR_E: u64 = 1 << 0;
const PMCR_P: u64 = 1 << 1;
const PMCR_C: u64 = 1 << 2;
const PMCR_X: u64 = 1 << 4;
const PMCR_DP: u64 = 1 << 5;
/// `LC`, RES1 on a processor without AArch32: the cycle counter overflows
/// at bit 63.
const PMCR_LC: u64 = 1 << 6;
const PMCR_LP: u64 = 1 << 7;
const PMCR_FZO: u64 = 1 << 9;
const PMCR_N_SHIFT: u32 = 11;

/// The cycle counter's bit of `PMCNTENSET_EL0` and its siblings.
const CYCLE_BIT: u64 = 1 << 31;

/// `PMEVTYPER<n>_EL0` and `PMCCFILTR_EL0` filter fields: `P` (EL1), `U`
/// (EL0) and `NSH` (EL2), the ones a processor with no EL3 and no Secure
/// state has.
const FILTER_P: u64 = 1 << 31;
const FILTER_U: u64 = 1 << 30;
const FILTER_NSH: u64 = 1 << 27;

/// `PMUSERENR_EL0` fields: EL0's access to the PMU (`EN`), and to
/// `PMSWINC_EL0` (`SW`), the cycle counter's reads (`CR`) and the event
/// counters' reads (`ER`) alone.
const USERENR_EN: u64 = 1 << 0;
const USERENR_SW: u64 = 1 << 1;
const USERENR_CR: u64 = 1 << 2;
const USERENR_ER: u64 = 1 << 3;
const USERENR_MASK: u64 = USERENR_EN | USERENR_SW | USERENR_CR | USERENR_ER;

/// `PMSELR_EL0.SEL`.
const SELR_MASK: u64 = 0x1F;

/// The `PMSELR_EL0.SEL` value that names the cycle counter's filter.
const SEL_CYCLE: usize = 31;

/// `MDCR_EL2.HPMN` as the PMU's ranges read it (Arm `EffectiveHPMN`), of a
/// PMU with `counters` event counters: the value written, but above
/// `counters` `counters` and at zero zero — every counter reserved for
/// EL2 — the Cortex-A57's answers (DDI 0488H B.2.20) to what Arm leaves
/// CONSTRAINED UNPREDICTABLE without FEAT_HPMN0 and FEAT_PMUv3_EXTPMN
/// (K1.2.6.3), which the Armv9.2 model gives as well. A read of the field
/// returns what was written (B.2.21).
pub(crate) const fn effective_hpmn(mdcr: u64, counters: usize) -> usize {
    let written = (mdcr & MDCR_HPMN) as usize;
    if written > counters {
        counters
    } else {
        written
    }
}

/// Bits `0..count` set.
const fn low_bits(count: usize) -> u64 {
    if count >= 64 {
        u64::MAX
    } else {
        (1 << count) - 1
    }
}

// ----- the registers -----

/// The PMU's controls: what decides which counters count, and the
/// accesses EL0 has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmuControls {
    /// `PMCR_EL0`'s stored fields: `E`, and `X`, `DP`, `LP` and `FZO`
    /// where the processor has them.
    pub(crate) pmcr: u64,
    /// `PMCNTENSET_EL0`, `PMINTENSET_EL1`: a bit each event counter and
    /// [`CYCLE_BIT`].
    pub(crate) cnten: u64,
    pub(crate) inten: u64,
    pub(crate) selr: u64,
    pub(crate) userenr: u64,
    pub(crate) ccfiltr: u64,
    pub(crate) evtyper: [u64; MAX_EVENT_COUNTERS],
}

impl PmuControls {
    /// Out of reset: every control zero (`cpu::reset`: a control is zero),
    /// as the Cortex-A57 TRM resets `PMCR_EL0`'s `E` and `PMUSERENR_EL0`
    /// and `PMCCFILTR_EL0` (Table 4-10).
    pub(crate) const RESET: Self = Self {
        pmcr: 0,
        cnten: 0,
        inten: 0,
        selr: 0,
        userenr: 0,
        ccfiltr: 0,
        evtyper: [0; MAX_EVENT_COUNTERS],
    };
}

/// The counters and their overflow flags, as of an instruction count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmuCounts {
    /// `PMCCNTR_EL0`.
    pub(crate) ccntr: u64,
    /// `PMEVCNTR<n>_EL0`: 32 bits without FEAT_PMUv3p5, 64 with it.
    pub(crate) evcntr: [u64; MAX_EVENT_COUNTERS],
    /// `PMOVSSET_EL0`: a bit each event counter and [`CYCLE_BIT`].
    pub(crate) ovs: u64,
    /// The instruction count ([`Cpu::retired`](super::Cpu)) the counters
    /// are current to.
    pub(crate) settled: u64,
}

/// The PMU of one processor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pmu {
    pub(crate) controls: PmuControls,
    pub(crate) counts: PmuCounts,
    /// Whether any counter is enabled, by its global enable and its bit of
    /// `PMCNTENSET_EL0`: without one, nothing counts at any level, and the
    /// exception paths leave the PMU alone. Derived
    /// ([`Cpu::pmu_controls_changed`]), so it is not saved.
    pub(crate) enabled: bool,
}

impl Pmu {
    /// Out of reset at instruction count `retired`, on a processor of
    /// `features` whose PMU is `shape`: the controls zero, each counter it
    /// has the board's fill naming it (`cpu::reset`: a value), kept to the
    /// bits the counter holds, where the Cortex-A57 TRM leaves the counters
    /// UNKNOWN (Table 4-10), and no overflow flag set (a status flag is
    /// clear). Without FEAT_PMUv3 every field is zero.
    pub(crate) fn reset(features: Features, shape: PmuShape, retired: u64) -> Self {
        let has = features.has(Feature::PmuV3);
        let mut evcntr = [0; MAX_EVENT_COUNTERS];
        if has {
            for (n, counter) in evcntr.iter_mut().enumerate().take(shape.counters.get()) {
                *counter = super::reset::value_of(event_counter_sysreg(n)) & counter_mask(features);
            }
        }
        Self {
            controls: PmuControls::RESET,
            counts: PmuCounts {
                ccntr: if has {
                    super::reset::value_of(super::regs::SysReg::PMCCNTR_EL0)
                } else {
                    0
                },
                evcntr,
                ovs: 0,
                settled: retired,
            },
            enabled: false,
        }
    }
}

/// `PMEVCNTR<n>_EL0` as a host names it: `op0` 3, `op1` 3, `CRn` 14, `CRm`
/// 0b10 and `n`'s bits 4:3, `op2` its bits 2:0.
const fn event_counter_sysreg(n: usize) -> super::regs::SysReg {
    super::regs::SysReg::encode(3, 3, 14, 0b1000 | (n >> 3) as u8, (n & 7) as u8)
}

/// The bits an event counter holds: 64 with FEAT_PMUv3p5, 32 without.
const fn counter_mask(features: Features) -> u64 {
    if features.has(Feature::PmuV3p5) {
        u64::MAX
    } else {
        u32::MAX as u64
    }
}

// ----- counting -----

/// What counting reads besides the PMU's own registers.
#[derive(Debug, Clone, Copy)]
struct Context {
    el: El,
    features: Features,
    shape: PmuShape,
    have_el2: bool,
    /// `MDCR_EL2`; zero without EL2.
    mdcr: u64,
}

impl Context {
    /// Arm `NUM_PMU_COUNTERS`: none without FEAT_PMUv3.
    fn counters(self) -> usize {
        if self.features.has(Feature::PmuV3) {
            self.shape.counters.get()
        } else {
            0
        }
    }

    /// The first counter of the second range (Arm `EffectiveHPMN`): with EL2
    /// [`effective_hpmn`], without it every counter is in the first.
    fn hpmn(self) -> usize {
        if self.have_el2 {
            effective_hpmn(self.mdcr, self.counters())
        } else {
            self.counters()
        }
    }

    /// Arm `GetNumEventCountersAccessible`: at EL0 and EL1 with EL2, the
    /// first range; otherwise every counter.
    fn accessible(self) -> usize {
        if self.have_el2 && self.el != El::El2 {
            self.hpmn()
        } else {
            self.counters()
        }
    }

    /// Arm `GetPMUReadMask` and `GetPMUWriteMask` with no FEAT_PMUv3p9 or
    /// FEAT_PMUv3_ICNTR: the cycle counter's bit and those of the
    /// accessible counters.
    fn access_mask(self) -> u64 {
        CYCLE_BIT | low_bits(self.accessible())
    }

    /// Whether event counter `n` is in the first range (Arm
    /// `GetPMUCounterRange`), the second being EL2's.
    fn first_range(self, n: usize) -> bool {
        n < self.hpmn()
    }

    /// The overflow flags of the event counters of the first range, or of
    /// the second.
    fn range_mask(self, first: bool) -> u64 {
        let hpmn = self.hpmn();
        if first {
            low_bits(hpmn)
        } else {
            low_bits(self.counters()) & !low_bits(hpmn)
        }
    }

    /// `PMEVTYPER<n>_EL0.evtCount`'s bits: 16 with FEAT_PMUv3p1, 10
    /// without.
    fn evtcount_mask(self) -> u64 {
        if self.features.has(Feature::PmuV3p1) {
            0xFFFF
        } else {
            0x3FF
        }
    }
}

/// The counting rules (Arm `CountPMUEvents` and its callers) over a PMU's
/// controls.
#[derive(Clone, Copy)]
struct Counting<'a> {
    controls: &'a PmuControls,
    ctx: Context,
}

impl Counting<'_> {
    /// The event counter `n` counts, if the processor implements it.
    fn event(self, n: usize) -> Option<PmuEvent> {
        let number = self.controls.evtyper[n] & self.ctx.evtcount_mask();
        self.ctx.shape.events.numbered(number)
    }

    /// The global enable of event counter `n`: `PMCR_EL0.E` for the first
    /// range, `MDCR_EL2.HPME` for the second.
    fn global_enable(self, n: usize) -> bool {
        if self.ctx.first_range(n) {
            self.controls.pmcr & PMCR_E != 0
        } else {
            self.ctx.mdcr & MDCR_HPME != 0
        }
    }

    /// Whether event counter `n` is enabled, by its global enable and its
    /// `PMCNTENSET_EL0` bit.
    fn event_enabled(self, n: usize) -> bool {
        self.global_enable(n) && self.controls.cnten & (1 << n) != 0
    }

    /// Whether the cycle counter is enabled, by `PMCR_EL0.E` and
    /// `PMCNTENSET_EL0.C`.
    fn cycles_enabled(self) -> bool {
        self.controls.pmcr & PMCR_E != 0 && self.controls.cnten & CYCLE_BIT != 0
    }

    /// Whether any counter is enabled.
    fn any_enabled(self) -> bool {
        self.cycles_enabled() || (0..self.ctx.counters()).any(|n| self.event_enabled(n))
    }

    /// Whether counting of the first range (`first`) or the second is
    /// prohibited at the current level: at EL2, the first range's under
    /// `MDCR_EL2.HPMD` (FEAT_PMUv3p1).
    fn prohibited(self, first: bool) -> bool {
        self.ctx.el == El::El2
            && first
            && self.ctx.features.has(Feature::PmuV3p1)
            && self.ctx.mdcr & MDCR_HPMD != 0
    }

    /// Whether the first range (`first`) or the second is frozen
    /// (FEAT_PMUv3p7, Arm `ShouldPMUFreeze`): `PMCR_EL0.FZO` or
    /// `MDCR_EL2.HPMFZO` set while an event counter of the range has its
    /// overflow flag set.
    fn frozen(self, first: bool, ovs: u64) -> bool {
        if !self.ctx.features.has(Feature::PmuV3p7) {
            return false;
        }
        let control = if first {
            self.controls.pmcr & PMCR_FZO
        } else {
            self.ctx.mdcr & MDCR_HPMFZO
        };
        control != 0 && ovs & self.ctx.range_mask(first) != 0
    }

    /// Whether `filter`'s `P`, `U` and `NSH` keep a counter from counting
    /// at the current level (the Non-secure filters of `CountPMUEvents`,
    /// `NSK` and `NSU` being 0 with no EL3).
    fn filtered(self, filter: u64) -> bool {
        match self.ctx.el {
            El::El0 => filter & FILTER_U != 0,
            El::El1 => filter & FILTER_P != 0,
            El::El2 => filter & FILTER_NSH == 0,
        }
    }

    /// Arm `CountPMUEvents(n)` for event counter `n`, with the overflow
    /// flags `ovs`.
    fn counts_event(self, n: usize, ovs: u64) -> bool {
        let first = self.ctx.first_range(n);
        self.event_enabled(n)
            && !self.prohibited(first)
            && !self.frozen(first, ovs)
            && !self.filtered(self.controls.evtyper[n])
    }

    /// Arm `CountPMUEvents(CYCLE_COUNTER_ID)`, with the overflow flags
    /// `ovs`: the first range's prohibition and freezing stop it only with
    /// `PMCR_EL0.DP` set, and with FEAT_PMUv3p5 `MDCR_EL2.HCCD` stops it at
    /// EL2.
    fn counts_cycles(self, ovs: u64) -> bool {
        let mut enabled = self.cycles_enabled();
        if self.prohibited(true) || self.frozen(true, ovs) {
            enabled = enabled && self.controls.pmcr & PMCR_DP == 0;
        }
        let hccd = self.ctx.el == El::El2
            && self.ctx.features.has(Feature::PmuV3p5)
            && self.ctx.mdcr & MDCR_HCCD != 0;
        enabled && !hccd && !self.filtered(self.controls.ccfiltr)
    }

    /// The bit event counter `n` overflows past (Arm
    /// `AArch64_IncrementEventCounter`): 64 with FEAT_PMUv3p5 and the
    /// range's `PMCR_EL0.LP` or `MDCR_EL2.HLP` set, 32 otherwise.
    fn overflow_bits(self, n: usize) -> u32 {
        if !self.ctx.features.has(Feature::PmuV3p5) {
            return 32;
        }
        let long = if self.ctx.first_range(n) {
            self.controls.pmcr & PMCR_LP
        } else {
            self.ctx.mdcr & MDCR_HLP
        };
        if long != 0 {
            64
        } else {
            32
        }
    }

    /// Add `amount` to event counter `n`, no more than takes it to its
    /// next overflow, setting its overflow flag if it overflows; an even
    /// counter's overflow of bits 31:0 is a `CHAIN` event for the odd
    /// counter above it in the same range.
    fn add_to_event(self, counts: &mut PmuCounts, n: usize, amount: u64) {
        let bits = self.overflow_bits(n);
        let overflowed = distance_to_overflow(counts.evcntr[n], bits) <= amount;
        counts.evcntr[n] = counts.evcntr[n].wrapping_add(amount) & counter_mask(self.ctx.features);
        if !overflowed {
            return;
        }
        counts.ovs |= 1 << n;
        let odd = n + 1;
        if n.is_multiple_of(2)
            && bits == 32
            && odd < self.ctx.counters()
            && self.ctx.first_range(n) == self.ctx.first_range(odd)
            && self.event(odd) == Some(PmuEvent::Chain)
            && self.counts_event(odd, counts.ovs)
        {
            self.add_to_event(counts, odd, 1);
        }
    }

    /// An occurrence of `event` at the current level (Arm `PMUEvent`),
    /// counted by each counter that counts it.
    fn occur(self, counts: &mut PmuCounts, event: PmuEvent) {
        for n in 0..self.ctx.counters() {
            if self.event(n) == Some(event) && self.counts_event(n, counts.ovs) {
                self.add_to_event(counts, n, 1);
            }
        }
    }

    /// `cycles` cycles at the current level, each of whose instructions did
    /// what `cycle` says (Arm `AArch64_PMUCycle`, a run of them): the cycle
    /// counter and the counters of per-cycle events advance together, in
    /// steps that end at each overflow, after which the counters that
    /// count are worked out again — a freeze on overflow takes effect from
    /// the cycle after the overflow.
    fn run(self, counts: &mut PmuCounts, cycles: u64, cycle: Cycle) {
        let mut left = cycles;
        while left > 0 {
            let cycle_counter = self.counts_cycles(counts.ovs);
            let mut step = left;
            if cycle_counter {
                step = step.min(distance_to_overflow(counts.ccntr, 64));
            }
            let mut counting = [false; MAX_EVENT_COUNTERS];
            for (n, on) in counting.iter_mut().enumerate().take(self.ctx.counters()) {
                *on = self.event(n).is_some_and(|event| event.per_cycle(cycle))
                    && self.counts_event(n, counts.ovs);
                if *on {
                    step = step.min(distance_to_overflow(
                        counts.evcntr[n],
                        self.overflow_bits(n),
                    ));
                }
            }
            if !cycle_counter && !counting.contains(&true) {
                return;
            }
            for (n, on) in counting.iter().enumerate() {
                if *on {
                    self.add_to_event(counts, n, step);
                }
            }
            if cycle_counter {
                // The cycle counter overflows at bit 63 alone: `LC` is RES1
                // with no AArch32.
                if distance_to_overflow(counts.ccntr, 64) <= step {
                    counts.ovs |= CYCLE_BIT;
                }
                counts.ccntr = counts.ccntr.wrapping_add(step);
            }
            left -= step;
        }
    }

    /// How many more cycles may pass before a counter overflows, at the
    /// soonest: every enabled counter of a per-cycle event, and the cycle
    /// counter, counting at every level from now on.
    fn cycles_to_overflow(self, counts: &PmuCounts) -> Option<u64> {
        let cycles = self
            .cycles_enabled()
            .then(|| distance_to_overflow(counts.ccntr, 64));
        (0..self.ctx.counters())
            .filter(|&n| {
                self.event_enabled(n)
                    && self
                        .event(n)
                        .is_some_and(|event| event.per_cycle(Cycle::Executed))
            })
            .map(|n| distance_to_overflow(counts.evcntr[n], self.overflow_bits(n)))
            .chain(cycles)
            .min()
    }

    /// PMUIRQ (Arm `CheckForPMUOverflow` with no FEAT_EBEP): an overflow
    /// flag whose `PMINTENSET_EL1` bit is set, of a counter whose global
    /// enable is set.
    fn interrupt(self, ovs: u64) -> bool {
        let mut enabled = 0;
        if self.controls.pmcr & PMCR_E != 0 {
            enabled |= CYCLE_BIT | self.ctx.range_mask(true);
        }
        if self.ctx.mdcr & MDCR_HPME != 0 {
            enabled |= self.ctx.range_mask(false);
        }
        ovs & self.controls.inten & enabled != 0
    }
}

/// How many increments take a counter holding `value` past bit `bits - 1`:
/// at least one, and for a 64-bit overflow at most `u64::MAX`.
const fn distance_to_overflow(value: u64, bits: u32) -> u64 {
    let low = value & low_bits(bits as usize);
    if bits >= 64 {
        if low == 0 {
            u64::MAX
        } else {
            low.wrapping_neg()
        }
    } else {
        (1 << bits) - low
    }
}

/// An instruction count: the clock the PMU's cycles are counted on
/// ([`Cpu::retired`](super::Cpu)), unlike the tick count, which goes on
/// while the processor waits (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct InstructionCount(u64);

impl InstructionCount {
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

/// What the PMU drives, as of now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmuOutputs {
    /// PMUIRQ.
    pub(crate) interrupt: bool,
    /// The soonest instruction count at which a counter can overflow, if
    /// any counts per cycle: the machine runs to it and samples the PMU
    /// again.
    pub(crate) next_overflow: Option<InstructionCount>,
}

// ----- the registers' encodings -----

/// A PMU register, by its encoding.
///
/// Closed (R5): every encoding Arm gives a PMU register a processor here
/// can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmuReg {
    Pmcr,
    CntenSet,
    CntenClr,
    OvsClr,
    Swinc,
    Selr,
    Ceid(Ceid),
    Ccntr,
    Xevtyper,
    Xevcntr,
    Userenr,
    IntenSet,
    IntenClr,
    OvsSet,
    Mmir,
    /// `PMEVCNTR<n>_EL0`, `n` 0 to 30.
    Evcntr(usize),
    /// `PMEVTYPER<n>_EL0`, `n` 0 to 30.
    Evtyper(usize),
    Ccfiltr,
}

impl PmuReg {
    /// The PMU register `key` encodes, if it encodes one.
    pub(crate) const fn of(key: SysRegKey) -> Option<Self> {
        if key.op0() != 3 || !matches!(key.crn(), 9 | 14) {
            return None;
        }
        let (op1, crn, crm, op2) = (key.op1(), key.crn(), key.crm(), key.op2());
        Some(match (op1, crn, crm, op2) {
            (3, 9, 12, 0) => Self::Pmcr,
            (3, 9, 12, 1) => Self::CntenSet,
            (3, 9, 12, 2) => Self::CntenClr,
            (3, 9, 12, 3) => Self::OvsClr,
            (3, 9, 12, 4) => Self::Swinc,
            (3, 9, 12, 5) => Self::Selr,
            (3, 9, 12, 6) => Self::Ceid(Ceid::Low),
            (3, 9, 12, 7) => Self::Ceid(Ceid::High),
            (3, 9, 13, 0) => Self::Ccntr,
            (3, 9, 13, 1) => Self::Xevtyper,
            (3, 9, 13, 2) => Self::Xevcntr,
            (3, 9, 14, 0) => Self::Userenr,
            (0, 9, 14, 1) => Self::IntenSet,
            (0, 9, 14, 2) => Self::IntenClr,
            (3, 9, 14, 3) => Self::OvsSet,
            (0, 9, 14, 6) => Self::Mmir,
            (3, 14, 8..=11, _) => {
                let n = ((crm - 8) * 8 + op2) as usize;
                if n == 31 {
                    return None;
                }
                Self::Evcntr(n)
            }
            (3, 14, 12..=15, _) => {
                let n = ((crm - 12) * 8 + op2) as usize;
                if n == 31 {
                    Self::Ccfiltr
                } else {
                    Self::Evtyper(n)
                }
            }
            _ => return None,
        })
    }
}

/// Which way an access goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Read,
    Write,
}

/// What `PMUSERENR_EL0` lets EL0 do with a register, by its accessor (no
/// FEAT_PMUv3p9): which of its enables, any one set, lets the access
/// through, or whether EL0 has no access at all (UNDEFINED).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum El0Access {
    /// `EN`.
    En,
    /// `EN` or `ER`: a read of an event counter, `PMSELR_EL0`.
    EnOrEr,
    /// `EN` or `CR`: a read of the cycle counter.
    EnOrCr,
    /// `EN` or `SW`: a write of `PMSWINC_EL0`.
    EnOrSw,
    /// Whatever `PMUSERENR_EL0` holds: a read of `PMUSERENR_EL0`.
    Always,
    /// Never: the EL1 registers, and a write of `PMUSERENR_EL0`.
    Never,
}

impl El0Access {
    /// Whether EL0's access goes through with `PMUSERENR_EL0` holding
    /// `userenr`; `None` where EL0 has no access at all.
    const fn reaches(self, userenr: u64) -> Option<bool> {
        let enables = match self {
            Self::En => USERENR_EN,
            Self::EnOrEr => USERENR_EN | USERENR_ER,
            Self::EnOrCr => USERENR_EN | USERENR_CR,
            Self::EnOrSw => USERENR_EN | USERENR_SW,
            Self::Always => return Some(true),
            Self::Never => return None,
        };
        Some(userenr & enables != 0)
    }
}

/// The event counter an access names, if one: `PMEVCNTR<n>_EL0` and
/// `PMEVTYPER<n>_EL0` directly, `PMXEVCNTR_EL0` and `PMXEVTYPER_EL0`
/// through `PMSELR_EL0.SEL`, the cycle counter's filter where `SEL` is 31
/// for `PMXEVTYPER_EL0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counter {
    Direct(usize),
    Selected(usize),
    CycleFilter,
}

/// Where an access that passed its checks goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// The register.
    Register,
    /// Nowhere: the register is RES0 ([`CounterBeyondReach::Res0`]).
    Res0,
}

impl PmuReg {
    /// What `PMUSERENR_EL0` lets EL0 do with the register in direction
    /// `dir`, if the register has an accessor in that direction.
    const fn el0_access(self, dir: Direction) -> Option<El0Access> {
        use Direction::{Read, Write};
        Some(match (self, dir) {
            (Self::Swinc, Read) | (Self::Ceid(_), Write) | (Self::Mmir, Write) => return None,
            (Self::Swinc, Write) => El0Access::EnOrSw,
            (Self::Selr, Read | Write) | (Self::Evcntr(_) | Self::Xevcntr, Read) => {
                El0Access::EnOrEr
            }
            (Self::Ccntr, Read) => El0Access::EnOrCr,
            (Self::Userenr, Read) => El0Access::Always,
            (Self::Userenr, Write)
            | (Self::IntenSet | Self::IntenClr | Self::Mmir, Read | Write) => El0Access::Never,
            (
                Self::Pmcr
                | Self::CntenSet
                | Self::CntenClr
                | Self::OvsClr
                | Self::OvsSet
                | Self::Ceid(_)
                | Self::Xevtyper
                | Self::Evtyper(_)
                | Self::Ccfiltr,
                Read | Write,
            )
            | (Self::Ccntr | Self::Xevcntr | Self::Evcntr(_), Write) => El0Access::En,
        })
    }
}

impl<L: Levels> Cpu<L> {
    /// What counting reads besides the PMU's registers, now.
    fn pmu_context(&self) -> Context {
        Context {
            el: self.pstate.el,
            features: self.imp.features,
            shape: self.imp.pmu,
            have_el2: self.have_el2(),
            mdcr: self.el2().mdcr,
        }
    }

    fn pmu_counting(&self) -> Counting<'_> {
        Counting {
            controls: &self.pmu.controls,
            ctx: self.pmu_context(),
        }
    }

    /// The counters as they stand once the instructions up to `until` are
    /// counted, at the current level and with the current controls.
    fn pmu_counts_at(&self, until: u64) -> PmuCounts {
        let mut counts = self.pmu.counts;
        if self.pmu.enabled && until > counts.settled {
            let cycles = until - counts.settled;
            self.pmu_counting()
                .run(&mut counts, cycles, Cycle::Executed);
        }
        counts.settled = counts.settled.max(until);
        counts
    }

    /// Count the instructions up to `until` ([`Cpu::pmu_counts_at`]).
    pub(crate) fn pmu_settle(&mut self, until: u64) {
        self.pmu.counts = self.pmu_counts_at(until);
    }

    /// Work out again which counters are enabled, after a write of a
    /// control, and have the machine sample PMUIRQ and the next overflow.
    pub(crate) fn pmu_controls_changed(&mut self) {
        self.pmu.enabled = self.pmu_counting().any_enabled();
        self.outputs_dirty = true;
    }

    /// An occurrence of `event` now, the instructions before the one
    /// executing counted first. An occurrence that overflows a counter has
    /// the machine sample PMUIRQ.
    fn pmu_occur(&mut self, event: PmuEvent) {
        self.pmu_settle(self.retired);
        let before = self.pmu.counts.ovs;
        let counting = self.pmu_counting();
        let mut counts = self.pmu.counts;
        counting.occur(&mut counts, event);
        self.pmu.counts = counts;
        if self.pmu.counts.ovs != before {
            self.outputs_dirty = true;
        }
    }

    /// The instruction executing is an `MSR` to `CONTEXTIDR_EL1`
    /// (`CID_WRITE_RETIRED`) or to `TTBR0_EL1` or `TTBR1_EL1`
    /// (`TTBR_WRITE_RETIRED`) that completes.
    pub(crate) fn pmu_register_written(&mut self, event: PmuEvent) {
        if self.pmu.enabled {
            self.pmu_occur(event);
        }
    }

    /// An exception is about to be taken from the current level: the
    /// instructions before it are counted at that level, and so is
    /// `EXC_TAKEN` (D13.7.3.1: an exception is counted where it was taken
    /// from).
    pub(crate) fn pmu_exception_taken(&mut self) {
        if self.pmu.enabled {
            self.pmu_occur(PmuEvent::ExcTaken);
        }
    }

    /// The instruction just counted as retired took a synchronous exception
    /// in its place, as `cycle` says of it: the instructions before it are
    /// counted, then its own cycle.
    pub(crate) fn pmu_synchronous_exception(&mut self, cycle: Cycle) {
        if !self.pmu.enabled {
            return;
        }
        self.pmu_settle(self.retired.saturating_sub(1));
        let counting = self.pmu_counting();
        let mut counts = self.pmu.counts;
        let before = counts.ovs;
        counting.run(&mut counts, 1, cycle);
        counts.settled = self.retired;
        self.pmu.counts = counts;
        if counts.ovs != before {
            self.outputs_dirty = true;
        }
    }

    /// The exception return instruction executing is about to change the
    /// level: it is counted at the level it executes at, as an instruction
    /// and as `EXC_RETURN`, before it retires.
    pub(crate) fn pmu_exception_return(&mut self) {
        if !self.pmu.enabled {
            return;
        }
        self.pmu_occur(PmuEvent::ExcReturn);
        self.pmu_settle(self.retired + 1);
    }

    /// PMUIRQ and the next overflow, with the instructions retired so far
    /// counted.
    pub(crate) fn pmu_outputs(&mut self) -> PmuOutputs {
        self.pmu_settle(self.retired);
        let counting = self.pmu_counting();
        let next_overflow = if self.pmu.enabled {
            counting
                .cycles_to_overflow(&self.pmu.counts)
                .map(|cycles| InstructionCount(self.pmu.counts.settled.saturating_add(cycles)))
        } else {
            None
        };
        PmuOutputs {
            interrupt: counting.interrupt(self.pmu.counts.ovs),
            next_overflow,
        }
    }

    // ----- register access -----

    /// The PMU register `key` names, if this processor has it: every one
    /// with FEAT_PMUv3, `PMMIR_EL1` with FEAT_PMUv3p4.
    pub(crate) fn pmu_register(&self, key: SysRegKey) -> Option<PmuReg> {
        if !self.implements(Feature::PmuV3) {
            return None;
        }
        match PmuReg::of(key)? {
            PmuReg::Mmir if !self.implements(Feature::PmuV3p4) => None,
            reg => Some(reg),
        }
    }

    /// The counter `reg` names, if it names one.
    fn pmu_counter(&self, reg: PmuReg) -> Option<Counter> {
        let sel = (self.pmu.controls.selr & SELR_MASK) as usize;
        match reg {
            PmuReg::Evcntr(n) | PmuReg::Evtyper(n) => Some(Counter::Direct(n)),
            PmuReg::Xevcntr => Some(Counter::Selected(sel)),
            PmuReg::Xevtyper if sel == SEL_CYCLE => Some(Counter::CycleFilter),
            PmuReg::Xevtyper => Some(Counter::Selected(sel)),
            _ => None,
        }
    }

    /// What an access to `counter`, which the processor does not implement,
    /// does ([`CounterBeyondReach`]): the RES0 the Cortex-A57 makes of a
    /// selected counter is the whole access, before any trap is checked.
    fn pmu_unimplemented_counter(&self, counter: Counter) -> Result<Reach, SysRegError> {
        match (self.imp.pmu.beyond_reach, counter) {
            (CounterBeyondReach::Undefined, _) | (CounterBeyondReach::Res0, Counter::Direct(_)) => {
                Err(SysRegError::Undefined)
            }
            (CounterBeyondReach::Res0, Counter::Selected(_) | Counter::CycleFilter) => {
                Ok(Reach::Res0)
            }
        }
    }

    /// What an access from EL0 or EL1 to a counter the processor implements
    /// and reserves for EL2 does ([`CounterBeyondReach`]).
    fn pmu_reserved_counter(&self) -> Result<Reach, SysRegError> {
        match self.imp.pmu.beyond_reach {
            CounterBeyondReach::Undefined => Err(SysRegError::Undefined),
            CounterBeyondReach::Res0 => Ok(Reach::Res0),
        }
    }

    /// The checks an access to `reg` in direction `dir` at `el` makes, in
    /// the order its accessor makes them (Arm's `Registers.json`, with no
    /// EL3, FEAT_FGT or FEAT_PMUv3p9): a counter the processor does not
    /// implement; at EL0 `PMUSERENR_EL0`, trapping to EL1; at EL0 and EL1
    /// `MDCR_EL2.TPM`, for `PMCR_EL0` `MDCR_EL2.TPMCR`, each to EL2, and a
    /// counter reserved for EL2.
    fn pmu_check(&self, reg: PmuReg, dir: Direction, el: El) -> Result<Reach, SysRegError> {
        let access = reg.el0_access(dir).ok_or(SysRegError::Undefined)?;
        let ctx = Context {
            el,
            ..self.pmu_context()
        };
        let counter = self.pmu_counter(reg);
        if let Some(c @ (Counter::Direct(n) | Counter::Selected(n))) = counter {
            if n >= ctx.counters() {
                return self.pmu_unimplemented_counter(c);
            }
        }
        if el == El::El0 {
            let reaches = access
                .reaches(self.pmu.controls.userenr)
                .ok_or(SysRegError::Undefined)?;
            trap_to_el1(!reaches)?;
        }
        if el == El::El2 {
            return Ok(Reach::Register);
        }
        trap_to_el2(self.mdcr_any(MDCR_TPM))?;
        if reg == PmuReg::Pmcr {
            trap_to_el2(self.mdcr_any(MDCR_TPMCR))?;
        }
        match counter {
            Some(Counter::Direct(n) | Counter::Selected(n))
                if self.el2_enabled() && n >= ctx.accessible() =>
            {
                self.pmu_reserved_counter()
            }
            Some(Counter::Direct(_) | Counter::Selected(_) | Counter::CycleFilter) | None => {
                Ok(Reach::Register)
            }
        }
    }

    /// `MRS` of the PMU register `reg` at `el`.
    pub(crate) fn pmu_read(&self, reg: PmuReg, el: El) -> Result<u64, SysRegError> {
        if self.pmu_check(reg, Direction::Read, el)? == Reach::Res0 {
            return Ok(0);
        }
        let ctx = Context {
            el,
            ..self.pmu_context()
        };
        let controls = &self.pmu.controls;
        let counts = || self.pmu_counts_at(self.retired);
        let sel = (controls.selr & SELR_MASK) as usize;
        Ok(match reg {
            PmuReg::Pmcr => {
                controls.pmcr
                    | self.imp.pmu.identification
                    | ((ctx.accessible() as u64) << PMCR_N_SHIFT)
                    | PMCR_LC
            }
            PmuReg::CntenSet | PmuReg::CntenClr => controls.cnten & ctx.access_mask(),
            PmuReg::IntenSet | PmuReg::IntenClr => controls.inten & ctx.access_mask(),
            PmuReg::OvsSet | PmuReg::OvsClr => counts().ovs & ctx.access_mask(),
            PmuReg::Selr => controls.selr,
            PmuReg::Userenr => controls.userenr,
            PmuReg::Ceid(register) => self.imp.pmu.events.ceid(register),
            // No SLOTS, bus or threshold to describe.
            PmuReg::Mmir => 0,
            PmuReg::Ccntr => counts().ccntr,
            PmuReg::Ccfiltr => controls.ccfiltr,
            PmuReg::Evcntr(n) => counts().evcntr[n],
            PmuReg::Evtyper(n) => controls.evtyper[n],
            PmuReg::Xevcntr => counts().evcntr[sel],
            PmuReg::Xevtyper if sel == SEL_CYCLE => controls.ccfiltr,
            PmuReg::Xevtyper => controls.evtyper[sel],
            // A write-only register's read was refused by its checks.
            PmuReg::Swinc => 0,
        })
    }

    /// `MSR` of `value` to the PMU register `reg` at `el`.
    pub(crate) fn pmu_write(
        &mut self,
        reg: PmuReg,
        el: El,
        value: u64,
    ) -> Result<WriteEffect, SysRegError> {
        if self.pmu_check(reg, Direction::Write, el)? == Reach::Res0 {
            return Ok(WriteEffect::None);
        }
        self.pmu_settle(self.retired);
        let ctx = Context {
            el,
            ..self.pmu_context()
        };
        let features = self.imp.features;
        let mask = ctx.access_mask();
        let sel = (self.pmu.controls.selr & SELR_MASK) as usize;
        let filter = self.pmu_filter_mask();
        let evtyper = self.pmu_evtyper_mask();
        let pmcr = pmcr_kept(features, self.imp.pmu, L::EL2);
        let controls = &mut self.pmu.controls;
        let counts = &mut self.pmu.counts;
        match reg {
            PmuReg::Pmcr => {
                controls.pmcr = value & pmcr;
                // Arm `SysRegZeroCycleCounter` and `SysRegZeroEventCounters`:
                // the cycle counter, and the accessible event counters.
                if value & PMCR_C != 0 {
                    counts.ccntr = 0;
                }
                if value & PMCR_P != 0 {
                    for (n, counter) in counts.evcntr.iter_mut().enumerate() {
                        if mask & (1 << n) != 0 {
                            *counter = 0;
                        }
                    }
                }
            }
            PmuReg::CntenSet => controls.cnten |= value & mask,
            PmuReg::CntenClr => controls.cnten &= !(value & mask),
            PmuReg::IntenSet => controls.inten |= value & mask,
            PmuReg::IntenClr => controls.inten &= !(value & mask),
            PmuReg::OvsSet => counts.ovs |= value & mask,
            PmuReg::OvsClr => counts.ovs &= !(value & mask),
            PmuReg::Selr => {
                controls.selr = value & SELR_MASK;
                return Ok(WriteEffect::None);
            }
            PmuReg::Userenr => {
                controls.userenr = value & USERENR_MASK;
                return Ok(WriteEffect::None);
            }
            PmuReg::Ccntr => counts.ccntr = value,
            PmuReg::Ccfiltr => controls.ccfiltr = value & filter,
            PmuReg::Evcntr(n) => counts.evcntr[n] = value & counter_mask(features),
            PmuReg::Evtyper(n) => controls.evtyper[n] = value & evtyper,
            PmuReg::Xevcntr => counts.evcntr[sel] = value & counter_mask(features),
            PmuReg::Xevtyper if sel == SEL_CYCLE => controls.ccfiltr = value & filter,
            PmuReg::Xevtyper => controls.evtyper[sel] = value & evtyper,
            // Arm `PMUSwIncrement`: the accessible counters' bits.
            PmuReg::Swinc => {
                let bits = value & mask & !CYCLE_BIT;
                let counting = Counting {
                    controls: &self.pmu.controls,
                    ctx,
                };
                let mut counts = self.pmu.counts;
                for n in 0..ctx.counters() {
                    if bits & (1 << n) != 0
                        && counting.event(n) == Some(PmuEvent::SwIncr)
                        && counting.counts_event(n, counts.ovs)
                    {
                        counting.add_to_event(&mut counts, n, 1);
                    }
                }
                self.pmu.counts = counts;
            }
            // Read-only registers' writes were refused by their checks.
            PmuReg::Ceid(_) | PmuReg::Mmir => {}
        }
        self.pmu.enabled = self.pmu_counting().any_enabled();
        Ok(WriteEffect::Pmu)
    }

    /// The `PMCCFILTR_EL0` bits this processor keeps: `P`, `U`, and with
    /// EL2 `NSH`; `NSK`, `NSU`, `M` and the rest are RES0 with no EL3, no
    /// Secure state, no FEAT_RME and no FEAT_PMUv3_SME.
    fn pmu_filter_mask(&self) -> u64 {
        filter_mask(L::EL2)
    }

    /// The `PMEVTYPER<n>_EL0` bits this processor keeps: the filters
    /// ([`Cpu::pmu_filter_mask`]) and `evtCount`; `MT` is RES0 with no
    /// multithreading and no FEAT_MTPMU, and there are no threshold fields
    /// without FEAT_PMUv3_TH.
    fn pmu_evtyper_mask(&self) -> u64 {
        evtyper_mask(self.imp.features, L::EL2)
    }

    // ----- snapshots -----

    /// The PMU's registers, its counters as the instructions retired so far
    /// leave them; nothing without FEAT_PMUv3.
    pub(crate) fn save_pmu<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        if !self.implements(Feature::PmuV3) {
            return Ok(());
        }
        let c = &self.pmu.controls;
        for value in [c.pmcr, c.cnten, c.inten, c.selr, c.userenr, c.ccfiltr] {
            w.write_u64(value)?;
        }
        for value in c.evtyper {
            w.write_u64(value)?;
        }
        let counts = self.pmu_counts_at(self.retired);
        w.write_u64(counts.ccntr)?;
        for value in counts.evcntr {
            w.write_u64(value)?;
        }
        w.write_u64(counts.ovs)
    }

    /// The inverse of [`Cpu::save_pmu`], once the instruction count is
    /// restored: the counters are current to it. Refuses a register
    /// holding a bit no write keeps, and a counter past those the processor
    /// has holding anything.
    pub(crate) fn restore_pmu<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        if !self.implements(Feature::PmuV3) {
            self.pmu = Pmu::reset(self.imp.features, self.imp.pmu, self.retired);
            return Ok(());
        }
        let features = self.imp.features;
        let implemented = CYCLE_BIT | low_bits(self.imp.pmu.counters.get());
        let mut read = || r.read_u64();
        let pmcr = read()?;
        let cnten = read()?;
        let inten = read()?;
        let selr = read()?;
        let userenr = read()?;
        let ccfiltr = read()?;
        let mut evtyper = [0; MAX_EVENT_COUNTERS];
        for value in evtyper.iter_mut() {
            *value = read()?;
        }
        let ccntr = read()?;
        let mut evcntr = [0; MAX_EVENT_COUNTERS];
        for value in evcntr.iter_mut() {
            *value = read()?;
        }
        let ovs = read()?;
        let beyond = !low_bits(self.imp.pmu.counters.get());
        let controls_kept = pmcr & !pmcr_kept(features, self.imp.pmu, L::EL2) == 0
            && (cnten | inten | ovs) & !implemented == 0
            && selr & !SELR_MASK == 0
            && userenr & !USERENR_MASK == 0
            && ccfiltr & !filter_mask(L::EL2) == 0
            && evtyper
                .iter()
                .all(|t| t & !evtyper_mask(features, L::EL2) == 0);
        let counters_kept = evcntr.iter().enumerate().all(|(n, &value)| {
            value & !counter_mask(features) == 0
                && (beyond & (1 << n) == 0 || (value == 0 && evtyper[n] == 0))
        });
        if !controls_kept || !counters_kept {
            return Err(SnapError::Invalid(
                "snapshot's PMU registers hold bits this processor cannot set",
            ));
        }
        self.pmu.controls = PmuControls {
            pmcr,
            cnten,
            inten,
            selr,
            userenr,
            ccfiltr,
            evtyper,
        };
        self.pmu.counts = PmuCounts {
            ccntr,
            evcntr,
            ovs,
            settled: self.retired,
        };
        self.pmu.enabled = self.pmu_counting().any_enabled();
        Ok(())
    }
}

/// The `PMCR_EL0` fields a processor of `features` and `shape` stores, with
/// EL2 or without (`el2`): `E`; `X` where the shape exports events; `DP`
/// with FEAT_PMUv3p1 and EL2, or with FEAT_PMUv3p7; `LP` with
/// FEAT_PMUv3p5; `FZO` with FEAT_PMUv3p7. `C` and `P` act and read as zero,
/// `LC` is RES1 and `D` RES0 with no AArch32 (Arm's `PMCR_EL0` fieldset).
pub(crate) const fn pmcr_kept(features: Features, shape: PmuShape, el2: bool) -> u64 {
    if !features.has(Feature::PmuV3) {
        return 0;
    }
    let mut kept = PMCR_E;
    if matches!(shape.export, EventExport::Implemented) {
        kept |= PMCR_X;
    }
    if (features.has(Feature::PmuV3p1) && el2) || features.has(Feature::PmuV3p7) {
        kept |= PMCR_DP;
    }
    if features.has(Feature::PmuV3p5) {
        kept |= PMCR_LP;
    }
    if features.has(Feature::PmuV3p7) {
        kept |= PMCR_FZO;
    }
    kept
}

/// [`Cpu::pmu_filter_mask`] for a processor with EL2 or without (`el2`).
const fn filter_mask(el2: bool) -> u64 {
    if el2 {
        FILTER_P | FILTER_U | FILTER_NSH
    } else {
        FILTER_P | FILTER_U
    }
}

/// [`Cpu::pmu_evtyper_mask`] for a processor of `features`, with EL2 or
/// without (`el2`).
const fn evtyper_mask(features: Features, el2: bool) -> u64 {
    let evtcount = if features.has(Feature::PmuV3p1) {
        0xFFFF
    } else {
        0x3FF
    };
    filter_mask(el2) | evtcount
}

#[cfg(test)]
#[path = "../../tests/unit/pmu.rs"]
mod tests;
