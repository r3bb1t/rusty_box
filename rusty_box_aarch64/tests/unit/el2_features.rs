//! The EL2 features Arm requires of an Armv9.2 processor with EL2 beside
//! FEAT_VHE, as a guest sees them (R9), on the processor with every feature
//! and EL2, the one this crate builds with them: FEAT_FGT's fine-grained
//! traps of System registers, System instructions, `SVC` and `ERET`, and
//! `MDCR_EL2.TDCC`; FEAT_HCX's `HCRX_EL2`; FEAT_EVT's and FEAT_EVT2's
//! `HCR_EL2` traps; FEAT_XNX's stage 2 execute-never for EL1 and EL0 apart;
//! FEAT_S2FWB's `HCR_EL2.FWB`; and the ID registers that report them, which
//! the Cortex-A57 and a processor without EL2 leave as they were. Which
//! access traps where in every state of the controls is `sysreg_spec.rs`'s,
//! against Arm's accessors.
//!
//! The rules are the Arm ARM's (DDI 0487M.d, D24.2's `HFGRTR_EL2`,
//! `HFGWTR_EL2`, `HFGITR_EL2`, `HDFGRTR_EL2`, `HDFGWTR_EL2`, `HCRX_EL2`,
//! `HCR_EL2` and ID register descriptions) and its pseudocode:
//! `AArch64_CheckForSVCTrap`, `AArch64_CheckForERetTrap`,
//! `AArch64_S2ApplyOutputPerms`, `AArch64_S2DirectBasePermissions`,
//! `AArch64_S2ApplyFWBMemAttrs`, `AArch64_S2Translate`, `EncodePARAttrs`.
//!
//! The words are GNU as's (2.42, `-march=armv8.7-a`), each built here from
//! its fields and checked against the assembler's for one of each form.

use std::vec::Vec;

use super::el2_tests::{Machine, restored};
use super::features::{Feature, Implementation};
use super::{El, NoEl2, WithEl2, id};

/// Where the word under test executes, and EL2's vector base (`el2.rs`'s).
const CODE: u64 = 0x4_0000;
const VBAR_EL2: u64 = 0xA000;
const LOWER_A64: u64 = 0x400;

const SVC: u32 = 0xD400_00A1; // svc #0x5
const ERET: u32 = 0xD69F_03E0;
const ERETAA: u32 = 0xD69F_0BFF;
const ERETAB: u32 = 0xD69F_0FFF;
const NOP: u32 = 0xD503_201F;
const LDR: u32 = 0xF940_0022; // ldr x2, [x1]

/// Exception classes and the `ESR_ELx.IL` bit.
const EC_UNKNOWN: u64 = 0x00;
const EC_SVC: u64 = 0x15;
const EC_SYSREG: u64 = 0x18;
const EC_ERET: u64 = 0x1A;
const EC_INSTRUCTION_ABORT_LOWER_EL: u64 = 0x20;
const EC_DATA_ABORT_LOWER_EL: u64 = 0x24;
const IL: u64 = 1 << 25;

/// `MRS Xt, S<op0>_<op1>_C<crn>_C<crm>_<op2>`.
const fn mrs(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> u32 {
    0xD520_0000 | (op0 << 19) | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5) | rt
}

