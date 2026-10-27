//! FEAT_EL2 as a guest sees it (R9): reset at EL2, the hypervisor call and
//! the trapped secure monitor call, exception routing to EL2
//! (`HCR_EL2.TGE`, `IMO`, `FMO`, `TEA`), exception entry to EL2 and return
//! from it, illegal returns included, EL2's own stage 1 translation regime
//! with its TLB entries kept apart from EL1&0's, the identity EL1 reads
//! through `VPIDR_EL2` and `VMPIDR_EL2`, and the SME instructions' EL2
//! enables (`CPTR_EL2.TSM` and `TFP`, `SMCR_EL2.EZT0` and `LEN`). Which
//! register accesses trap where is `sysreg_spec.rs`'s, against Arm's
//! accessors.
//!
//! The rules are the Arm ARM's pseudocode: `AArch64_TakeReset`,
//! `AArch64_CallHypervisor`, `AArch64_CheckForSMCUndefOrTrap`,
//! `AArch64_TakeException`, `AArch64_TakePhysicalIRQException` and its FIQ
//! sibling, `AArch64_ExceptionReturn` with `IllegalExceptionReturn`,
//! `AArch64_Undefined`, `AArch64_CallSupervisor`, `AArch64_DataAbort`,
//! `AArch64_S1Translate` with `AArch64_S1DirectBasePermissions` for a regime
//! of one privilege level, `AArch64_AT`, `CheckSMEEnabled`,
//! `CheckSMEZT0Enabled`, `SMEAccessTrap` and `CurrentSVL`.
//!
//! The words are GNU as's (2.42, `-march=armv8.2-a`, objdump -d), the SME
//! ones with `-march=armv9.4-a+sme2`.

use std::vec::Vec;

use rusty_box_core::{ClockHz, SnapError, VmClock};

use super::el2::{
    CPTR_TFP, CPTR_TSM, HCR_AMO, HCR_DC, HCR_FMO, HCR_HCD, HCR_IMO, HCR_TEA, HCR_TGE, HCR_TSC, HCR_VF, HCR_VI, HCR_VSE,
    MDCR_TDE,
};
use super::features::{Feature, Features, Implementation};
use super::sysreg::{CPACR_FPEN, CPACR_SMEN, SMCR_EZT0};
use super::{id, Cpu, CpuConfig, El, Levels, NoEl2, Pstate, Step, Tlb, WithEl2};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::{ExceptionKind, NoInstrumentation};

const HVC: u32 = 0xD402_4682; // hvc #0x1234
const SMC: u32 = 0xD400_0EE3; // smc #0x77
const SVC: u32 = 0xD400_00A1; // svc #0x5
const UDF: u32 = 0x0000_0000; // udf #0
const ERET: u32 = 0xD69F_03E0;
const NOP: u32 = 0xD503_201F;
const MRS_MIDR: u32 = 0xD538_0000; // mrs x0, midr_el1
const MRS_MPIDR: u32 = 0xD538_00A0; // mrs x0, mpidr_el1
const LDR: u32 = 0xF940_0022; // ldr x2, [x1]
const STR: u32 = 0xF900_0022; // str x2, [x1]
const BR_X1: u32 = 0xD61F_0020; // br x1
const TLBI_ALLE2: u32 = 0xD50C_871F;
const AT_S1E2R: u32 = 0xD50C_7800; // at s1e2r, x0
const AT_S1E2W: u32 = 0xD50C_7820; // at s1e2w, x0

/// Where the instruction under test executes, the vector bases, and the
/// end of RAM: an access at or above [`RAM_BYTES`] is to no memory.
const CODE: u64 = 0x4_0000;
const VBAR_EL1: u64 = 0x8000;
const VBAR_EL2: u64 = 0xA000;
const RAM_BYTES: usize = 0x100_0000;

/// Exception classes.
const EC_UNKNOWN: u64 = 0x00;
const EC_ILLEGAL_STATE: u64 = 0x0E;
const EC_SVC: u64 = 0x15;
const EC_HVC: u64 = 0x16;
const EC_SMC: u64 = 0x17;
const EC_INSTRUCTION_ABORT_SAME_EL: u64 = 0x21;
const EC_DATA_ABORT_LOWER_EL: u64 = 0x24;
const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
/// `ESR_ELx.IL`: a 32-bit instruction.
const IL: u64 = 1 << 25;
/// DFSC/IFSC values and `WnR`.
const SEA: u64 = 0b01_0000;
const TRANSLATION_L0: u64 = 0b00_0100;
const PERMISSION_L2: u64 = 0b00_1110;
const WNR: u64 = 1 << 6;

/// The vector offsets: from the current level with `SP_ELx`, and from a
/// lower level in AArch64; synchronous, IRQ, FIQ.
const CURRENT_SPX: u64 = 0x200;
const LOWER_A64: u64 = 0x400;
const IRQ: u64 = 0x80;
const FIQ: u64 = 0x100;

/// `PSTATE.M` encodings.
const EL0T: u64 = 0b0000;
const EL1H: u64 = 0b0101;
const EL2T: u64 = 0b1000;
const EL2H: u64 = 0b1001;

struct Ram(Vec<u8>);

impl PhysBus for Ram {
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, _clock: VmClock) -> Result<(), BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get_mut(a..a + n).ok_or(BusFault::Unmapped)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

impl Ram {
    fn put64(&mut self, at: u64, value: u64) {
        let a = at as usize;
        self.0[a..a + 8].copy_from_slice(&value.to_le_bytes());
    }
}

/// An exception the step took: the level, the vector offset from that
/// level's `VBAR`, and its `ESR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    to: El,
    offset: u64,
    esr: u64,
}

struct Machine<L: Levels> {
    cpu: Cpu<L>,
    ram: Ram,
}

impl<L: Levels> Machine<L> {
    /// A processor of the levels `L` implementing `imp`, out of reset, its
    /// vector bases set.
    fn new(imp: Implementation) -> Self {
        let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
        let mut cpu = Cpu::<L>::implementing(
            CpuConfig {
                ips,
                counter_hz: ips,
                reset_pc: CODE,
            },
            imp,
        );
        cpu.sys.vbar_el1 = VBAR_EL1;
        if let Some(el2) = cpu.el2_mut() {
            el2.vbar = VBAR_EL2;
        }
        Self {
            cpu,
            ram: Ram(std::vec![0; RAM_BYTES]),
        }
    }

    /// Execute at `el` from [`CODE`] with `SP_ELx` selected (`SP_EL0` at
    /// EL0), every interrupt masked.
    fn at(&mut self, el: El) {
        self.cpu.pstate = Pstate {
            el,
            sp_sel: el != El::El0,
            ..Pstate::RESET
        };
        self.cpu.pc = CODE;
    }

    /// Step `insn` at [`CODE`]: the exception it took, if any.
    fn run(&mut self, insn: u32) -> Option<Taken> {
        self.run_at(CODE, insn)
    }

    fn run_at(&mut self, at: u64, insn: u32) -> Option<Taken> {
        let a = at as usize;
        self.ram.0[a..a + 4].copy_from_slice(&insn.to_le_bytes());
        self.cpu.pc = at;
        assert_eq!(self.cpu.step(&mut self.ram, &mut NoInstrumentation), Step::Retired, "{insn:#010x}");
        self.taken()
    }

    /// The exception the processor is now at the vector of, if it is at one.
    fn taken(&self) -> Option<Taken> {
        let pc = self.cpu.pc;
        let (to, vbar, esr) = match self.cpu.pstate.el {
            El::El2 => (El::El2, VBAR_EL2, self.cpu.el2().esr),
            El::El1 => (El::El1, VBAR_EL1, self.cpu.sys.esr_el1),
            El::El0 => return None,
        };
        (pc & !0x7FF == vbar).then_some(Taken {
            to,
            offset: pc - vbar,
            esr,
        })
    }
}

impl Machine<WithEl2> {
    fn with_el2() -> Self {
        Self::new(Implementation::CORTEX_A57)
    }

    fn hcr(&mut self, bits: u64) {
        self.cpu.el2_state.regs.hcr |= bits;
    }
}

fn sync(to: El, offset: u64, ec: u64, iss: u64) -> Option<Taken> {
    Some(Taken {
        to,
        offset,
        esr: (ec << 26) | IL | iss,
    })
}

