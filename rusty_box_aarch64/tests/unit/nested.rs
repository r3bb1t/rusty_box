//! Nested virtualization (FEAT_NV, FEAT_NV2) as a guest hypervisor at EL1
//! and its hypervisor at EL2 see it (R9), on the Armv9.2 model with EL2: a
//! guest hypervisor under `HCR_EL2.NV` reads `CurrentEL` as EL2, its
//! accesses to EL2's registers and instructions and its `ERET` trap to EL2
//! with Arm's syndromes, its exceptions to EL1 report EL2 in `SPSR_EL1`,
//! and under `NV2` its accesses land in the page `VNCR_EL2` names, at each
//! register's offset, with a fault there taken to EL2 as the NV2 Data Abort
//! or watchpoint; `NV1` gives the EL1&0 regime the EL2 regime's permission
//! layout. The Cortex-A57, an Armv8.0 processor, has none of it. Which
//! access does what in every state of the controls is `sysreg_spec.rs`'s,
//! against Arm's accessors.
//!
//! The rules are the Arm ARM's (DDI 0487M.d, D24.2's `HCR_EL2`,
//! `VNCR_EL2`, `ID_AA64MMFR2_EL1` and `ESR_EL2` descriptions) and its
//! pseudocode: `EffectiveHCR_EL2_NVx`, `NVMem`, `AArch64_CheckForERetTrap`,
//! `AArch64_TakeException`, `AArch64_DataAbort`,
//! `AArch64_WatchpointException`, `AArch64_FaultSyndrome`,
//! `AArch64_IsUnprivAccessPriv`, `AArch64_S1ApplyOutputPerms` and
//! `AArch64_S1DirectBasePermissions`; the offsets are `Registers.json`'s
//! `NVMem(offset)`.

use super::el2_tests::Machine;
use super::features::Implementation;
use super::{El, WithEl2};

/// Where the word under test executes, and EL2's and EL1's vector bases
/// (`el2.rs`'s).
const CODE: u64 = 0x4_0000;
const VBAR_EL2: u64 = 0xA000;
const VBAR_EL1: u64 = 0x8000;
const LOWER_A64: u64 = 0x400;
const CURRENT_SPX: u64 = 0x200;
/// The page `VNCR_EL2` names: EL2's stage 1 is off, so it is the physical
/// page.
const VNCR_PAGE: u64 = 0x5_0000;

const ERET: u32 = 0xD69F_03E0;
const ERETAA: u32 = 0xD69F_0BFF;
const SVC: u32 = 0xD400_00A1; // svc #0x5
const LDR: u32 = 0xF940_0022; // ldr x2, [x1]
const LDTR: u32 = 0xF840_0822; // ldtr x2, [x1]
const NOP: u32 = 0xD503_201F;
const MRS_CURRENT_EL: u32 = 0xD538_4240; // mrs x0, currentel
const TLBI_ALLE2: u32 = 0xD50C_871F;

/// Exception classes and `ESR_ELx.IL`.
const EC_UNKNOWN: u64 = 0x00;
const EC_SVC: u64 = 0x15;
const EC_SYSREG: u64 = 0x18;
const EC_ERET: u64 = 0x1A;
const EC_INSTRUCTION_ABORT_SAME_EL: u64 = 0x21;
const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
const EC_WATCHPOINT_SAME_EL: u64 = 0x35;
const IL: u64 = 1 << 25;
/// `ISS.VNCR`, `WnR`, and the fault status codes these tests raise.
const ISS_VNCR: u64 = 1 << 13;
const WNR: u64 = 1 << 6;
const SYNC_EXTERNAL: u64 = 0b01_0000;
const PERMISSION_L2: u64 = 0b00_1110;
const DEBUG_EXCEPTION: u64 = 0b10_0010;
/// `ISS.WPTV` of a watchpoint (FEAT_Debugv8p2), watchpoint 0.
const WPTV: u64 = 1 << 17;