/// `MSR S<op0>_<op1>_C<crn>_C<crm>_<op2>, Xt`.
const fn msr(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> u32 {
    mrs(op0, op1, crn, crm, op2, rt) & !(1 << 21)
}

/// `SYS #<op1>, C<crn>, C<crm>, #<op2>, Xt`: a DC, IC, TLBI or AT.
const fn sys(op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> u32 {
    0xD508_0000 | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5) | rt
}

/// A register's `(op0, op1, CRn, CRm, op2)`.
type Encoding = (u32, u32, u32, u32, u32);

const HCR_EL2: Encoding = (3, 4, 1, 1, 0);
const MDCR_EL2: Encoding = (3, 4, 1, 1, 1);
const HFGRTR_EL2: Encoding = (3, 4, 1, 1, 4);
const HFGWTR_EL2: Encoding = (3, 4, 1, 1, 5);
const HFGITR_EL2: Encoding = (3, 4, 1, 1, 6);
const HDFGRTR_EL2: Encoding = (3, 4, 3, 1, 4);
const HDFGWTR_EL2: Encoding = (3, 4, 3, 1, 5);
const HCRX_EL2: Encoding = (3, 4, 1, 2, 2);
const VTTBR_EL2: Encoding = (3, 4, 2, 1, 0);
const VTCR_EL2: Encoding = (3, 4, 2, 1, 2);
const SPSR_EL2: Encoding = (3, 4, 4, 0, 0);
const ELR_EL2: Encoding = (3, 4, 4, 0, 1);
const SCTLR_EL1: Encoding = (3, 0, 1, 0, 0);
const TPIDR_EL0: Encoding = (3, 3, 13, 0, 2);
const TPIDR2_EL0: Encoding = (3, 3, 13, 0, 5);
const PAR_EL1: Encoding = (3, 0, 7, 4, 0);
const MDSCR_EL1: Encoding = (2, 0, 0, 2, 2);
const MDCCSR_EL0: Encoding = (2, 3, 0, 1, 0);
const PMCCNTR_EL0: Encoding = (3, 3, 9, 13, 0);
const CCSIDR_EL1: Encoding = (3, 1, 0, 0, 0);
const CLIDR_EL1: Encoding = (3, 1, 0, 0, 1);
const CSSELR_EL1: Encoding = (3, 2, 0, 0, 0);
const CTR_EL0: Encoding = (3, 3, 0, 0, 1);
const ID_AA64MMFR0_EL1: Encoding = (3, 0, 0, 7, 0);
const ID_AA64MMFR1_EL1: Encoding = (3, 0, 0, 7, 1);
const ID_AA64MMFR2_EL1: Encoding = (3, 0, 0, 7, 2);

/// `PMEVCNTR<n>_EL0`.
const fn pmevcntr(n: u32) -> Encoding {
    (3, 3, 14, 8 + n / 8, n % 8)
}

const fn mrs_of(r: Encoding) -> u32 {
    mrs(r.0, r.1, r.2, r.3, r.4, 0)
}

const fn msr_of(r: Encoding) -> u32 {
    msr(r.0, r.1, r.2, r.3, r.4, 0)
}

/// The System instructions this file runs, on X0 (X1 for `DC ZVA`).
const IC_IALLUIS: u32 = sys(0, 7, 1, 0, 31);
const IC_IALLU: u32 = sys(0, 7, 5, 0, 31);
const IC_IVAU: u32 = sys(3, 7, 5, 1, 0);
const DC_CVAU: u32 = sys(3, 7, 11, 1, 0);
const DC_CVAC: u32 = sys(3, 7, 10, 1, 0);
const DC_ZVA: u32 = sys(3, 7, 4, 1, 1);
const TLBI_VMALLE1: u32 = sys(0, 8, 7, 0, 31);
const TLBI_VMALLE1IS: u32 = sys(0, 8, 3, 0, 31);
const TLBI_VMALLE1OS: u32 = sys(0, 8, 1, 0, 31);
const TLBI_VMALLE1NXS: u32 = sys(0, 9, 7, 0, 31);
const TLBI_VMALLS12E1: u32 = sys(4, 8, 7, 6, 31);
const AT_S12E1R: u32 = sys(4, 7, 8, 4, 0);

/// The words are the assembler's: `mrs x0, sctlr_el1`, `msr hfgrtr_el2,
/// x0`, `tlbi vmalle1`, `tlbi vmalle1nxs`, `dc cvau, x0`, `ic ialluis`, `at
/// s12e1r, x0`, `eretaa` and `eretab`.
#[test]
fn the_words_are_the_assemblers() {
    assert_eq!(mrs_of(SCTLR_EL1), 0xD538_1000);
    assert_eq!(msr_of(HFGRTR_EL2), 0xD51C_1180);
    assert_eq!(TLBI_VMALLE1, 0xD508_871F);
    assert_eq!(TLBI_VMALLE1NXS, 0xD508_971F);
    assert_eq!(DC_CVAU, 0xD50B_7B20);
    assert_eq!(IC_IALLUIS, 0xD508_711F);
    assert_eq!(AT_S12E1R, 0xD50C_7880);
    assert_eq!((ERETAA, ERETAB), (0xD69F_0BFF, 0xD69F_0FFF));
}

/// A step of `insn` at [`CODE`] at the processor's level: `None` where it
/// completed, the level it took an exception to and the `ESR` there
/// otherwise.
fn step(m: &mut Machine<WithEl2>, insn: u32) -> Option<(El, u64)> {
    let taken = m.run_at(CODE, insn);
    if m.cpu.pc == CODE + 4 && taken.is_none() {
        return None;
    }
    let esr = match m.cpu.pstate.el {
        El::El2 => m.cpu.el2().esr,
        El::El1 => m.cpu.sys.esr_el1,
        El::El0 => panic!("{insn:#010x}: neither completed nor took an exception"),
    };
    Some((m.cpu.pstate.el, esr))
}

/// Step `insn` at `el`.
fn step_at(m: &mut Machine<WithEl2>, el: El, insn: u32) -> Option<(El, u64)> {
    m.at(el);
    step(m, insn)
}

/// `MSR` of `value` to `reg` at EL2, which completes.
fn write_el2(m: &mut Machine<WithEl2>, reg: Encoding, value: u64) {
    m.cpu.x[0] = value;
    assert_eq!(step_at(m, El::El2, msr_of(reg)), None, "{reg:?}");
}

/// `MRS` of `reg` at `el`, which completes: the value read.
fn read_at(m: &mut Machine<WithEl2>, el: El, reg: Encoding) -> u64 {
    assert_eq!(step_at(m, el, mrs_of(reg)), None, "{reg:?} at {el:?}");
    m.cpu.x[0]
}

/// The exception a trap to EL2 with class `ec` and syndrome `iss` takes.
const fn to_el2(ec: u64, iss: u64) -> Option<(El, u64)> {
    Some((El::El2, (ec << 26) | IL | iss))
}

/// `AArch64_SystemAccessTrapSyndrome` of the `MRS` (`read`) or `MSR` of
/// `reg`, or the `SYS` of its encoding, naming register `rt`.
const fn sysreg_iss(r: Encoding, rt: u64, read: bool) -> u64 {
    ((r.0 as u64) << 20)
        | ((r.4 as u64) << 17)
        | ((r.1 as u64) << 14)
        | ((r.2 as u64) << 10)
        | (rt << 5)
        | ((r.3 as u64) << 1)
        | read as u64
}

/// The trap to EL2 of the `MRS` of `reg` into X0.
const fn mrs_trap(r: Encoding) -> Option<(El, u64)> {
    to_el2(EC_SYSREG, sysreg_iss(r, 0, true))
}

/// The trap to EL2 of the `MSR` of `reg` from X0.
const fn msr_trap(r: Encoding) -> Option<(El, u64)> {
    to_el2(EC_SYSREG, sysreg_iss(r, 0, false))
}

/// The trap to EL2 of the System instruction `insn`.
const fn sys_trap(insn: u32) -> Option<(El, u64)> {
    let op1 = (insn >> 16) & 7;
    let crn = (insn >> 12) & 15;
    let crm = (insn >> 8) & 15;
    let op2 = (insn >> 5) & 7;
    let rt = (insn & 31) as u64;
    to_el2(EC_SYSREG, sysreg_iss((1, op1, crn, crm, op2), rt, false))
}

/// The processor with every feature and EL2, at EL2 out of reset.
fn every() -> Machine<WithEl2> {
    Machine::<WithEl2>::new(Implementation::EVERY_FEATURE)
}

// ----- the ID registers -----

/// The processor with every feature reports FEAT_FGT (`ID_AA64MMFR0_EL1.FGT`
/// 1), FEAT_XNX and FEAT_HCX (`ID_AA64MMFR1_EL1.XNX` and `HCX` 1), FEAT_S2FWB
/// (`ID_AA64MMFR2_EL1.FWB` 1) and FEAT_EVT2 (`EVT` 2) to EL1 with EL2, and
/// with FEAT_S2FWB a `CLIDR_EL1` with no level of cache to the Point of
/// Unification (`LoUU` and `LoUIS` 0); without EL2, none of them, and the
/// hierarchy's `CLIDR_EL1`. The Cortex-A57, an Armv8.0 processor, reports
/// none either way, its registers as before.
#[test]
fn the_id_registers_report_the_features_where_el2_brings_them() {
    let field = |value: u64, lsb: u32| (value >> lsb) & 0xF;
    let mut m = every();
    let (mmfr0, mmfr1, mmfr2) = (
        read_at(&mut m, El::El1, ID_AA64MMFR0_EL1),
        read_at(&mut m, El::El1, ID_AA64MMFR1_EL1),
        read_at(&mut m, El::El1, ID_AA64MMFR2_EL1),
    );
    assert_eq!(field(mmfr0, 56), 1, "FGT");
    assert_eq!((field(mmfr1, 28), field(mmfr1, 40)), (1, 1), "XNX, HCX");
    assert_eq!((field(mmfr2, 40), field(mmfr2, 56)), (1, 2), "FWB, EVT");
    let clidr = read_at(&mut m, El::El1, CLIDR_EL1);
    assert_eq!(clidr, 0x0200_0023, "LoUU and LoUIS 0, LoC 2");

    let without = Implementation::EVERY_FEATURE.with_levels::<NoEl2>().get();
    assert_eq!((id::id_aa64mmfr0(without.features) >> 56) & 0xF, 0);
    assert_eq!(
        id::id_aa64mmfr1(without.features) & ((0xF << 28) | (0xF << 40)),
        0
    );
    assert_eq!(
        id::id_aa64mmfr2(without.features) & ((0xF << 40) | (0xF << 56)),
        0
    );
    assert_eq!(id::clidr_el1(without.features), id::CLIDR_EL1);

    let mut a57 = Machine::<WithEl2>::new(Implementation::CORTEX_A57);
    let a57_ids = [
        read_at(&mut a57, El::El1, ID_AA64MMFR0_EL1),
        read_at(&mut a57, El::El1, ID_AA64MMFR1_EL1),
        read_at(&mut a57, El::El1, ID_AA64MMFR2_EL1),
        read_at(&mut a57, El::El1, CLIDR_EL1),
    ];
    assert_eq!(a57_ids, [0x24, 0, 0, 0x0A20_0023], "the Cortex-A57's");
    // Its EL2 has none of the registers.
    for reg in [
        HFGRTR_EL2,
        HFGWTR_EL2,
        HFGITR_EL2,
        HDFGRTR_EL2,
        HDFGWTR_EL2,
        HCRX_EL2,
    ] {
        assert_eq!(
            step_at(&mut a57, El::El2, mrs_of(reg)),
            Some((El::El2, (EC_UNKNOWN << 26) | IL)),
            "{reg:?} on the Cortex-A57"
        );
    }
}

// ----- FEAT_FGT -----

/// A fine-grained read trap: with `HFGRTR_EL2.SCTLR_EL1` set, EL1's `MRS` of
/// `SCTLR_EL1` is taken to EL2 with EC 0x18 and the `MRS`'s syndrome, and
/// its `MSR` is not; `HFGWTR_EL2.SCTLR_EL1` traps the `MSR` alone. EL2's
/// own access is never trapped.
#[test]
fn hfgrtr_and_hfgwtr_trap_an_el1_register_by_direction() {
    const SCTLR_FIELD: u64 = 1 << 29;
    let mut m = every();
    let sctlr = read_at(&mut m, El::El1, SCTLR_EL1);
    write_el2(&mut m, HFGRTR_EL2, SCTLR_FIELD);
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(SCTLR_EL1)),
        mrs_trap(SCTLR_EL1)
    );
    assert_eq!(m.cpu.el2().elr, CODE, "returning to the MRS");
    m.cpu.x[0] = sctlr;
    assert_eq!(step_at(&mut m, El::El1, msr_of(SCTLR_EL1)), None);

    write_el2(&mut m, HFGRTR_EL2, 0);
    write_el2(&mut m, HFGWTR_EL2, SCTLR_FIELD);
    assert_eq!(read_at(&mut m, El::El1, SCTLR_EL1), sctlr, "untrapped read");
    m.cpu.x[0] = sctlr;
    assert_eq!(
        step_at(&mut m, El::El1, msr_of(SCTLR_EL1)),
        msr_trap(SCTLR_EL1)
    );
    write_el2(&mut m, HFGRTR_EL2, SCTLR_FIELD);
    assert_eq!(read_at(&mut m, El::El2, SCTLR_EL1), sctlr, "EL2's own read");
}