/// A processor with EL2 comes out of reset at EL2 with `SP_EL2` selected,
/// one without at EL1 with `SP_EL1` (`AArch64_TakeReset`: the highest
/// implemented level, `PSTATE.SP` 1), every interrupt masked either way.
#[test]
fn reset_is_at_the_highest_exception_level() {
    /// `PSTATE` and the PC of a processor of the levels `L` out of reset.
    fn reset<L: Levels>(imp: Implementation) -> (Pstate, u64) {
        let ips = ClockHz::BOCHS_DEFAULT;
        let cpu = Cpu::<L>::implementing(
            CpuConfig {
                ips,
                counter_hz: ips,
                reset_pc: CODE,
            },
            imp,
        );
        (cpu.pstate, cpu.pc)
    }
    for ((p, pc), el) in [
        (reset::<WithEl2>(Implementation::CORTEX_A57), El::El2),
        (reset::<NoEl2>(Implementation::CORTEX_A57), El::El1),
        (reset::<WithEl2>(Implementation::EVERY_FEATURE), El::El2),
        (reset::<NoEl2>(Implementation::EVERY_FEATURE), El::El1),
    ] {
        assert_eq!((p.el, p.sp_sel), (el, true));
        assert!(p.d && p.a && p.i && p.f);
        assert_eq!(pc, CODE);
    }
}

/// At EL2 with `PSTATE.SP` set the stack pointer is `SP_EL2`, a register of
/// its own (Arm `SP[]`): writing it leaves `SP_EL1` as it was, as `MRS
/// SP_EL1` at EL2 reads it, and an exception from EL1 to EL2 runs on it.
#[test]
fn el2h_runs_on_sp_el2() {
    const MOV_X0_SP: u32 = 0x9100_03E0; // mov x0, sp
    const MOV_SP_X1: u32 = 0x9100_003F; // mov sp, x1
    const MRS_SP_EL1: u32 = 0xD53C_4100; // mrs x0, sp_el1
    let mut m = Machine::with_el2();
    m.cpu.sp_el1 = 0x1_1000;
    m.cpu.el2_state.sp_el2 = 0x2_2000;
    m.at(El::El2);
    assert_eq!(m.run(MOV_X0_SP), None);
    assert_eq!(m.cpu.x[0], 0x2_2000);
    m.cpu.x[1] = 0x3_3000;
    assert_eq!(m.run(MOV_SP_X1), None);
    assert_eq!(m.run(MOV_X0_SP), None);
    assert_eq!(m.cpu.x[0], 0x3_3000);
    assert_eq!(m.run(MRS_SP_EL1), None);
    assert_eq!(m.cpu.x[0], 0x1_1000, "SP_EL1 as it was");

    m.at(El::El1);
    assert_eq!(m.run(MOV_X0_SP), None);
    assert_eq!(m.cpu.x[0], 0x1_1000);
    assert_eq!(m.run(HVC), sync(El::El2, LOWER_A64, EC_HVC, 0x1234));
    let vector = (VBAR_EL2 + LOWER_A64) as usize;
    m.ram.0[vector..vector + 4].copy_from_slice(&MOV_X0_SP.to_le_bytes());
    assert_eq!(m.cpu.step(&mut m.ram, &mut NoInstrumentation), Step::Retired);
    assert_eq!(m.cpu.x[0], 0x3_3000, "the vector runs on SP_EL2");
}

/// `HVC` from EL1 or EL2 takes an exception to EL2 with class 0x16 and the
/// immediate as its ISS, returning to the next instruction, from EL1
/// through the lower-level vector and at EL2 through the current-level one;
/// `HCR_EL2.HCD` makes it UNDEFINED, and at EL0 it always is.
#[test]
fn hvc_calls_the_hypervisor_unless_hcd_disables_it() {
    let mut m = Machine::with_el2();
    m.at(El::El1);
    m.cpu.pstate.n = true;
    let spsr = m.cpu.pstate.to_spsr();
    assert_eq!(m.run(HVC), sync(El::El2, LOWER_A64, EC_HVC, 0x1234));
    assert_eq!(m.cpu.el2_state.regs.elr, CODE + 4, "returns to the next instruction");
    assert_eq!(m.cpu.el2_state.regs.spsr, spsr);
    assert_eq!(spsr & 0x1F, EL1H);
    assert!(m.cpu.pstate.d && m.cpu.pstate.a && m.cpu.pstate.i && m.cpu.pstate.f);
    assert!(m.cpu.pstate.sp_sel);

    m.at(El::El2);
    assert_eq!(m.run(HVC), sync(El::El2, CURRENT_SPX, EC_HVC, 0x1234));
    assert_eq!(m.cpu.el2_state.regs.spsr & 0x1F, EL2H);

    m.hcr(HCR_HCD);
    m.at(El::El1);
    assert_eq!(m.run(HVC), sync(El::El1, CURRENT_SPX, EC_UNKNOWN, 0));
    m.at(El::El2);
    assert_eq!(m.run(HVC), sync(El::El2, CURRENT_SPX, EC_UNKNOWN, 0));

    let mut m = Machine::with_el2();
    m.at(El::El0);
    assert_eq!(m.run(HVC), sync(El::El1, LOWER_A64, EC_UNKNOWN, 0));
    m.hcr(HCR_TGE);
    m.at(El::El0);
    assert_eq!(m.run(HVC), sync(El::El2, LOWER_A64, EC_UNKNOWN, 0));
}

/// With EL2, `SMC` at EL1 under `HCR_EL2.TSC` traps to EL2 with class 0x17,
/// returning to the `SMC` itself; otherwise, at EL1 or EL2, it is the PSCI
/// call the machine's firmware answers. At EL0 it is UNDEFINED. Without
/// EL2, `HVC` is the firmware call and `SMC` is UNDEFINED.
#[test]
fn smc_is_the_firmware_call_with_el2_unless_tsc_traps_it() {
    const PSCI_VERSION: u64 = 0x8400_0000;
    const VERSION_1_1: u64 = 0x0001_0001;
    let mut m = Machine::with_el2();
    for el in [El::El1, El::El2] {
        m.at(el);
        m.cpu.x[0] = PSCI_VERSION;
        assert_eq!(m.run(SMC), None, "{el:?}");
        assert_eq!((m.cpu.x[0], m.cpu.pc), (VERSION_1_1, CODE + 4), "{el:?}");
    }
    m.hcr(HCR_TSC);
    m.at(El::El1);
    assert_eq!(m.run(SMC), sync(El::El2, LOWER_A64, EC_SMC, 0x77));
    assert_eq!(m.cpu.el2_state.regs.elr, CODE, "returns to the SMC");
    m.at(El::El2);
    m.cpu.x[0] = PSCI_VERSION;
    assert_eq!(m.run(SMC), None, "TSC traps EL1's alone");
    m.at(El::El0);
    assert_eq!(m.run(SMC), sync(El::El1, LOWER_A64, EC_UNKNOWN, 0));

    let mut v80 = Machine::<NoEl2>::new(Implementation::CORTEX_A57);
    v80.at(El::El1);
    assert_eq!(v80.run(SMC), sync(El::El1, CURRENT_SPX, EC_UNKNOWN, 0));
    v80.cpu.x[0] = PSCI_VERSION;
    assert_eq!(v80.run(HVC), None);
    assert_eq!(v80.cpu.x[0], VERSION_1_1);
}

/// `ERET` at EL2 returns to EL2, EL1 or EL0 as `SPSR_EL2.M` says, with the
/// stack pointer and masks it names, at `ELR_EL2`. A return to EL3, to
/// AArch32, with `M[1]` set, to EL1 under `HCR_EL2.TGE`, or from EL1 to
/// EL2, is illegal: the level and stack pointer stay, `PSTATE.IL` is set,
/// and the next instruction takes an Illegal Execution State exception.
#[test]
fn eret_from_el2_returns_to_each_level_and_refuses_illegal_targets() {
    const TARGET: u64 = 0x5_0000;
    let mut m = Machine::with_el2();
    for (m_field, el, sp_sel) in [(EL1H, El::El1, true), (EL0T, El::El0, false), (EL2T, El::El2, false), (EL2H, El::El2, true)] {
        m.at(El::El2);
        m.cpu.el2_state.regs.spsr = m_field | (1 << 7) | (1 << 31);
        m.cpu.el2_state.regs.elr = TARGET;
        assert_eq!(m.run(ERET), None, "M {m_field:#06b}");
        let p = m.cpu.pstate;
        assert_eq!((p.el, p.sp_sel, m.cpu.pc), (el, sp_sel, TARGET), "M {m_field:#06b}");
        assert!(p.i && !p.f && p.n && !p.il, "M {m_field:#06b}");
    }

    let illegal = [
        (0b1101, 0),       // EL3h: not implemented
        (EL1H | 0x10, 0),  // AArch32
        (0b0111, 0),       // M[1] set
        (EL1H, HCR_TGE),   // EL1 under TGE (E2H = 0)
    ];
    for (spsr, hcr) in illegal {
        let mut m = Machine::with_el2();
        m.hcr(hcr);
        m.at(El::El2);
        m.cpu.el2_state.regs.spsr = spsr;
        m.cpu.el2_state.regs.elr = TARGET;
        assert_eq!(m.run(ERET), None, "SPSR {spsr:#x}");
        let p = m.cpu.pstate;
        assert_eq!((p.el, p.sp_sel, p.il, m.cpu.pc), (El::El2, true, true, TARGET), "SPSR {spsr:#x}");
        assert_eq!(m.run_at(TARGET, NOP), sync(El::El2, CURRENT_SPX, EC_ILLEGAL_STATE, 0), "SPSR {spsr:#x}");
    }

    let mut m = Machine::with_el2();
    m.at(El::El1);
    m.cpu.sys.spsr_el1 = EL2H;
    m.cpu.sys.elr_el1 = TARGET;
    assert_eq!(m.run(ERET), None);
    assert_eq!((m.cpu.pstate.el, m.cpu.pstate.il), (El::El1, true), "EL1 cannot return to EL2");
    assert_eq!(m.run_at(TARGET, NOP), sync(El::El1, CURRENT_SPX, EC_ILLEGAL_STATE, 0));
}

