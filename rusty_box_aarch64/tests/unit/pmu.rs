//! The PMU as a guest's `MRS` and `MSR` see it (R9): which accesses each
//! level has and the exception the others take, what `MDCR_EL2.HPMN`
//! reserves for EL2, and what the counters count — `SW_INCR`, `CHAIN`, the
//! register-write events, freezing on overflow and the controls that keep
//! the counters off at EL2 — on each processor, with EL2 and without.

use std::vec::Vec;

use super::super::features::{Feature, Implementation};
use super::super::step::{exec, processor_with, Taken, UNDEFINED};
use super::super::{Cpu, El, Levels, NoEl2, Pstate, WithEl2};

/// A System register's `op0`, `op1`, `CRn`, `CRm` and `op2`.
type Enc = (u32, u32, u32, u32, u32);

const PMCR: Enc = (3, 3, 9, 12, 0);
const PMCNTENSET: Enc = (3, 3, 9, 12, 1);
const PMOVSSET: Enc = (3, 3, 9, 14, 3);
const PMSWINC: Enc = (3, 3, 9, 12, 4);
const PMSELR: Enc = (3, 3, 9, 12, 5);
const PMCCNTR: Enc = (3, 3, 9, 13, 0);
const PMXEVTYPER: Enc = (3, 3, 9, 13, 1);
const PMXEVCNTR: Enc = (3, 3, 9, 13, 2);
const PMUSERENR: Enc = (3, 3, 9, 14, 0);
const PMINTENSET: Enc = (3, 0, 9, 14, 1);
const PMMIR: Enc = (3, 0, 9, 14, 6);
const PMCCFILTR: Enc = (3, 3, 14, 15, 7);
const MDCR_EL2: Enc = (3, 4, 1, 1, 1);
const CONTEXTIDR_EL1: Enc = (3, 0, 13, 0, 1);
const TTBR0_EL1: Enc = (3, 0, 2, 0, 0);
const TTBR1_EL1: Enc = (3, 0, 2, 0, 1);
const ID_AA64DFR0_EL1: Enc = (3, 0, 0, 5, 0);

const fn pmevcntr(n: u32) -> Enc {
    (3, 3, 14, 8 | (n >> 3), n & 7)
}

const fn pmevtyper(n: u32) -> Enc {
    (3, 3, 14, 12 | (n >> 3), n & 7)
}

/// `mrs x0, reg`.
const fn mrs(reg: Enc) -> u32 {
    let (op0, op1, crn, crm, op2) = reg;
    0xD520_0000 | (op0 << 19) | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5)
}

/// `msr reg, x0`.
const fn msr(reg: Enc) -> u32 {
    mrs(reg) & !(1 << 21)
}

const NOP: u32 = 0xD503_201F;

/// The event numbers and filter bits the tests use.
const SW_INCR: u64 = 0x00;
const INST_RETIRED: u64 = 0x08;
const CID_WRITE_RETIRED: u64 = 0x0B;
const TTBR_WRITE_RETIRED: u64 = 0x1C;
const CHAIN: u64 = 0x1E;
/// `L1D_CACHE`, which no processor here counts.
const L1D_CACHE: u64 = 0x04;
const FILTER_P: u64 = 1 << 31;
const FILTER_NSH: u64 = 1 << 27;
const CYCLES: u64 = 1 << 31;
const PMCR_E: u64 = 1;
const PMCR_P: u64 = 1 << 1;
const PMCR_DP: u64 = 1 << 5;
const PMCR_LP: u64 = 1 << 7;
const PMCR_FZO: u64 = 1 << 9;
/// `PMUSERENR_EL0` enables.
const EN: u64 = 1;
const SW: u64 = 1 << 1;
const CR: u64 = 1 << 2;
const ER: u64 = 1 << 3;
/// `MDCR_EL2` fields.
const TPMCR: u64 = 1 << 5;
const TPM: u64 = 1 << 6;
const HPME: u64 = 1 << 7;
const HPMD: u64 = 1 << 17;
const HCCD: u64 = 1 << 23;

/// What an access did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// It completed; a read's value.
    Done(u64),
    /// It took an exception to EL1, as `ESR_EL1` reports it.
    El1(Taken),
    /// It took an exception to EL2, as `ESR_EL2` reports it.
    El2(Taken),
}