/// `TPIDR_EL0`'s field traps EL0's read too, outside the host; in the host
/// (`HCR_EL2.{E2H, TGE}` {1, 1}) no fine-grained trap applies. The
/// negative fields trap where clear: `nTPIDR2_EL0` 0, as it resets, traps
/// EL1's `MRS` of `TPIDR2_EL0`, and 1 lets it through.
#[test]
fn fine_grained_traps_reach_el0_outside_the_host_and_negative_fields_trap_when_clear() {
    const TPIDR_EL0_FIELD: u64 = 1 << 35;
    const N_TPIDR2_EL0: u64 = 1 << 55;
    const E2H_TGE: u64 = (1 << 34) | (1 << 27);
    let mut m = every();
    write_el2(&mut m, HFGRTR_EL2, TPIDR_EL0_FIELD | N_TPIDR2_EL0);
    assert_eq!(
        step_at(&mut m, El::El0, mrs_of(TPIDR_EL0)),
        mrs_trap(TPIDR_EL0)
    );
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(TPIDR_EL0)),
        mrs_trap(TPIDR_EL0)
    );
    assert_eq!(step_at(&mut m, El::El1, mrs_of(TPIDR2_EL0)), None);
    write_el2(&mut m, HCR_EL2, E2H_TGE);
    assert_eq!(
        step_at(&mut m, El::El0, mrs_of(TPIDR_EL0)),
        None,
        "in the host"
    );
    write_el2(&mut m, HCR_EL2, 0);

    write_el2(&mut m, HFGRTR_EL2, 0);
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(TPIDR2_EL0)),
        mrs_trap(TPIDR2_EL0),
        "nTPIDR2_EL0 clear"
    );
}