/// Under `HCR_EL2.TGE`, the exceptions EL0 raises that EL1 would take go to
/// EL2: a supervisor call, an UNDEFINED instruction, a data abort, and
/// `CPACR_EL1`'s SIMD&FP trap, which arrives as an uncategorized exception
/// (`AArch64_AdvSIMDFPAccessTrap`). Without it they go to EL1.
#[test]
fn tge_takes_el0s_exceptions_to_el2() {
    const NOWHERE: u64 = RAM_BYTES as u64;
    const FMOV: u32 = 0x1E60_4020; // fmov d0, d1
    const EC_FP_ACCESS: u64 = 0x07;
    const CV_COND_AL: u64 = (1 << 24) | (0xE << 20);
    for (tge, to) in [(0, El::El1), (HCR_TGE, El::El2)] {
        let mut m = Machine::with_el2();
        m.hcr(tge);
        m.at(El::El0);
        let fp_trap = if tge == 0 {
            sync(to, LOWER_A64, EC_FP_ACCESS, CV_COND_AL)
        } else {
            sync(to, LOWER_A64, EC_UNKNOWN, 0)
        };
        assert_eq!(m.run(FMOV), fp_trap, "CPACR_EL1.FPEN 00, TGE {tge}");
        m.at(El::El0);
        assert_eq!(m.run(SVC), sync(to, LOWER_A64, EC_SVC, 5), "TGE {tge}");
        m.at(El::El0);
        assert_eq!(m.run(UDF), sync(to, LOWER_A64, EC_UNKNOWN, 0), "TGE {tge}");
        m.at(El::El0);
        m.cpu.x[1] = NOWHERE;
        assert_eq!(m.run(LDR), sync(to, LOWER_A64, EC_DATA_ABORT_LOWER_EL, SEA), "TGE {tge}");
        let far = if to == El::El2 { m.cpu.el2_state.regs.far } else { m.cpu.sys.far_el1 };
        assert_eq!(far, NOWHERE);
    }
}

/// `MDCR_EL2.TDE` takes a `BRK` at EL0 or EL1 to EL2, as `HCR_EL2.TGE`
/// does EL0's (Arm `AArch64_SoftwareBreakpoint`); at EL2 it is EL2's own.
#[test]
fn tde_takes_breakpoint_instructions_to_el2() {
    const BRK: u32 = 0xD420_00E0; // brk #0x7
    const EC_BRK: u64 = 0x3C;
    for (el, tge, tde, want) in [
        (El::El1, 0, 0, sync(El::El1, CURRENT_SPX, EC_BRK, 7)),
        (El::El0, 0, 0, sync(El::El1, LOWER_A64, EC_BRK, 7)),
        (El::El1, 0, MDCR_TDE, sync(El::El2, LOWER_A64, EC_BRK, 7)),
        (El::El0, 0, MDCR_TDE, sync(El::El2, LOWER_A64, EC_BRK, 7)),
        (El::El0, HCR_TGE, 0, sync(El::El2, LOWER_A64, EC_BRK, 7)),
        (El::El2, 0, MDCR_TDE, sync(El::El2, CURRENT_SPX, EC_BRK, 7)),
    ] {
        let mut m = Machine::with_el2();
        m.hcr(tge);
        m.cpu.el2_state.regs.mdcr |= tde;
        m.at(el);
        assert_eq!(m.run(BRK), want, "{el:?} TGE {tge:#x} TDE {tde:#x}");
    }
}

/// `HCR_EL2.TEA` takes EL1's synchronous External aborts to EL2, with the
/// faulting address in `FAR_EL2`; EL1's other exceptions stay at EL1.
/// `HCR_EL2.TGE` alone leaves EL1's at EL1: Arm `SyncExternalAbortTarget`
/// takes them to EL2 under `TGE` from EL0 only.
#[test]
fn tea_takes_el1s_external_aborts_to_el2() {
    const NOWHERE: u64 = RAM_BYTES as u64 + 0x40;
    let mut m = Machine::with_el2();
    m.at(El::El1);
    m.cpu.x[1] = NOWHERE;
    assert_eq!(m.run(LDR), sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, SEA));
    m.hcr(HCR_TEA);
    m.at(El::El1);
    assert_eq!(m.run(LDR), sync(El::El2, LOWER_A64, EC_DATA_ABORT_LOWER_EL, SEA));
    assert_eq!(m.cpu.el2_state.regs.far, NOWHERE);
    m.at(El::El1);
    assert_eq!(m.run(SVC), sync(El::El1, CURRENT_SPX, EC_SVC, 5));

    let mut m = Machine::with_el2();
    m.hcr(HCR_TGE);
    m.at(El::El1);
    m.cpu.x[1] = NOWHERE;
    assert_eq!(m.run(LDR), sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, SEA), "TGE, from EL1");
}

/// A physical IRQ or FIQ goes where `HCR_EL2.IMO` or `FMO` (or `TGE`) routes
/// it: routed to EL2, EL0 and EL1 take it whatever `PSTATE` masks, through
/// the lower-level vector, and EL2 takes it when its own mask is clear;
/// left to EL1, EL1 takes it when unmasked and EL2 never does. A host's
/// `take_irq` or `take_fiq` overrides `PSTATE`'s mask and nothing else.
#[test]
fn physical_interrupts_go_where_imo_and_fmo_route_them() {
    for (line, routing, vector) in [(0, HCR_IMO, IRQ), (1, HCR_FMO, FIQ)] {
        let assert_line = |m: &mut Machine<WithEl2>| m.cpu.set_interrupt_lines(line == 0, line == 1);
        let take = |m: &mut Machine<WithEl2>| if line == 0 { m.cpu.take_irq() } else { m.cpu.take_fiq() };
        let mut m = Machine::with_el2();
        assert_line(&mut m);

        m.at(El::El1);
        assert!(!m.cpu.interrupt_deliverable(), "masked at EL1");
        m.cpu.pstate.i = false;
        m.cpu.pstate.f = false;
        let taken = m.cpu.deliver_interrupt().expect("unmasked at EL1");
        assert_eq!((taken.to, taken.vector), (El::El1, VBAR_EL1 + CURRENT_SPX + vector));

        m.at(El::El2);
        m.cpu.pstate.i = false;
        m.cpu.pstate.f = false;
        assert!(!m.cpu.interrupt_deliverable(), "an interrupt for EL1 is not taken at EL2");
        assert!(take(&mut m).is_none(), "not even by a host");
        assert_eq!((m.cpu.pstate.el, m.cpu.pc), (El::El2, CODE));

        m.hcr(routing);
        m.at(El::El1);
        let taken = m.cpu.deliver_interrupt().expect("routed to EL2, PSTATE masks it not");
        assert_eq!((taken.to, taken.vector), (El::El2, VBAR_EL2 + LOWER_A64 + vector));
        assert_eq!(m.cpu.el2_state.regs.elr, CODE);

        m.at(El::El2);
        assert!(!m.cpu.interrupt_deliverable(), "EL2's own mask");
        let taken = take(&mut m).expect("a host takes it whatever PSTATE masks");
        assert_eq!((taken.to, taken.vector), (El::El2, VBAR_EL2 + CURRENT_SPX + vector));

        m.at(El::El2);
        m.cpu.pstate.i = false;
        m.cpu.pstate.f = false;
        let taken = m.cpu.deliver_interrupt().expect("unmasked at EL2");
        assert_eq!((taken.to, taken.vector), (El::El2, VBAR_EL2 + CURRENT_SPX + vector));

        let mut m = Machine::with_el2();
        assert_line(&mut m);
        m.hcr(HCR_TGE);
        m.at(El::El0);
        let taken = m.cpu.deliver_interrupt().expect("TGE routes it to EL2");
        assert_eq!((taken.to, taken.vector), (El::El2, VBAR_EL2 + LOWER_A64 + vector));
    }
}

