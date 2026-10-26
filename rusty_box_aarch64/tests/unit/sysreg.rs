//! System register accesses as a guest sees them (R9): what an `MRS`
//! reads, what an `MSR` keeps, and the exception an access takes, as
//! `ESR_EL1` reports it.

use super::super::features::{Feature, Implementation};
use super::super::step::{exec, processor, Taken, UNDEFINED};
use super::super::{Cpu, El, Pstate};

/// The words are GNU as's (`-march=armv9.2-a+predres`, objdump -d).
const MRS_MIDR: u32 = 0xD538_0000; // mrs x0, midr_el1
const MRS_ID_AA64ISAR2: u32 = 0xD538_0640; // mrs x0, id_aa64isar2_el1
const MRS_S3_1_C0_C0_3: u32 = 0xD539_0060; // unallocated, op1 = 1
const MRS_S3_3_C0_C0_0: u32 = 0xD53B_0000; // unallocated, op1 = 3
const MRS_S3_0_C0_C3_7: u32 = 0xD538_03E0; // reserved: RAZ
const MSR_MIDR: u32 = 0xD518_0000; // msr midr_el1, x0
const MRS_CTR: u32 = 0xD53B_0020; // mrs x0, ctr_el0
const MRS_SCTLR: u32 = 0xD538_1000; // mrs x0, sctlr_el1
const MRS_CNTPCTSS: u32 = 0xD53B_E0A0; // mrs x0, cntpctss_el0
const MRS_CNTVCTSS: u32 = 0xD53B_E0C0; // mrs x0, cntvctss_el0
const CFP_RCTX: u32 = 0xD50B_7380; // cfp rctx, x0
const DVP_RCTX: u32 = 0xD50B_73A0; // dvp rctx, x0
const CPP_RCTX: u32 = 0xD50B_73E0; // cpp rctx, x0

/// `SCTLR_EL1.UCT` and `EnRCTX`; `CNTKCTL_EL1.EL0PCTEN` and `EL0VCTEN`.
const UCT: u64 = 1 << 15;
const ENRCTX: u64 = 1 << 10;
const EL0PCTEN: u64 = 1 << 0;
const EL0VCTEN: u64 = 1 << 1;

/// A trapped `MRS`/`MSR`/`SYS`: EC 0x18 and the ISS naming the encoding.
const fn sysreg_trap(iss: u64) -> Taken {
    Taken { ec: 0x18, iss }
}

/// `MRS X0, MDCCSR_EL0`, `MRS X0, DBGDTR_EL0`, `MRS X0, DBGDTRRX_EL0`.
const MRS_MDCCSR: u32 = 0xD533_0100;
const MRS_DBGDTR: u32 = 0xD533_0400;
const MRS_DBGDTRRX: u32 = 0xD533_0500;
/// `MSR DBGDTR_EL0, X0`, `MSR DBGDTRTX_EL0, X0`.
const MSR_DBGDTR: u32 = 0xD513_0400;
const MSR_DBGDTRTX: u32 = 0xD513_0500;
/// The `MSR` form of `MDCCSR_EL0`'s encoding, which is read-only.
const MSR_MDCCSR: u32 = 0xD513_0100;

fn at_el0(cpu: &mut Cpu) {
    cpu.pstate = Pstate {
        el: El::El0,
        sp_sel: false,
        ..Pstate::RESET
    };
}

/// The Debug Communications Channel with no debugger attached, as QEMU's
/// `virt` has it: `MDCCSR_EL0` reads 0 (RXfull and TXfull clear),
/// `DBGDTR_EL0` and `DBGDTRRX_EL0` read 0, and a word written to
/// `DBGDTR_EL0` or `DBGDTRTX_EL0` is gone. EL0 reaches all four unless
/// `MDSCR_EL1.TDCC` is set, which traps EL0's accesses to EL1 (EC 0x18,
/// the ISS naming the register); EL1's go through. `MDCCSR_EL0` cannot be
/// written.
#[test]
fn the_debug_communications_channel_is_idle_and_tdcc_traps_el0() {
    // ISS: op0 2, op2 0, op1 3, CRn 0, Rt 0, CRm and the direction.
    let reads = [(MRS_MDCCSR, 0x0020_C003), (MRS_DBGDTR, 0x0020_C009), (MRS_DBGDTRRX, 0x0020_C00B)];
    let writes = [(MSR_DBGDTR, 0x0020_C008), (MSR_DBGDTRTX, 0x0020_C00A)];
    let mut cpu = processor(Implementation::CORTEX_A57);
    for el0 in [false, true] {
        if el0 {
            at_el0(&mut cpu);
        }
        for (insn, _) in writes {
            cpu.x[0] = 0x1234_5678;
            assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}, EL0 {el0}");
        }
        for (insn, _) in reads {
            cpu.x[0] = 0xDEAD;
            assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}, EL0 {el0}");
            assert_eq!(cpu.x[0], 0, "{insn:#010x}, EL0 {el0}");
        }
        assert_eq!(exec(&mut cpu, MSR_MDCCSR), Some(UNDEFINED), "EL0 {el0}");
    }

    cpu.sys.mdscr_el1 = 1 << 12;
    for (insn, iss) in reads.into_iter().chain(writes) {
        assert_eq!(exec(&mut cpu, insn), Some(Taken { ec: 0x18, iss }), "{insn:#010x} at EL0");
    }
    cpu.pstate = Pstate::RESET;
    for (insn, _) in reads.into_iter().chain(writes) {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL1");
    }
}