/// The debug and PMU registers' traps (`HDFGRTR_EL2`, `HDFGWTR_EL2`) and
/// `MDCR_EL2.TDCC`, which traps the Debug Communications Channel's
/// registers at EL0 and EL1. With FEAT_FGT, an access to an event counter
/// `MDCR_EL2.HPMN` reserves for EL2 traps to EL2 (where the Cortex-A57's
/// PMU makes it RES0), and one to a counter the PMU does not implement is
/// UNDEFINED.
#[test]
fn hdfgrtr_hdfgwtr_and_tdcc_trap_the_debug_and_pmu_registers() {
    const MDSCR_FIELD: u64 = 1 << 4;
    const PMCCNTR_FIELD: u64 = 1 << 15;
    const TDCC: u64 = 1 << 27;
    let mut m = every();
    write_el2(&mut m, HDFGRTR_EL2, MDSCR_FIELD | PMCCNTR_FIELD);
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(MDSCR_EL1)),
        mrs_trap(MDSCR_EL1)
    );
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(PMCCNTR_EL0)),
        mrs_trap(PMCCNTR_EL0)
    );
    m.cpu.x[0] = 0;
    assert_eq!(
        step_at(&mut m, El::El1, msr_of(PMCCNTR_EL0)),
        None,
        "written"
    );
    write_el2(&mut m, HDFGWTR_EL2, PMCCNTR_FIELD);
    assert_eq!(
        step_at(&mut m, El::El1, msr_of(PMCCNTR_EL0)),
        msr_trap(PMCCNTR_EL0)
    );

    let hpmn = read_at(&mut m, El::El2, MDCR_EL2) & 0x1F;
    assert_eq!(hpmn, 6, "every counter EL1's out of reset");
    assert_eq!(step_at(&mut m, El::El1, mrs_of(MDCCSR_EL0)), None);
    write_el2(&mut m, MDCR_EL2, TDCC | 2);
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(MDCCSR_EL0)),
        mrs_trap(MDCCSR_EL0)
    );
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(pmevcntr(1))),
        None,
        "below HPMN"
    );
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(pmevcntr(2))),
        mrs_trap(pmevcntr(2))
    );
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(pmevcntr(6))),
        Some((El::El1, (EC_UNKNOWN << 26) | IL)),
        "no seventh counter"
    );

    let mut a57 = Machine::<WithEl2>::new(Implementation::CORTEX_A57);
    write_el2(&mut a57, MDCR_EL2, TDCC | 2);
    assert_eq!(
        step_at(&mut a57, El::El1, mrs_of(MDCCSR_EL0)),
        None,
        "no TDCC"
    );
    assert_eq!(
        read_at(&mut a57, El::El1, pmevcntr(2)),
        0,
        "RES0 on the Cortex-A57"
    );
}