/// `ISR_EL1` read at EL1 shows an interrupt's virtual kind where
/// `HCR_EL2.IMO`, `FMO` or `AMO` (or `TGE`, all three) routes its physical
/// kind to EL2, pending with `HCR_EL2.VI`, `VF` or `VSE` unless `TGE` is
/// set, and its physical kind elsewhere; EL2 reads the physical lines.
#[test]
fn isr_shows_the_virtual_interrupts_where_el2_takes_the_physical_ones() {
    const MRS_ISR: u32 = 0xD538_C100; // mrs x0, isr_el1
    const A: u64 = 1 << 8;
    const I: u64 = 1 << 7;
    const F: u64 = 1 << 6;
    const ROUTED: u64 = HCR_IMO | HCR_FMO | HCR_AMO;
    const VIRTUAL: u64 = HCR_VI | HCR_VF | HCR_VSE;
    let mut m = Machine::with_el2();
    let reset = m.cpu.el2_state.regs.hcr;
    let isr = |m: &mut Machine<WithEl2>, el: El, hcr: u64| {
        m.cpu.el2_state.regs.hcr = reset | hcr;
        m.at(el);
        assert_eq!(m.run(MRS_ISR), None, "EL{} HCR {hcr:#x}", el.index());
        m.cpu.x[0]
    };
    m.cpu.set_interrupt_lines(true, true);
    assert_eq!(isr(&mut m, El::El1, 0), I | F, "nothing routed: the physical lines");
    assert_eq!(isr(&mut m, El::El1, HCR_IMO), F, "IRQ routed: no virtual IRQ pending");
    assert_eq!(isr(&mut m, El::El1, ROUTED), 0);
    assert_eq!(isr(&mut m, El::El1, ROUTED | VIRTUAL), A | I | F);
    assert_eq!(isr(&mut m, El::El1, HCR_FMO | VIRTUAL), I | F, "VI and VSE show only where routed");
    assert_eq!(isr(&mut m, El::El1, HCR_AMO | HCR_VSE), A | I | F);
    assert_eq!(isr(&mut m, El::El1, HCR_TGE | VIRTUAL), 0, "TGE: virtual, none pending");
    assert_eq!(isr(&mut m, El::El2, ROUTED | VIRTUAL), I | F, "EL2: physical");
    m.cpu.set_interrupt_lines(false, false);
    assert_eq!(isr(&mut m, El::El2, ROUTED | VIRTUAL), 0);
    assert_eq!(isr(&mut m, El::El1, VIRTUAL), 0, "nothing routed: the physical lines");
}

/// `HCR_EL2.VI`, `VF` and `VSE` raise a virtual IRQ, FIQ and SError for EL0
/// and EL1 where `IMO`, `FMO` and `AMO` route the physical kind to EL2 and
/// `TGE` is clear. Each is taken to EL1 when its `PSTATE` mask is clear and
/// never at EL2, after a deliverable physical interrupt, IRQ before FIQ
/// before SError (Arm leaves the order IMPLEMENTATION DEFINED). A
/// virtual SError reports EC 0x2F with `VSESR_EL2`'s syndrome (FEAT_RAS)
/// and stops being pending when taken; a virtual IRQ stays pending until
/// EL2 clears `VI`. A `WFI` with one pending, masked or not, does not wait.
#[test]
fn virtual_interrupts_are_taken_at_el1() {
    const SERROR: u64 = 0x180;
    const EC_SERROR: u64 = 0x2F;
    const WFI: u32 = 0xD503_207F;
    let mut m = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    let reset = m.cpu.el2_state.regs.hcr;
    let unmasked_at = |m: &mut Machine<WithEl2>, el: El, hcr: u64| {
        m.cpu.el2_state.regs.hcr = reset | hcr;
        m.at(el);
        m.cpu.pstate.i = false;
        m.cpu.pstate.f = false;
        m.cpu.pstate.a = false;
    };
    unmasked_at(&mut m, El::El1, HCR_VI);
    assert!(m.cpu.deliver_interrupt().is_none(), "IMO clear");
    unmasked_at(&mut m, El::El1, HCR_VI | HCR_IMO | HCR_TGE);
    assert!(m.cpu.deliver_interrupt().is_none(), "TGE set");
    unmasked_at(&mut m, El::El2, HCR_VI | HCR_IMO);
    assert!(m.cpu.deliver_interrupt().is_none(), "at EL2");
    unmasked_at(&mut m, El::El1, HCR_VI | HCR_IMO);
    m.cpu.pstate.i = true;
    assert!(m.cpu.deliver_interrupt().is_none(), "PSTATE.I set");
    assert_eq!((m.run(WFI), m.cpu.pstate.el), (None, El::El1), "WFI does not wait");

    for (el, offset) in [(El::El1, CURRENT_SPX), (El::El0, LOWER_A64)] {
        unmasked_at(&mut m, el, HCR_VI | HCR_IMO | HCR_VF | HCR_FMO);
        let taken = m.cpu.deliver_interrupt().expect("a virtual IRQ");
        let want = (ExceptionKind::VirtualIrq, El::El1, VBAR_EL1 + offset + IRQ);
        assert_eq!((taken.kind, taken.to, taken.vector), want, "{el:?}");
        assert_eq!(m.cpu.el2_state.regs.hcr & HCR_VI, HCR_VI, "VI stays set");
        unmasked_at(&mut m, el, HCR_VF | HCR_FMO);
        let taken = m.cpu.deliver_interrupt().expect("a virtual FIQ");
        assert_eq!((taken.kind, taken.vector), (ExceptionKind::VirtualFiq, VBAR_EL1 + offset + FIQ));
    }

    unmasked_at(&mut m, El::El1, HCR_VI | HCR_IMO);
    m.cpu.set_interrupt_lines(true, false);
    let taken = m.cpu.deliver_interrupt().expect("the physical IRQ first");
    assert_eq!((taken.kind, taken.to), (ExceptionKind::Irq, El::El2));
    m.cpu.set_interrupt_lines(false, false);

    m.cpu.el2_state.regs.vsesr = 0x0100_0ABC;
    unmasked_at(&mut m, El::El1, HCR_VSE | HCR_AMO);
    let taken = m.cpu.deliver_interrupt().expect("a virtual SError");
    assert_eq!((taken.kind, taken.vector), (ExceptionKind::VirtualSError, VBAR_EL1 + CURRENT_SPX + SERROR));
    assert_eq!(m.cpu.sys.esr_el1, (EC_SERROR << 26) | IL | 0x0100_0ABC);
    assert_eq!(m.cpu.el2_state.regs.hcr & HCR_VSE, 0, "taking it clears VSE");
    assert!(m.cpu.deliver_interrupt().is_none());

    // All three raised and unmasked: taken in the order chosen for the
    // IMPLEMENTATION DEFINED priority, each lowered once taken.
    let mut hcr = HCR_VI | HCR_IMO | HCR_VF | HCR_FMO | HCR_VSE | HCR_AMO;
    for (kind, raised) in [
        (ExceptionKind::VirtualIrq, HCR_VI),
        (ExceptionKind::VirtualFiq, HCR_VF),
        (ExceptionKind::VirtualSError, HCR_VSE),
    ] {
        unmasked_at(&mut m, El::El1, hcr);
        let taken = m.cpu.deliver_interrupt().expect("a virtual interrupt");
        assert_eq!(taken.kind, kind);
        hcr &= !raised;
    }

    let mut v80 = Machine::with_el2();
    v80.cpu.el2_state.regs.hcr |= HCR_VSE | HCR_AMO;
    v80.at(El::El1);
    v80.cpu.pstate.a = false;
    v80.cpu.deliver_interrupt().expect("a virtual SError");
    assert_eq!(v80.cpu.sys.esr_el1, (EC_SERROR << 26) | IL, "no syndrome without FEAT_RAS");
}