/// `HCR_EL2`'s bits these tests set.
const HCR_E2H: u64 = 1 << 34;
const HCR_NV: u64 = 1 << 42;
const HCR_NV1: u64 = 1 << 43;
const HCR_AT: u64 = 1 << 44;
const HCR_NV2: u64 = 1 << 45;
/// `HCR_EL2.RW`, RES1.
const HCR_RW: u64 = 1 << 31;

/// `MRS Xt, S<op0>_<op1>_C<crn>_C<crm>_<op2>`.
const fn mrs(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> u32 {
    0xD520_0000 | (op0 << 19) | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5) | rt
}

/// `MSR S<op0>_<op1>_C<crn>_C<crm>_<op2>, Xt`.
const fn msr(op0: u32, op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> u32 {
    mrs(op0, op1, crn, crm, op2, rt) & !(1 << 21)
}

/// `SYS #<op1>, C<crn>, C<crm>, #<op2>, Xt`.
const fn sys(op1: u32, crn: u32, crm: u32, op2: u32, rt: u32) -> u32 {
    0xD508_0000 | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5) | rt
}

/// A register's `(op0, op1, CRn, CRm, op2)`.
type Encoding = (u32, u32, u32, u32, u32);

const HCR_EL2: Encoding = (3, 4, 1, 1, 0);
const SCTLR_EL2: Encoding = (3, 4, 1, 0, 0);
const VBAR_EL2_REG: Encoding = (3, 4, 12, 0, 0);
const VTTBR_EL2: Encoding = (3, 4, 2, 1, 0);
const VNCR_EL2: Encoding = (3, 4, 2, 2, 0);
const ELR_EL2: Encoding = (3, 4, 4, 0, 1);
const SP_EL1: Encoding = (3, 4, 4, 1, 0);
const MDCR_EL2: Encoding = (3, 4, 1, 1, 1);
const CNTHCTL_EL2: Encoding = (3, 4, 14, 1, 0);
const SCTLR_EL1: Encoding = (3, 0, 1, 0, 0);
const SPSR_EL1: Encoding = (3, 0, 4, 0, 0);
const ELR_EL1: Encoding = (3, 0, 4, 0, 1);
const SCTLR_EL12: Encoding = (3, 5, 1, 0, 0);
const ELR_EL12: Encoding = (3, 5, 4, 0, 1);
const CNTP_CTL_EL02: Encoding = (3, 5, 14, 2, 1);
const MDSCR_EL1: Encoding = (2, 0, 0, 2, 2);
const DBGWVR0_EL1: Encoding = (2, 0, 0, 0, 6);
const DBGWCR0_EL1: Encoding = (2, 0, 0, 0, 7);
const OSLAR_EL1: Encoding = (2, 0, 1, 0, 4);
const ID_AA64MMFR2_EL1: Encoding = (3, 0, 0, 7, 2);

const fn mrs_of(r: Encoding) -> u32 {
    mrs(r.0, r.1, r.2, r.3, r.4, 0)
}

const fn msr_of(r: Encoding) -> u32 {
    msr(r.0, r.1, r.2, r.3, r.4, 0)
}

/// `AT S1E1R, X0`.
const AT_S1E1R: u32 = sys(0, 7, 8, 0, 0);

/// The offsets `Registers.json` gives the registers these tests reach in
/// the `VNCR_EL2` page.
const VNCR_VTTBR_EL2: u64 = 0x020;
const VNCR_HCR_EL2: u64 = 0x078;
const VNCR_SCTLR_EL1: u64 = 0x110;
const VNCR_ELR_EL1: u64 = 0x230;
const VNCR_SP_EL1: u64 = 0x240;
const VNCR_CNTP_CTL_EL0: u64 = 0x180;

/// The words are the assembler's (GNU as 2.42, `-march=armv9.2-a`): `mrs
/// x0, currentel`, `msr hcr_el2, x0`, `msr sctlr_el12, x0`, `mrs x0,
/// vncr_el2`, `ldtr x2, [x1]` and `at s1e1r, x0`.
#[test]
fn the_words_are_the_assemblers() {
    assert_eq!(MRS_CURRENT_EL, 0xD538_4240);
    assert_eq!(msr_of(HCR_EL2), 0xD51C_1100);
    assert_eq!(msr_of(SCTLR_EL12), 0xD51D_1000);
    assert_eq!(mrs_of(VNCR_EL2), 0xD53C_2200);
    assert_eq!(LDTR, 0xF840_0822);
    assert_eq!(AT_S1E1R, 0xD508_7800);
}