/// The trap of `insn` (EC 0x18), its ISS naming the access
/// (`AArch64_SystemAccessTrapSyndrome`).
fn trapped(insn: u32) -> Taken {
    let w = u64::from(insn);
    Taken {
        ec: 0x18,
        iss: ((w >> 19) & 3) << 20
            | ((w >> 5) & 7) << 17
            | ((w >> 16) & 7) << 14
            | ((w >> 12) & 0xF) << 10
            | (w & 0x1F) << 5
            | ((w >> 8) & 0xF) << 1
            | ((w >> 21) & 1),
    }
}

/// Put the processor at `el`, its instructions so far counted where they
/// ran.
fn at<L: Levels>(cpu: &mut Cpu<L>, el: El) {
    let pstate = Pstate {
        el,
        sp_sel: el != El::El0,
        ..cpu.pstate
    };
    cpu.set_pstate(pstate).expect("a level the processor has");
}

/// Step `insn` at `el`, with X0 `x0`, and leave the processor at the level
/// it was at.
fn access<L: Levels>(cpu: &mut Cpu<L>, el: El, insn: u32, x0: u64) -> Outcome {
    let before = cpu.pstate;
    at(cpu, el);
    if let Some(el2) = cpu.el2_mut() {
        el2.esr = 0x3F << 26;
    }
    cpu.x[0] = x0;
    let outcome = match exec(cpu, insn) {
        Some(taken) => Outcome::El1(taken),
        None if L::EL2 && cpu.el2().esr >> 26 != 0x3F => {
            let esr = cpu.el2().esr;
            Outcome::El2(Taken {
                ec: esr >> 26,
                iss: esr & 0x01FF_FFFF,
            })
        }
        None => Outcome::Done(cpu.x[0]),
    };
    at(cpu, before.el);
    cpu.pstate = before;
    outcome
}

fn read<L: Levels>(cpu: &mut Cpu<L>, el: El, reg: Enc) -> Outcome {
    access(cpu, el, mrs(reg), 0xDEAD)
}

fn write<L: Levels>(cpu: &mut Cpu<L>, el: El, reg: Enc, value: u64) -> Outcome {
    access(cpu, el, msr(reg), value)
}

/// Write `reg` at the processor's highest level, which every write here
/// reaches.
fn set<L: Levels>(cpu: &mut Cpu<L>, reg: Enc, value: u64) {
    let el = if L::EL2 { El::El2 } else { El::El1 };
    assert_eq!(write(cpu, el, reg, value), Outcome::Done(value), "{reg:?}");
}

/// Read `reg` at the processor's highest level.
fn get<L: Levels>(cpu: &mut Cpu<L>, reg: Enc) -> u64 {
    let el = if L::EL2 { El::El2 } else { El::El1 };
    match read(cpu, el, reg) {
        Outcome::Done(value) => value,
        other => panic!("{reg:?}: {other:?}"),
    }
}

/// Run `n` `NOP`s at the current level.
fn nops<L: Levels>(cpu: &mut Cpu<L>, n: usize) {
    for _ in 0..n {
        assert_eq!(exec(cpu, NOP), None);
    }
}

/// Each processor model, without EL2 and with it.
fn models() -> [Implementation; 2] {
    [Implementation::CORTEX_A57, Implementation::ARMV9P2]
}