/// ESB at EL0 or EL1 defers a virtual SError that `PSTATE.A` masks into
/// `VDISR_EL2` (`A`, with `VSESR_EL2`'s syndrome), which EL1 then reads as
/// `DISR_EL1` under `HCR_EL2.AMO`, and it is no longer pending; at EL2 the
/// virtual SError is none of ESB's.
#[test]
fn esb_defers_a_masked_virtual_serror() {
    const ESB: u32 = 0xD503_221F;
    const MRS_DISR: u32 = 0xD538_C120; // mrs x0, disr_el1
    let mut m = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    m.cpu.el2_state.regs.vsesr = 0x123;
    m.hcr(HCR_VSE | HCR_AMO);
    m.at(El::El1);
    assert!(m.cpu.pstate.a);
    assert_eq!(m.run(ESB), None);
    assert_eq!(m.cpu.el2_state.regs.vdisr, (1 << 31) | 0x123);
    assert_eq!(m.cpu.el2_state.regs.hcr & HCR_VSE, 0);
    assert_eq!((m.run(MRS_DISR), m.cpu.x[0]), (None, (1 << 31) | 0x123));

    m.cpu.el2_state.regs.vdisr = 0;
    assert_eq!(m.run(ESB), None);
    assert_eq!(m.cpu.el2_state.regs.vdisr, 0, "nothing pending, nothing deferred");

    m.hcr(HCR_VSE);
    m.at(El::El2);
    assert_eq!(m.run(ESB), None);
    assert_eq!((m.cpu.el2_state.regs.vdisr, m.cpu.el2_state.regs.hcr & HCR_VSE), (0, HCR_VSE));
}

/// EL1 reads `VPIDR_EL2` and `VMPIDR_EL2` as `MIDR_EL1` and `MPIDR_EL1`;
/// EL2 reads the processor's own, and out of reset the stand-ins are those.
#[test]
fn el1_reads_the_identity_el2_gives_it() {
    let mut m = Machine::with_el2();
    m.at(El::El1);
    assert_eq!((m.run(MRS_MIDR), m.cpu.x[0]), (None, id::MIDR_EL1));
    assert_eq!((m.run(MRS_MPIDR), m.cpu.x[0]), (None, id::MPIDR_EL1));
    m.cpu.el2_state.regs.vpidr = 0x4100_1234;
    m.cpu.el2_state.regs.vmpidr = 0x8000_0003;
    assert_eq!((m.run(MRS_MIDR), m.cpu.x[0]), (None, 0x4100_1234));
    assert_eq!((m.run(MRS_MPIDR), m.cpu.x[0]), (None, 0x8000_0003));
    m.at(El::El2);
    assert_eq!((m.run(MRS_MIDR), m.cpu.x[0]), (None, id::MIDR_EL1));
    assert_eq!((m.run(MRS_MPIDR), m.cpu.x[0]), (None, id::MPIDR_EL1));
}

// ----- SME under EL2 -----

/// `zero {zt0}`, which takes `CheckSMEEnabled` and then
/// `CheckSMEZT0Enabled`.
const ZERO_ZT0: u32 = 0xC048_0001;
/// `rdsvl x0, #1`: the Streaming SVE vector length in bytes, in or out of
/// Streaming SVE mode.
const RDSVL_X0: u32 = 0x04BF_5820;
/// `cntb x0`: the bytes of a vector at the current length, which in
/// Streaming SVE mode is the Streaming SVE vector length.
const CNTB_X0: u32 = 0x0420_E3E0;
const MSR_SMCR_EL1_X1: u32 = 0xD518_12C1; // msr smcr_el1, x1
const MSR_SMCR_EL2_X1: u32 = 0xD51C_12C1; // msr smcr_el2, x1
const EC_FP_ACCESS: u64 = 0x07;
const EC_SME: u64 = 0x1D;
/// The SIMD&FP access trap's ISS: `CV` set, `COND` 0b1110.
const FP_ACCESS_ISS: u64 = 0x01E0_0000;
/// `ESR_ELx.ISS.SMTC` of the SME traps: disabled, ZA storage inactive, ZT0
/// inaccessible.
const SMTC_DISABLED: u64 = 0;
const SMTC_INACTIVE_ZA: u64 = 3;
const SMTC_INACCESSIBLE_ZT0: u64 = 4;

/// A processor with EL2 and every feature, FEAT_SME2 among them, with
/// `CPACR_EL1` enabling SME and SIMD&FP, `CPTR_EL2` trapping neither, and
/// ZA storage enabled.
fn sme_under_el2() -> Machine<WithEl2> {
    let mut m = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    m.cpu.sys.cpacr_el1 = CPACR_SMEN | CPACR_FPEN;
    m.cpu.set_pstate_za(true);
    m
}

/// Arm `CheckSMEZT0Enabled`: with ZA storage enabled, `SMCR_EL1.EZT0` at
/// EL0 and EL1 traps to EL1 (to EL2 from EL0 under `HCR_EL2.TGE`), then
/// `SMCR_EL2.EZT0` at EL0, EL1 and EL2 traps to EL2; with ZA storage
/// disabled, the inactive-ZA trap to the current level comes first.
#[test]
fn zt0_is_trapped_by_each_levels_ezt0() {
    let mut m = sme_under_el2();
    let zt0 = |m: &mut Machine<WithEl2>, el: El, el1: bool, el2: bool| {
        m.cpu.sys.smcr_el1 = if el1 { SMCR_EZT0 } else { 0 };
        m.cpu.el2_state.regs.smcr = if el2 { SMCR_EZT0 } else { 0 };
        m.at(el);
        m.run(ZERO_ZT0)
    };
    assert_eq!(zt0(&mut m, El::El1, true, true), None);
    assert_eq!(zt0(&mut m, El::El1, true, false), sync(El::El2, LOWER_A64, EC_SME, SMTC_INACCESSIBLE_ZT0));
    assert_eq!(zt0(&mut m, El::El1, false, true), sync(El::El1, CURRENT_SPX, EC_SME, SMTC_INACCESSIBLE_ZT0));
    assert_eq!(zt0(&mut m, El::El1, false, false), sync(El::El1, CURRENT_SPX, EC_SME, SMTC_INACCESSIBLE_ZT0));
    // At EL2 `SMCR_EL1` does not apply.
    assert_eq!(zt0(&mut m, El::El2, false, true), None);
    assert_eq!(zt0(&mut m, El::El2, true, false), sync(El::El2, CURRENT_SPX, EC_SME, SMTC_INACCESSIBLE_ZT0));
    // `HCR_EL2.TGE` takes EL0's trap to EL1 to EL2.
    m.hcr(HCR_TGE);
    assert_eq!(zt0(&mut m, El::El0, false, true), sync(El::El2, LOWER_A64, EC_SME, SMTC_INACCESSIBLE_ZT0));
    m.cpu.set_pstate_za(false);
    assert_eq!(zt0(&mut m, El::El0, false, false), sync(El::El2, LOWER_A64, EC_SME, SMTC_INACTIVE_ZA));
    m.cpu.el2_state.regs.hcr &= !HCR_TGE;
    assert_eq!(zt0(&mut m, El::El1, false, false), sync(El::El1, CURRENT_SPX, EC_SME, SMTC_INACTIVE_ZA));
    assert_eq!(zt0(&mut m, El::El2, false, false), sync(El::El2, CURRENT_SPX, EC_SME, SMTC_INACTIVE_ZA));
}

/// Arm `CheckSMEEnabled` on an SME instruction: `CPACR_EL1.SMEN`, then
/// `CPACR_EL1.FPEN`, each to EL1; then `CPTR_EL2.TSM`, then `CPTR_EL2.TFP`,
/// each to EL2, which they trap at EL2 too.
#[test]
fn an_sme_instruction_takes_cptr_el2s_traps_after_cpacr_el1s() {
    let mut m = sme_under_el2();
    m.cpu.sys.smcr_el1 = SMCR_EZT0;
    m.cpu.el2_state.regs.smcr = SMCR_EZT0;
    let run = |m: &mut Machine<WithEl2>, el: El, cpacr: u64, cptr: u64| {
        m.cpu.sys.cpacr_el1 = cpacr;
        m.cpu.el2_state.regs.cptr = cptr;
        m.at(el);
        m.run(ZERO_ZT0)
    };
    let enabled = CPACR_SMEN | CPACR_FPEN;
    assert_eq!(run(&mut m, El::El1, enabled, CPTR_TSM), sync(El::El2, LOWER_A64, EC_SME, SMTC_DISABLED));
    assert_eq!(run(&mut m, El::El1, enabled, CPTR_TFP), sync(El::El2, LOWER_A64, EC_FP_ACCESS, FP_ACCESS_ISS));
    assert_eq!(run(&mut m, El::El1, enabled, CPTR_TSM | CPTR_TFP), sync(El::El2, LOWER_A64, EC_SME, SMTC_DISABLED));
    assert_eq!(run(&mut m, El::El1, CPACR_FPEN, CPTR_TSM), sync(El::El1, CURRENT_SPX, EC_SME, SMTC_DISABLED));
    assert_eq!(run(&mut m, El::El1, CPACR_SMEN, CPTR_TSM), sync(El::El1, CURRENT_SPX, EC_FP_ACCESS, FP_ACCESS_ISS));
    assert_eq!(run(&mut m, El::El2, 0, CPTR_TSM), sync(El::El2, CURRENT_SPX, EC_SME, SMTC_DISABLED));
    assert_eq!(run(&mut m, El::El2, 0, CPTR_TFP), sync(El::El2, CURRENT_SPX, EC_FP_ACCESS, FP_ACCESS_ISS));
    assert_eq!(run(&mut m, El::El2, 0, 0), None);
    // Under `HCR_EL2.TGE`, `CPACR_EL1`'s traps of EL0 go to EL2: SME's as
    // itself (`SMEAccessTrap`), SIMD&FP's as an uncategorized exception
    // (`AArch64_AdvSIMDFPAccessTrap`).
    m.hcr(HCR_TGE);
    assert_eq!(run(&mut m, El::El0, CPACR_FPEN, 0), sync(El::El2, LOWER_A64, EC_SME, SMTC_DISABLED));
    assert_eq!(run(&mut m, El::El0, CPACR_SMEN, 0), sync(El::El2, LOWER_A64, EC_UNKNOWN, 0));
    assert_eq!(run(&mut m, El::El0, enabled, CPTR_TSM), sync(El::El2, LOWER_A64, EC_SME, SMTC_DISABLED));
    assert_eq!(run(&mut m, El::El0, enabled, 0), None);
}