/// `HFGITR_EL2` traps System instructions at EL1, and at EL0 outside the
/// host: `TLBI VMALLE1`, `DC ZVA` (EL0's under `SCTLR_EL1.DZE`), and the
/// nXS form of a TLBI by the plain form's field unless `HCRX_EL2.FGTnXS`
/// exempts it (FEAT_HCX).
#[test]
fn hfgitr_traps_system_instructions_and_fgtnxs_exempts_the_nxs_forms() {
    const TLBIVMALLE1: u64 = 1 << 42;
    const DCZVA: u64 = 1 << 11;
    const FGTNXS: u64 = 1 << 4;
    const DZE: u64 = 1 << 14;
    let mut m = every();
    assert_eq!(step_at(&mut m, El::El1, TLBI_VMALLE1), None);
    write_el2(&mut m, HFGITR_EL2, TLBIVMALLE1 | DCZVA);
    assert_eq!(
        step_at(&mut m, El::El1, TLBI_VMALLE1),
        sys_trap(TLBI_VMALLE1)
    );
    assert_eq!(
        step_at(&mut m, El::El1, TLBI_VMALLE1NXS),
        sys_trap(TLBI_VMALLE1NXS)
    );
    assert_eq!(step_at(&mut m, El::El2, TLBI_VMALLE1), None, "EL2's");
    m.cpu.sys.sctlr_el1 |= DZE;
    m.cpu.x[1] = 0x20_0000;
    assert_eq!(step_at(&mut m, El::El0, DC_ZVA), sys_trap(DC_ZVA));

    write_el2(&mut m, HCRX_EL2, FGTNXS);
    assert_eq!(read_at(&mut m, El::El2, HCRX_EL2), FGTNXS);
    assert_eq!(step_at(&mut m, El::El1, TLBI_VMALLE1NXS), None, "FGTnXS");
    assert_eq!(
        step_at(&mut m, El::El1, TLBI_VMALLE1),
        sys_trap(TLBI_VMALLE1)
    );
}

/// `HFGITR_EL2.SVC_EL1` and `SVC_EL0` take an `SVC` to EL2 with its
/// immediate, returning to the `SVC` itself (`AArch64_CheckForSVCTrap`),
/// where without them it is a call to EL1 returning past it; EL0's in the
/// host is the host's own call, to EL2 past the `SVC`.
#[test]
fn hfgitr_traps_svc_to_el2_returning_to_it() {
    const SVC_EL0: u64 = 1 << 52;
    const SVC_EL1: u64 = 1 << 53;
    const E2H_TGE: u64 = (1 << 34) | (1 << 27);
    let svc = to_el2(EC_SVC, 5);
    let mut m = every();
    m.at(El::El1);
    assert_eq!(m.run_at(CODE, SVC).map(|t| t.to), Some(El::El1));
    assert_eq!(m.cpu.sys.elr_el1, CODE + 4, "the call");

    write_el2(&mut m, HFGITR_EL2, SVC_EL1);
    assert_eq!(step_at(&mut m, El::El1, SVC), svc);
    assert_eq!(m.cpu.pc, VBAR_EL2 + LOWER_A64);
    assert_eq!(m.cpu.el2().elr, CODE, "returning to the SVC");
    assert_eq!(
        step_at(&mut m, El::El0, SVC).map(|(el, _)| el),
        Some(El::El1)
    );

    write_el2(&mut m, HFGITR_EL2, SVC_EL0);
    assert_eq!(step_at(&mut m, El::El0, SVC), svc);
    assert_eq!(m.cpu.el2().elr, CODE);
    write_el2(&mut m, HCR_EL2, E2H_TGE);
    assert_eq!(step_at(&mut m, El::El0, SVC), svc);
    assert_eq!(m.cpu.el2().elr, CODE + 4, "the host's call");
}

/// `HFGITR_EL2.ERET` takes EL1's `ERET`, `ERETAA` and `ERETAB` to EL2
/// with EC 0x1A, `ISS[1]` set for the authenticated ones and `ISS[0]` for
/// key B, returning to the instruction, before `ELR_EL1` is read; EL2's own
/// `ERET` is not trapped.
#[test]
fn hfgitr_traps_el1s_exception_returns() {
    const ERET_FIELD: u64 = 1 << 51;
    let mut m = every();
    write_el2(&mut m, HFGITR_EL2, ERET_FIELD);
    for (insn, iss) in [(ERET, 0), (ERETAA, 0b10), (ERETAB, 0b11)] {
        assert_eq!(
            step_at(&mut m, El::El1, insn),
            to_el2(EC_ERET, iss),
            "{insn:#010x}"
        );
        assert_eq!(m.cpu.el2().elr, CODE);
    }
    // At EL2 the return completes, to EL0t at `ELR_EL2`.
    write_el2(&mut m, SPSR_EL2, 0);
    write_el2(&mut m, ELR_EL2, CODE);
    m.at(El::El2);
    assert_eq!(m.run_at(CODE, ERET), None);
    assert_eq!((m.cpu.pstate.el, m.cpu.pc), (El::El0, CODE));
}