/// The Armv9.2 model with EL2, at EL2 out of reset, its `VNCR_EL2` naming
/// [`VNCR_PAGE`].
fn armv9p2() -> Machine<WithEl2> {
    let mut m = Machine::<WithEl2>::new(Implementation::ARMV9P2);
    write_el2(&mut m, VNCR_EL2, VNCR_PAGE);
    m
}

/// Step `insn` at [`CODE`] at `el`: `None` where it completed, the level it
/// took an exception to and the `ESR` there otherwise.
fn step_at(m: &mut Machine<WithEl2>, el: El, insn: u32) -> Option<(El, u64)> {
    m.at(el);
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

/// `MSR` of `value` to `reg` at EL1, which completes.
fn write_el1(m: &mut Machine<WithEl2>, reg: Encoding, value: u64) {
    m.cpu.x[0] = value;
    assert_eq!(step_at(m, El::El1, msr_of(reg)), None, "{reg:?}");
}

/// The exception a trap to EL2 with class `ec` and syndrome `iss` takes.
const fn to_el2(ec: u64, iss: u64) -> Option<(El, u64)> {
    Some((El::El2, (ec << 26) | IL | iss))
}

/// `AArch64_SystemAccessTrapSyndrome` of the `MRS` (`read`) or `MSR` of
/// `reg`, naming X0.
const fn sysreg_iss(r: Encoding, read: bool) -> u64 {
    ((r.0 as u64) << 20)
        | ((r.4 as u64) << 17)
        | ((r.1 as u64) << 14)
        | ((r.2 as u64) << 10)
        | ((r.3 as u64) << 1)
        | read as u64
}

/// The doubleword at `offset` in the `VNCR_EL2` page.
fn vncr(m: &Machine<WithEl2>, offset: u64) -> u64 {
    m.ram.get64(VNCR_PAGE + offset)
}

// ----- the ID registers and HCR_EL2 -----

/// The Armv9.2 model with EL2 reports FEAT_NV2, `ID_AA64MMFR2_EL1.NV` 2,
/// and keeps `HCR_EL2.NV`, `NV1`, `AT` and `NV2`; the Cortex-A57, an
/// Armv8.0 processor, reports 0 there, its four bits are RES0 and it has
/// no `VNCR_EL2`.
#[test]
fn the_armv9p2_model_has_nested_virtualization_and_the_cortex_a57_has_none() {
    let nested = HCR_NV | HCR_NV1 | HCR_AT | HCR_NV2;
    let mut m = armv9p2();
    assert_eq!((read_at(&mut m, El::El1, ID_AA64MMFR2_EL1) >> 24) & 0xF, 2);
    write_el2(&mut m, HCR_EL2, nested);
    assert_eq!(read_at(&mut m, El::El2, HCR_EL2), nested | HCR_RW);
    assert_eq!(read_at(&mut m, El::El2, VNCR_EL2), VNCR_PAGE);
    write_el2(&mut m, VNCR_EL2, u64::MAX);
    assert_eq!(read_at(&mut m, El::El2, VNCR_EL2), !0xFFF, "BADDR, 63:12");

    let mut a57 = Machine::<WithEl2>::new(Implementation::CORTEX_A57);
    assert_eq!(
        (read_at(&mut a57, El::El1, ID_AA64MMFR2_EL1) >> 24) & 0xF,
        0
    );
    write_el2(&mut a57, HCR_EL2, nested);
    assert_eq!(read_at(&mut a57, El::El2, HCR_EL2), HCR_RW, "RES0");
    assert_eq!(
        step_at(&mut a57, El::El2, mrs_of(VNCR_EL2)),
        Some((El::El2, (EC_UNKNOWN << 26) | IL)),
        "no VNCR_EL2"
    );
    assert_eq!(
        read_at(&mut a57, El::El1, (3, 0, 4, 2, 2)),
        1 << 2,
        "CurrentEL"
    );
}

// ----- FEAT_NV -----

/// Under `HCR_EL2.NV` a guest hypervisor at EL1 reads `CurrentEL` as EL2;
/// without it, as EL1.
#[test]
fn a_guest_hypervisor_reads_current_el_as_el2() {
    let mut m = armv9p2();
    assert_eq!(step_at(&mut m, El::El1, MRS_CURRENT_EL), None);
    assert_eq!(m.cpu.x[0], 1 << 2, "EL1");
    for nested in [HCR_NV, HCR_NV | HCR_NV1, HCR_NV | HCR_NV2] {
        write_el2(&mut m, HCR_EL2, nested);
        assert_eq!(step_at(&mut m, El::El1, MRS_CURRENT_EL), None);
        assert_eq!(m.cpu.x[0], 2 << 2, "EL2 under HCR_EL2 {nested:#x}");
    }
}

/// Under `NV` a guest hypervisor's `MSR` and `MRS` of an EL2 register, of a
/// `*_EL12` name and of `SP_EL1`, UNDEFINED at EL1 without it, trap to EL2
/// with EC 0x18 and the access's syndrome, returning to the access.
#[test]
fn its_el2_register_accesses_trap_to_el2() {
    let mut m = armv9p2();
    for (insn, reg, read) in [
        (msr_of(SCTLR_EL2), SCTLR_EL2, false),
        (mrs_of(VBAR_EL2_REG), VBAR_EL2_REG, true),
        (msr_of(HCR_EL2), HCR_EL2, false),
        (mrs_of(SP_EL1), SP_EL1, true),
        (msr_of(SCTLR_EL12), SCTLR_EL12, false),
    ] {
        write_el2(&mut m, HCR_EL2, 0);
        assert_eq!(
            step_at(&mut m, El::El1, insn),
            Some((El::El1, (EC_UNKNOWN << 26) | IL)),
            "{reg:?} without NV"
        );
        write_el2(&mut m, HCR_EL2, HCR_NV);
        assert_eq!(
            step_at(&mut m, El::El1, insn),
            to_el2(EC_SYSREG, sysreg_iss(reg, read)),
            "{reg:?} under NV"
        );
        assert_eq!(m.cpu.pc, VBAR_EL2 + LOWER_A64);
        assert_eq!(m.cpu.el2().elr, CODE, "returning to the access");
    }
}

/// Under `NV` a guest hypervisor's EL2 System instructions trap to EL2
/// (`TLBI ALLE2`), UNDEFINED at EL1 without it; and `HCR_EL2.AT`, without
/// `NV`, traps its `AT S1E1R`.
#[test]
fn its_el2_instructions_and_with_at_its_address_translations_trap() {
    let mut m = armv9p2();
    assert_eq!(
        step_at(&mut m, El::El1, TLBI_ALLE2),
        Some((El::El1, (EC_UNKNOWN << 26) | IL))
    );
    write_el2(&mut m, HCR_EL2, HCR_NV);
    assert_eq!(
        step_at(&mut m, El::El1, TLBI_ALLE2),
        to_el2(EC_SYSREG, sysreg_iss((1, 4, 8, 7, 0), false) | (31 << 5))
    );
    m.cpu.x[0] = CODE;
    assert_eq!(step_at(&mut m, El::El1, AT_S1E1R), None, "no AT trap");
    write_el2(&mut m, HCR_EL2, HCR_AT);
    m.cpu.x[0] = CODE;
    assert_eq!(
        step_at(&mut m, El::El1, AT_S1E1R),
        to_el2(EC_SYSREG, sysreg_iss((1, 0, 7, 8, 0), false))
    );
}

/// A guest hypervisor's `ERET` and `ERETAA` trap to EL2 under `NV` with EC
/// 0x1A (`ISS[1]` set for the authenticating form, `ISS[0]` 0 for key A),
/// before reading `ELR_EL1`; without `NV` the `ERET` returns.
#[test]
fn a_guest_hypervisors_eret_traps() {
    let mut m = armv9p2();
    m.cpu.sys.elr_el1 = CODE + 0x100;
    m.cpu.sys.spsr_el1 = 0b0101; // EL1h
    m.at(El::El1);
    assert_eq!(m.run_at(CODE, ERET), None);
    assert_eq!(m.cpu.pc, CODE + 0x100, "the ERET returned");
    write_el2(&mut m, HCR_EL2, HCR_NV);
    assert_eq!(step_at(&mut m, El::El1, ERET), to_el2(EC_ERET, 0));
    assert_eq!(m.cpu.el2().elr, CODE, "returning to the ERET");
    assert_eq!(step_at(&mut m, El::El1, ERETAA), to_el2(EC_ERET, 0b10));
}

/// An exception a guest hypervisor takes to EL1 reports EL2h in
/// `SPSR_EL1.M` under `HCR_EL2.{NV2, NV1, NV}` {0, 0, 1}, {1, 0, 1} and
/// {1, 1, 1}, and its own EL1h under {0, 1, 1} and without `NV`.
#[test]
fn its_exceptions_to_el1_report_el2() {
    const EL1H: u64 = 0b0101;
    const EL2H: u64 = 0b1001;
    let mut m = armv9p2();
    for (hcr, m_field) in [
        (0, EL1H),
        (HCR_NV, EL2H),
        (HCR_NV | HCR_NV1, EL1H),
        (HCR_NV | HCR_NV2, EL2H),
        (HCR_NV | HCR_NV1 | HCR_NV2, EL2H),
    ] {
        write_el2(&mut m, HCR_EL2, hcr);
        assert_eq!(
            step_at(&mut m, El::El1, SVC),
            Some((El::El1, (EC_SVC << 26) | IL | 5)),
            "HCR_EL2 {hcr:#x}"
        );
        assert_eq!(m.cpu.pc, VBAR_EL1 + CURRENT_SPX);
        assert_eq!(m.cpu.sys.spsr_el1 & 0xF, m_field, "HCR_EL2 {hcr:#x}");
    }
}

/// Under `NV` and `NV1` a guest hypervisor's `SPSR_EL1`, `ELR_EL1` and
/// `VBAR_EL1`, which it would share with its guest, trap to EL2; under `NV`
/// alone they are its own.
#[test]
fn with_nv1_its_exception_registers_trap() {
    let mut m = armv9p2();
    write_el2(&mut m, HCR_EL2, HCR_NV);
    write_el1(&mut m, ELR_EL1, 0x1234);
    assert_eq!(read_at(&mut m, El::El1, ELR_EL1), 0x1234);
    write_el2(&mut m, HCR_EL2, HCR_NV | HCR_NV1);
    for reg in [SPSR_EL1, ELR_EL1, (3, 0, 12, 0, 0)] {
        assert_eq!(
            step_at(&mut m, El::El1, mrs_of(reg)),
            to_el2(EC_SYSREG, sysreg_iss(reg, true)),
            "{reg:?}"
        );
    }
}

// ----- FEAT_NV2 -----

/// Under `NV` and `NV2`, a guest hypervisor with `E2H` (`NV1` clear): its
/// `MSR HCR_EL2` stores at offset 0x78 of the `VNCR_EL2` page and its `MRS
/// VTTBR_EL2` loads from 0x20, leaving the processor's own registers as
/// they were; its `MSR SCTLR_EL12` and `ELR_EL12`, its guest's EL1
/// registers, store at 0x110 and 0x230; its `MRS ELR_EL2` reads the
/// processor's `ELR_EL1`, which an exception to its EL2 fills; and `MRS
/// SP_EL1` loads from 0x240. Its own `SCTLR_EL1` is the processor's.
#[test]
fn with_nv2_its_accesses_land_in_the_vncr_page() {
    let mut m = armv9p2();
    write_el2(&mut m, HCR_EL2, HCR_NV | HCR_NV2 | HCR_E2H);
    let sctlr = m.cpu.sys.sctlr_el1;

    write_el1(&mut m, HCR_EL2, 0x0123_4567_89AB_CDEF);
    assert_eq!(vncr(&m, VNCR_HCR_EL2), 0x0123_4567_89AB_CDEF);
    assert_eq!(
        read_at(&mut m, El::El2, HCR_EL2),
        HCR_NV | HCR_NV2 | HCR_E2H | HCR_RW,
        "EL2's own HCR_EL2"
    );
    m.ram.put64(VNCR_PAGE + VNCR_VTTBR_EL2, 0xFEED_0000_1000);
    assert_eq!(read_at(&mut m, El::El1, VTTBR_EL2), 0xFEED_0000_1000);
    m.ram.put64(VNCR_PAGE + VNCR_SP_EL1, 0x7777_0000);
    assert_eq!(read_at(&mut m, El::El1, SP_EL1), 0x7777_0000);

    write_el1(&mut m, SCTLR_EL12, 0x5555_0000);
    assert_eq!(vncr(&m, VNCR_SCTLR_EL1), 0x5555_0000);
    write_el1(&mut m, ELR_EL12, 0x6666_0000);
    assert_eq!(vncr(&m, VNCR_ELR_EL1), 0x6666_0000);
    assert_eq!(m.cpu.sys.sctlr_el1, sctlr, "the processor's SCTLR_EL1");

    m.cpu.sys.elr_el1 = 0x8888_0000;
    assert_eq!(read_at(&mut m, El::El1, ELR_EL2), 0x8888_0000, "ELR_EL1");
    assert_eq!(read_at(&mut m, El::El1, SCTLR_EL1), sctlr, "its own");
}

/// Under {1, 1, 1}, a guest hypervisor without `E2H`, its `SCTLR_EL1` and
/// `ELR_EL1` are its guest's, in the page; the processor's are its own
/// EL2 registers' stand-ins and stay as they were.
#[test]
fn with_nv2_and_nv1_its_el1_registers_are_its_guests_in_the_page() {
    let mut m = armv9p2();
    write_el2(&mut m, HCR_EL2, HCR_NV | HCR_NV1 | HCR_NV2);
    let (sctlr, elr) = (m.cpu.sys.sctlr_el1, m.cpu.sys.elr_el1);
    write_el1(&mut m, SCTLR_EL1, 0x30D0_0800);
    write_el1(&mut m, ELR_EL1, 0x4000_1000);
    assert_eq!(vncr(&m, VNCR_SCTLR_EL1), 0x30D0_0800);
    assert_eq!(vncr(&m, VNCR_ELR_EL1), 0x4000_1000);
    assert_eq!((m.cpu.sys.sctlr_el1, m.cpu.sys.elr_el1), (sctlr, elr));
    m.ram.put64(VNCR_PAGE + VNCR_ELR_EL1, 0x4000_2000);
    assert_eq!(read_at(&mut m, El::El1, ELR_EL1), 0x4000_2000);
}

/// Under {1, 0, 1} `CNTHCTL_EL2.EL1NVPCT` (FEAT_ECV) traps a guest
/// hypervisor's `CNTP_CTL_EL02`, which is otherwise in the page at 0x180.
#[test]
fn el1nvpct_traps_the_el02_physical_timer_in_place_of_the_page() {
    const EL1NVPCT: u64 = 1 << 15;
    let mut m = armv9p2();
    write_el2(&mut m, HCR_EL2, HCR_NV | HCR_NV2 | HCR_E2H);
    write_el1(&mut m, CNTP_CTL_EL02, 1);
    assert_eq!(vncr(&m, VNCR_CNTP_CTL_EL0), 1);
    write_el2(&mut m, CNTHCTL_EL2, EL1NVPCT);
    m.cpu.x[0] = 3;
    assert_eq!(
        step_at(&mut m, El::El1, msr_of(CNTP_CTL_EL02)),
        to_el2(EC_SYSREG, sysreg_iss(CNTP_CTL_EL02, false))
    );
    assert_eq!(vncr(&m, VNCR_CNTP_CTL_EL0), 1, "nothing stored");
}

/// A load or store `VNCR_EL2` makes that faults is taken to EL2 with EC
/// 0x25 whatever level it came from, `ISS.VNCR` set, `WnR` for the store
/// and `FAR_EL2` its address: here a synchronous External abort, the page
/// being where the bus has nothing.
#[test]
fn a_fault_on_the_vncr_page_is_the_nv2_data_abort() {
    const NOWHERE: u64 = 0x80_0000_0000;
    let mut m = armv9p2();
    write_el2(&mut m, VNCR_EL2, NOWHERE);
    write_el2(&mut m, HCR_EL2, HCR_NV | HCR_NV2 | HCR_E2H);
    m.cpu.x[0] = 1;
    assert_eq!(
        step_at(&mut m, El::El1, msr_of(HCR_EL2)),
        to_el2(EC_DATA_ABORT_SAME_EL, ISS_VNCR | WNR | SYNC_EXTERNAL)
    );
    assert_eq!(m.cpu.el2().far, NOWHERE + VNCR_HCR_EL2);
    assert_eq!(m.cpu.el2().elr, CODE, "returning to the MSR");
    assert_eq!(m.cpu.pc, VBAR_EL2 + LOWER_A64);
    assert_eq!(
        step_at(&mut m, El::El1, mrs_of(VTTBR_EL2)),
        to_el2(EC_DATA_ABORT_SAME_EL, ISS_VNCR | SYNC_EXTERNAL)
    );
}

/// A watchpoint on the page, set for EL2 (`HMC`, all levels) with debug
/// exceptions targeting EL2 (`MDCR_EL2.TDE`, `MDSCR_EL1.KDE` and `MDE`,
/// the OS Lock, locked out of reset, unlocked),
/// fires on a guest hypervisor's transformed `MSR` with EC 0x35 and
/// `ISS.VNCR`, though `PSTATE.D` is set: the access is EL2's, from which a
/// debug exception to EL2 is one to its own level, which `PSTATE.D` masks
/// for EL2's own accesses and Arm does not apply to this one
/// (`AArch64_CheckDebug`).
#[test]
fn a_watchpoint_on_the_vncr_page_is_the_nv2_watchpoint() {
    const MDCR_TDE: u64 = 1 << 8;
    const MDSCR_KDE_MDE: u64 = (1 << 13) | (1 << 15);
    // E, PAC EL0 and EL1, LSC loads and stores, BAS all eight bytes, HMC.
    const WATCH_ALL: u64 = 1 | (0b11 << 1) | (0b11 << 3) | (0xFF << 5) | (1 << 13);
    let mut m = armv9p2();
    write_el2(&mut m, OSLAR_EL1, 0);
    write_el2(&mut m, DBGWVR0_EL1, VNCR_PAGE + VNCR_HCR_EL2);
    write_el2(&mut m, DBGWCR0_EL1, WATCH_ALL);
    write_el2(&mut m, MDSCR_EL1, MDSCR_KDE_MDE);
    write_el2(&mut m, MDCR_EL2, MDCR_TDE);
    write_el2(&mut m, HCR_EL2, HCR_NV | HCR_NV2 | HCR_E2H);
    m.at(El::El1);
    assert!(m.cpu.pstate.d, "PSTATE.D set");
    m.cpu.x[0] = 1;
    assert_eq!(
        step_at(&mut m, El::El1, msr_of(HCR_EL2)),
        to_el2(
            EC_WATCHPOINT_SAME_EL,
            ISS_VNCR | WNR | WPTV | DEBUG_EXCEPTION
        )
    );
    assert_eq!(m.cpu.el2().far, VNCR_PAGE + VNCR_HCR_EL2);
    assert_ne!(vncr(&m, VNCR_HCR_EL2), 1, "the store was not made");
}

// ----- NV1's translation -----

/// The EL1&0 regime's tables: T0SZ 25, a level 1 table whose first entry
/// is a level 2 table of 2 MiB blocks.
const L1: u64 = 0x1_2000;
const L2: u64 = 0x1_3000;
/// Block 0, the code's, EL1 read-write and executable; block 1 EL0's too
/// (`AP[1]` set); block 2 with descriptor bit 54 set, `UXN`, which `NV1`
/// makes the privileged execute-never.
const CODE_BLOCK: u64 = 0;
const SHARED_BLOCK: u64 = 0x20_0000;
const BIT54_BLOCK: u64 = 0x40_0000;
const AF: u64 = 1 << 10;
const INNER_SHAREABLE: u64 = 3 << 8;
const BLOCK: u64 = 0b01;
const TABLE: u64 = 0b11;
const AP1: u64 = 1 << 6;
const BIT54: u64 = 1 << 54;

/// [`armv9p2`] with the EL1&0 regime's stage 1 on, mapping its first 6 MiB
/// to themselves.
fn translating() -> Machine<WithEl2> {
    let mut m = armv9p2();
    let attrs = AF | INNER_SHAREABLE | BLOCK;
    m.ram.put64(L1, L2 | TABLE);
    m.ram.put64(L2, CODE_BLOCK | attrs);
    m.ram.put64(L2 + 8, SHARED_BLOCK | attrs | AP1);
    m.ram.put64(L2 + 16, BIT54_BLOCK | attrs | BIT54);
    m.ram.put64(SHARED_BLOCK, 0x1111);
    m.ram.put64(CODE_BLOCK + 0x100, 0x2222);
    let sys = &mut m.cpu.sys;
    sys.tcr_el1 = 25 | (1 << 23);
    sys.ttbr0_el1 = L1;
    sys.mair_el1 = 0xFF;
    sys.sctlr_el1 |= 1;
    m.cpu.flush_tlb();
    m
}

/// Under `NV` and `NV1` the EL1&0 regime's descriptors take the EL2
/// regime's layout: `AP[1]` reads as 0, so PSTATE.PAN, which the regime
/// then lacks, refuses EL1 nothing, and an `LDTR`, privileged there, reads
/// memory EL0 has no access to; and bit 54 is EL1's execute-never. Without
/// `NV1` each is the EL1&0 regime's own.
#[test]
fn nv1_gives_the_el1_and_0_regime_the_el2_layout() {
    let mut m = translating();
    let pan_load = |m: &mut Machine<WithEl2>| {
        m.at(El::El1);
        m.cpu.pstate.pan = true;
        m.cpu.x[1] = SHARED_BLOCK;
        let taken = m.run_at(CODE, LDR);
        taken.map(|t| t.esr)
    };
    let ldtr = |m: &mut Machine<WithEl2>| {
        m.cpu.x[1] = CODE_BLOCK + 0x100;
        step_at(m, El::El1, LDTR)
    };
    let fetch = |m: &mut Machine<WithEl2>| {
        m.at(El::El1);
        m.run_at(BIT54_BLOCK, NOP).map(|t| t.esr)
    };
    write_el2(&mut m, HCR_EL2, HCR_NV);
    assert_eq!(
        pan_load(&mut m),
        Some((EC_DATA_ABORT_SAME_EL << 26) | IL | PERMISSION_L2),
        "PAN refuses EL0's memory"
    );
    assert_eq!(
        ldtr(&mut m),
        Some((El::El1, (EC_DATA_ABORT_SAME_EL << 26) | IL | PERMISSION_L2)),
        "an LDTR checked as EL0"
    );
    assert_eq!(fetch(&mut m), None, "bit 54 is UXN");

    write_el2(&mut m, HCR_EL2, HCR_NV | HCR_NV1);
    assert_eq!(pan_load(&mut m), None, "no PAN under NV1");
    assert_eq!(m.cpu.x[2], 0x1111);
    assert_eq!(ldtr(&mut m), None, "a privileged LDTR");
    assert_eq!(m.cpu.x[2], 0x2222);
    assert_eq!(
        fetch(&mut m),
        Some((EC_INSTRUCTION_ABORT_SAME_EL << 26) | IL | PERMISSION_L2),
        "bit 54 is EL1's execute-never"
    );
}