/// Arm `CurrentSVL`, as RDSVL and, in Streaming SVE mode, CNTB read it: at
/// EL1 the shorter of the lengths the `LEN` fields of `SMCR_EL1` and
/// `SMCR_EL2` ask for, at EL2 the one `SMCR_EL2.LEN` asks for, whatever
/// `EZT0` beside either holds.
#[test]
fn the_streaming_vector_length_is_each_registers_len_field() {
    let mut m = sme_under_el2();
    m.cpu.set_pstate_sm(true);
    let msr = |m: &mut Machine<WithEl2>, insn: u32, value: u64| {
        m.at(El::El2);
        m.cpu.x[1] = value;
        assert_eq!(m.run(insn), None, "{insn:#010x} {value:#x}");
    };
    // The bytes of the length RDSVL and CNTB report at `el`.
    let length = |m: &mut Machine<WithEl2>, el: El| {
        m.at(el);
        assert_eq!(m.run(RDSVL_X0), None);
        let rdsvl = m.cpu.x[0];
        m.at(el);
        assert_eq!(m.run(CNTB_X0), None);
        assert_eq!(m.cpu.x[0], rdsvl, "CNTB in Streaming SVE mode at {el:?}");
        rdsvl
    };
    msr(&mut m, MSR_SMCR_EL1_X1, 3);
    msr(&mut m, MSR_SMCR_EL2_X1, SMCR_EZT0 | 1);
    assert_eq!((length(&mut m, El::El1), length(&mut m, El::El2)), (32, 32));
    msr(&mut m, MSR_SMCR_EL1_X1, SMCR_EZT0 | 1);
    msr(&mut m, MSR_SMCR_EL2_X1, 3);
    assert_eq!((length(&mut m, El::El1), length(&mut m, El::El2)), (32, 64));
    msr(&mut m, MSR_SMCR_EL1_X1, SMCR_EZT0 | 3);
    assert_eq!((length(&mut m, El::El1), length(&mut m, El::El2)), (64, 64));
    msr(&mut m, MSR_SMCR_EL2_X1, SMCR_EZT0);
    assert_eq!((length(&mut m, El::El1), length(&mut m, El::El2)), (16, 16));
}

/// An image whose `SMCR_EL2` holds a bit no `MSR` leaves there is refused:
/// `EZT0` is kept with FEAT_SME2 alone, and bits 8:4 never.
#[test]
fn a_snapshot_whose_smcr_el2_holds_a_bit_no_msr_keeps_is_refused() {
    let refused = |imp: Implementation, smcr: u64| {
        let mut saved = Machine::<WithEl2>::new(imp);
        saved.cpu.el2_state.regs.smcr = smcr;
        let mut image = Vec::new();
        saved.cpu.save(&mut image).expect("save");
        Machine::<WithEl2>::new(imp).cpu.restore(&mut image.as_slice()).is_err()
    };
    let without_sme2: Vec<Feature> = Feature::ALL.into_iter().filter(|&f| f != Feature::Sme2).collect();
    let sme_alone = Implementation {
        features: Features::of(&without_sme2),
        max_svl: Implementation::EVERY_FEATURE.max_svl,
    };
    assert!(!refused(Implementation::EVERY_FEATURE, SMCR_EZT0 | 3));
    assert!(refused(Implementation::EVERY_FEATURE, 0x10), "SMCR_EL2 bit 4");
    assert!(!refused(sme_alone, 3));
    assert!(refused(sme_alone, SMCR_EZT0), "SMCR_EL2.EZT0 without FEAT_SME2");
}

// ----- the EL2 translation regime -----

/// The tables: each regime's level 1 table points at a level 2 table of
/// 2 MiB blocks. Both map the code (block 0) to itself, EL2's with AP[1]
/// set, which a regime of one privilege level ignores (in the EL1&0 regime
/// it would make the block EL0-writable, and so never EL1-executable); at
/// `SHARED` the EL2 regime finds `EL2_DATA` and the EL1&0 regime
/// `EL1_DATA`. EL2 alone maps `READ_ONLY` (AP[2] set), `NO_EXECUTE` (XN,
/// bit 54) and `PXN_ONLY` (bit 53, which a regime of one privilege level
/// ignores too).
const EL2_L1: u64 = 0x1_0000;
const EL2_L2: u64 = 0x1_1000;
const EL1_L1: u64 = 0x1_2000;
const EL1_L2: u64 = 0x1_3000;
const SHARED: u64 = 0x20_0000;
const EL2_DATA: u64 = 0x40_0000;
const EL1_DATA: u64 = 0x60_0000;
const READ_ONLY: u64 = 0x80_0000;
const NO_EXECUTE: u64 = 0xA0_0000;
const PXN_ONLY: u64 = 0xC0_0000;
const AF: u64 = 1 << 10;
const INNER_SHAREABLE: u64 = 3 << 8;
const BLOCK: u64 = 0b01;
const TABLE: u64 = 0b11;
const AP2: u64 = 1 << 7;
const AP1: u64 = 1 << 6;
const XN: u64 = 1 << 54;
const PXN: u64 = 1 << 53;

/// A processor at EL2 with both regimes' MMUs on over the tables above,
/// 39-bit virtual addresses (T0SZ 25) and 4 KiB pages, all memory Normal.
fn translating() -> Machine<WithEl2> {
    let mut m = Machine::with_el2();
    let attrs = AF | INNER_SHAREABLE | BLOCK;
    for (l1, l2, ap) in [(EL2_L1, EL2_L2, AP1), (EL1_L1, EL1_L2, 0)] {
        m.ram.put64(l1, l2 | TABLE);
        m.ram.put64(l2, attrs | ap);
    }
    m.ram.put64(EL2_L2 + 8, EL2_DATA | attrs);
    m.ram.put64(EL1_L2 + 8, EL1_DATA | attrs);
    m.ram.put64(EL2_L2 + 8 * (READ_ONLY >> 21), READ_ONLY | attrs | AP2 | AP1);
    m.ram.put64(EL2_L2 + 8 * (NO_EXECUTE >> 21), NO_EXECUTE | attrs | XN);
    m.ram.put64(EL2_L2 + 8 * (PXN_ONLY >> 21), PXN_ONLY | attrs | PXN);
    m.ram.put64(EL2_DATA, 0x2222);
    m.ram.put64(EL1_DATA, 0x1111);
    let features = m.cpu.imp.features;
    let el2 = &mut m.cpu.el2_state.regs;
    // TCR_EL2: T0SZ 25, PS 40 bits; its RES1 bits come with the write.
    el2.write(super::el2::El2Reg::Tcr, features, 0, 25 | (0b010 << 16));
    el2.ttbr0 = EL2_L1;
    el2.mair = 0xFF;
    el2.sctlr |= 1;
    let sys = &mut m.cpu.sys;
    sys.tcr_el1 = 25 | (1 << 23);
    sys.ttbr0_el1 = EL1_L1;
    sys.mair_el1 = 0xFF;
    sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1;
    m.at(El::El2);
    m
}