/// At EL0 a PMU register is reached only as `PMUSERENR_EL0` allows (no
/// FEAT_PMUv3p9): `EN` reaches every one EL0 has; `CR` alone the cycle
/// counter's read, `ER` alone the event counters' reads and `PMSELR_EL0`,
/// `SW` alone `PMSWINC_EL0`; `PMUSERENR_EL0` reads whatever it holds. Any
/// other access traps to EL1. `PMINTENSET_EL1` and the write of
/// `PMUSERENR_EL0` are UNDEFINED at EL0, and `PMSWINC_EL0`'s read at every
/// level.
#[test]
fn pmuserenr_lets_el0_reach_what_its_enables_name() {
    // The enables tried, and for each access whether each lets it through.
    let enables = [0, EN, SW, CR, ER];
    let cases = [
        (mrs(PMCR), [false, true, false, false, false]),
        (msr(PMCR), [false, true, false, false, false]),
        (mrs(PMCCNTR), [false, true, false, true, false]),
        (msr(PMCCNTR), [false, true, false, false, false]),
        (mrs(pmevcntr(0)), [false, true, false, false, true]),
        (msr(pmevcntr(0)), [false, true, false, false, false]),
        (mrs(PMXEVCNTR), [false, true, false, false, true]),
        (mrs(PMSELR), [false, true, false, false, true]),
        (msr(PMSELR), [false, true, false, false, true]),
        (msr(PMSWINC), [false, true, true, false, false]),
        (mrs(pmevtyper(0)), [false, true, false, false, false]),
        (mrs(PMCNTENSET), [false, true, false, false, false]),
        (mrs(PMUSERENR), [true, true, true, true, true]),
    ];
    for imp in models() {
        let mut cpu = processor_with::<NoEl2>(imp);
        for (insn, allowed) in cases {
            for (enable, allowed) in enables.into_iter().zip(allowed) {
                set(&mut cpu, PMUSERENR, enable);
                set(&mut cpu, PMSELR, 0);
                let outcome = access(&mut cpu, El::El0, insn, 0);
                if allowed {
                    assert!(
                        matches!(outcome, Outcome::Done(_)),
                        "{insn:#010x}, PMUSERENR_EL0 {enable:#x}: {outcome:?}"
                    );
                } else {
                    assert_eq!(
                        outcome,
                        Outcome::El1(trapped(insn)),
                        "{insn:#010x}, PMUSERENR_EL0 {enable:#x}"
                    );
                }
            }
        }
        set(&mut cpu, PMUSERENR, EN | SW | CR | ER);
        for insn in [mrs(PMINTENSET), msr(PMINTENSET), msr(PMUSERENR)] {
            assert_eq!(
                access(&mut cpu, El::El0, insn, 0),
                Outcome::El1(UNDEFINED),
                "{insn:#010x}"
            );
        }
        for el in [El::El0, El::El1] {
            assert_eq!(
                access(&mut cpu, el, mrs(PMSWINC), 0),
                Outcome::El1(UNDEFINED),
                "{el:?}"
            );
        }
    }
}

/// `MDCR_EL2.TPM` traps EL0's and EL1's accesses to the PMU to EL2, after
/// `PMUSERENR_EL0`'s own trap of EL0's; `TPMCR` traps `PMCR_EL0` alone. EL2
/// is trapped by neither.
#[test]
fn mdcr_el2_traps_the_pmu_to_el2() {
    for imp in models() {
        let mut cpu = processor_with::<WithEl2>(imp);
        set(&mut cpu, MDCR_EL2, 6 | TPM);
        assert_eq!(
            read(&mut cpu, El::El0, PMCCNTR),
            Outcome::El1(trapped(mrs(PMCCNTR))),
            "PMUSERENR_EL0 first"
        );
        set(&mut cpu, PMUSERENR, EN);
        for el in [El::El0, El::El1] {
            assert_eq!(
                read(&mut cpu, el, PMCCNTR),
                Outcome::El2(trapped(mrs(PMCCNTR))),
                "{el:?}"
            );
            assert_eq!(
                write(&mut cpu, el, PMCR, 0),
                Outcome::El2(trapped(msr(PMCR))),
                "{el:?}"
            );
        }
        assert!(matches!(read(&mut cpu, El::El2, PMCCNTR), Outcome::Done(_)));

        set(&mut cpu, MDCR_EL2, 6 | TPMCR);
        for el in [El::El0, El::El1] {
            assert_eq!(
                read(&mut cpu, el, PMCR),
                Outcome::El2(trapped(mrs(PMCR))),
                "{el:?}"
            );
            assert!(
                matches!(read(&mut cpu, el, PMCCNTR), Outcome::Done(_)),
                "{el:?}"
            );
        }
        assert!(matches!(read(&mut cpu, El::El2, PMCR), Outcome::Done(_)));
    }
}