/// FEAT_IDST: a read of the feature ID space that would be UNDEFINED traps
/// with EC 0x18 instead, the ISS naming the register: EL0's reads of the
/// ID registers (each accessor's `AArch64_SystemAccessTraptoEL1orEL2`), and
/// an unallocated encoding at any EL (`UnimplementedIDRegister`). The
/// reserved encodings EL1 reads as zero stay so, a write stays UNDEFINED,
/// and so does a read outside the space. Without FEAT_IDST every such read
/// is UNDEFINED.
#[test]
fn idst_traps_the_reads_of_the_id_space_that_would_be_undefined() {
    for imp in [Implementation::EVERY_FEATURE, Implementation::CORTEX_A57] {
        let idst = imp.has(Feature::Idst);
        let refused = |iss| if idst { Some(sysreg_trap(iss)) } else { Some(UNDEFINED) };
        let mut cpu = processor(imp);
        cpu.sys.sctlr_el1 |= UCT;

        assert_eq!(exec(&mut cpu, MRS_S3_1_C0_C0_3), refused(0x0036_4001), "IDST {idst}");
        assert_eq!(exec(&mut cpu, MRS_S3_3_C0_C0_0), refused(0x0030_C001), "IDST {idst}");
        cpu.x[0] = 0xDEAD;
        assert_eq!(exec(&mut cpu, MRS_S3_0_C0_C3_7), None);
        assert_eq!(cpu.x[0], 0, "a reserved ID register is RAZ at EL1");
        assert_eq!(exec(&mut cpu, MRS_ID_AA64ISAR2), None);
        assert_eq!(cpu.x[0], if imp.has(Feature::WfxT) { 2 } else { 0 }, "ISAR2.WFxT");
        assert_eq!(exec(&mut cpu, MSR_MIDR), Some(UNDEFINED), "IDST {idst}: writes are not trapped");

        at_el0(&mut cpu);
        assert_eq!(exec(&mut cpu, MRS_MIDR), refused(0x0030_0001), "IDST {idst}");
        assert_eq!(exec(&mut cpu, MRS_ID_AA64ISAR2), refused(0x0034_000D), "IDST {idst}");
        assert_eq!(exec(&mut cpu, MRS_S3_3_C0_C0_0), refused(0x0030_C001), "IDST {idst}");
        assert_eq!(exec(&mut cpu, MRS_SCTLR), Some(UNDEFINED), "IDST {idst}: outside the ID space");
        assert_eq!(exec(&mut cpu, MSR_MIDR), Some(UNDEFINED), "IDST {idst}");
        assert_eq!(exec(&mut cpu, MRS_CTR), None, "IDST {idst}: EL0 may read CTR_EL0 with UCT");
    }
}

/// FEAT_SPECRES: `CFP`, `DVP` and `CPP RCTX` complete at EL1 and, with
/// `SCTLR_EL1.EnRCTX`, at EL0; without it EL0's trap with EC 0x18, the ISS
/// naming the operation as a write. Without FEAT_SPECRES they are
/// UNDEFINED.
#[test]
fn the_prediction_restriction_instructions_need_feat_specres_and_enrctx_at_el0() {
    let ops = [(CFP_RCTX, 0x0018_DC06), (DVP_RCTX, 0x001A_DC06), (CPP_RCTX, 0x001E_DC06)];
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for (insn, _) in ops {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL1");
    }
    at_el0(&mut cpu);
    for (insn, iss) in ops {
        assert_eq!(exec(&mut cpu, insn), Some(sysreg_trap(iss)), "{insn:#010x} at EL0");
    }
    cpu.sys.sctlr_el1 |= ENRCTX;
    for (insn, _) in ops {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL0 with EnRCTX");
    }
    let mut v80 = processor(Implementation::CORTEX_A57);
    for (insn, _) in ops {
        assert_eq!(exec(&mut v80, insn), Some(UNDEFINED), "{insn:#010x} without FEAT_SPECRES");
    }
}

/// FEAT_ECV's `CNTPCTSS_EL0` and `CNTVCTSS_EL0` read the count as
/// `CNTPCT_EL0` and `CNTVCT_EL0` do, under the same EL0 enables; without
/// FEAT_ECV they are UNDEFINED.
#[test]
fn the_self_synchronized_counter_views_need_feat_ecv() {
    let views = [(MRS_CNTPCTSS, EL0PCTEN, 0x003A_F801), (MRS_CNTVCTSS, EL0VCTEN, 0x003C_F801)];
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.wait_until(rusty_box_core::VmInstant::from_ticks(12_345));
    for (insn, _, _) in views {
        let count = cpu.counter().get();
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL1");
        assert_eq!(cpu.x[0], count);
    }
    at_el0(&mut cpu);
    for (insn, enable, iss) in views {
        cpu.sys.cntkctl_el1 = 0;
        assert_eq!(exec(&mut cpu, insn), Some(sysreg_trap(iss)), "{insn:#010x} at EL0");
        cpu.sys.cntkctl_el1 = enable;
        let count = cpu.counter().get();
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL0, enabled");
        assert_eq!(cpu.x[0], count);
    }
    let mut v80 = processor(Implementation::CORTEX_A57);
    for (insn, _, _) in views {
        assert_eq!(exec(&mut v80, insn), Some(UNDEFINED), "{insn:#010x} without FEAT_ECV");
    }
}