/// EL2's accesses translate through `TTBR0_EL2` and EL1's through
/// `TTBR0_EL1`, and the TLB keeps the two apart: the same virtual address
/// reads each regime's own memory, in either order. After `TLBI ALLE2`, a
/// changed EL2 table is what EL2 sees.
#[test]
fn el2_translates_through_its_own_tables() {
    let mut m = translating();
    for el in [El::El2, El::El1, El::El2, El::El1] {
        m.at(el);
        m.cpu.x[1] = SHARED;
        assert_eq!(m.run(LDR), None, "{el:?}");
        let want = if el == El::El2 { 0x2222 } else { 0x1111 };
        assert_eq!(m.cpu.x[2], want, "{el:?}");
    }
    m.ram.put64(EL2_L2 + 8, EL1_DATA | AF | INNER_SHAREABLE | BLOCK);
    m.at(El::El2);
    assert_eq!(m.run(TLBI_ALLE2), None);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x1111, "the new mapping after TLBI ALLE2");
}

/// `HCR_EL2.DC` and `TGE` turn stage 1 of the EL1&0 regime off whatever
/// `SCTLR_EL1.M` says (Arm `AArch64_S1Enabled`): an EL1 or EL0 access
/// reaches the physical address it names. With stage 1 off a data access
/// is to Device-nGnRnE memory, where a misaligned one takes an alignment
/// fault, but under `DC` to Normal memory (`AArch64_S1DisabledOutput`).
#[test]
fn dc_and_tge_turn_off_el1_and_0_stage_1() {
    const PHYSICAL: u64 = 0x5555;
    const ALIGNMENT: u64 = 0b10_0001;
    let mut m = translating();
    let reset = m.cpu.el2_state.regs.hcr;
    m.ram.put64(SHARED, PHYSICAL);
    m.ram.put64(SHARED + 8, 0);
    for (el, hcr, want) in [
        (El::El1, 0, 0x1111),
        (El::El1, HCR_DC, PHYSICAL),
        (El::El0, HCR_DC, PHYSICAL),
        (El::El0, HCR_TGE, PHYSICAL),
        (El::El1, 0, 0x1111),
    ] {
        m.cpu.el2_state.regs.hcr = reset | hcr;
        m.at(el);
        m.cpu.x[1] = SHARED;
        assert_eq!(m.run(LDR), None, "{el:?} HCR {hcr:#x}");
        assert_eq!(m.cpu.x[2], want, "{el:?} HCR {hcr:#x}");
    }

    m.cpu.sys.sctlr_el1 &= !1;
    for (hcr, want) in [
        (0, sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, ALIGNMENT)),
        (HCR_DC, None),
    ] {
        m.cpu.el2_state.regs.hcr = reset | hcr;
        m.at(El::El1);
        m.cpu.x[1] = SHARED + 1;
        assert_eq!(m.run(LDR), want, "misaligned, HCR {hcr:#x}");
    }
    assert_eq!(m.cpu.x[2], PHYSICAL >> 8, "the Normal read");
}

/// `ID_AA64MMFR0_EL1` offers no stage 2 granule on a processor with EL2,
/// which has no stage 2 translation: `TGran4_2`, `TGran16_2` and
/// `TGran64_2` read 1, not supported at stage 2 (FEAT_GTG, which
/// `Features.json` makes hold exactly when all three are non-zero). Without
/// EL2 all three read 0, and either way stage 1 offers the 4 KiB granule
/// alone.
#[test]
fn the_id_registers_offer_no_stage_2_granule() {
    const MRS_MMFR0: u32 = 0xD538_0700; // mrs x0, id_aa64mmfr0_el1
    // TGran4_2, TGran16_2, TGran64_2, then TGran4, TGran16, TGran64.
    let granules = |mmfr0: u64| [40, 32, 36, 28, 20, 24].map(|lsb| (mmfr0 >> lsb) & 0xF);
    let mut with = Machine::with_el2();
    with.at(El::El1);
    assert_eq!(with.run(MRS_MMFR0), None);
    assert_eq!(granules(with.cpu.x[0]), [1, 1, 1, 0, 0, 0xF]);
    let mut without = Machine::<NoEl2>::new(Implementation::CORTEX_A57);
    without.at(El::El1);
    assert_eq!(without.run(MRS_MMFR0), None);
    assert_eq!(granules(without.cpu.x[0]), [0, 0, 0, 0, 0, 0xF]);
}

/// `DC IVAC` at EL1 translates as an invalidate even where it is carried
/// out as a clean and invalidate: under `HCR_EL2.VM` a page stage 1 makes
/// read-only refuses it a permission (`AArch64_DC` translates before it
/// changes the operation), while under `HCR_EL2.DC`, which turns off stage
/// 1 for EL1 and EL0, there is no permission to check.
#[test]
fn dc_ivac_at_el1_translates_as_an_invalidate_under_vm_and_dc() {
    const DC_IVAC: u32 = 0xD508_7621; // dc ivac, x1
    const HCR_VM: u64 = 1 << 0;
    const CM: u64 = 1 << 8;
    let mut m = translating();
    m.ram.put64(EL1_L2 + 8 * (READ_ONLY >> 21), READ_ONLY | AF | INNER_SHAREABLE | BLOCK | AP2);
    let reset = m.cpu.el2_state.regs.hcr;
    let refused = sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, PERMISSION_L2 | CM | WNR);
    for (hcr, want) in [(0, refused), (HCR_VM, refused), (HCR_DC, None)] {
        m.cpu.el2_state.regs.hcr = reset | hcr;
        m.cpu.sys.far_el1 = 0;
        m.at(El::El1);
        m.cpu.x[1] = READ_ONLY;
        assert_eq!(m.run(DC_IVAC), want, "HCR {hcr:#x}");
        let far = if want.is_some() { READ_ONLY } else { 0 };
        assert_eq!(m.cpu.sys.far_el1, far, "HCR {hcr:#x}");
    }
}

/// The EL2 regime has one privilege level: AP[2] makes a page read-only
/// whatever AP[1] holds, and XN (bit 54) refuses execution while bit 53, the
/// PXN of a regime with EL0, does not.
#[test]
fn el2_permissions_are_those_of_one_privilege_level() {
    let mut m = translating();
    m.cpu.x[1] = READ_ONLY;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.run(STR), sync(El::El2, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, PERMISSION_L2 | WNR));
    assert_eq!(m.cpu.el2_state.regs.far, READ_ONLY);

    m.at(El::El2);
    m.ram.0[PXN_ONLY as usize..PXN_ONLY as usize + 4].copy_from_slice(&NOP.to_le_bytes());
    m.cpu.x[1] = PXN_ONLY;
    assert_eq!(m.run(BR_X1), None);
    assert_eq!(m.cpu.step(&mut m.ram, &mut NoInstrumentation), Step::Retired);
    assert_eq!(m.cpu.pc, PXN_ONLY + 4, "bit 53 does not stop EL2 executing");

    m.at(El::El2);
    m.cpu.x[1] = NO_EXECUTE;
    assert_eq!(m.run(BR_X1), None);
    assert_eq!(m.cpu.step(&mut m.ram, &mut NoInstrumentation), Step::Retired);
    assert_eq!(
        m.taken(),
        sync(El::El2, CURRENT_SPX, EC_INSTRUCTION_ABORT_SAME_EL, PERMISSION_L2)
    );
    assert_eq!(m.cpu.el2_state.regs.far, NO_EXECUTE);
}

/// A snapshot's TLB entry of the EL2 regime is one a processor without EL2
/// never holds, nor one with EL2 when it gives EL0 a permission, which the
/// EL2 walk never does: each restore refuses it, saying which.
#[test]
fn a_snapshot_el2_tlb_entry_is_refused_where_no_walk_makes_it() {
    /// EL0's read permission in an entry's permissions byte.
    const PERM_EL0_R: u8 = 1 << 3;
    let mut m = translating();
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None, "the EL2 pages of the fetch and the load are cached");
    let mut image = Vec::new();
    assert_eq!(m.cpu.tlb.save(&mut image), Ok(()));
    // The entry count, then each entry: its page, its PA, then its
    // permissions, Device, level and regime bytes.
    let count = u32::from_le_bytes([image[0], image[1], image[2], image[3]]) as usize;
    assert_eq!((count, image.len()), (2, 4 + count * (8 + 8 + 4)));
    let tcr = m.cpu.sys.tcr_el1;
    let restore = |features: Features, image: &[u8]| Tlb::new().restore(&mut &image[..], features, tcr);
    let with = m.cpu.imp.features;
    let without = Implementation::CORTEX_A57.with_levels::<NoEl2>().features;
    assert_eq!(restore(with, &image), Ok(()));
    assert_eq!(
        restore(without, &image),
        Err(SnapError::Invalid("TLB entry is of a regime this processor lacks"))
    );
    let mut el0 = image.clone();
    el0[4 + 8 + 8] |= PERM_EL0_R;
    assert_eq!(restore(with, &el0), Err(SnapError::Invalid("TLB entry is not one a walk can produce")));
}