/// `MDCR_EL2.HPMN` gives EL0 and EL1 the event counters below it:
/// `PMCR_EL0.N` reads it there and six at EL2, the enables read without
/// the counters past it, and `PMCR_EL0.P` there zeroes only those below
/// it. A value above six, or zero, reads back as written and counts as six,
/// or as none (the Cortex-A57's answers, DDI 0488H B.2.20 and B.2.21). A
/// counter past it is RES0 from EL0 and EL1 on the Cortex-A57 (B.2.25) and
/// traps to EL2 on the Armv9.2 model, which with EL2 has FEAT_FGT (Arm's
/// accessors); one past the six the processor has is
/// UNDEFINED on both, at every level, but through `PMXEVCNTR_EL0` the
/// Cortex-A57 makes it RES0 (B.2.22), even to EL0 without `PMUSERENR_EL0`.
#[test]
fn hpmn_reserves_the_counters_above_it_for_el2() {
    for imp in models() {
        let a57 = imp.midr == Implementation::CORTEX_A57.midr;
        let mut cpu = processor_with::<WithEl2>(imp);
        let n = |pmcr: u64| (pmcr >> 11) & 0x1F;
        assert_eq!(get(&mut cpu, MDCR_EL2) & 0x1F, 6, "HPMN resets to N");
        set(&mut cpu, MDCR_EL2, 2);
        assert_eq!(read(&mut cpu, El::El1, PMCR).map_done(n), Some(2), "EL1");
        assert_eq!(n(get(&mut cpu, PMCR)), 6, "EL2");
        for counter in 0..6 {
            set(&mut cpu, pmevcntr(counter), 0x100 + u64::from(counter));
        }
        set(&mut cpu, PMCNTENSET, CYCLES | 0x3F);
        assert_eq!(
            read(&mut cpu, El::El1, PMCNTENSET),
            Outcome::Done(CYCLES | 0x3),
            "the reserved counters' enables read as zero below EL2"
        );
        assert_eq!(
            write(&mut cpu, El::El1, PMCR, PMCR_P),
            Outcome::Done(PMCR_P)
        );
        for counter in 0..6 {
            let want = if counter < 2 {
                0
            } else {
                0x100 + u64::from(counter)
            };
            assert_eq!(get(&mut cpu, pmevcntr(counter)), want, "counter {counter}");
        }
        let beyond = read(&mut cpu, El::El1, pmevcntr(4));
        let written = write(&mut cpu, El::El1, pmevcntr(4), 7);
        if a57 {
            assert_eq!(beyond, Outcome::Done(0), "RES0");
            assert_eq!(written, Outcome::Done(7));
            assert_eq!(get(&mut cpu, pmevcntr(4)), 0x104, "the write is ignored");
        } else {
            // With FEAT_FGT, which the Armv9.2 model has with EL2, Arm's
            // accessor traps it to EL2.
            assert_eq!(beyond, Outcome::El2(trapped(mrs(pmevcntr(4)))));
            assert_eq!(written, Outcome::El2(trapped(msr(pmevcntr(4)))));
        }
        for el in [El::El0, El::El1, El::El2] {
            set(&mut cpu, PMUSERENR, EN);
            assert_eq!(
                read(&mut cpu, el, pmevcntr(6)),
                Outcome::El1(UNDEFINED).routed(el),
                "{el:?}: no counter 6"
            );
        }
        set(&mut cpu, PMUSERENR, 0);
        set(&mut cpu, PMSELR, 7);
        let selected = read(&mut cpu, El::El0, PMXEVCNTR);
        if a57 {
            assert_eq!(selected, Outcome::Done(0), "RES0");
        } else {
            assert_eq!(selected, Outcome::El1(UNDEFINED));
        }
        set(&mut cpu, PMSELR, 31);
        set(&mut cpu, PMCCFILTR, FILTER_P);
        assert_eq!(
            get(&mut cpu, PMXEVTYPER),
            FILTER_P,
            "SEL 31 selects PMCCFILTR_EL0"
        );

        set(&mut cpu, MDCR_EL2, 9);
        assert_eq!(get(&mut cpu, MDCR_EL2) & 0x1F, 9, "read back as written");
        assert_eq!(read(&mut cpu, El::El1, PMCR).map_done(n), Some(6));
        set(&mut cpu, MDCR_EL2, 0);
        assert_eq!(read(&mut cpu, El::El1, PMCR).map_done(n), Some(0));
    }
}

impl Outcome {
    /// The value a completed read gives, through `f`.
    fn map_done(self, f: impl Fn(u64) -> u64) -> Option<u64> {
        match self {
            Self::Done(value) => Some(f(value)),
            Self::El1(_) | Self::El2(_) => None,
        }
    }

    /// An UNDEFINED exception from `el`: at EL2, taken to EL2.
    fn routed(self, el: El) -> Self {
        match (self, el) {
            (Self::El1(taken), El::El2) => Self::El2(taken),
            (outcome, _) => outcome,
        }
    }
}

