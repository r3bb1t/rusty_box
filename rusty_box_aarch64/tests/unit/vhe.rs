//! FEAT_VHE as a guest sees it (R9), on the processor with every feature
//! and EL2, the one this crate builds with FEAT_VHE: a host at EL2 under
//! `HCR_EL2.E2H` and `TGE` running EL0 in the EL2&0 regime through
//! `TTBR0_EL2` and `TTBR1_EL2` with ASIDs; the EL1 register names reaching
//! EL2's at EL2 and the `*_EL12` and `*_EL02` names reaching EL1's and
//! EL0's; the TLB maintenance instructions under `E2H` (`TLBIMatch` with
//! `AArch64_TLBI_ASID`, `AArch64_TLBI_VA`, `AArch64_TLBI_VMALL`); the EL2
//! virtual timer; exceptions to EL2 from EL0 in the host, PAN at EL2, and
//! unprivileged accesses there; and the Cortex-A57, which has no FEAT_VHE,
//! keeping none of it. Which accesses trap where is `sysreg_spec.rs`'s,
//! against Arm's accessors.
//!
//! The rules are the Arm ARM's (D8.13) and its pseudocode: `ELIsInHost`,
//! `TranslationRegime`, `AArch64_S1TTWParamsEL20`, `AArch64_TLBContextEL20`,
//! `AArch64_S1DirectBasePermissions`, `AArch64_IsUnprivAccessPriv`,
//! `AArch64_TakeException`, `AArch64_CheckTimerConditions` and
//! `VirtualCounterTimer`.
//!
//! The words are GNU as's (2.42, `-march=armv8.2-a`), each built here from
//! its fields (`mrs`, `msr`, `sys`) and checked against the assembler's for
//! one register of each form.

use std::vec::Vec;

use super::el2::{HCR_E2H, HCR_TGE};
use super::el2_tests::{restored, Machine};
use super::features::Implementation;
use super::mmu::saved_layout::NO_CONTEXT;
use super::mmu::{El2RegimeContext, TlbContext};
use super::{id, El, WithEl2};

const CODE: u64 = 0x4_0000;
const VBAR_EL2: u64 = 0xA000;
const LOWER_A64: u64 = 0x400;
const CURRENT_SPX: u64 = 0x200;

const ERET: u32 = 0xD69F_03E0;
const SVC: u32 = 0xD400_00A1; // svc #0x5
const LDR: u32 = 0xF940_0022; // ldr x2, [x1]
const LDTR: u32 = 0xF840_0822; // ldtr x2, [x1]

/// `MRS Xt, S<op0>_<op1>_C<crn>_C<crm>_<op2>`.
const fn mrs(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> u32 {
    0xD520_0000 | (op0 << 19) | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5) | rt
}