/// `LDTR` and `STTR` at EL2 are ordinary EL2 accesses: with no FEAT_NV and
/// no FEAT_VHE, `AArch64_IsUnprivAccessPriv` makes them privileged there,
/// and the EL2 regime has no EL0 permissions to check them against. At EL1
/// they are checked against EL0's, which the EL1 tables refuse `SHARED`.
#[test]
fn ldtr_and_sttr_at_el2_are_ordinary_accesses() {
    const LDTR: u32 = 0xF840_0822; // ldtr x2, [x1]
    const STTR: u32 = 0xF800_0822; // sttr x2, [x1]
    let mut m = translating();
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDTR), None);
    assert_eq!(m.cpu.x[2], 0x2222);
    m.cpu.x[2] = 0x3333;
    assert_eq!(m.run(STTR), None);
    m.cpu.x[2] = 0;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x3333, "STTR wrote EL2's page");

    m.at(El::El1);
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x1111);
    m.at(El::El1);
    assert_eq!(m.run(LDTR), sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, PERMISSION_L2));
}

/// In each regime the walk refuses an address beyond the output size its
/// `TCR_ELx` gives (`IPS` for EL1&0, `PS` for EL2; `AArch64_S1OAOutOfRange`)
/// with an address size fault: the table base at level 0, a next-level
/// table at the level of the descriptor naming it, an output address at the
/// level of its block. A larger size lets the same output address through,
/// to no memory here: an external abort.
#[test]
fn the_walk_refuses_addresses_beyond_the_output_size() {
    const HIGH: u64 = 0x1_0000_0000;
    const ADDRESS_SIZE: u64 = 0b00_0000;
    const BLOCK_VA: u64 = 0xE0_0000;
    const TABLE_VA: u64 = 0x4000_0000;
    // The output size field and its shift in each regime's TCR.
    let regimes = [(El::El1, EL1_L1, EL1_L2, 32), (El::El2, EL2_L1, EL2_L2, 16)];
    for (el, l1, l2, shift) in regimes {
        let size = |m: &mut Machine<WithEl2>, encoded: u64| {
            let tcr = if el == El::El2 { &mut m.cpu.el2_state.regs.tcr } else { &mut m.cpu.sys.tcr_el1 };
            *tcr = (*tcr & !(7 << shift)) | (encoded << shift);
        };
        let load = |m: &mut Machine<WithEl2>, va: u64| {
            m.at(el);
            m.cpu.x[1] = va;
            m.run(LDR)
        };
        let fault = |dfsc: u64| sync(el, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, dfsc);

        // A 2 MiB block at 4 GiB: beyond 32 bits, within 36.
        let mut m = translating();
        m.ram.put64(l2 + 8 * (BLOCK_VA >> 21), HIGH | AF | INNER_SHAREABLE | BLOCK);
        size(&mut m, 0b000);
        assert_eq!(load(&mut m, BLOCK_VA), fault(ADDRESS_SIZE | 2), "{el:?}: block");
        let mut m = translating();
        m.ram.put64(l2 + 8 * (BLOCK_VA >> 21), HIGH | AF | INNER_SHAREABLE | BLOCK);
        size(&mut m, 0b001);
        assert_eq!(load(&mut m, BLOCK_VA), fault(SEA), "{el:?}: block within 36 bits");

        // A level 2 table at 4 GiB.
        let mut m = translating();
        m.ram.put64(l1 + 8 * (TABLE_VA >> 30), HIGH | TABLE);
        size(&mut m, 0b000);
        assert_eq!(load(&mut m, TABLE_VA), fault(ADDRESS_SIZE | 1), "{el:?}: table");

        // The table base at 4 GiB: the fetch of the load itself faults.
        let mut m = translating();
        size(&mut m, 0b000);
        if el == El::El2 {
            m.cpu.el2_state.regs.ttbr0 = HIGH;
        } else {
            m.cpu.sys.ttbr0_el1 = HIGH;
        }
        assert_eq!(
            load(&mut m, BLOCK_VA),
            sync(el, CURRENT_SPX, EC_INSTRUCTION_ABORT_SAME_EL, ADDRESS_SIZE),
            "{el:?}: base"
        );
    }
}

/// The EL2 regime has one address range, from zero: an address with any of
/// bits 63:39 set is out of range (a level 0 translation fault), unless
/// `TCR_EL2.TBI` ignores the top byte, which leaves the rest to translate.
#[test]
fn el2_has_one_address_range_and_its_own_top_byte_ignore() {
    let tagged = SHARED | (0x5A << 56);
    let mut m = translating();
    m.cpu.x[1] = tagged;
    assert_eq!(m.run(LDR), sync(El::El2, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, TRANSLATION_L0));
    assert_eq!(m.cpu.el2_state.regs.far, tagged);
    let mut m = translating();
    m.cpu.x[1] = SHARED | (1 << 39);
    assert_eq!(m.run(LDR), sync(El::El2, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, TRANSLATION_L0));

    let mut m = translating();
    m.cpu.el2_state.regs.tcr |= super::el2::TCR_EL2_TBI;
    m.cpu.x[1] = tagged;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x2222);
}

/// `AT S1E2R` and `AT S1E2W` translate through the EL2 regime into
/// `PAR_EL1`: the output address of a mapped page, and of the read-only
/// one only for a read; a refused translation sets `PAR_EL1.F` with the
/// fault's status, and takes no exception.
#[test]
fn at_s1e2_translates_through_the_el2_regime() {
    const PAR_F: u64 = 1;
    const PA: u64 = 0x000F_FFFF_FFFF_F000;
    let mut m = translating();
    m.cpu.x[0] = SHARED + 0x123;
    assert_eq!(m.run(AT_S1E2R), None);
    let par = m.cpu.sys.par_el1;
    assert_eq!((par & PAR_F, par & PA), (0, EL2_DATA), "{par:#x}");

    m.cpu.x[0] = READ_ONLY;
    assert_eq!(m.run(AT_S1E2R), None);
    assert_eq!(m.cpu.sys.par_el1 & (PA | PAR_F), READ_ONLY);
    assert_eq!(m.run(AT_S1E2W), None);
    let par = m.cpu.sys.par_el1;
    assert_eq!(par & PAR_F, PAR_F, "{par:#x}");
    assert_eq!((par >> 1) & 0x3F, PERMISSION_L2, "FST: a level 2 permission fault");

    m.cpu.x[0] = 1 << 39;
    assert_eq!(m.run(AT_S1E2R), None);
    let par = m.cpu.sys.par_el1;
    assert_eq!((par & PAR_F, (par >> 1) & 0x3F), (PAR_F, TRANSLATION_L0), "{par:#x}");
}

/// From EL2, `AT S12E1R` and `S12E1W` translate through the EL1&0 regime
/// with EL1's permissions and `AT S12E0R` and `S12E0W` with EL0's: with
/// no stage 2 translation, `PAR_EL1` holds the stage 1 result. EL1's are
/// UNDEFINED.
#[test]
fn at_s12_translates_the_el1_and_0_regime_from_el2() {
    const AT_S12E1R: u32 = 0xD50C_7880; // at s12e1r, x0
    const AT_S12E1W: u32 = 0xD50C_78A0; // at s12e1w, x0
    const AT_S12E0R: u32 = 0xD50C_78C0; // at s12e0r, x0
    const AT_S12E0W: u32 = 0xD50C_78E0; // at s12e0w, x0
    const PAR_F: u64 = 1;
    const PA: u64 = 0x000F_FFFF_FFFF_F000;
    let mut m = translating();
    for at in [AT_S12E1R, AT_S12E1W] {
        m.cpu.x[0] = SHARED + 0x123;
        assert_eq!(m.run(at), None);
        let par = m.cpu.sys.par_el1;
        assert_eq!((par & PAR_F, par & PA), (0, EL1_DATA), "{at:#010x}: {par:#x}");
    }
    for at in [AT_S12E0R, AT_S12E0W] {
        m.cpu.x[0] = SHARED;
        assert_eq!(m.run(at), None);
        let par = m.cpu.sys.par_el1;
        let fault = (par & PAR_F, (par >> 1) & 0x3F);
        assert_eq!(fault, (PAR_F, PERMISSION_L2), "{at:#010x}: EL0 has no access: {par:#x}");
    }
    m.at(El::El1);
    assert_eq!(m.run(AT_S12E1R), sync(El::El1, CURRENT_SPX, EC_UNKNOWN, 0));
}