// ----- FEAT_EVT and FEAT_EVT2 -----

/// `HCR_EL2.TID4` traps EL1's accesses to `CCSIDR_EL1`, `CLIDR_EL1` and
/// `CSSELR_EL1`, not `CTR_EL0`, which `TID2` traps with them; `TICAB` traps
/// `IC IALLUIS`; `TOCU` the maintenance to the Point of Unification, `IC
/// IALLU`, `IC IVAU` and `DC CVAU`, not `DC CVAC`; `TTLBIS` the Inner
/// Shareable TLB maintenance and `TTLBOS` the Outer Shareable.
#[test]
fn the_enhanced_virtualization_traps() {
    const TID4: u64 = 1 << 49;
    const TICAB: u64 = 1 << 50;
    const TOCU: u64 = 1 << 52;
    const TTLBIS: u64 = 1 << 54;
    const TTLBOS: u64 = 1 << 55;
    let mut m = every();
    m.cpu.x[0] = 0x20_0000;
    write_el2(&mut m, HCR_EL2, TID4);
    for reg in [CCSIDR_EL1, CLIDR_EL1, CSSELR_EL1] {
        assert_eq!(
            step_at(&mut m, El::El1, mrs_of(reg)),
            mrs_trap(reg),
            "{reg:?}"
        );
    }
    assert_eq!(step_at(&mut m, El::El1, mrs_of(CTR_EL0)), None, "CTR_EL0");

    let cases: [(u64, u32, bool); 9] = [
        (TICAB, IC_IALLUIS, true),
        (TICAB, IC_IALLU, false),
        (TOCU, IC_IALLU, true),
        (TOCU, IC_IVAU, true),
        (TOCU, DC_CVAU, true),
        (TOCU, DC_CVAC, false),
        (TTLBIS, TLBI_VMALLE1IS, true),
        (TTLBIS, TLBI_VMALLE1, false),
        (TTLBOS, TLBI_VMALLE1OS, true),
    ];
    for (control, insn, traps) in cases {
        write_el2(&mut m, HCR_EL2, control);
        m.cpu.x[0] = 0x20_0000;
        let want = if traps { sys_trap(insn) } else { None };
        assert_eq!(
            step_at(&mut m, El::El1, insn),
            want,
            "{control:#x}, {insn:#010x}"
        );
    }
    write_el2(&mut m, HCR_EL2, TTLBOS);
    assert_eq!(step_at(&mut m, El::El1, TLBI_VMALLE1IS), None, "TTLBOS");
}

// ----- FEAT_HCX -----

/// `HCRX_EL2` keeps the fields of the features this processor has —
/// `FnXS` and `FGTnXS` (FEAT_XS), and `FNB` — and nothing else: `SMPME` is
/// RES0 while `SMIDR_EL1.SMPS` is 0, and the fields of FEAT_MOPS,
/// FEAT_SCTLR2, FEAT_TCR2, FEAT_NMI and the rest, which it lacks, are RES0.
/// It resets to 0.
#[test]
fn hcrx_el2_keeps_the_controls_of_the_features_the_processor_has() {
    const FNXS: u64 = 1 << 3;
    const FGTNXS: u64 = 1 << 4;
    const FNB: u64 = 1 << 36;
    let mut m = every();
    assert_eq!(read_at(&mut m, El::El2, HCRX_EL2), 0);
    write_el2(&mut m, HCRX_EL2, u64::MAX);
    assert_eq!(read_at(&mut m, El::El2, HCRX_EL2), FNXS | FGTNXS | FNB);
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(HCRX_EL2)),
        Some((El::El1, (EC_UNKNOWN << 26) | IL)),
        "EL1 has no HCRX_EL2"
    );
}

// ----- FEAT_XNX -----

/// Stage 2's level 1 table: its first entry maps the first GiB of IPAs to
/// itself.
const S2_L1: u64 = 0x1_4000;
const S2_AF: u64 = 1 << 10;
const S2_ISH: u64 = 3 << 8;
const S2AP_RW: u64 = (1 << 6) | (1 << 7);
const S2_BLOCK: u64 = 0b01;
/// `VTCR_EL2`: T0SZ 25 (a 39-bit IPA), SL0 1 (a start at level 1), PS 40
/// bits, the 4 KiB granule.
const VTCR_IDENTITY: u64 = 25 | (1 << 6) | (0b010 << 16);
const HCR_VM: u64 = 1 << 0;
const HCR_DC: u64 = 1 << 12;
const HCR_FWB: u64 = 1 << 46;
/// A stage 2 descriptor's `MemAttr` (bits 5:2).
const fn memattr(attr: u64) -> u64 {
    attr << 2
}

/// The processor `imp` with EL2 and stage 2 on (`HCR_EL2.VM`, and the
/// `HCR_EL2` bits `hcr` besides), its EL1&0 stage 1 off, the first GiB of
/// IPAs mapped to itself by one level 1 block with the stage 2 attributes
/// `attrs` beside read, write and the Access flag, each written as a
/// hypervisor writes it.
fn stage2_on(imp: Implementation, hcr: u64, attrs: u64) -> Machine<WithEl2> {
    let mut m = Machine::<WithEl2>::new(imp);
    m.ram
        .put64(S2_L1, S2_AF | S2_ISH | S2AP_RW | S2_BLOCK | attrs);
    write_el2(&mut m, VTCR_EL2, VTCR_IDENTITY);
    write_el2(&mut m, VTTBR_EL2, S2_L1);
    write_el2(&mut m, HCR_EL2, HCR_VM | hcr);
    m
}