/// A processor without FEAT_PMUv3 has none of the PMU's registers, its
/// `ID_AA64DFR0_EL1.PMUVer` reads 0 and `MDCR_EL2` keeps none of its PMU
/// fields.
#[test]
fn a_processor_without_a_pmu_has_none_of_its_registers() {
    let none = Implementation::every_feature_but(&[
        Feature::PmuV3,
        Feature::PmuV3p1,
        Feature::PmuV3p4,
        Feature::PmuV3p5,
        Feature::PmuV3p7,
    ]);
    let mut cpu = processor_with::<WithEl2>(none);
    for reg in [
        PMCR,
        PMCCNTR,
        PMUSERENR,
        PMINTENSET,
        PMMIR,
        pmevcntr(0),
        PMCCFILTR,
    ] {
        for el in [El::El1, El::El2] {
            assert_eq!(
                read(&mut cpu, el, reg),
                Outcome::El1(UNDEFINED).routed(el),
                "{reg:?} at {el:?}"
            );
        }
    }
    assert_eq!((get(&mut cpu, ID_AA64DFR0_EL1) >> 8) & 0xF, 0);
    set(&mut cpu, MDCR_EL2, u64::MAX);
    assert_eq!(
        get(&mut cpu, MDCR_EL2) & 0xFF,
        0,
        "no HPMN, TPMCR, TPM or HPME"
    );
}

/// `PMMIR_EL1` is FEAT_PMUv3p4's: the Armv9.2 model has it, reading zero
/// (no slots, bus or thresholds to describe), and the Cortex-A57 does not.
#[test]
fn pmmir_is_the_armv9_2_model_s() {
    let mut cpu = processor_with::<NoEl2>(Implementation::ARMV9P2);
    assert_eq!(read(&mut cpu, El::El1, PMMIR), Outcome::Done(0));
    let mut cpu = processor_with::<NoEl2>(Implementation::CORTEX_A57);
    assert_eq!(read(&mut cpu, El::El1, PMMIR), Outcome::El1(UNDEFINED));
}

/// A write of `PMSWINC_EL0` increments each counter counting `SW_INCR`
/// whose bit it sets and that counts at the level the write is made at;
/// one not enabled, or filtered out there, does not move.
#[test]
fn sw_incr_counts_the_writes_at_the_levels_filtered() {
    for imp in models() {
        let mut cpu = processor_with::<NoEl2>(imp);
        set(&mut cpu, pmevtyper(0), SW_INCR);
        set(&mut cpu, pmevtyper(1), SW_INCR | FILTER_P);
        set(&mut cpu, pmevtyper(2), SW_INCR);
        for counter in 0..3 {
            set(&mut cpu, pmevcntr(counter), 0);
        }
        set(&mut cpu, PMCNTENSET, 0b011);
        set(&mut cpu, PMCR, PMCR_E);
        set(&mut cpu, PMSWINC, 0b111);
        set(&mut cpu, PMUSERENR, SW);
        assert_eq!(
            write(&mut cpu, El::El0, PMSWINC, 0b111),
            Outcome::Done(0b111)
        );
        assert_eq!(get(&mut cpu, pmevcntr(0)), 2, "{imp:?}: EL1 and EL0");
        assert_eq!(get(&mut cpu, pmevcntr(1)), 1, "{imp:?}: EL0 alone");
        assert_eq!(get(&mut cpu, pmevcntr(2)), 0, "{imp:?}: not enabled");
    }
}

/// An event a processor does not advertise in `PMCEIDn_EL0` is one it does
/// not count: a counter set to `L1D_CACHE` stays where it is.
#[test]
fn an_event_not_advertised_counts_nothing() {
    for imp in models() {
        let mut cpu = processor_with::<NoEl2>(imp);
        let ceid0 = get(&mut cpu, (3, 3, 9, 12, 6));
        assert_eq!(ceid0 & (1 << L1D_CACHE), 0);
        set(&mut cpu, pmevtyper(0), L1D_CACHE);
        set(&mut cpu, pmevcntr(0), 5);
        set(&mut cpu, PMCNTENSET, 1);
        set(&mut cpu, PMCR, PMCR_E);
        nops(&mut cpu, 10);
        assert_eq!(get(&mut cpu, pmevcntr(0)), 5, "{imp:?}");
    }
}