/// `MSR S<op0>_<op1>_C<crn>_C<crm>_<op2>, Xt`.
const fn msr(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> u32 {
    mrs(op0, op1, crn, crm, op2, rt) & !(1 << 21)
}

/// `SYS #<op1>, C<crn>, C<crm>, #<op2>, Xt`: a TLBI or an AT.
const fn sys(op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> u32 {
    0xD508_0000 | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5) | rt
}

/// The registers this file names, as `(op0, op1, CRn, CRm, op2)`.
const HCR_EL2: (u32, u32, u32, u32, u32) = (3, 4, 1, 1, 0);
const SCTLR_EL1: (u32, u32, u32, u32, u32) = (3, 0, 1, 0, 0);
const SCTLR_EL12: (u32, u32, u32, u32, u32) = (3, 5, 1, 0, 0);
const TTBR0_EL1: (u32, u32, u32, u32, u32) = (3, 0, 2, 0, 0);
const TTBR1_EL1: (u32, u32, u32, u32, u32) = (3, 0, 2, 0, 1);
const TCR_EL1: (u32, u32, u32, u32, u32) = (3, 0, 2, 0, 2);
const TCR_EL12: (u32, u32, u32, u32, u32) = (3, 5, 2, 0, 2);
const SPSR_EL1: (u32, u32, u32, u32, u32) = (3, 0, 4, 0, 0);
const ELR_EL1: (u32, u32, u32, u32, u32) = (3, 0, 4, 0, 1);
const ELR_EL12: (u32, u32, u32, u32, u32) = (3, 5, 4, 0, 1);
const MAIR_EL1: (u32, u32, u32, u32, u32) = (3, 0, 10, 2, 0);
const CNTKCTL_EL1: (u32, u32, u32, u32, u32) = (3, 0, 14, 1, 0);
const CNTKCTL_EL12: (u32, u32, u32, u32, u32) = (3, 5, 14, 1, 0);
const CNTVCT_EL0: (u32, u32, u32, u32, u32) = (3, 3, 14, 0, 2);
const CNTV_CTL_EL0: (u32, u32, u32, u32, u32) = (3, 3, 14, 3, 1);
const CNTV_CVAL_EL0: (u32, u32, u32, u32, u32) = (3, 3, 14, 3, 2);
const CNTV_CTL_EL02: (u32, u32, u32, u32, u32) = (3, 5, 14, 3, 1);
const ID_AA64MMFR1_EL1: (u32, u32, u32, u32, u32) = (3, 0, 0, 7, 1);

const fn mrs_of(r: (u32, u32, u32, u32, u32)) -> u32 {
    mrs(r.0, r.1, r.2, r.3, r.4, 0)
}

const fn msr_of(r: (u32, u32, u32, u32, u32)) -> u32 {
    msr(r.0, r.1, r.2, r.3, r.4, 0)
}

/// The words are the assembler's: `mrs x0, midr_el1`, `msr smcr_el2, x1`,
/// `tlbi alle2`, `at s1e2r, x0`, `msr sctlr_el12, x0` and `tlbi aside1, x0`.
#[test]
fn the_words_are_the_assemblers() {
    assert_eq!(mrs(3, 0, 0, 0, 0, 0), 0xD538_0000);
    assert_eq!(msr(3, 4, 1, 2, 6, 1), 0xD51C_12C1);
    assert_eq!(sys(4, 8, 7, 0, 31), 0xD50C_871F);
    assert_eq!(sys(4, 7, 8, 0, 0), 0xD50C_7800);
    assert_eq!(msr_of(SCTLR_EL12), 0xD51D_1000);
    assert_eq!(sys(0, 8, 7, 2, 0), 0xD508_8740);
}

/// The TLBI and AT words this file runs, on X0.
const TLBI_VMALLE1: u32 = sys(0, 8, 7, 0, 0);
const TLBI_ASIDE1: u32 = sys(0, 8, 7, 2, 0);
const TLBI_VAE2: u32 = sys(4, 8, 7, 1, 0);
const AT_S1E0R: u32 = sys(0, 7, 8, 2, 0);
const AT_S1E2R: u32 = sys(4, 7, 8, 0, 0);

/// The host's translation tables, 4 KiB granule and 39-bit addresses in
/// each range (`T0SZ` and `T1SZ` 25), 2 MiB blocks: the lower range
/// (`TTBR0_EL2`) maps the code read-only to EL0 and EL2, which both
/// execute it, and `SHARED` to `DATA_A`, EL0's to read and write and
/// non-global (`nG`); the upper range (`TTBR1_EL2`) maps `UPPER` to
/// `DATA_HI`, EL2's alone and global. Each data address is a 2 MiB
/// block's.
const L1_LO: u64 = 0x1_0000;
const L2_LO: u64 = 0x1_1000;
const L1_HI: u64 = 0x1_2000;
const L2_HI: u64 = 0x1_3000;
const SHARED: u64 = 0x20_0000;
const UPPER: u64 = 0xFFFF_FF80_0020_0000;
const DATA_A: u64 = 0x40_0000;
const DATA_HI: u64 = 0x60_0000;
const DATA_B: u64 = 0x80_0000;
const DATA_HI2: u64 = 0xA0_0000;
const AF: u64 = 1 << 10;
const ISH: u64 = 3 << 8;
const BLOCK: u64 = 0b01;
const TABLE: u64 = 0b11;
const AP1: u64 = 1 << 6;
const AP2: u64 = 1 << 7;
const NG: u64 = 1 << 11;
/// The host process's ASID, and one it is not.
const ASID: u64 = 0x42;
const OTHER_ASID: u64 = 0x43;
/// `TCR_EL2` with `E2H` 1, `TCR_EL1`'s layout: `T0SZ` and `T1SZ` 25, `TG1`
/// 4 KiB, `IPS` 40 bits, `AS` (16-bit ASIDs), `A1` clear: the ASID is
/// `TTBR0_EL2`'s.
const TCR_HOST: u64 = 25 | (25 << 16) | (0b10 << 30) | (0b010 << 32) | (1 << 36);
/// `SPSR` of EL0t with every interrupt masked.
const SPSR_EL0T: u64 = 0b1111 << 6;
/// `SCTLR_EL2.SPAN` and `M`.
const SPAN: u64 = 1 << 23;
const SCTLR_M: u64 = 1;

/// Data abort classes, and the fault status of a level 2 permission fault.
const EC_DATA_ABORT_LOWER_EL: u64 = 0x24;
const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
const EC_SVC: u64 = 0x15;
const EC_SYSREG: u64 = 0x18;
const EC_UNKNOWN: u64 = 0x00;
const PERMISSION_L2: u64 = 0b00_1110;

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

/// Step `insn` at `el`, expecting it to complete.
fn completes(m: &mut Machine<WithEl2>, el: El, insn: u32) {
    m.at(el);
    assert_eq!(step(m, insn), None, "{insn:#010x} at {el:?}");
}

/// `MSR` of `value` to `reg` at EL2.
fn write_el2(m: &mut Machine<WithEl2>, reg: (u32, u32, u32, u32, u32), value: u64) {
    m.cpu.x[0] = value;
    completes(m, El::El2, msr_of(reg));
}

/// `MRS` of `reg` at `el`.
fn read_at(m: &mut Machine<WithEl2>, el: El, reg: (u32, u32, u32, u32, u32)) -> u64 {
    completes(m, el, mrs_of(reg));
    m.cpu.x[0]
}

/// `LDR` of the doubleword at `va` at `el`: its value, or the level the
/// fault was taken to and its `ESR`.
fn load(m: &mut Machine<WithEl2>, el: El, insn: u32, va: u64) -> Result<u64, (El, u64)> {
    m.at(el);
    m.cpu.x[1] = va;
    match step(m, insn) {
        None => Ok(m.cpu.x[2]),
        Some(taken) => Err(taken),
    }
}

/// The processor with every feature, at EL2 in the host: `HCR_EL2.E2H` and
/// `TGE` set and its MMU on over the host's tables, every register written
/// by its EL1 name, which at EL2 under `E2H` reaches EL2's.
fn host() -> Machine<WithEl2> {
    let mut m = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    let attrs = AF | ISH | BLOCK;
    m.ram.put64(L1_LO, L2_LO | TABLE);
    m.ram.put64(L2_LO, attrs | AP2 | AP1);
    m.ram.put64(L2_LO + 8, DATA_A | attrs | AP1 | NG);
    m.ram.put64(L1_HI, L2_HI | TABLE);
    m.ram.put64(L2_HI + 8, DATA_HI | attrs);
    m.ram.put64(DATA_A, 0xA0A0);
    m.ram.put64(DATA_B, 0xB0B0);
    m.ram.put64(DATA_HI, 0x1111);
    m.ram.put64(DATA_HI2, 0x2222);
    write_el2(&mut m, HCR_EL2, HCR_E2H | HCR_TGE);
    write_el2(&mut m, TCR_EL1, TCR_HOST);
    write_el2(&mut m, TTBR0_EL1, L1_LO | (ASID << 48));
    write_el2(&mut m, TTBR1_EL1, L1_HI);
    write_el2(&mut m, MAIR_EL1, 0xFF);
    let sctlr = m.cpu.el2().sctlr | SCTLR_M;
    write_el2(&mut m, SCTLR_EL1, sctlr);
    m
}

/// At EL2 under `HCR_EL2.E2H` the EL1 register names reach the EL2
/// registers (Arm ARM D8.13.2), leaving EL1's as they were, and the
/// `*_EL12` names reach EL1's (D8.13.3), leaving EL2's.
#[test]
fn el1_names_at_el2_reach_el2s_registers_and_el12_names_el1s() {
    let mut m = host();
    let el1 = m.cpu.sys.clone();
    let el2 = m.cpu.el2();
    assert_eq!(el2.tcr, TCR_HOST, "TCR_EL1 named TCR_EL2");
    assert_eq!(el2.ttbr0, L1_LO | (ASID << 48), "TTBR0_EL1 named TTBR0_EL2");
    assert_eq!(el2.ttbr1, L1_HI, "TTBR1_EL1 named TTBR1_EL2");
    assert_eq!(el2.mair, 0xFF, "MAIR_EL1 named MAIR_EL2");
    assert_eq!(el2.sctlr & SCTLR_M, SCTLR_M, "SCTLR_EL1 named SCTLR_EL2");
    assert_eq!(read_at(&mut m, El::El2, TCR_EL1), TCR_HOST);
    assert_eq!(
        (m.cpu.sys.tcr_el1, m.cpu.sys.ttbr0_el1, m.cpu.sys.mair_el1),
        (el1.tcr_el1, el1.ttbr0_el1, el1.mair_el1),
        "EL1's own registers are as they were"
    );

    // The aliases reach EL1's.
    let sctlr_el1 = id::SCTLR_EL1_RESET ^ (1 << 12);
    write_el2(&mut m, SCTLR_EL12, sctlr_el1);
    assert_eq!(m.cpu.sys.sctlr_el1, sctlr_el1);
    assert_eq!(read_at(&mut m, El::El2, SCTLR_EL12), sctlr_el1);
    assert_eq!(
        m.cpu.el2().sctlr & SCTLR_M,
        SCTLR_M,
        "SCTLR_EL2 is as it was"
    );
    write_el2(&mut m, TCR_EL12, 16);
    assert_eq!((m.cpu.sys.tcr_el1, m.cpu.el2().tcr), (16, TCR_HOST));
    write_el2(&mut m, ELR_EL12, 0x1234);
    write_el2(&mut m, ELR_EL1, 0x5678);
    assert_eq!((m.cpu.sys.elr_el1, m.cpu.el2().elr), (0x1234, 0x5678));
    write_el2(&mut m, CNTKCTL_EL12, 0x3);
    assert_eq!(m.cpu.sys.cntkctl_el1, 0x3);
    // `CNTKCTL_EL1` is `CNTHCTL_EL2` there, in its `E2H` 1 layout.
    write_el2(&mut m, CNTKCTL_EL1, 0x300);
    assert_eq!((m.cpu.el2().cnthctl, m.cpu.sys.cntkctl_el1), (0x300, 0x3));

    // Without `E2H` the names are their own, and the aliases UNDEFINED.
    write_el2(&mut m, HCR_EL2, 0);
    write_el2(&mut m, ELR_EL1, 0x9ABC);
    assert_eq!((m.cpu.sys.elr_el1, m.cpu.el2().elr), (0x9ABC, 0x5678));
    m.at(El::El2);
    assert_eq!(
        step(&mut m, mrs_of(SCTLR_EL12)),
        Some((El::El2, (EC_UNKNOWN << 26) | (1 << 25)))
    );
}

/// The Cortex-A57 has no FEAT_VHE: `ID_AA64MMFR1_EL1.VH` reads 0,
/// `HCR_EL2.E2H` is RES0, and the `*_EL12` and `*_EL02` encodings are
/// UNDEFINED at EL2, whatever `HCR_EL2` holds; the processor with every
/// feature reports `VH` 1.
#[test]
fn the_cortex_a57_has_no_vhe() {
    let mut a57 = Machine::<WithEl2>::new(Implementation::CORTEX_A57);
    assert_eq!((read_at(&mut a57, El::El2, ID_AA64MMFR1_EL1) >> 8) & 0xF, 0);
    write_el2(&mut a57, HCR_EL2, HCR_E2H | HCR_TGE);
    assert_eq!(a57.cpu.el2().hcr & HCR_E2H, 0, "E2H is RES0");
    for alias in [SCTLR_EL12, TCR_EL12, ELR_EL12, CNTKCTL_EL12, CNTV_CTL_EL02] {
        a57.at(El::El2);
        assert_eq!(
            step(&mut a57, mrs_of(alias)),
            Some((El::El2, (EC_UNKNOWN << 26) | (1 << 25))),
            "{alias:?}"
        );
    }
    // `TCR_EL1` is EL1's own at EL2 here.
    write_el2(&mut a57, TCR_EL1, 25);
    assert_eq!(a57.cpu.sys.tcr_el1, 25);

    let mut every = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    assert_eq!(
        (read_at(&mut every, El::El2, ID_AA64MMFR1_EL1) >> 8) & 0xF,
        1
    );
}

/// EL0 in the host runs in the EL2&0 regime (`TranslationRegime(EL0)`
/// under `E2H` and `TGE`): its load finds `SHARED` through `TTBR0_EL2`,
/// a load of `UPPER`, EL2's alone, takes a permission fault to EL2, and
/// EL2 finds `UPPER` through `TTBR1_EL2`. An `ERET` from EL2 reaches EL0
/// there, and an `SVC` returns to EL2 with its syndrome, from a lower
/// level.
#[test]
fn el0_in_the_host_runs_in_the_el2_and_0_regime() {
    let mut m = host();
    assert_eq!(load(&mut m, El::El2, LDR, UPPER), Ok(0x1111), "TTBR1_EL2");
    // ERET to EL0 by `ELR_EL1` and `SPSR_EL1`, which are EL2's here.
    write_el2(&mut m, ELR_EL1, CODE);
    write_el2(&mut m, SPSR_EL1, SPSR_EL0T);
    m.at(El::El2);
    m.ram.0[CODE as usize..CODE as usize + 4].copy_from_slice(&ERET.to_le_bytes());
    m.cpu.pc = CODE;
    assert_eq!(
        m.cpu
            .step(&mut m.ram, &mut crate::instrumentation::NoInstrumentation),
        super::Step::Retired
    );
    assert_eq!((m.cpu.pstate.el, m.cpu.pc), (El::El0, CODE));
    m.cpu.x[1] = SHARED;
    assert_eq!(step(&mut m, LDR), None, "EL0's load after the ERET");
    assert_eq!(m.cpu.x[2], 0xA0A0);
    assert_eq!(load(&mut m, El::El0, LDR, SHARED), Ok(0xA0A0), "TTBR0_EL2");
    assert_eq!(
        load(&mut m, El::El0, LDR, UPPER),
        Err((
            El::El2,
            (EC_DATA_ABORT_LOWER_EL << 26) | (1 << 25) | PERMISSION_L2
        )),
        "EL2's page refuses EL0"
    );
    assert_eq!(m.cpu.el2().far, UPPER);
    assert_eq!(m.cpu.pc, VBAR_EL2 + LOWER_A64);
    m.at(El::El0);
    assert_eq!(
        step(&mut m, SVC),
        Some((El::El2, (EC_SVC << 26) | (1 << 25) | 5))
    );
    assert_eq!(
        (m.cpu.pc, m.cpu.el2().elr, m.cpu.el2().spsr & 0x1F),
        (VBAR_EL2 + LOWER_A64, CODE + 4, 0),
        "to EL2's vector from a lower level, returning after the SVC to EL0t"
    );
    // `AT S1E0R` at EL2 in the host translates as EL0 in EL2&0, `AT S1E2R`
    // as EL2.
    m.cpu.x[0] = SHARED;
    completes(&mut m, El::El2, AT_S1E0R);
    assert_eq!(m.cpu.sys.par_el1 & 1, 0, "translated");
    assert_eq!(m.cpu.sys.par_el1 & 0x000F_FFFF_FFFF_F000, DATA_A);
    m.cpu.x[0] = UPPER;
    completes(&mut m, El::El2, AT_S1E0R);
    assert_eq!(m.cpu.sys.par_el1 & 1, 1, "EL0 has no access to UPPER");
    m.cpu.x[0] = UPPER;
    completes(&mut m, El::El2, AT_S1E2R);
    assert_eq!(m.cpu.sys.par_el1 & 0x000F_FFFF_FFFF_F001, DATA_HI);
}

/// The EL2&0 regime's translations carry the ASID (`AArch64_TLBContextEL20`):
/// with `SHARED`'s descriptor changed and no maintenance, EL0 keeps the
/// translation it made; `TLBI ASIDE1` of another ASID at EL2 in the host,
/// which reaches the EL2&0 regime, leaves it, and of its own removes it.
/// `TLBI VAE2` under `E2H` reaches EL2&0 too, by address and ASID: a global
/// translation whatever the ASID it names, a non-global one of that ASID
/// alone. `TLBI VMALLE1` there removes every one of them.
#[test]
fn the_tlb_maintenance_of_the_host_follows_tlbimatch() {
    let mut m = host();
    let attrs = AF | ISH | BLOCK;
    assert_eq!(load(&mut m, El::El0, LDR, SHARED), Ok(0xA0A0));
    m.ram.put64(L2_LO + 8, DATA_B | attrs | AP1 | NG);
    assert_eq!(load(&mut m, El::El0, LDR, SHARED), Ok(0xA0A0), "kept");
    m.cpu.x[0] = OTHER_ASID << 48;
    completes(&mut m, El::El2, TLBI_ASIDE1);
    assert_eq!(
        load(&mut m, El::El0, LDR, SHARED),
        Ok(0xA0A0),
        "another ASID's"
    );
    m.cpu.x[0] = ASID << 48;
    completes(&mut m, El::El2, TLBI_ASIDE1);
    assert_eq!(
        load(&mut m, El::El0, LDR, SHARED),
        Ok(0xB0B0),
        "its own ASID's"
    );

    // A non-global translation by address: of its ASID alone.
    m.ram.put64(L2_LO + 8, DATA_A | attrs | AP1 | NG);
    m.cpu.x[0] = (OTHER_ASID << 48) | (SHARED >> 12);
    completes(&mut m, El::El2, TLBI_VAE2);
    assert_eq!(load(&mut m, El::El0, LDR, SHARED), Ok(0xB0B0));
    m.cpu.x[0] = (ASID << 48) | (SHARED >> 12);
    completes(&mut m, El::El2, TLBI_VAE2);
    assert_eq!(load(&mut m, El::El0, LDR, SHARED), Ok(0xA0A0));

    // A global one, EL2's in the upper range, whatever the ASID.
    assert_eq!(load(&mut m, El::El2, LDR, UPPER), Ok(0x1111));
    m.ram.put64(L2_HI + 8, DATA_HI2 | attrs);
    assert_eq!(load(&mut m, El::El2, LDR, UPPER), Ok(0x1111), "kept");
    m.cpu.x[0] = (OTHER_ASID << 48) | ((UPPER >> 12) & ((1 << 44) - 1));
    completes(&mut m, El::El2, TLBI_VAE2);
    assert_eq!(load(&mut m, El::El2, LDR, UPPER), Ok(0x2222), "global");

    // `TLBI VMALLE1` in the host: every translation of EL2&0.
    m.ram.put64(L2_LO + 8, DATA_B | attrs | AP1 | NG);
    m.ram.put64(L2_HI + 8, DATA_HI | attrs);
    completes(&mut m, El::El2, TLBI_VMALLE1);
    assert_eq!(load(&mut m, El::El0, LDR, SHARED), Ok(0xB0B0));
    assert_eq!(load(&mut m, El::El2, LDR, UPPER), Ok(0x1111));
}

/// A TLB holding the EL2&0 regime's translations, global and not,
/// round-trips through a snapshot of the processor whose registers made
/// them, and is refused by one whose `HCR_EL2.E2H` makes EL2's regime the
/// EL2 regime, which made none of them.
#[test]
fn a_snapshot_keeps_the_el2_and_0_regimes_translations() {
    let mut m = host();
    assert_eq!(load(&mut m, El::El0, LDR, SHARED), Ok(0xA0A0));
    assert_eq!(load(&mut m, El::El2, LDR, UPPER), Ok(0x1111));
    let image = m.saved_tlb().0;
    assert_eq!(
        restored::<WithEl2>(&m.tlb_context(), &image),
        Ok(image.clone())
    );
    let mut regs = m.cpu.el2().clone();
    regs.hcr &= !HCR_E2H;
    let el2 = El2RegimeContext::of(&regs);
    let without = TlbContext {
        el2,
        ..m.tlb_context()
    };
    assert_eq!(restored::<WithEl2>(&without, &image), Err(NO_CONTEXT));

    // The processor's own snapshot, `HCR_EL2.E2H` with it.
    let mut saved = Vec::new();
    m.cpu.save(&mut saved).expect("save");
    let mut back = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    back.ram.0.copy_from_slice(&m.ram.0);
    assert_eq!(back.cpu.restore(&mut saved.as_slice()), Ok(()));
    assert_eq!(load(&mut back, El::El0, LDR, SHARED), Ok(0xA0A0));
    assert_eq!(back.cpu.el2().tcr, TCR_HOST);
}

/// A snapshot whose register `HCR_EL2.E2H` redefines holds a bit its
/// layout then has no field at is refused: `TCR_EL2.T1SZ`'s bit 19 with
/// `E2H` clear, where the `E2H` 0 layout has it RES0.
#[test]
fn a_snapshot_holds_each_register_in_its_layout() {
    let m = host();
    let mut saved = Vec::new();
    m.cpu.save(&mut saved).expect("save");
    let mut back = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    assert_eq!(back.cpu.restore(&mut saved.as_slice()), Ok(()));

    let mut torn = host();
    torn.cpu.el2_state.regs.hcr &= !HCR_E2H;
    torn.cpu.el2_state.regs.tcr = (1 << 19) | super::el2::TCR_EL2_RES1;
    let mut image = Vec::new();
    torn.cpu.save(&mut image).expect("save");
    let mut refusing = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    assert!(refusing.cpu.restore(&mut image.as_slice()).is_err());
}

/// PAN applies to EL2's data accesses in the EL2&0 regime (Arm ARM
/// D8.13.1): with `PSTATE.PAN` set, EL2's load of memory EL0 can reach
/// takes a permission fault, at EL2; an `LDTR` there is EL0's in the host
/// (`AArch64_IsUnprivAccessPriv`), which PAN leaves alone and EL2's own
/// page refuses. An exception from EL0 in the host sets `PSTATE.PAN` where
/// `SCTLR_EL2.SPAN` is clear, and leaves it where it is set.
#[test]
fn pan_and_unprivileged_accesses_at_el2_in_the_host() {
    let mut m = host();
    assert_eq!(load(&mut m, El::El2, LDR, SHARED), Ok(0xA0A0), "PAN clear");
    m.at(El::El2);
    m.cpu.pstate.pan = true;
    m.cpu.x[1] = SHARED;
    assert_eq!(
        step(&mut m, LDR),
        Some((
            El::El2,
            (EC_DATA_ABORT_SAME_EL << 26) | (1 << 25) | PERMISSION_L2
        ))
    );
    m.at(El::El2);
    m.cpu.pstate.pan = true;
    m.cpu.x[1] = SHARED;
    assert_eq!(step(&mut m, LDTR), None, "EL0's access");
    assert_eq!(m.cpu.x[2], 0xA0A0);
    assert!(
        load(&mut m, El::El2, LDTR, UPPER).is_err(),
        "EL0's access to EL2's page"
    );

    for (span, pan) in [(0, true), (SPAN, false)] {
        let sctlr = (m.cpu.el2().sctlr & !SPAN) | span;
        write_el2(&mut m, SCTLR_EL1, sctlr);
        m.at(El::El0);
        assert_eq!(step(&mut m, SVC).map(|(el, _)| el), Some(El::El2));
        assert_eq!(m.cpu.pstate.pan, pan, "SPAN {span:#x}");
    }
}

/// A physical IRQ, which `HCR_EL2.TGE` routes to EL2, is masked at EL0 in
/// the host by `PSTATE.I` (Arm ARM D8.13.1, RHGWKR), and taken to EL2's
/// vector for a lower level once it is clear; with `E2H` clear, the same
/// IRQ routed to EL2 is taken from EL0 whatever `PSTATE.I` says.
#[test]
fn el0_in_the_host_masks_the_interrupts_el2_takes() {
    const IRQ: u64 = 0x80;
    let mut m = host();
    m.at(El::El0);
    m.cpu.set_interrupt_lines(true, false);
    assert!(m.cpu.pstate.i, "masked");
    assert!(m.cpu.deliver_interrupt().is_none(), "masked in the host");
    m.cpu.pstate.i = false;
    let taken = m.cpu.deliver_interrupt().expect("taken once unmasked");
    assert_eq!((taken.to, m.cpu.pc), (El::El2, VBAR_EL2 + LOWER_A64 + IRQ));

    write_el2(&mut m, HCR_EL2, HCR_TGE);
    m.at(El::El0);
    assert!(m.cpu.pstate.i);
    let taken = m
        .cpu
        .deliver_interrupt()
        .expect("not masked outside the host");
    assert_eq!(taken.to, El::El2);
}

/// The EL2 virtual timer (`CNTHV_*_EL2`): at EL2 under `E2H` the `CNTV_*`
/// names reach it, and its line, the hypervisor virtual PPI's, rises where
/// the physical count reaches its `CVAL`, the EL1 virtual timer's staying
/// low; `CNTV_CTL_EL02` reaches the EL1 virtual timer. EL0 in the host
/// reaches it by the `CNTV_*` names too, where `CNTHCTL_EL2.EL0VTEN` lets
/// it, and traps to EL2 where it does not.
#[test]
fn the_el2_virtual_timer_fires_on_its_own_line() {
    let mut m = host();
    let cval = m.cpu.counter().get() + 50;
    write_el2(&mut m, CNTV_CVAL_EL0, cval);
    write_el2(&mut m, CNTV_CTL_EL0, 1);
    assert_eq!((m.cpu.el2().cnthv_cval, m.cpu.el2().cnthv_ctl), (cval, 1));
    assert_eq!(m.cpu.sys.cntv_ctl_el0, 0, "the EL1 virtual timer is off");
    write_el2(&mut m, CNTV_CTL_EL02, 0b10);
    assert_eq!(m.cpu.sys.cntv_ctl_el0, 0b10, "CNTV_CTL_EL02");
    let lines = m.cpu.timer_lines();
    assert!(!lines.hypervisor_virtual && !lines.virtual_);
    let deadline = m.cpu.next_timer_deadline().expect("a deadline").ticks();
    m.cpu.icount = deadline;
    assert_eq!(m.cpu.counter().get(), cval);
    let lines = m.cpu.timer_lines();
    assert!(lines.hypervisor_virtual, "CNTHV's line");
    assert!(!lines.virtual_ && !lines.hypervisor && !lines.physical);

    // EL0 in the host: `CNTV_CTL_EL0` is `CNTHV_CTL_EL2`, ISTATUS set.
    write_el2(&mut m, CNTKCTL_EL1, 1 << 8);
    assert_eq!(read_at(&mut m, El::El0, CNTV_CTL_EL0), 0b101);
    write_el2(&mut m, CNTKCTL_EL1, 0);
    m.at(El::El0);
    assert_eq!(
        step(&mut m, mrs_of(CNTV_CTL_EL0)).map(|(el, esr)| (el, esr >> 26)),
        Some((El::El2, EC_SYSREG)),
        "EL0VTEN clear"
    );
}

/// `CNTVCT_EL0` in the host reads the physical count, the virtual offset
/// treated as 0 (Arm `VirtualCounterTimer`), at EL2 under `E2H` and at EL0
/// under `E2H` and `TGE`; at EL1, with `TGE` clear, the physical count less
/// `CNTVOFF_EL2`.
#[test]
fn the_virtual_count_has_no_offset_in_the_host() {
    const OFFSET: u64 = 0x30;
    let mut m = host();
    m.cpu.el2_state.regs.cntvoff = OFFSET;
    write_el2(&mut m, CNTKCTL_EL1, 0b11);
    let count = m.cpu.counter().get();
    assert_eq!(read_at(&mut m, El::El2, CNTVCT_EL0), count);
    let count = m.cpu.counter().get();
    assert_eq!(read_at(&mut m, El::El0, CNTVCT_EL0), count);
    write_el2(&mut m, HCR_EL2, HCR_E2H);
    write_el2(&mut m, CNTKCTL_EL1, 0b11 << 10);
    let count = m.cpu.counter().get();
    assert_eq!(read_at(&mut m, El::El1, CNTVCT_EL0), count - OFFSET);
}

/// A write of `HCR_EL2.E2H` leaves the registers it redefines in the new
/// layout's form, which the EL2 regime's walk and EL2&0's read alike: the
/// host's `TCR_EL2` with `E2H` cleared keeps `T0SZ`, and with it set again
/// the bits the `E2H` 0 layout kept. The EL2 regime then translates by
/// `TTBR0_EL2` alone, a regime of one range.
#[test]
fn a_change_of_e2h_changes_el2s_regime() {
    let mut m = host();
    assert_eq!(load(&mut m, El::El2, LDR, UPPER), Ok(0x1111));
    write_el2(&mut m, HCR_EL2, 0);
    assert_eq!(m.cpu.el2().tcr & 0x3F, 25, "T0SZ");
    // In the EL2 regime the upper range is out of the one range's
    // addresses: a level 0 translation fault.
    assert_eq!(
        load(&mut m, El::El2, LDR, UPPER),
        Err((
            El::El2,
            (EC_DATA_ABORT_SAME_EL << 26) | (1 << 25) | 0b00_0100
        ))
    );
    assert_eq!(m.cpu.pc, VBAR_EL2 + CURRENT_SPX);
    assert_eq!(
        load(&mut m, El::El2, LDR, CODE),
        Ok(m.ram.get64(CODE)),
        "TTBR0_EL2 still maps the code"
    );
}