/// A stage 2 Permission fault at level 1 on an instruction fetch from EL0
/// or EL1, taken to EL2.
const STAGE2_FETCH_REFUSED: Option<(El, u64)> = to_el2(EC_INSTRUCTION_ABORT_LOWER_EL, 0b00_1101);

/// FEAT_XNX gives stage 2's execute-never two bits, `XN[1:0]` (descriptor
/// bits 54:53): 0b00 executable at EL1 and EL0, 0b01 at EL0 alone, 0b10 at
/// neither, 0b11 at EL1 alone, each refusal a stage 2 Permission fault
/// taken to EL2. The Cortex-A57, without it, reads bit 54 alone.
#[test]
fn xnx_gives_el1_and_el0_their_own_stage_2_execute_never() {
    let xn = |bits: u64| bits << 53;
    for (bits, el1, el0) in [
        (0b00, true, true),
        (0b01, false, true),
        (0b10, false, false),
        (0b11, true, false),
    ] {
        let s2 = memattr(0b1111) | xn(bits);
        let mut m = stage2_on(Implementation::EVERY_FEATURE, 0, s2);
        let outcome = |runs: bool| if runs { None } else { STAGE2_FETCH_REFUSED };
        assert_eq!(
            step_at(&mut m, El::El1, NOP),
            outcome(el1),
            "XN {bits:#04b} at EL1"
        );
        assert_eq!(
            step_at(&mut m, El::El0, NOP),
            outcome(el0),
            "XN {bits:#04b} at EL0"
        );
        // Each level's permission comes from one walk: the TLB holds both.
        assert_eq!(
            step_at(&mut m, El::El1, NOP),
            outcome(el1),
            "XN {bits:#04b}, held"
        );
    }
    let mut a57 = stage2_on(Implementation::CORTEX_A57, 0, memattr(0b1111) | xn(0b01));
    assert_eq!(step_at(&mut a57, El::El1, NOP), None, "bit 53 RES0 there");
    assert_eq!(step_at(&mut a57, El::El0, NOP), None);
}

// ----- FEAT_S2FWB -----

/// The `PAR_EL1` an `AT S12E1R` of `va` at EL2 leaves: its `ATTR` (bits
/// 63:56) and `SH` (8:7), or `None` where it reports a fault.
fn par_after_at(m: &mut Machine<WithEl2>, va: u64) -> Option<(u64, u64)> {
    m.cpu.x[0] = va;
    assert_eq!(step_at(m, El::El2, AT_S12E1R), None);
    let par = read_at(m, El::El2, PAR_EL1);
    (par & 1 == 0).then_some((par >> 56, (par >> 7) & 3))
}

/// Under `HCR_EL2.FWB` stage 2's `MemAttr` makes the access's attributes
/// from stage 1's (`AArch64_S2ApplyFWBMemAttrs`), as `AT S12E1R` reports
/// them in `PAR_EL1`: with stage 1 off, its Device-nGnRnE memory, 0b0110
/// forces Normal Write-Back, read- and write-allocate (0xFF), where without
/// `FWB` the same encoding, Normal Write-Through inner and Non-cacheable
/// outer at stage 2, leaves stage 1's Device memory (0x00); 0b0111 is
/// stage 1's attributes; 0b0101 Normal Non-cacheable, but stage 1's Device
/// memory stays Device; and with `MemAttr[2]` clear Device, the more
/// restrictive of the stages' types. Shareability is the wider of the
/// stages', Device memory's Outer.
#[test]
fn fwb_forces_write_back_where_stage_2_says() {
    const VA: u64 = 0x20_0000;
    let attr_of = |hcr: u64, s2: u64| {
        let mut m = stage2_on(Implementation::EVERY_FEATURE, hcr, memattr(s2));
        par_after_at(&mut m, VA)
    };
    assert_eq!(attr_of(0, 0b0110), Some((0x00, 0b10)), "Device by stage 1");
    assert_eq!(
        attr_of(HCR_FWB, 0b0110),
        Some((0xFF, 0b10)),
        "forced Write-Back"
    );
    assert_eq!(attr_of(HCR_FWB, 0b0111), Some((0x00, 0b10)), "stage 1's");
    assert_eq!(
        attr_of(HCR_FWB, 0b0101),
        Some((0x00, 0b10)),
        "stage 1's Device"
    );
    assert_eq!(
        attr_of(HCR_FWB, 0b0010),
        Some((0x00, 0b10)),
        "nGnRnE, the stronger"
    );
    // With `HCR_EL2.DC` stage 1's data accesses are to Normal Write-Back
    // Non-shareable memory, whose `XS` is 0 (FEAT_XS): the forced
    // Non-cacheable and Device memory keep stage 1's `XS`, which `PAR_EL1`
    // reports in FEAT_XS's encodings, Normal Non-cacheable 0x40 and
    // Device-nGRE 0x09.
    assert_eq!(
        attr_of(HCR_DC | HCR_FWB, 0b0111),
        Some((0xFF, 0b11)),
        "stage 1's, Inner"
    );
    assert_eq!(
        attr_of(HCR_DC | HCR_FWB, 0b0101),
        Some((0x40, 0b10)),
        "Non-cacheable"
    );
    assert_eq!(
        attr_of(HCR_DC | HCR_FWB, 0b1010),
        Some((0x09, 0b10)),
        "Device-nGRE"
    );
    assert_eq!(
        attr_of(HCR_DC, 0b0101),
        Some((0x40, 0b10)),
        "combined, Non-cacheable"
    );
    // `HCR_EL2.FWB` on a processor without FEAT_S2FWB is RES0.
    let mut a57 = stage2_on(Implementation::CORTEX_A57, HCR_FWB, memattr(0b0110));
    assert_eq!(read_at(&mut a57, El::El2, HCR_EL2) & HCR_FWB, 0);
    assert_eq!(par_after_at(&mut a57, VA), Some((0x00, 0b10)));
}