/// `CID_WRITE_RETIRED` counts the `MSR`s to `CONTEXTIDR_EL1`,
/// `TTBR_WRITE_RETIRED` those to `TTBR0_EL1` and `TTBR1_EL1`.
#[test]
fn the_register_write_events_count_their_msrs() {
    for imp in models() {
        let mut cpu = processor_with::<NoEl2>(imp);
        set(&mut cpu, pmevtyper(0), CID_WRITE_RETIRED);
        set(&mut cpu, pmevtyper(1), TTBR_WRITE_RETIRED);
        set(&mut cpu, pmevcntr(0), 0);
        set(&mut cpu, pmevcntr(1), 0);
        set(&mut cpu, PMCNTENSET, 0b11);
        set(&mut cpu, PMCR, PMCR_E);
        for reg in [CONTEXTIDR_EL1, TTBR0_EL1, TTBR1_EL1, TTBR0_EL1] {
            set(&mut cpu, reg, 0x1000);
        }
        nops(&mut cpu, 3);
        assert_eq!(get(&mut cpu, pmevcntr(0)), 1, "{imp:?}");
        assert_eq!(get(&mut cpu, pmevcntr(1)), 3, "{imp:?}");
    }
}

/// An even counter's overflow of bits 31:0 is a `CHAIN` event of the odd
/// counter above it, which makes a 64-bit counter of the pair on the
/// Cortex-A57. On the Armv9.2 model the event counters are 64 bits wide
/// (FEAT_PMUv3p5): with `PMCR_EL0.LP` clear they overflow at bit 31 and
/// chain alike, with it set they count on past it and neither overflows
/// nor chains.
#[test]
fn chain_counts_the_overflows_of_the_counter_below() {
    for (imp, long) in [
        (Implementation::CORTEX_A57, 0),
        (Implementation::ARMV9P2, 0),
        (Implementation::ARMV9P2, PMCR_LP),
    ] {
        let mut cpu = processor_with::<NoEl2>(imp);
        set(&mut cpu, pmevtyper(0), INST_RETIRED);
        set(&mut cpu, pmevtyper(1), CHAIN);
        set(&mut cpu, pmevcntr(0), 0xFFFF_FFFE);
        set(&mut cpu, pmevcntr(1), 0);
        set(&mut cpu, PMCNTENSET, 0b11);
        // The `MSR` that starts the counters and two `NOP`s: the second
        // instruction takes counter 0 past bit 31.
        set(&mut cpu, PMCR, PMCR_E | long);
        nops(&mut cpu, 2);
        let (low, high, ovs) = match (imp.has(Feature::PmuV3p5), long) {
            (false, _) => (1, 1, 1),
            (true, 0) => (0x1_0000_0001, 1, 1),
            (true, _) => (0x1_0000_0001, 0, 0),
        };
        assert_eq!(get(&mut cpu, pmevcntr(0)), low, "{imp:?}, LP {long:#x}");
        assert_eq!(get(&mut cpu, pmevcntr(1)), high, "{imp:?}, LP {long:#x}");
        assert_eq!(get(&mut cpu, PMOVSSET), ovs, "{imp:?}, LP {long:#x}");
    }
}

/// With FEAT_PMUv3p7's `PMCR_EL0.FZO`, an overflow of an event counter
/// stops the event counters, from the instruction after the one that
/// overflowed it, and with `PMCR_EL0.DP` the cycle counter with them;
/// without `DP` the cycle counter counts on, through the four `NOP`s and
/// the two reads before its own. Clearing the overflow flag starts them
/// again, from the `MSR` that clears it.
#[test]
fn an_overflow_freezes_the_counters_under_fzo() {
    for dp in [0, PMCR_DP] {
        let mut cpu = processor_with::<NoEl2>(Implementation::ARMV9P2);
        set(&mut cpu, pmevtyper(0), INST_RETIRED);
        set(&mut cpu, pmevtyper(1), INST_RETIRED);
        set(&mut cpu, pmevcntr(0), 0xFFFF_FFFF);
        set(&mut cpu, pmevcntr(1), 0);
        set(&mut cpu, PMCCNTR, 0);
        set(&mut cpu, PMCNTENSET, CYCLES | 0b11);
        set(&mut cpu, PMCR, PMCR_E | PMCR_FZO | dp);
        nops(&mut cpu, 4);
        assert_eq!(get(&mut cpu, PMOVSSET), 1, "DP {dp:#x}");
        assert_eq!(
            get(&mut cpu, pmevcntr(1)),
            1,
            "DP {dp:#x}: frozen after one"
        );
        let cycles = if dp == 0 { 7 } else { 1 };
        assert_eq!(get(&mut cpu, PMCCNTR), cycles, "DP {dp:#x}");
        set(&mut cpu, (3, 3, 9, 12, 3), 1);
        nops(&mut cpu, 2);
        assert_eq!(get(&mut cpu, pmevcntr(1)), 4, "DP {dp:#x}: counting again");
    }
}