/// Under `FWB` a `MemAttr` whose bit 2 is clear is Device memory, which a
/// misaligned load refuses at stage 2 (`AArch64_S2HasAlignmentFaultDueToMemType`),
/// where without `FWB` the same encoding, 0b1001, is Normal memory a
/// misaligned load reaches. A hypervisor's `MSR` of `HCR_EL2` that sets
/// `FWB` makes the next access find stage 2's memory anew, without TLB
/// maintenance; the TLB's entries round-trip through a snapshot.
#[test]
fn fwb_makes_device_memory_of_an_encoding_that_is_normal_without() {
    const MISALIGNED: u64 = 0x20_0001;
    const ALIGNMENT_FAULT: u64 = 0b10_0001;
    let mut m = stage2_on(Implementation::EVERY_FEATURE, HCR_DC, memattr(0b1001));
    m.cpu.x[1] = MISALIGNED;
    assert_eq!(step_at(&mut m, El::El1, LDR), None, "Normal memory");
    let image = m.saved_tlb().0;
    assert_eq!(
        restored::<WithEl2>(&m.tlb_context(), &image),
        Ok(image.clone())
    );

    write_el2(&mut m, HCR_EL2, HCR_VM | HCR_DC | HCR_FWB);
    m.cpu.x[1] = MISALIGNED;
    let taken = step_at(&mut m, El::El1, LDR);
    let (el, esr) = taken.expect("a stage 2 alignment fault");
    assert_eq!(
        (el, esr >> 26, esr & 0x3F),
        (El::El2, EC_DATA_ABORT_LOWER_EL, ALIGNMENT_FAULT)
    );
    m.cpu.x[1] = MISALIGNED & !7;
    assert_eq!(step_at(&mut m, El::El1, LDR), None, "an aligned load");
    let image = m.saved_tlb().0;
    assert_eq!(
        restored::<WithEl2>(&m.tlb_context(), &image),
        Ok(image.clone())
    );
}

// ----- the registers in a snapshot -----

/// The fine-grained trap registers and `HCRX_EL2` round-trip through a
/// snapshot of the processor, their traps acting after the restore; one
/// holding a bit no `MSR` keeps is refused.
#[test]
fn a_snapshot_keeps_the_trap_registers() {
    const SCTLR_FIELD: u64 = 1 << 29;
    const FGTNXS: u64 = 1 << 4;
    let mut m = every();
    write_el2(&mut m, HFGRTR_EL2, SCTLR_FIELD);
    write_el2(&mut m, HFGITR_EL2, 1 << 51);
    write_el2(&mut m, HDFGWTR_EL2, 1 << 8);
    write_el2(&mut m, HCRX_EL2, FGTNXS);
    let mut image = Vec::new();
    m.cpu.save(&mut image).expect("save");
    let mut back = every();
    assert_eq!(back.cpu.restore(&mut image.as_slice()), Ok(()));
    assert_eq!(back.cpu.el2(), m.cpu.el2());
    assert_eq!(
        step_at(&mut back, El::El1, mrs_of(SCTLR_EL1)),
        mrs_trap(SCTLR_EL1)
    );

    let mut torn = every();
    torn.cpu.el2_state.regs.hcrx = 1 << 5;
    let mut image = Vec::new();
    torn.cpu.save(&mut image).expect("save");
    assert!(every().cpu.restore(&mut image.as_slice()).is_err(), "SMPME");
    let features = Implementation::EVERY_FEATURE
        .with_levels::<WithEl2>()
        .features;
    assert!(features.has(Feature::Fgt) && features.has(Feature::Hcx));
}

// ----- the stage 2 TLB maintenance the tests above rely on -----

/// `TLBI VMALLS12E1` at EL2 is untouched by every trap above: they trap
/// EL1 and EL0 alone.
#[test]
fn el2s_tlb_maintenance_is_never_trapped() {
    let mut m = every();
    write_el2(&mut m, HFGITR_EL2, u64::MAX);
    write_el2(&mut m, HCR_EL2, (1 << 54) | (1 << 55) | (1 << 25));
    assert_eq!(step_at(&mut m, El::El2, TLBI_VMALLS12E1), None);
    assert_eq!(step_at(&mut m, El::El2, TLBI_VMALLE1IS), None);
}