/// At EL2, `MDCR_EL2.HPMD` (FEAT_PMUv3p1) keeps the counters below
/// `HPMN` from counting and leaves those above it, and `MDCR_EL2.HCCD`
/// (FEAT_PMUv3p5) keeps the cycle counter from counting; each counts at
/// EL2 only with its `NSH` filter set.
#[test]
fn hpmd_and_hccd_keep_counters_off_at_el2() {
    let mut cpu = processor_with::<WithEl2>(Implementation::EVERY_FEATURE);
    set(&mut cpu, MDCR_EL2, 2 | HPME | HPMD | HCCD);
    set(&mut cpu, pmevtyper(0), INST_RETIRED | FILTER_NSH);
    set(&mut cpu, pmevtyper(1), INST_RETIRED);
    set(&mut cpu, pmevtyper(2), INST_RETIRED | FILTER_NSH);
    set(&mut cpu, PMCCFILTR, FILTER_NSH);
    for counter in 0..3 {
        set(&mut cpu, pmevcntr(counter), 0);
    }
    set(&mut cpu, PMCCNTR, 0);
    set(&mut cpu, PMCNTENSET, CYCLES | 0b111);
    set(&mut cpu, PMCR, PMCR_E);
    nops(&mut cpu, 5);
    assert_eq!(get(&mut cpu, pmevcntr(0)), 0, "HPMD");
    assert_eq!(get(&mut cpu, pmevcntr(1)), 0, "NSH clear");
    // Counter 2, enabled by `HPME` and its bit alone, counts from the
    // `MSR` that set its bit: that, the `MSR` to `PMCR_EL0`, five `NOP`s
    // and the two reads.
    assert_eq!(get(&mut cpu, pmevcntr(2)), 9, "reserved for EL2");
    assert_eq!(get(&mut cpu, PMCCNTR), 0, "HCCD");
    set(&mut cpu, MDCR_EL2, 2 | HPME);
    nops(&mut cpu, 5);
    // The `MSR` to `MDCR_EL2` and five `NOP`s; the cycle counter the read
    // of counter 0 too.
    assert_eq!(get(&mut cpu, pmevcntr(0)), 6);
    assert_eq!(get(&mut cpu, PMCCNTR), 7);
}

/// A snapshot holds the PMU: the processor restored from it reads the same
/// controls and counters, and counts on from them alike.
#[test]
fn the_pmu_survives_a_snapshot() {
    for imp in models() {
        let mut cpu = processor_with::<NoEl2>(imp);
        set(&mut cpu, pmevtyper(0), INST_RETIRED);
        set(&mut cpu, pmevcntr(0), 40);
        set(&mut cpu, PMCCNTR, 100);
        set(&mut cpu, PMINTENSET, 1);
        set(&mut cpu, PMCNTENSET, CYCLES | 1);
        set(&mut cpu, PMCR, PMCR_E);
        nops(&mut cpu, 5);
        let mut image = Vec::new();
        cpu.save(&mut image).expect("save");
        let mut restored = processor_with::<NoEl2>(imp);
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        // 40, and the `MSR` that started the counter, five `NOP`s and three
        // more.
        for c in [&mut cpu, &mut restored] {
            nops(c, 3);
            assert_eq!(get(c, pmevcntr(0)), 49, "{imp:?}");
        }
        for reg in [
            PMCR,
            PMCNTENSET,
            PMINTENSET,
            pmevtyper(0),
            pmevcntr(0),
            PMCCNTR,
        ] {
            let want = get(&mut cpu, reg);
            assert_eq!(get(&mut restored, reg), want, "{imp:?}: {reg:?}");
        }
    }
}
