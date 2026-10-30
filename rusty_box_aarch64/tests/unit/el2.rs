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

use std::boxed::Box;
use std::vec::Vec;

use rusty_box_core::{ClockHz, SnapError, VmClock, VmInstant};

use super::el2::{
    El2Reg, CNTHCTL_ECV_POFF, CPTR_TFP, CPTR_TSM, HCR_AMO, HCR_CD, HCR_DC, HCR_FMO, HCR_HCD,
    HCR_IMO, HCR_PTW, HCR_TEA, HCR_TGE, HCR_TSC, HCR_VF, HCR_VI, HCR_VM, HCR_VSE, MDCR_TDE,
};
use super::exception::AccessSyndrome;
use super::exec::Fault;
use super::features::{Feature, Implementation};
use super::mmu::saved_layout::{
    CONTEXTS, CONTEXT_BYTES, CONTEXT_HCR, CONTEXT_SCTLR, CONTEXT_VTCR, CONTEXT_VTTBR, ENTRY_BYTES,
    ENTRY_S2_DEVICE, ENTRY_S2_LEVEL, ENTRY_S2_PERMS, ENTRY_STAGE1, KEPT_BYTES,
    STAGE2_CONTEXT_BYTES,
};
use super::mmu::{DataAccess, El10Context, TlbContext};
use super::op::Extend;
use super::sysreg::{CPACR_FPEN, CPACR_SMEN, SMCR_EZT0};
use super::{
    id, Cpu, CpuConfig, El, Ipa, Levels, NoEl2, NoTranslation, Pstate, Step, SysReg, SysRegRefusal,
    TimerLines, Tlb, TranslationFault, TranslationStage, WithEl2, WriteEffect,
};
use crate::bus::{AccessWidth, BusFault, CodeEpoch, FetchSource, PhysAddr, PhysBus};
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
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
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

    fn get64(&self, at: u64) -> u64 {
        let a = at as usize;
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.0[a..a + 8]);
        u64::from_le_bytes(bytes)
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
                reset_pc: CODE,
            },
            Implementation {
                counter_hz: ips,
                ..imp
            },
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
        assert_eq!(
            self.cpu.step(&mut self.ram, &mut NoInstrumentation),
            Step::Retired,
            "{insn:#010x}"
        );
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
        (pc & !0x7FF == vbar).then(|| Taken {
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

    /// Set `bits` of `HCR_EL2`, the EL1&0 regime's context left to be
    /// found again as an `MSR`'s change of it leaves it.
    fn hcr(&mut self, bits: u64) {
        self.cpu.el2_state.regs.hcr |= bits;
        self.cpu.el10_context_changed();
    }

    /// The registers a snapshot's TLB is judged by, as the processor holds
    /// them.
    fn tlb_context(&self) -> TlbContext {
        TlbContext {
            features: self.cpu.imp.features,
            el10: self.cpu.el10_context(),
            sctlr_el2: self.cpu.el2_state.regs.sctlr,
        }
    }

    /// The processor's TLB, saved.
    fn saved_tlb(&self) -> SavedTlb {
        let mut image = Vec::new();
        assert_eq!(self.cpu.tlb.save(&mut image), Ok(()));
        SavedTlb(image)
    }
}

/// `image` restored into the TLB of a processor of the level set `L` whose
/// registers make `context`: the TLB's own image of what it restored, which
/// a faithful restore makes `image` again.
fn restored<L: Levels>(context: &TlbContext, image: &[u8]) -> Result<Vec<u8>, SnapError> {
    let mut tlb = Box::new(Tlb::<L>::new());
    tlb.restore(&mut &image[..], context)?;
    let mut saved = Vec::new();
    assert_eq!(tlb.save(&mut saved), Ok(()));
    Ok(saved)
}

/// A saved TLB ([`Tlb::save`], laid out as
/// [`saved_layout`](super::mmu::saved_layout) says): its table of EL1&0
/// contexts and its table of stage 2 contexts, each with each place a byte
/// saying whether it holds one and then the context's registers, each
/// place's generation, the places from the one found most recently to the
/// least, and the current context's place plus one; the entries; and the
/// kept stage 2 translations.
struct SavedTlb(Vec<u8>);

/// Where one of a saved TLB's tables of contexts lies.
struct SavedTable {
    /// Where each place holding a context begins, by place.
    held: Vec<(usize, usize)>,
    /// Where the places' generations lie, past the places.
    generations_at: usize,
}

impl SavedTable {
    /// Where the current context's place lies, past the generations and
    /// the order the places were found in.
    fn current_at(&self) -> usize {
        self.generations_at + 2 * CONTEXTS
    }

    /// Where the table ends.
    fn end(&self) -> usize {
        self.current_at() + 1
    }
}

impl SavedTlb {
    /// The table of contexts `bytes` long each that begins at `at`.
    fn table(&self, mut at: usize, bytes: usize) -> SavedTable {
        let mut held = Vec::new();
        for place in 0..CONTEXTS {
            if self.0[at] == 1 {
                held.push((place, at + 1));
                at += 1 + bytes;
            } else {
                at += 1;
            }
        }
        SavedTable {
            held,
            generations_at: at,
        }
    }

    /// The table of EL1&0 contexts, first.
    fn el10(&self) -> SavedTable {
        self.table(0, CONTEXT_BYTES)
    }

    /// The table of stage 2 contexts, past it.
    fn stage2(&self) -> SavedTable {
        self.table(self.el10().end(), STAGE2_CONTEXT_BYTES)
    }

    /// Where each place of the table of EL1&0 contexts holding one begins,
    /// by place.
    fn contexts(&self) -> Vec<(usize, usize)> {
        self.el10().held
    }

    /// Where the current EL1&0 context's place lies.
    fn current_at(&self) -> usize {
        self.el10().current_at()
    }

    /// The current EL1&0 context's place plus one, 0 for none.
    fn current(&self) -> u8 {
        self.0[self.current_at()]
    }

    /// The generation of the place `index` of the table of EL1&0 contexts.
    fn generation(&self, index: usize) -> u8 {
        self.0[self.el10().generations_at + index]
    }

    /// The little-endian doubleword at `at`.
    fn u64_at(&self, at: usize) -> u64 {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.0[at..at + 8]);
        u64::from_le_bytes(bytes)
    }

    /// Where the entries' count lies, past both tables.
    fn entries_at(&self) -> usize {
        self.stage2().end()
    }

    /// How many entries the TLB held.
    fn entries(&self) -> usize {
        snapshot_count(&self.0[self.entries_at()..])
    }

    /// Where the kept stage 2 translations' count lies.
    fn kept_at(&self) -> usize {
        self.entries_at() + 4 + self.entries() * ENTRY_BYTES
    }

    /// How many stage 2 translations the TLB kept.
    fn kept(&self) -> usize {
        snapshot_count(&self.0[self.kept_at()..])
    }

    /// The IPA page number of each kept stage 2 translation.
    fn kept_pages(&self) -> Vec<u64> {
        (0..self.kept())
            .map(|i| self.u64_at(self.kept_at() + 4 + i * KEPT_BYTES) & ((1 << 52) - 1))
            .collect()
    }

    /// The context tag of each entry.
    fn entry_tags(&self) -> Vec<u8> {
        (0..self.entries())
            .map(|i| self.0[self.entries_at() + 4 + i * ENTRY_BYTES + 6] >> 4)
            .collect()
    }

    /// Where the entry of the page of `va` is, if the TLB held one.
    fn find_entry(&self, va: u64) -> Option<usize> {
        let at = self.entries_at();
        (0..self.entries())
            .map(|i| at + 4 + i * ENTRY_BYTES)
            .find(|&at| {
                let mut page = [0; 8];
                page.copy_from_slice(&self.0[at..at + 8]);
                u64::from_le_bytes(page) & ((1 << 52) - 1) == va >> 12
            })
    }

    /// Whether the TLB held an entry of the page of `va`.
    fn has_entry(&self, va: u64) -> bool {
        self.find_entry(va).is_some()
    }

    /// Where the entry of the page of `va` is.
    fn entry_at(&self, va: u64) -> usize {
        self.find_entry(va)
            .unwrap_or_else(|| panic!("no entry for {va:#x}"))
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
                reset_pc: CODE,
            },
            Implementation {
                counter_hz: ips,
                ..imp
            },
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
    assert_eq!(
        m.cpu.step(&mut m.ram, &mut NoInstrumentation),
        Step::Retired
    );
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
    assert_eq!(
        m.cpu.el2_state.regs.elr,
        CODE + 4,
        "returns to the next instruction"
    );
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
    for (m_field, el, sp_sel) in [
        (EL1H, El::El1, true),
        (EL0T, El::El0, false),
        (EL2T, El::El2, false),
        (EL2H, El::El2, true),
    ] {
        m.at(El::El2);
        m.cpu.el2_state.regs.spsr = m_field | (1 << 7) | (1 << 31);
        m.cpu.el2_state.regs.elr = TARGET;
        assert_eq!(m.run(ERET), None, "M {m_field:#06b}");
        let p = m.cpu.pstate;
        assert_eq!(
            (p.el, p.sp_sel, m.cpu.pc),
            (el, sp_sel, TARGET),
            "M {m_field:#06b}"
        );
        assert!(p.i && !p.f && p.n && !p.il, "M {m_field:#06b}");
    }

    let illegal = [
        (0b1101, 0),      // EL3h: not implemented
        (EL1H | 0x10, 0), // AArch32
        (0b0111, 0),      // M[1] set
        (EL1H, HCR_TGE),  // EL1 under TGE (E2H = 0)
    ];
    for (spsr, hcr) in illegal {
        let mut m = Machine::with_el2();
        m.hcr(hcr);
        m.at(El::El2);
        m.cpu.el2_state.regs.spsr = spsr;
        m.cpu.el2_state.regs.elr = TARGET;
        assert_eq!(m.run(ERET), None, "SPSR {spsr:#x}");
        let p = m.cpu.pstate;
        assert_eq!(
            (p.el, p.sp_sel, p.il, m.cpu.pc),
            (El::El2, true, true, TARGET),
            "SPSR {spsr:#x}"
        );
        assert_eq!(
            m.run_at(TARGET, NOP),
            sync(El::El2, CURRENT_SPX, EC_ILLEGAL_STATE, 0),
            "SPSR {spsr:#x}"
        );
    }

    let mut m = Machine::with_el2();
    m.at(El::El1);
    m.cpu.sys.spsr_el1 = EL2H;
    m.cpu.sys.elr_el1 = TARGET;
    assert_eq!(m.run(ERET), None);
    assert_eq!(
        (m.cpu.pstate.el, m.cpu.pstate.il),
        (El::El1, true),
        "EL1 cannot return to EL2"
    );
    assert_eq!(
        m.run_at(TARGET, NOP),
        sync(El::El1, CURRENT_SPX, EC_ILLEGAL_STATE, 0)
    );
}

/// ERET from EL2 sets PSTATE.SS from SPSR_EL2.SS only for a return, with
/// `MDSCR_EL1.SS` set, from where debug exceptions are disabled to where
/// they are enabled (`DebugExceptionReturnSS`), and EL2 decides where they
/// are taken (`DebugTargetFrom`, `AArch64_GenerateDebugExceptionsFrom`):
/// at EL1 by default, so never from EL2 and from EL1 with `MDSCR_EL1.KDE`
/// set and the mask (SPSR_EL2.D) clear; at EL2 under `MDCR_EL2.TDE` or
/// `HCR_EL2.TGE`, so from EL1 and EL0 always, and from EL2 with KDE set and
/// PSTATE.D clear, at the source as at the destination.
#[test]
fn eret_from_el2_restores_software_step_by_the_debug_target() {
    const SPSR_SS: u64 = 1 << 21;
    const SPSR_D: u64 = 1 << 9;
    const MDSCR_SS: u64 = 1;
    const MDSCR_KDE: u64 = 1 << 13;
    const TARGET: u64 = 0x5_0000;
    // MDSCR_EL1, PSTATE.D at EL2, MDCR_EL2 and HCR_EL2 bits, SPSR_EL2,
    // then PSTATE.SS.
    let cases = [
        // Debug exceptions target EL1.
        (MDSCR_SS, true, 0, 0, EL1H | SPSR_SS, false),
        (MDSCR_SS | MDSCR_KDE, true, 0, 0, EL1H | SPSR_SS, true),
        (
            MDSCR_SS | MDSCR_KDE,
            true,
            0,
            0,
            EL1H | SPSR_SS | SPSR_D,
            false,
        ),
        (MDSCR_SS, true, 0, 0, EL0T | SPSR_SS, true),
        (0, true, 0, 0, EL0T | SPSR_SS, false),
        (MDSCR_SS | MDSCR_KDE, true, 0, 0, EL2H | SPSR_SS, false),
        // `MDCR_EL2.TDE` takes them to EL2.
        (MDSCR_SS, true, MDCR_TDE, 0, EL1H | SPSR_SS, true),
        (
            MDSCR_SS | MDSCR_KDE,
            false,
            MDCR_TDE,
            0,
            EL1H | SPSR_SS,
            false,
        ),
        (
            MDSCR_SS | MDSCR_KDE,
            true,
            MDCR_TDE,
            0,
            EL2H | SPSR_SS,
            true,
        ),
        (MDSCR_SS, true, MDCR_TDE, 0, EL2H | SPSR_SS, false),
        // So does `HCR_EL2.TGE`, under which EL1 is no return's target.
        (MDSCR_SS, true, 0, HCR_TGE, EL0T | SPSR_SS, true),
        (
            MDSCR_SS | MDSCR_KDE,
            false,
            0,
            HCR_TGE,
            EL0T | SPSR_SS,
            false,
        ),
    ];
    for (mdscr, d, mdcr, hcr, spsr, ss) in cases {
        let mut m = Machine::with_el2();
        m.hcr(hcr);
        m.cpu.el2_state.regs.mdcr |= mdcr;
        m.at(El::El2);
        // The OS Lock, locked out of reset, disables debug exceptions.
        m.cpu.sys.oslsr_el1 &= !0b10;
        m.cpu.sys.mdscr_el1 = mdscr;
        m.cpu.pstate.d = d;
        m.cpu.el2_state.regs.spsr = spsr;
        m.cpu.el2_state.regs.elr = TARGET;
        assert_eq!(m.run(ERET), None);
        assert!(!m.cpu.pstate.il, "SPSR {spsr:#x}: a legal return");
        assert_eq!(
            m.cpu.pstate.ss, ss,
            "MDSCR {mdscr:#x}, D {d}, MDCR {mdcr:#x}, HCR {hcr:#x}, SPSR {spsr:#x}"
        );
    }
}

/// With EL2 the highest level, the reset management registers are EL2's
/// (Arm's data: `IsHighestEL`): `RVBAR_EL2` reads the address the last
/// reset started execution from and `RMR_EL2` reads AA64 as one and keeps
/// the Warm reset request RR, while `RVBAR_EL1` and `RMR_EL1`, which only a
/// processor whose highest level is EL1 has, are UNDEFINED at EL1 and EL2.
#[test]
fn el2_holds_the_reset_management_registers() {
    const MRS_RVBAR_EL2: u32 = 0xD53C_C020; // mrs x0, rvbar_el2
    const MRS_RMR_EL2: u32 = 0xD53C_C040; // mrs x0, rmr_el2
    const MSR_RMR_EL2: u32 = 0xD51C_C040; // msr rmr_el2, x0
    const MRS_RVBAR_EL1: u32 = 0xD538_C020; // mrs x0, rvbar_el1
    const MRS_RMR_EL1: u32 = 0xD538_C040; // mrs x0, rmr_el1
    const MSR_RMR_EL1: u32 = 0xD518_C040; // msr rmr_el1, x0
    const RESET_PC: u64 = 0x6_0000;
    let mut m = Machine::with_el2();
    m.cpu.reset_keeping_time(RESET_PC);
    m.cpu.el2_state.regs.vbar = VBAR_EL2;
    m.cpu.sys.vbar_el1 = VBAR_EL1;
    m.at(El::El2);
    assert_eq!(m.run(MRS_RVBAR_EL2), None);
    assert_eq!(m.cpu.x[0], RESET_PC, "the last reset's address");
    assert_eq!(m.run(MRS_RMR_EL2), None);
    assert_eq!(m.cpu.x[0], 0b01, "AA64");
    m.cpu.x[0] = !0;
    assert_eq!(m.run(MSR_RMR_EL2), None);
    assert_eq!(m.run(MRS_RMR_EL2), None);
    assert_eq!(m.cpu.x[0], 0b11, "AA64 and RR, nothing else");
    for el in [El::El2, El::El1] {
        for insn in [MRS_RVBAR_EL1, MRS_RMR_EL1, MSR_RMR_EL1] {
            m.at(el);
            assert_eq!(
                m.run(insn),
                sync(el, CURRENT_SPX, EC_UNKNOWN, 0),
                "{el:?}: {insn:#010x}"
            );
        }
    }
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
        assert_eq!(
            m.run(LDR),
            sync(to, LOWER_A64, EC_DATA_ABORT_LOWER_EL, SEA),
            "TGE {tge}"
        );
        let far = if to == El::El2 {
            m.cpu.el2_state.regs.far
        } else {
            m.cpu.sys.far_el1
        };
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
    assert_eq!(
        m.run(LDR),
        sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, SEA)
    );
    m.hcr(HCR_TEA);
    m.at(El::El1);
    assert_eq!(
        m.run(LDR),
        sync(El::El2, LOWER_A64, EC_DATA_ABORT_LOWER_EL, SEA)
    );
    assert_eq!(m.cpu.el2_state.regs.far, NOWHERE);
    m.at(El::El1);
    assert_eq!(m.run(SVC), sync(El::El1, CURRENT_SPX, EC_SVC, 5));

    let mut m = Machine::with_el2();
    m.hcr(HCR_TGE);
    m.at(El::El1);
    m.cpu.x[1] = NOWHERE;
    assert_eq!(
        m.run(LDR),
        sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, SEA),
        "TGE, from EL1"
    );
}

/// A physical IRQ or FIQ goes where `HCR_EL2.IMO` or `FMO` (or `TGE`) routes
/// it: routed to EL2, EL0 and EL1 take it whatever `PSTATE` masks, through
/// the lower-level vector, and EL2 takes it when its own mask is clear;
/// left to EL1, EL1 takes it when unmasked and EL2 never does. A host's
/// `take_irq` or `take_fiq` overrides `PSTATE`'s mask and nothing else.
#[test]
fn physical_interrupts_go_where_imo_and_fmo_route_them() {
    for (line, routing, vector) in [(0, HCR_IMO, IRQ), (1, HCR_FMO, FIQ)] {
        let assert_line =
            |m: &mut Machine<WithEl2>| m.cpu.set_interrupt_lines(line == 0, line == 1);
        let take = |m: &mut Machine<WithEl2>| {
            if line == 0 {
                m.cpu.take_irq()
            } else {
                m.cpu.take_fiq()
            }
        };
        let mut m = Machine::with_el2();
        assert_line(&mut m);

        m.at(El::El1);
        assert!(!m.cpu.interrupt_deliverable(), "masked at EL1");
        m.cpu.pstate.i = false;
        m.cpu.pstate.f = false;
        let taken = m.cpu.deliver_interrupt().expect("unmasked at EL1");
        assert_eq!(
            (taken.to, taken.vector),
            (El::El1, VBAR_EL1 + CURRENT_SPX + vector)
        );

        m.at(El::El2);
        m.cpu.pstate.i = false;
        m.cpu.pstate.f = false;
        assert!(
            !m.cpu.interrupt_deliverable(),
            "an interrupt for EL1 is not taken at EL2"
        );
        assert!(take(&mut m).is_none(), "not even by a host");
        assert_eq!((m.cpu.pstate.el, m.cpu.pc), (El::El2, CODE));

        m.hcr(routing);
        m.at(El::El1);
        let taken = m
            .cpu
            .deliver_interrupt()
            .expect("routed to EL2, PSTATE masks it not");
        assert_eq!(
            (taken.to, taken.vector),
            (El::El2, VBAR_EL2 + LOWER_A64 + vector)
        );
        assert_eq!(m.cpu.el2_state.regs.elr, CODE);

        m.at(El::El2);
        assert!(!m.cpu.interrupt_deliverable(), "EL2's own mask");
        let taken = take(&mut m).expect("a host takes it whatever PSTATE masks");
        assert_eq!(
            (taken.to, taken.vector),
            (El::El2, VBAR_EL2 + CURRENT_SPX + vector)
        );

        m.at(El::El2);
        m.cpu.pstate.i = false;
        m.cpu.pstate.f = false;
        let taken = m.cpu.deliver_interrupt().expect("unmasked at EL2");
        assert_eq!(
            (taken.to, taken.vector),
            (El::El2, VBAR_EL2 + CURRENT_SPX + vector)
        );

        let mut m = Machine::with_el2();
        assert_line(&mut m);
        m.hcr(HCR_TGE);
        m.at(El::El0);
        let taken = m.cpu.deliver_interrupt().expect("TGE routes it to EL2");
        assert_eq!(
            (taken.to, taken.vector),
            (El::El2, VBAR_EL2 + LOWER_A64 + vector)
        );
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
    assert_eq!(
        isr(&mut m, El::El1, 0),
        I | F,
        "nothing routed: the physical lines"
    );
    assert_eq!(
        isr(&mut m, El::El1, HCR_IMO),
        F,
        "IRQ routed: no virtual IRQ pending"
    );
    assert_eq!(isr(&mut m, El::El1, ROUTED), 0);
    assert_eq!(isr(&mut m, El::El1, ROUTED | VIRTUAL), A | I | F);
    assert_eq!(
        isr(&mut m, El::El1, HCR_FMO | VIRTUAL),
        I | F,
        "VI and VSE show only where routed"
    );
    assert_eq!(isr(&mut m, El::El1, HCR_AMO | HCR_VSE), A | I | F);
    assert_eq!(
        isr(&mut m, El::El1, HCR_TGE | VIRTUAL),
        0,
        "TGE: virtual, none pending"
    );
    assert_eq!(
        isr(&mut m, El::El2, ROUTED | VIRTUAL),
        I | F,
        "EL2: physical"
    );
    m.cpu.set_interrupt_lines(false, false);
    assert_eq!(isr(&mut m, El::El2, ROUTED | VIRTUAL), 0);
    assert_eq!(
        isr(&mut m, El::El1, VIRTUAL),
        0,
        "nothing routed: the physical lines"
    );
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
    assert_eq!(
        (m.run(WFI), m.cpu.pstate.el),
        (None, El::El1),
        "WFI does not wait"
    );

    for (el, offset) in [(El::El1, CURRENT_SPX), (El::El0, LOWER_A64)] {
        unmasked_at(&mut m, el, HCR_VI | HCR_IMO | HCR_VF | HCR_FMO);
        let taken = m.cpu.deliver_interrupt().expect("a virtual IRQ");
        let want = (ExceptionKind::VirtualIrq, El::El1, VBAR_EL1 + offset + IRQ);
        assert_eq!((taken.kind, taken.to, taken.vector), want, "{el:?}");
        assert_eq!(m.cpu.el2_state.regs.hcr & HCR_VI, HCR_VI, "VI stays set");
        unmasked_at(&mut m, el, HCR_VF | HCR_FMO);
        let taken = m.cpu.deliver_interrupt().expect("a virtual FIQ");
        assert_eq!(
            (taken.kind, taken.vector),
            (ExceptionKind::VirtualFiq, VBAR_EL1 + offset + FIQ)
        );
    }

    unmasked_at(&mut m, El::El1, HCR_VI | HCR_IMO);
    m.cpu.set_interrupt_lines(true, false);
    let taken = m.cpu.deliver_interrupt().expect("the physical IRQ first");
    assert_eq!((taken.kind, taken.to), (ExceptionKind::Irq, El::El2));
    m.cpu.set_interrupt_lines(false, false);

    m.cpu.el2_state.regs.vsesr = 0x0100_0ABC;
    unmasked_at(&mut m, El::El1, HCR_VSE | HCR_AMO);
    let taken = m.cpu.deliver_interrupt().expect("a virtual SError");
    assert_eq!(
        (taken.kind, taken.vector),
        (
            ExceptionKind::VirtualSError,
            VBAR_EL1 + CURRENT_SPX + SERROR
        )
    );
    assert_eq!(m.cpu.sys.esr_el1, (EC_SERROR << 26) | IL | 0x0100_0ABC);
    assert_eq!(
        m.cpu.el2_state.regs.hcr & HCR_VSE,
        0,
        "taking it clears VSE"
    );
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
    assert_eq!(
        v80.cpu.sys.esr_el1,
        (EC_SERROR << 26) | IL,
        "no syndrome without FEAT_RAS"
    );
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
    assert_eq!(
        m.cpu.el2_state.regs.vdisr, 0,
        "nothing pending, nothing deferred"
    );

    m.hcr(HCR_VSE);
    m.at(El::El2);
    assert_eq!(m.run(ESB), None);
    assert_eq!(
        (
            m.cpu.el2_state.regs.vdisr,
            m.cpu.el2_state.regs.hcr & HCR_VSE
        ),
        (0, HCR_VSE)
    );
}

/// EL1 reads `VPIDR_EL2` and `VMPIDR_EL2` as `MIDR_EL1` and `MPIDR_EL1`;
/// EL2 reads the processor's own, and out of reset the stand-ins are those.
#[test]
fn el1_reads_the_identity_el2_gives_it() {
    let mut m = Machine::with_el2();
    m.at(El::El1);
    assert_eq!((m.run(MRS_MIDR), m.cpu.x[0]), (None, id::MIDR_CORTEX_A57));
    assert_eq!((m.run(MRS_MPIDR), m.cpu.x[0]), (None, id::MPIDR_EL1));
    m.cpu.el2_state.regs.vpidr = 0x4100_1234;
    m.cpu.el2_state.regs.vmpidr = 0x8000_0003;
    assert_eq!((m.run(MRS_MIDR), m.cpu.x[0]), (None, 0x4100_1234));
    assert_eq!((m.run(MRS_MPIDR), m.cpu.x[0]), (None, 0x8000_0003));
    m.at(El::El2);
    assert_eq!((m.run(MRS_MIDR), m.cpu.x[0]), (None, id::MIDR_CORTEX_A57));
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
    assert_eq!(
        zt0(&mut m, El::El1, true, false),
        sync(El::El2, LOWER_A64, EC_SME, SMTC_INACCESSIBLE_ZT0)
    );
    assert_eq!(
        zt0(&mut m, El::El1, false, true),
        sync(El::El1, CURRENT_SPX, EC_SME, SMTC_INACCESSIBLE_ZT0)
    );
    assert_eq!(
        zt0(&mut m, El::El1, false, false),
        sync(El::El1, CURRENT_SPX, EC_SME, SMTC_INACCESSIBLE_ZT0)
    );
    // At EL2 `SMCR_EL1` does not apply.
    assert_eq!(zt0(&mut m, El::El2, false, true), None);
    assert_eq!(
        zt0(&mut m, El::El2, true, false),
        sync(El::El2, CURRENT_SPX, EC_SME, SMTC_INACCESSIBLE_ZT0)
    );
    // `HCR_EL2.TGE` takes EL0's trap to EL1 to EL2.
    m.hcr(HCR_TGE);
    assert_eq!(
        zt0(&mut m, El::El0, false, true),
        sync(El::El2, LOWER_A64, EC_SME, SMTC_INACCESSIBLE_ZT0)
    );
    m.cpu.set_pstate_za(false);
    assert_eq!(
        zt0(&mut m, El::El0, false, false),
        sync(El::El2, LOWER_A64, EC_SME, SMTC_INACTIVE_ZA)
    );
    m.cpu.el2_state.regs.hcr &= !HCR_TGE;
    assert_eq!(
        zt0(&mut m, El::El1, false, false),
        sync(El::El1, CURRENT_SPX, EC_SME, SMTC_INACTIVE_ZA)
    );
    assert_eq!(
        zt0(&mut m, El::El2, false, false),
        sync(El::El2, CURRENT_SPX, EC_SME, SMTC_INACTIVE_ZA)
    );
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
    assert_eq!(
        run(&mut m, El::El1, enabled, CPTR_TSM),
        sync(El::El2, LOWER_A64, EC_SME, SMTC_DISABLED)
    );
    assert_eq!(
        run(&mut m, El::El1, enabled, CPTR_TFP),
        sync(El::El2, LOWER_A64, EC_FP_ACCESS, FP_ACCESS_ISS)
    );
    assert_eq!(
        run(&mut m, El::El1, enabled, CPTR_TSM | CPTR_TFP),
        sync(El::El2, LOWER_A64, EC_SME, SMTC_DISABLED)
    );
    assert_eq!(
        run(&mut m, El::El1, CPACR_FPEN, CPTR_TSM),
        sync(El::El1, CURRENT_SPX, EC_SME, SMTC_DISABLED)
    );
    assert_eq!(
        run(&mut m, El::El1, CPACR_SMEN, CPTR_TSM),
        sync(El::El1, CURRENT_SPX, EC_FP_ACCESS, FP_ACCESS_ISS)
    );
    assert_eq!(
        run(&mut m, El::El2, 0, CPTR_TSM),
        sync(El::El2, CURRENT_SPX, EC_SME, SMTC_DISABLED)
    );
    assert_eq!(
        run(&mut m, El::El2, 0, CPTR_TFP),
        sync(El::El2, CURRENT_SPX, EC_FP_ACCESS, FP_ACCESS_ISS)
    );
    assert_eq!(run(&mut m, El::El2, 0, 0), None);
    // Under `HCR_EL2.TGE`, `CPACR_EL1`'s traps of EL0 go to EL2: SME's as
    // itself (`SMEAccessTrap`), SIMD&FP's as an uncategorized exception
    // (`AArch64_AdvSIMDFPAccessTrap`).
    m.hcr(HCR_TGE);
    assert_eq!(
        run(&mut m, El::El0, CPACR_FPEN, 0),
        sync(El::El2, LOWER_A64, EC_SME, SMTC_DISABLED)
    );
    assert_eq!(
        run(&mut m, El::El0, CPACR_SMEN, 0),
        sync(El::El2, LOWER_A64, EC_UNKNOWN, 0)
    );
    assert_eq!(
        run(&mut m, El::El0, enabled, CPTR_TSM),
        sync(El::El2, LOWER_A64, EC_SME, SMTC_DISABLED)
    );
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
        Machine::<WithEl2>::new(imp)
            .cpu
            .restore(&mut image.as_slice())
            .is_err()
    };
    let sme_alone = Implementation::every_feature_but(&[Feature::Sme2]);
    assert!(!refused(Implementation::EVERY_FEATURE, SMCR_EZT0 | 3));
    assert!(
        refused(Implementation::EVERY_FEATURE, 0x10),
        "SMCR_EL2 bit 4"
    );
    assert!(!refused(sme_alone, 3));
    assert!(
        refused(sme_alone, SMCR_EZT0),
        "SMCR_EL2.EZT0 without FEAT_SME2"
    );
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
/// Stage 2's level 1 table, whose first entry maps the first GiB of IPAs
/// to itself, Normal Write-Back memory every stage 2 permission allows.
const S2_L1: u64 = 0x1_4000;
/// A stage 2 descriptor's `S2AP` read and write bits, and its `MemAttr` of
/// Normal Inner and Outer Write-Back memory.
const S2AP_R: u64 = 1 << 6;
const S2AP_W: u64 = 1 << 7;
const S2_NORMAL: u64 = 0b1111 << 2;
/// `VTCR_EL2` of the identity stage 2: T0SZ 25 (a 39-bit IPA), SL0 1 (a
/// start at level 1), PS 40 bits.
const VTCR_IDENTITY: u64 = 25 | (1 << 6) | (0b010 << 16);

/// A Cortex-A57 at EL2 with both regimes' MMUs on over the tables above,
/// 39-bit virtual addresses (T0SZ 25) and 4 KiB pages, all memory Normal,
/// and `VTTBR_EL2` and `VTCR_EL2` describing the identity stage 2, which
/// translates nothing until `HCR_EL2.VM` or `DC` is set.
fn translating() -> Machine<WithEl2> {
    translating_on(Implementation::CORTEX_A57)
}

/// [`translating`] on a processor implementing `imp`.
fn translating_on(imp: Implementation) -> Machine<WithEl2> {
    let mut m = Machine::<WithEl2>::new(imp);
    let attrs = AF | INNER_SHAREABLE | BLOCK;
    for (l1, l2, ap) in [(EL2_L1, EL2_L2, AP1), (EL1_L1, EL1_L2, 0)] {
        m.ram.put64(l1, l2 | TABLE);
        m.ram.put64(l2, attrs | ap);
    }
    m.ram.put64(EL2_L2 + 8, EL2_DATA | attrs);
    m.ram.put64(EL1_L2 + 8, EL1_DATA | attrs);
    m.ram.put64(
        EL2_L2 + 8 * (READ_ONLY >> 21),
        READ_ONLY | attrs | AP2 | AP1,
    );
    m.ram
        .put64(EL2_L2 + 8 * (NO_EXECUTE >> 21), NO_EXECUTE | attrs | XN);
    m.ram
        .put64(EL2_L2 + 8 * (PXN_ONLY >> 21), PXN_ONLY | attrs | PXN);
    m.ram.put64(EL2_DATA, 0x2222);
    m.ram.put64(EL1_DATA, 0x1111);
    m.ram.put64(
        S2_L1,
        AF | INNER_SHAREABLE | S2AP_R | S2AP_W | S2_NORMAL | BLOCK,
    );
    let features = m.cpu.imp.features;
    let el2 = &mut m.cpu.el2_state.regs;
    // TCR_EL2: T0SZ 25, PS 40 bits; its RES1 bits come with the write.
    el2.write(El2Reg::Tcr, features, 0, 25 | (0b010 << 16));
    el2.ttbr0 = EL2_L1;
    el2.mair = 0xFF;
    el2.sctlr |= 1;
    el2.write(El2Reg::Vtcr, features, 0, VTCR_IDENTITY);
    el2.vttbr = S2_L1;
    let sys = &mut m.cpu.sys;
    sys.tcr_el1 = 25 | (1 << 23);
    sys.ttbr0_el1 = EL1_L1;
    sys.mair_el1 = 0xFF;
    sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1;
    m.at(El::El2);
    m
}

/// A write of an EL2 regime register bit its walk does not read keeps the
/// regime's translations, and one a walk reads empties the TLB: with
/// `SHARED`'s EL2 descriptor rewritten and no TLB maintenance, an `MSR` of
/// `SCTLR_EL2` flipping `C`, which the TLB holds nothing of, leaves the old
/// translation in use, and one of `MAIR_EL2` changing an attribute no
/// descriptor names makes the change seen.
#[test]
fn only_an_el2_write_its_walk_reads_empties_the_tlb() {
    const MSR_SCTLR_EL2: u32 = 0xD51C_1000; // msr sctlr_el2, x0
    const MSR_MAIR_EL2: u32 = 0xD51C_A200; // msr mair_el2, x0
    const SCTLR_C: u64 = 1 << 2;
    let mut m = translating();
    m.ram.put64(READ_ONLY, 0x7777);
    let load = |m: &mut Machine<WithEl2>| {
        m.at(El::El2);
        m.cpu.x[1] = SHARED;
        assert_eq!(m.run(LDR), None);
        m.cpu.x[2]
    };
    assert_eq!(load(&mut m), 0x2222, "SHARED's EL2 data");
    m.ram.put64(
        EL2_L2 + 8 * (SHARED >> 21),
        READ_ONLY | AF | INNER_SHAREABLE | BLOCK,
    );
    m.at(El::El2);
    m.cpu.x[0] = m.cpu.el2().sctlr ^ SCTLR_C;
    assert_eq!(m.run(MSR_SCTLR_EL2), None);
    assert_eq!(
        m.cpu.el2().sctlr & SCTLR_C,
        m.cpu.x[0] & SCTLR_C,
        "C is written"
    );
    assert_eq!(load(&mut m), 0x2222, "C flipped: the translation stays");
    m.at(El::El2);
    m.cpu.x[0] = m.cpu.el2().mair | (0x44 << 56);
    assert_eq!(m.run(MSR_MAIR_EL2), None);
    assert_eq!(
        load(&mut m),
        0x7777,
        "MAIR_EL2 changed: the walk is made again"
    );
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
    m.ram
        .put64(EL2_L2 + 8, EL1_DATA | AF | INNER_SHAREABLE | BLOCK);
    m.at(El::El2);
    assert_eq!(m.run(TLBI_ALLE2), None);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x1111, "the new mapping after TLBI ALLE2");
}

/// `HCR_EL2.DC` and `TGE` turn stage 1 of the EL1&0 regime off whatever
/// `SCTLR_EL1.M` says (Arm `AArch64_S1Enabled`): an EL1 or EL0 access
/// reaches the intermediate physical address it names, which under `DC`
/// stage 2 translates (here to itself). With stage 1 off a data access is
/// to Device-nGnRnE memory, where a misaligned one takes an alignment
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
        set_hcr(&mut m, reset | hcr);
        m.at(el);
        m.cpu.x[1] = SHARED;
        assert_eq!(m.run(LDR), None, "{el:?} HCR {hcr:#x}");
        assert_eq!(m.cpu.x[2], want, "{el:?} HCR {hcr:#x}");
    }

    m.cpu.sys.sctlr_el1 &= !1;
    m.cpu.flush_tlb();
    for (hcr, want) in [
        (
            0,
            sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, ALIGNMENT),
        ),
        (HCR_DC, None),
    ] {
        set_hcr(&mut m, reset | hcr);
        m.at(El::El1);
        m.cpu.x[1] = SHARED + 1;
        assert_eq!(m.run(LDR), want, "misaligned, HCR {hcr:#x}");
    }
    assert_eq!(m.cpu.x[2], PHYSICAL >> 8, "the Normal read");
}

/// `ID_AA64MMFR0_EL1` offers the 4 KiB granule alone at both stages on a
/// processor with EL2: `TGran4_2` reads 2 (supported), `TGran16_2` and
/// `TGran64_2` 1 (not), which is FEAT_GTG (`Features.json` makes it hold
/// exactly when all three are non-zero). Without EL2 all three read 0, and
/// either way stage 1 offers the 4 KiB granule alone.
#[test]
fn the_id_registers_offer_the_4k_granule_at_both_stages() {
    const MRS_MMFR0: u32 = 0xD538_0700; // mrs x0, id_aa64mmfr0_el1
                                        // TGran4_2, TGran16_2, TGran64_2, then TGran4, TGran16, TGran64.
    let granules = |mmfr0: u64| [40, 32, 36, 28, 20, 24].map(|lsb| (mmfr0 >> lsb) & 0xF);
    let mut with = Machine::with_el2();
    with.at(El::El1);
    assert_eq!(with.run(MRS_MMFR0), None);
    assert_eq!(granules(with.cpu.x[0]), [2, 1, 1, 0, 0, 0xF]);
    let mut without = Machine::<NoEl2>::new(Implementation::CORTEX_A57);
    without.at(El::El1);
    assert_eq!(without.run(MRS_MMFR0), None);
    assert_eq!(granules(without.cpu.x[0]), [0, 0, 0, 0, 0, 0xF]);
}

/// `DC IVAC` at EL1 translates as an invalidate even where it is carried
/// out as a clean and invalidate: under `HCR_EL2.VM` a page stage 1 makes
/// read-only refuses it a permission (`AArch64_DC` translates before it
/// changes the operation), while under `HCR_EL2.DC`, which turns off stage
/// 1 for EL1 and EL0, stage 1 has no permission to check and the identity
/// stage 2 refuses nothing.
#[test]
fn dc_ivac_at_el1_translates_as_an_invalidate_under_vm_and_dc() {
    const DC_IVAC: u32 = 0xD508_7621; // dc ivac, x1
    let mut m = translating();
    m.ram.put64(
        EL1_L2 + 8 * (READ_ONLY >> 21),
        READ_ONLY | AF | INNER_SHAREABLE | BLOCK | AP2,
    );
    let reset = m.cpu.el2_state.regs.hcr;
    let refused = sync(
        El::El1,
        CURRENT_SPX,
        EC_DATA_ABORT_SAME_EL,
        PERMISSION_L2 | CM | WNR,
    );
    for (hcr, want) in [(0, refused), (HCR_VM, refused), (HCR_DC, None)] {
        set_hcr(&mut m, reset | hcr);
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
    assert_eq!(
        m.run(STR),
        sync(
            El::El2,
            CURRENT_SPX,
            EC_DATA_ABORT_SAME_EL,
            PERMISSION_L2 | WNR
        )
    );
    assert_eq!(m.cpu.el2_state.regs.far, READ_ONLY);

    m.at(El::El2);
    m.ram.0[PXN_ONLY as usize..PXN_ONLY as usize + 4].copy_from_slice(&NOP.to_le_bytes());
    m.cpu.x[1] = PXN_ONLY;
    assert_eq!(m.run(BR_X1), None);
    assert_eq!(
        m.cpu.step(&mut m.ram, &mut NoInstrumentation),
        Step::Retired
    );
    assert_eq!(m.cpu.pc, PXN_ONLY + 4, "bit 53 does not stop EL2 executing");

    m.at(El::El2);
    m.cpu.x[1] = NO_EXECUTE;
    assert_eq!(m.run(BR_X1), None);
    assert_eq!(
        m.cpu.step(&mut m.ram, &mut NoInstrumentation),
        Step::Retired
    );
    assert_eq!(
        m.taken(),
        sync(
            El::El2,
            CURRENT_SPX,
            EC_INSTRUCTION_ABORT_SAME_EL,
            PERMISSION_L2
        )
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
    assert_eq!(
        m.run(LDR),
        None,
        "the EL2 pages of the fetch and the load are cached"
    );
    let saved = m.saved_tlb();
    // No EL1&0 context, the EL2 regime's entries carrying none; then the
    // entries; then no stage 2 translations kept, none without stage 2.
    assert!(saved.contexts().is_empty());
    assert_eq!(saved.entry_tags(), [0, 0]);
    assert_eq!(saved.0.len(), saved.kept_at() + 4);
    let image = saved.0.clone();
    let context = m.tlb_context();
    let restore = |image: &[u8]| restored::<WithEl2>(&context, image);
    let without = TlbContext {
        features: Implementation::CORTEX_A57.with_levels::<NoEl2>().features,
        ..context
    };
    assert_eq!(restore(&image), Ok(image.clone()));
    assert_eq!(
        restored::<NoEl2>(&without, &image),
        Err(SnapError::Invalid(
            "TLB entry is of a regime this processor lacks"
        ))
    );
    let mut el0 = image.clone();
    el0[saved.entries_at() + 4 + 8 + 8] |= PERM_EL0_R;
    assert_eq!(restore(&el0), Err(NOT_WALKED));
    let mut tagged = image;
    tagged[saved.entries_at() + 4 + 6] |= 1 << 4;
    assert_eq!(
        restore(&tagged),
        Err(SnapError::Invalid(
            "TLB entry's context is not one the TLB holds"
        )),
        "an EL2 entry tagged with a context"
    );
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
    assert_eq!(
        m.run(LDTR),
        sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, PERMISSION_L2)
    );
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
            let tcr = if el == El::El2 {
                &mut m.cpu.el2_state.regs.tcr
            } else {
                &mut m.cpu.sys.tcr_el1
            };
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
        m.ram.put64(
            l2 + 8 * (BLOCK_VA >> 21),
            HIGH | AF | INNER_SHAREABLE | BLOCK,
        );
        size(&mut m, 0b000);
        assert_eq!(
            load(&mut m, BLOCK_VA),
            fault(ADDRESS_SIZE | 2),
            "{el:?}: block"
        );
        let mut m = translating();
        m.ram.put64(
            l2 + 8 * (BLOCK_VA >> 21),
            HIGH | AF | INNER_SHAREABLE | BLOCK,
        );
        size(&mut m, 0b001);
        assert_eq!(
            load(&mut m, BLOCK_VA),
            fault(SEA),
            "{el:?}: block within 36 bits"
        );

        // A level 2 table at 4 GiB.
        let mut m = translating();
        m.ram.put64(l1 + 8 * (TABLE_VA >> 30), HIGH | TABLE);
        size(&mut m, 0b000);
        assert_eq!(
            load(&mut m, TABLE_VA),
            fault(ADDRESS_SIZE | 1),
            "{el:?}: table"
        );

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
    assert_eq!(
        m.run(LDR),
        sync(El::El2, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, TRANSLATION_L0)
    );
    assert_eq!(m.cpu.el2_state.regs.far, tagged);
    let mut m = translating();
    m.cpu.x[1] = SHARED | (1 << 39);
    assert_eq!(
        m.run(LDR),
        sync(El::El2, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, TRANSLATION_L0)
    );

    let mut m = translating();
    m.cpu.el2_state.regs.tcr |= super::el2::TCR_EL2_TBI;
    m.cpu.x[1] = tagged;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x2222);
}

/// A branch's target is the address `AArch64_BranchAddr` makes of it:
/// under top-byte-ignore of instruction addresses, its top byte becomes
/// copies of bit 55 in the EL1&0 regime, ones in the upper range and zeros
/// in the lower, and zeros in the EL2 regime, of one range; without it the
/// target stays as it is, for the fetch to fault on.
#[test]
fn a_branch_drops_the_tag_where_top_byte_ignore_covers_it() {
    const UPPER: u64 = 0x5A80_0000_0000_1000;
    const LOWER: u64 = 0x5A00_0000_0000_1000;
    // `TCR_EL1.TBI0` and `TBI1`.
    const TBI0: u64 = 1 << 37;
    const TBI1: u64 = 1 << 38;
    for (tcr, target, pc) in [
        (TBI0 | TBI1, UPPER, 0xFF80_0000_0000_1000),
        (TBI0 | TBI1, LOWER, 0x0000_0000_0000_1000),
        (TBI0, UPPER, UPPER),
        (TBI1, LOWER, LOWER),
    ] {
        let mut m = Machine::<NoEl2>::new(Implementation::CORTEX_A57);
        m.at(El::El1);
        m.cpu.sys.tcr_el1 = tcr;
        m.cpu.x[1] = target;
        assert_eq!(m.run(BR_X1), None);
        assert_eq!(m.cpu.pc, pc, "TCR {tcr:#x}, target {target:#x}");
    }
    let mut m = Machine::with_el2();
    m.at(El::El2);
    m.cpu.el2_state.regs.tcr |= super::el2::TCR_EL2_TBI;
    m.cpu.x[1] = UPPER;
    assert_eq!(m.run(BR_X1), None);
    assert_eq!(m.cpu.pc, 0x0080_0000_0000_1000, "EL2: zeros");
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
    assert_eq!(
        (par >> 1) & 0x3F,
        PERMISSION_L2,
        "FST: a level 2 permission fault"
    );

    m.cpu.x[0] = 1 << 39;
    assert_eq!(m.run(AT_S1E2R), None);
    let par = m.cpu.sys.par_el1;
    assert_eq!(
        (par & PAR_F, (par >> 1) & 0x3F),
        (PAR_F, TRANSLATION_L0),
        "{par:#x}"
    );
}

/// From EL2, `AT S12E1R` and `S12E1W` translate through the EL1&0 regime
/// with EL1's permissions and `AT S12E0R` and `S12E0W` with EL0's: with
/// stage 2 off (`HCR_EL2.{DC,VM}` {0,0}), `PAR_EL1` holds the stage 1
/// result. EL1's are UNDEFINED.
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
        assert_eq!(
            (par & PAR_F, par & PA),
            (0, EL1_DATA),
            "{at:#010x}: {par:#x}"
        );
    }
    for at in [AT_S12E0R, AT_S12E0W] {
        m.cpu.x[0] = SHARED;
        assert_eq!(m.run(at), None);
        let par = m.cpu.sys.par_el1;
        let fault = (par & PAR_F, (par >> 1) & 0x3F);
        assert_eq!(
            fault,
            (PAR_F, PERMISSION_L2),
            "{at:#010x}: EL0 has no access: {par:#x}"
        );
    }
    m.at(El::El1);
    assert_eq!(m.run(AT_S12E1R), sync(El::El1, CURRENT_SPX, EC_UNKNOWN, 0));
}

// ----- stage 2 -----
//
// Arm `AArch64_FullTranslate`, `AArch64_S2Translate`, `AArch64_S2Walk`,
// `AArch64_S2CheckPermissions`, `S2CombineS1MemAttrs`, `IPAValid`,
// `AArch64_FaultSyndrome` and `AArch64_InstructionSyndromeValid` with the
// Arm ARM's `ESR_ELx.ISV`, `AArch64_ReportException` (`HPFAR_EL2`), and
// `AArch64_AT` with `AArch64_EncodePAR`.

/// `ESR_ELx.ISS` of an abort: the instruction syndrome (`ISV`, `SAS`,
/// `SSE`, `SRT`, `SF`, `AR`), `CM` and `S1PTW`.
const ISV: u64 = 1 << 24;
const SSE: u64 = 1 << 21;
const SF: u64 = 1 << 15;
const AR: u64 = 1 << 14;
const CM: u64 = 1 << 8;
const S1PTW: u64 = 1 << 7;
const fn sas(log2: u64) -> u64 {
    log2 << 22
}
const fn srt(rt: u64) -> u64 {
    rt << 16
}
/// More fault status codes.
const ADDRESS_SIZE_L2: u64 = 0b00_0010;
const TRANSLATION_L2: u64 = 0b00_0110;
const ACCESS_FLAG_L2: u64 = 0b00_1010;
const ALIGNMENT_FAULT: u64 = 0b10_0001;
const EC_INSTRUCTION_ABORT_LOWER_EL: u64 = 0x20;

/// Stage 2's level 2 table under `S2_L1`'s first entry, of 2 MiB blocks.
const S2_L2: u64 = 0x1_5000;
/// Where stage 2 puts `EL1_DATA`'s IPA.
const GUEST_DATA: u64 = 0xE0_0000;
/// Each a 2 MiB block stage 1 of the EL1&0 regime maps to the same IPA:
/// stage 2 maps `S2_READ_ONLY` to itself read-only, `S2_DEVICE` to itself
/// as Device-nGnRE memory, `S2_NO_EXECUTE` to itself execute-never, and
/// `S2_UNMAPPED` not at all.
const S2_READ_ONLY: u64 = 0x80_0000;
const S2_DEVICE: u64 = 0xA0_0000;
const S2_NO_EXECUTE: u64 = 0xC0_0000;
const S2_UNMAPPED: u64 = 0x40_0000;
/// A stage 2 descriptor's `MemAttr` of Device-nGnRE memory.
const S2_DEVICE_NGNRE: u64 = 0b0001 << 2;
/// What `HPFAR_EL2` holds before an abort, so that one that does not
/// report an IPA is seen to leave it.
const HPFAR_UNTOUCHED: u64 = 0xABC0;
const MSR_HCR_EL2: u32 = 0xD51C_1100; // msr hcr_el2, x0
const MSR_VTTBR_EL2: u32 = 0xD51C_2100; // msr vttbr_el2, x0
const DC_ZVA: u32 = 0xD50B_7421; // dc zva, x1
const TLBI_VMALLS12E1: u32 = 0xD50C_87DF;
const NOT_WALKED: SnapError = SnapError::Invalid("TLB entry is not one a walk can produce");
const NO_CONTEXT: SnapError = SnapError::Invalid("TLB entry's context is not one the TLB holds");

/// `HPFAR_EL2.FIPA` reporting `ipa`: its bits 47:12 in bits 39:4.
const fn fipa(ipa: u64) -> u64 {
    (ipa >> 12) << 4
}

/// The count a saved TLB's section starts with.
fn snapshot_count(image: &[u8]) -> usize {
    u32::from_le_bytes([image[0], image[1], image[2], image[3]]) as usize
}

/// [`translating_on`] with stage 2 on (`HCR_EL2.VM`) over [`S2_L2`]: the
/// first 4 MiB of IPAs (the code, the tables and `SHARED`) map to
/// themselves, `EL1_DATA`'s IPA to [`GUEST_DATA`], and the IPAs above as
/// their names say, each of which stage 1 of the EL1&0 regime maps to
/// itself.
fn stage2_on(imp: Implementation) -> Machine<WithEl2> {
    let mut m = translating_on(imp);
    let block = AF | INNER_SHAREABLE | BLOCK;
    let rw = S2AP_R | S2AP_W;
    m.ram.put64(S2_L1, S2_L2 | TABLE);
    for (ipa, out, attrs) in [
        (0, 0, rw | S2_NORMAL),
        (SHARED, SHARED, rw | S2_NORMAL),
        (EL1_DATA, GUEST_DATA, rw | S2_NORMAL),
        (S2_READ_ONLY, S2_READ_ONLY, S2AP_R | S2_NORMAL),
        (S2_DEVICE, S2_DEVICE, rw | S2_DEVICE_NGNRE),
        (S2_NO_EXECUTE, S2_NO_EXECUTE, rw | S2_NORMAL | XN),
    ] {
        m.ram.put64(S2_L2 + 8 * (ipa >> 21), out | block | attrs);
    }
    for va in [S2_READ_ONLY, S2_DEVICE, S2_NO_EXECUTE, S2_UNMAPPED] {
        m.ram.put64(EL1_L2 + 8 * (va >> 21), va | block);
    }
    m.ram.put64(GUEST_DATA, 0x3333);
    m.cpu.el2_state.regs.hpfar = HPFAR_UNTOUCHED;
    m.hcr(HCR_VM);
    m
}

/// [`stage2_on`] a Cortex-A57.
fn stage2() -> Machine<WithEl2> {
    stage2_on(Implementation::CORTEX_A57)
}

/// Write `value` to `HCR_EL2` with an `MSR` at EL2, as a hypervisor does.
fn set_hcr(m: &mut Machine<WithEl2>, value: u64) {
    m.at(El::El2);
    m.cpu.x[0] = value;
    assert_eq!(m.run(MSR_HCR_EL2), None, "HCR_EL2 {value:#x}");
}

/// A Data Abort from EL1 taken to EL2 with `iss`.
fn stage2_abort(iss: u64) -> Option<Taken> {
    sync(El::El2, LOWER_A64, EC_DATA_ABORT_LOWER_EL, iss)
}

/// Under `HCR_EL2.VM` stage 2 translates every address the EL1&0 regime's
/// stage 1 outputs, and with stage 1 off the virtual address itself; EL2's
/// own regime has no stage 2. `HCR_EL2.TGE`, which turns EL0's stage 1 off,
/// leaves stage 2 on (`AArch64_NSS2TTWParams`). A write of `HCR_EL2` or
/// `VTTBR_EL2` changes what the next access finds.
#[test]
fn stage_2_translates_what_stage_1_outputs() {
    let mut m = stage2();
    m.at(El::El1);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None);
    assert_eq!(
        m.cpu.x[2], 0x3333,
        "SHARED, by EL1_DATA's IPA, to GUEST_DATA"
    );
    m.cpu.x[2] = 0x4444;
    assert_eq!(m.run(STR), None);
    assert_eq!(m.ram.get64(GUEST_DATA), 0x4444);

    m.at(El::El2);
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x2222, "EL2's own regime");

    m.cpu.sys.sctlr_el1 &= !1;
    m.cpu.flush_tlb();
    m.at(El::El1);
    m.cpu.x[1] = EL1_DATA;
    assert_eq!(m.run(LDR), None);
    assert_eq!(
        m.cpu.x[2], 0x4444,
        "stage 1 off: the virtual address is the IPA"
    );

    m.hcr(HCR_TGE);
    m.cpu.flush_tlb();
    m.at(El::El0);
    m.cpu.x[1] = EL1_DATA;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x4444, "EL0 under TGE");
    m.cpu.el2_state.regs.hcr &= !HCR_TGE;
    m.cpu.sys.sctlr_el1 |= 1;
    m.cpu.flush_tlb();

    // Stage 2 off by `MSR HCR_EL2`: EL1 reaches `EL1_DATA` itself.
    m.at(El::El1);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x4444);
    m.at(El::El2);
    m.cpu.x[0] = m.cpu.el2().hcr & !HCR_VM;
    assert_eq!(m.run(MSR_HCR_EL2), None);
    m.at(El::El1);
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x1111, "stage 2 off");

    // Back on, over tables that map every IPA to itself, then over
    // `S2_L1`'s again, each by `MSR VTTBR_EL2`.
    const IDENTITY_L1: u64 = 0x1_6000;
    m.ram.put64(
        IDENTITY_L1,
        AF | INNER_SHAREABLE | S2AP_R | S2AP_W | S2_NORMAL | BLOCK,
    );
    m.at(El::El2);
    m.cpu.x[0] = m.cpu.el2().hcr | HCR_VM;
    assert_eq!(m.run(MSR_HCR_EL2), None);
    for (vttbr, want) in [(IDENTITY_L1, 0x1111), (S2_L1, 0x4444)] {
        m.at(El::El2);
        m.cpu.x[0] = vttbr;
        assert_eq!(m.run(MSR_VTTBR_EL2), None);
        m.at(El::El1);
        m.cpu.x[1] = SHARED;
        assert_eq!(m.run(LDR), None);
        assert_eq!(m.cpu.x[2], want, "VTTBR_EL2 {vttbr:#x}");
    }
}

/// A stage 2 Translation, Access flag or Address size fault on an EL1
/// access is taken to EL2 (`IsSecondStage`), `FAR_EL2` holding the virtual
/// address and `HPFAR_EL2` the IPA (`IPAValid`), with the load's
/// instruction syndrome; EL1's registers are untouched.
#[test]
fn a_stage_2_fault_is_taken_to_el2_with_the_ipa() {
    let mut m = stage2();
    m.at(El::El1);
    m.cpu.x[1] = S2_UNMAPPED;
    let syndrome = ISV | sas(3) | srt(2) | SF;
    assert_eq!(m.run(LDR), stage2_abort(syndrome | TRANSLATION_L2));
    let el2 = m.cpu.el2();
    assert_eq!((el2.far, el2.hpfar), (S2_UNMAPPED, fipa(S2_UNMAPPED)));
    assert_eq!((m.cpu.sys.esr_el1, m.cpu.sys.far_el1), (0, 0));

    let entry = S2_L2 + 8 * (S2_UNMAPPED >> 21);
    let rw = INNER_SHAREABLE | S2AP_R | S2AP_W | S2_NORMAL | BLOCK;
    for (descriptor, fault) in [
        (S2_UNMAPPED | rw, ACCESS_FLAG_L2),
        // An output address beyond `VTCR_EL2.PS`'s 40 bits.
        ((1 << 40) | AF | rw, ADDRESS_SIZE_L2),
    ] {
        m.ram.put64(entry, descriptor);
        m.cpu.el2_state.regs.hpfar = HPFAR_UNTOUCHED;
        m.at(El::El1);
        assert_eq!(m.run(LDR), stage2_abort(syndrome | fault), "{fault:#x}");
        assert_eq!(m.cpu.el2().hpfar, fipa(S2_UNMAPPED), "{fault:#x}");
    }
}

/// A permission stage 2 refuses is a Permission fault taken to EL2, which
/// reports no IPA (`HPFAR_EL2` is ARBITRARY: it keeps its value), whether
/// the TLB holds the page or a walk finds it; a fetch stage 2 refuses
/// execution is an Instruction Abort to EL2.
#[test]
fn stage_2_permission_faults_are_taken_to_el2() {
    let mut m = stage2();
    m.at(El::El1);
    m.cpu.x[1] = S2_READ_ONLY;
    assert_eq!(
        m.run(LDR),
        None,
        "the read is permitted, and caches the page"
    );
    let refused = stage2_abort(ISV | sas(3) | srt(2) | SF | WNR | PERMISSION_L2);
    assert_eq!(m.run(STR), refused, "the write, from the TLB");
    m.cpu.flush_tlb();
    m.at(El::El1);
    assert_eq!(m.run(STR), refused, "the write, from a walk");
    assert_eq!(
        (m.cpu.el2().far, m.cpu.el2().hpfar),
        (S2_READ_ONLY, HPFAR_UNTOUCHED)
    );

    m.at(El::El1);
    m.cpu.x[1] = S2_NO_EXECUTE;
    assert_eq!(m.run(BR_X1), None);
    assert_eq!(
        m.cpu.step(&mut m.ram, &mut NoInstrumentation),
        Step::Retired
    );
    assert_eq!(
        m.taken(),
        sync(
            El::El2,
            LOWER_A64,
            EC_INSTRUCTION_ABORT_LOWER_EL,
            PERMISSION_L2
        )
    );
    assert_eq!(m.cpu.el2().far, S2_NO_EXECUTE);
}

/// A stage 2 Data Abort on the access of a load or store of one
/// general-purpose register without writeback reports its syndrome
/// (`ISV`): the access size, sign extension, register, its width and
/// acquire or release semantics. One with writeback, a pair, an exclusive
/// and a SIMD&FP load report none.
#[test]
fn a_stage_2_data_abort_reports_a_single_register_access_syndrome() {
    let cases: [(u32, u64); 19] = [
        (0xF940_0022, ISV | sas(3) | srt(2) | SF),  // ldr x2, [x1]
        (0x3940_0023, ISV | sas(0) | srt(3)),       // ldrb w3, [x1]
        (0x39C0_0023, ISV | sas(0) | SSE | srt(3)), // ldrsb w3, [x1]
        (0x7980_0024, ISV | sas(1) | SSE | srt(4) | SF), // ldrsh x4, [x1]
        (0x79C0_0024, ISV | sas(1) | SSE | srt(4)), // ldrsh w4, [x1]
        (0xB980_0025, ISV | sas(2) | SSE | srt(5) | SF), // ldrsw x5, [x1]
        (0xB900_0026, ISV | sas(2) | srt(6) | WNR), // str w6, [x1]
        (0x7900_0426, ISV | sas(1) | srt(6) | WNR), // strh w6, [x1, #2]
        (0xF840_8027, ISV | sas(3) | srt(7) | SF),  // ldur x7, [x1, #8]
        (0xF869_6828, ISV | sas(3) | srt(8) | SF),  // ldr x8, [x1, x9]
        (0xF840_082A, ISV | sas(3) | srt(10) | SF), // ldtr x10, [x1]
        (0xF800_0822, ISV | sas(3) | srt(2) | SF | WNR), // sttr x2, [x1]
        (0xC8DF_FC2B, ISV | sas(3) | srt(11) | SF | AR), // ldar x11, [x1]
        (0x889F_FC2C, ISV | sas(2) | srt(12) | AR | WNR), // stlr w12, [x1]
        (0xF840_842E, 0),                           // ldr x14, [x1], #8
        (0xF840_8C2F, 0),                           // ldr x15, [x1, #8]!
        (0xA940_0C22, 0),                           // ldp x2, x3, [x1]
        (0xC85F_7C22, 0),                           // ldxr x2, [x1]
        (0xFD40_0020, 0),                           // ldr d0, [x1]
    ];
    let mut m = stage2();
    m.cpu.sys.cpacr_el1 = CPACR_FPEN;
    // EL0 may read and write the page at stage 1 too (AP[1]), which LDTR
    // and STTR at EL1 are checked against.
    m.ram.put64(
        EL1_L2 + 8 * (S2_UNMAPPED >> 21),
        S2_UNMAPPED | AF | INNER_SHAREABLE | BLOCK | AP1,
    );
    for (insn, iss) in cases {
        m.at(El::El1);
        m.cpu.x[1] = S2_UNMAPPED;
        m.cpu.x[9] = 0;
        assert_eq!(
            m.run(insn),
            stage2_abort(iss | TRANSLATION_L2),
            "{insn:#010x}"
        );
        assert_eq!(m.cpu.x[1], S2_UNMAPPED, "{insn:#010x}: no writeback");
    }

    // LDR (literal) with stage 1 off, from the end of the IPAs stage 2 maps
    // to the next, which it does not.
    m.cpu.sys.sctlr_el1 &= !1;
    m.cpu.flush_tlb();
    m.at(El::El1);
    let ldr_literal = 0x5800_0082; // ldr x2, .+16
    assert_eq!(
        m.run_at(S2_UNMAPPED - 16, ldr_literal),
        stage2_abort(ISV | sas(3) | srt(2) | SF | TRANSLATION_L2)
    );
}

/// The later single-register loads and stores report their syndrome too —
/// LDAPR, LDAPUR* and STLUR (acquire or release), and LDRAA without
/// writeback — and an atomic access none.
#[test]
fn later_single_register_loads_and_stores_report_their_syndrome() {
    let cases: [(u32, u64); 7] = [
        (0xF8BF_C02D, ISV | sas(3) | srt(13) | SF | AR), // ldapr x13, [x1]
        (0x19C0_002E, ISV | sas(0) | SSE | srt(14) | AR), // ldapursb w14, [x1]
        (0xD900_002F, ISV | sas(3) | srt(15) | SF | AR | WNR), // stlur x15, [x1]
        (0xF820_0430, ISV | sas(3) | srt(16) | SF),      // ldraa x16, [x1]
        (0xF820_1C31, 0),                                // ldraa x17, [x1, #8]!
        (0xF822_0023, 0),                                // ldadd x2, x3, [x1]
        (0xF822_8023, 0),                                // swp x2, x3, [x1]
    ];
    let mut m = stage2_on(Implementation::EVERY_FEATURE);
    for (insn, iss) in cases {
        m.at(El::El1);
        m.cpu.x[1] = S2_UNMAPPED;
        assert_eq!(
            m.run(insn),
            stage2_abort(iss | TRANSLATION_L2),
            "{insn:#010x}"
        );
    }
}

/// A misaligned access split into bytes reports the syndrome only for a
/// fault on its first byte (`accdesc.lowestaddress`).
#[test]
fn a_misaligned_access_reports_no_syndrome_past_its_first_byte() {
    let mut m = stage2();
    m.at(El::El1);
    m.cpu.x[1] = S2_UNMAPPED - 4;
    assert_eq!(m.run(LDR), stage2_abort(TRANSLATION_L2));
    assert_eq!(m.cpu.el2().far, S2_UNMAPPED, "the first byte that faulted");
    m.at(El::El1);
    m.cpu.x[1] = S2_UNMAPPED + 0xFFC;
    assert_eq!(
        m.run(LDR),
        stage2_abort(ISV | sas(3) | srt(2) | SF | TRANSLATION_L2)
    );
}

/// A stage 2 fault on a descriptor stage 1's walk reads is taken to EL2
/// with `S1PTW` and no instruction syndrome, reporting the descriptor's IPA
/// (`IPAValid` on a walk, for a Permission fault too). Stage 2 maps a
/// table in Device memory as Normal Non-cacheable for the walk unless
/// `HCR_EL2.PTW` refuses it, and a table needs stage 2's read permission.
#[test]
fn a_stage_2_fault_on_stage_1s_walk_reports_s1ptw() {
    // The virtual address `EL1_L1`'s second entry translates.
    const WALKED: u64 = 1 << 30;
    let mut m = stage2();
    m.ram.put64(EL1_L1 + 8, S2_UNMAPPED | TABLE);
    m.at(El::El1);
    m.cpu.x[1] = WALKED;
    assert_eq!(m.run(LDR), stage2_abort(S1PTW | TRANSLATION_L2));
    assert_eq!(
        (m.cpu.el2().far, m.cpu.el2().hpfar),
        (WALKED, fipa(S2_UNMAPPED))
    );
    m.at(El::El1);
    assert_eq!(
        m.run(STR),
        stage2_abort(S1PTW | WNR | TRANSLATION_L2),
        "a store's"
    );

    m.ram.put64(EL1_L1 + 8, S2_DEVICE | TABLE);
    m.ram
        .put64(S2_DEVICE, EL1_DATA | AF | INNER_SHAREABLE | BLOCK);
    m.at(El::El1);
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x3333, "through a table in Device memory");
    m.hcr(HCR_PTW);
    m.cpu.flush_tlb();
    m.cpu.el2_state.regs.hpfar = HPFAR_UNTOUCHED;
    m.at(El::El1);
    assert_eq!(m.run(LDR), stage2_abort(S1PTW | PERMISSION_L2), "PTW");
    assert_eq!(m.cpu.el2().hpfar, fipa(S2_DEVICE));

    m.cpu.el2_state.regs.hcr &= !HCR_PTW;
    m.ram.put64(
        S2_L2 + 8 * (S2_DEVICE >> 21),
        S2_DEVICE | AF | INNER_SHAREABLE | S2AP_W | S2_NORMAL | BLOCK,
    );
    m.cpu.flush_tlb();
    m.at(El::El1);
    assert_eq!(
        m.run(LDR),
        stage2_abort(S1PTW | PERMISSION_L2),
        "a table stage 2 gives no read permission"
    );
}

/// Device memory at stage 2 refuses a misaligned access, and `DC ZVA`
/// whatever its alignment, with an Alignment fault taken to EL2
/// (`AArch64_S2HasAlignmentFaultDueToMemType`), which reports no IPA;
/// Device memory at stage 1 refuses the misaligned access first, to EL1.
#[test]
fn device_memory_at_stage_2_refuses_misaligned_accesses() {
    let mut m = stage2();
    m.at(El::El1);
    m.cpu.x[1] = S2_DEVICE;
    assert_eq!(m.run(LDR), None, "aligned");
    m.cpu.x[1] = S2_DEVICE + 1;
    assert_eq!(
        m.run(LDR),
        stage2_abort(ISV | sas(3) | srt(2) | SF | ALIGNMENT_FAULT)
    );
    assert_eq!(
        (m.cpu.el2().far, m.cpu.el2().hpfar),
        (S2_DEVICE + 1, HPFAR_UNTOUCHED)
    );
    m.at(El::El1);
    m.cpu.x[1] = S2_DEVICE;
    assert_eq!(m.run(DC_ZVA), stage2_abort(WNR | ALIGNMENT_FAULT));

    // `MAIR_EL1.Attr1` Device-nGnRnE for the page at stage 1 as well.
    m.cpu.sys.mair_el1 = 0x00FF;
    m.ram.put64(
        EL1_L2 + 8 * (S2_DEVICE >> 21),
        S2_DEVICE | AF | INNER_SHAREABLE | BLOCK | (1 << 2),
    );
    m.cpu.flush_tlb();
    m.at(El::El1);
    m.cpu.x[1] = S2_DEVICE + 1;
    assert_eq!(
        m.run(LDR),
        sync(El::El1, CURRENT_SPX, EC_DATA_ABORT_SAME_EL, ALIGNMENT_FAULT)
    );
}

/// Stage 1's faults come before stage 2 translates: a write stage 1 refuses
/// is EL1's Permission fault though stage 2 maps nothing there, and the
/// read it permits meets stage 2's fault.
#[test]
fn stage_1_faults_come_before_stage_2s() {
    let mut m = stage2();
    m.ram.put64(
        EL1_L2 + 8 * (S2_UNMAPPED >> 21),
        S2_UNMAPPED | AF | INNER_SHAREABLE | BLOCK | AP2,
    );
    m.at(El::El1);
    m.cpu.x[1] = S2_UNMAPPED;
    assert_eq!(
        m.run(STR),
        sync(
            El::El1,
            CURRENT_SPX,
            EC_DATA_ABORT_SAME_EL,
            PERMISSION_L2 | WNR
        )
    );
    assert_eq!(m.cpu.sys.far_el1, S2_UNMAPPED);
    m.at(El::El1);
    assert_eq!(
        m.run(LDR),
        stage2_abort(ISV | sas(3) | srt(2) | SF | TRANSLATION_L2)
    );
}

/// An atomic access needs read and write permission at each stage, checked
/// in Arm's order — stage 1's read and write, then stage 2's read and
/// write — `WnR` saying whether a write was refused.
#[test]
fn an_atomic_access_meets_each_stages_permissions_in_order() {
    const LDADD: u32 = 0xF822_0023; // ldadd x2, x3, [x1]
    let mut m = stage2_on(Implementation::EVERY_FEATURE);
    let stage1 = EL1_L2 + 8 * (S2_DEVICE >> 21);
    let stage2 = S2_L2 + 8 * (S2_DEVICE >> 21);
    m.ram
        .put64(stage1, S2_DEVICE | AF | INNER_SHAREABLE | BLOCK | AP2);
    m.ram.put64(
        stage2,
        S2_DEVICE | AF | INNER_SHAREABLE | S2AP_W | S2_NORMAL | BLOCK,
    );
    m.at(El::El1);
    m.cpu.x[1] = S2_DEVICE;
    assert_eq!(
        m.run(LDADD),
        sync(
            El::El1,
            CURRENT_SPX,
            EC_DATA_ABORT_SAME_EL,
            PERMISSION_L2 | WNR
        ),
        "stage 1's write before stage 2's read"
    );
    m.ram
        .put64(stage1, S2_DEVICE | AF | INNER_SHAREABLE | BLOCK);
    m.cpu.flush_tlb();
    m.at(El::El1);
    assert_eq!(m.run(LDADD), stage2_abort(PERMISSION_L2), "stage 2's read");
    m.at(El::El1);
    m.cpu.x[1] = S2_READ_ONLY;
    assert_eq!(
        m.run(LDADD),
        stage2_abort(PERMISSION_L2 | WNR),
        "stage 2's write"
    );
    m.at(El::El1);
    m.cpu.x[1] = SHARED;
    m.cpu.x[2] = 5;
    assert_eq!(m.run(LDADD), None);
    assert_eq!((m.cpu.x[3], m.ram.get64(GUEST_DATA)), (0x3333, 0x3338));
}

/// `AT S12E*` from EL2 translates by both stages and reports the output
/// address with the attributes `S2CombineS1MemAttrs` gives it, and a fault
/// of either stage, stage 2's with `PAR_EL1.S`; `AT S1E*` reports the IPA.
/// Stage 1's Normal memory is Non-cacheable with `SCTLR_EL1.C` clear and
/// stage 2's with `HCR_EL2.CD` set. A stage 2 fault on stage 1's walk is
/// reported with `PAR_EL1.PTW` from EL2, and taken to EL2 from EL1.
#[test]
fn at_translates_through_both_stages() {
    const AT_S12E1R: u32 = 0xD50C_7880; // at s12e1r, x0
    const AT_S12E1W: u32 = 0xD50C_78A0; // at s12e1w, x0
    const AT_S1E1R: u32 = 0xD508_7800; // at s1e1r, x0
    const SCTLR_C: u64 = 1 << 2;
    const PAR_F: u64 = 1;
    const PAR_PTW: u64 = 1 << 8;
    const PAR_S: u64 = 1 << 9;
    const NS: u64 = 1 << 9;
    const RES1: u64 = 1 << 11;
    let par = |attr: u64, pa: u64, sh: u64| (attr << 56) | pa | NS | (sh << 7) | RES1;
    let fault = |fst: u64, stage: u64| RES1 | stage | (fst << 1) | PAR_F;
    let mut m = stage2();
    m.cpu.sys.sctlr_el1 |= SCTLR_C;
    let at = |m: &mut Machine<WithEl2>, insn: u32, va: u64| {
        m.at(El::El2);
        m.cpu.x[0] = va;
        assert_eq!(m.run(insn), None, "{insn:#010x} {va:#x}");
        m.cpu.sys.par_el1
    };
    assert_eq!(
        at(&mut m, AT_S12E1R, SHARED + 0x123),
        par(0xFF, GUEST_DATA, 0b11)
    );
    assert_eq!(
        at(&mut m, AT_S1E1R, SHARED + 0x123),
        par(0xFF, EL1_DATA, 0b11),
        "the IPA"
    );
    assert_eq!(
        at(&mut m, AT_S12E1R, S2_DEVICE),
        par(0x04, S2_DEVICE, 0b10),
        "Device-nGnRE"
    );
    assert_eq!(
        at(&mut m, AT_S12E1W, S2_READ_ONLY),
        fault(PERMISSION_L2, PAR_S)
    );
    assert_eq!(
        at(&mut m, AT_S12E1R, S2_UNMAPPED),
        fault(TRANSLATION_L2, PAR_S)
    );
    m.hcr(HCR_CD);
    assert_eq!(
        at(&mut m, AT_S12E1R, SHARED),
        par(0x44, GUEST_DATA, 0b10),
        "HCR_EL2.CD"
    );
    m.cpu.el2_state.regs.hcr &= !HCR_CD;
    m.cpu.sys.sctlr_el1 &= !SCTLR_C;
    assert_eq!(
        at(&mut m, AT_S12E1R, SHARED),
        par(0x44, GUEST_DATA, 0b10),
        "SCTLR_EL1.C clear"
    );

    m.ram.put64(EL1_L1 + 8, S2_UNMAPPED | TABLE);
    assert_eq!(
        at(&mut m, AT_S1E1R, 1 << 30),
        fault(TRANSLATION_L2, PAR_S | PAR_PTW)
    );
    let before = m.cpu.sys.par_el1;
    m.at(El::El1);
    m.cpu.x[0] = 1 << 30;
    assert_eq!(
        m.run(AT_S1E1R),
        stage2_abort(CM | WNR | S1PTW | TRANSLATION_L2)
    );
    assert_eq!(m.cpu.sys.par_el1, before, "PAR_EL1 left as it was");
    assert_eq!(m.cpu.el2().hpfar, fipa(S2_UNMAPPED));
}

/// The stage 2 configurations `AArch64_S2Translate` refuses each take a
/// level 0 Translation fault, reporting the IPA: `T0SZ` below 64 - PAMax
/// (20) or above 39 (the IMPLEMENTATION DEFINED faults, as stage 1's),
/// `SL0` = 3 without FEAT_TTST, a start level that cannot resolve the
/// input size (`AArch64_S2InconsistentSL`), and an IPA wider than `T0SZ`
/// gives.
#[test]
fn stage_2_configurations_arm_refuses_take_a_level_0_translation_fault() {
    const PS_40: u64 = 0b010 << 16;
    let mut m = stage2();
    m.cpu.sys.sctlr_el1 &= !1;
    let features = m.cpu.imp.features;
    for vtcr in [
        19 | (2 << 6) | PS_40,
        40 | (2 << 6) | PS_40,
        25 | (3 << 6) | PS_40,
        25 | (2 << 6) | PS_40,
        34 | (1 << 6) | PS_40,
    ] {
        m.cpu.el2_state.regs.write(El2Reg::Vtcr, features, 0, vtcr);
        m.cpu.flush_tlb();
        m.cpu.el2_state.regs.hpfar = HPFAR_UNTOUCHED;
        m.at(El::El1);
        assert_eq!(
            m.run(NOP),
            sync(
                El::El2,
                LOWER_A64,
                EC_INSTRUCTION_ABORT_LOWER_EL,
                TRANSLATION_L0
            ),
            "VTCR_EL2 {vtcr:#x}: the fetch's"
        );
        assert_eq!(m.cpu.el2().hpfar, fipa(CODE), "VTCR_EL2 {vtcr:#x}");
    }

    // A 30-bit IPA space from level 2, at `S2_L2`; the data at 1 GiB.
    m.cpu
        .el2_state
        .regs
        .write(El2Reg::Vtcr, features, 0, 34 | PS_40);
    m.cpu.el2_state.regs.vttbr = S2_L2;
    m.cpu.flush_tlb();
    m.at(El::El1);
    m.cpu.x[1] = 1 << 30;
    assert_eq!(
        m.run(LDR),
        stage2_abort(ISV | sas(3) | srt(2) | SF | TRANSLATION_L0)
    );
    assert_eq!(m.cpu.el2().hpfar, fipa(1 << 30));
}

/// With a 40-bit IPA from level 1, the start level is two concatenated
/// tables, the second translating IPAs from 512 GiB; `VTTBR_EL2.BADDR`'s
/// bits below the tables' alignment are ignored (`AArch64_S2TTBaseAddress`).
#[test]
fn stage_2_starts_in_concatenated_tables() {
    const CONCATENATED: u64 = 0x1_6000;
    const HIGH: u64 = 1 << 39;
    let mut m = stage2();
    let block = AF | INNER_SHAREABLE | S2AP_R | S2AP_W | S2_NORMAL | BLOCK;
    m.ram.put64(CONCATENATED, block);
    m.ram.put64(CONCATENATED + 0x1000, block);
    let features = m.cpu.imp.features;
    m.cpu
        .el2_state
        .regs
        .write(El2Reg::Vtcr, features, 0, 24 | (1 << 6) | (0b010 << 16));
    m.cpu.sys.sctlr_el1 &= !1;
    for vttbr in [CONCATENATED, CONCATENATED + 0x1000] {
        m.cpu.el2_state.regs.vttbr = vttbr;
        m.cpu.flush_tlb();
        m.at(El::El1);
        m.cpu.x[1] = HIGH + EL1_DATA;
        assert_eq!(m.run(LDR), None, "VTTBR_EL2 {vttbr:#x}");
        assert_eq!(m.cpu.x[2], 0x1111, "VTTBR_EL2 {vttbr:#x}");
    }
}

/// A debugger's walk goes through both stages, reporting the stage of a
/// fault, and checks none of the access's permissions.
#[test]
fn a_debugger_walks_both_stages() {
    let mut m = stage2();
    m.at(El::El1);
    let walk = |m: &mut Machine<WithEl2>, va: u64| m.cpu.debug_translate(&mut m.ram, va);
    assert_eq!(walk(&mut m, SHARED + 8), Ok(GUEST_DATA + 8));
    assert_eq!(walk(&mut m, S2_READ_ONLY), Ok(S2_READ_ONLY));
    let level2 = TranslationFault::Translation { level: 2 };
    assert_eq!(
        walk(&mut m, S2_UNMAPPED),
        Err(NoTranslation {
            stage: TranslationStage::Two {
                ipa: Ipa::new(S2_UNMAPPED)
            },
            fault: level2,
        })
    );
    m.ram.put64(EL1_L1 + 8, S2_UNMAPPED | TABLE);
    assert_eq!(
        walk(&mut m, 1 << 30),
        Err(NoTranslation {
            stage: TranslationStage::TwoOnWalk {
                ipa: Ipa::new(S2_UNMAPPED)
            },
            fault: level2,
        })
    );
}

/// Cache maintenance by address needs at stage 2 what it needs at stage 1:
/// `DC IVAC` write permission, a clean at EL1 none; its faults are taken
/// to EL2 with `CM` and `WnR`.
#[test]
fn cache_maintenance_meets_stage_2s_permissions() {
    const DC_IVAC: u32 = 0xD508_7621; // dc ivac, x1
    const DC_CVAC: u32 = 0xD50B_7A21; // dc cvac, x1
    let mut m = stage2();
    m.at(El::El1);
    m.cpu.x[1] = S2_READ_ONLY;
    assert_eq!(m.run(DC_CVAC), None);
    assert_eq!(m.run(DC_IVAC), stage2_abort(CM | WNR | PERMISSION_L2));
    m.at(El::El1);
    m.cpu.x[1] = S2_UNMAPPED;
    assert_eq!(m.run(DC_CVAC), stage2_abort(CM | WNR | TRANSLATION_L2));
    assert_eq!(m.cpu.el2().hpfar, fipa(S2_UNMAPPED));
}

/// The TLB holds both stages' translation: after stage 2's table changes,
/// `TLBI VMALLS12E1` from EL2 makes the change seen. A snapshot keeps such
/// an entry, and refuses one no stage 2 walk makes.
#[test]
fn the_tlb_holds_both_stages() {
    let mut m = stage2();
    m.at(El::El1);
    m.cpu.x[1] = S2_READ_ONLY;
    assert_eq!(m.run(LDR), None);

    let saved = m.saved_tlb();
    let image = saved.0.clone();
    let context = m.tlb_context();
    let restore = |context: TlbContext, image: &[u8]| restored::<WithEl2>(&context, image);
    assert_eq!(restore(context, &image), Ok(image.clone()));
    let (place, context_at) = saved.contexts()[0];
    assert_eq!(
        (place, saved.current()),
        (0, 1),
        "the guest's context, current"
    );
    let data = saved.entry_at(S2_READ_ONLY);
    // Stage 2's permissions byte: read at EL1 but not at EL0 is no stage 2
    // descriptor's.
    let mut odd = image.clone();
    odd[data + ENTRY_S2_PERMS] = 1;
    assert_eq!(restore(context, &odd), Err(NOT_WALKED));
    let without = TlbContext {
        features: Implementation::CORTEX_A57.with_levels::<NoEl2>().features,
        ..context
    };
    assert_eq!(
        restored::<NoEl2>(&without, &image),
        Err(SnapError::Invalid(
            "TLB holds a context no processor of its features makes"
        )),
        "a stage 2 context on a processor without EL2"
    );
    let elsewhere = TlbContext {
        el10: El10Context {
            vttbr_el2: context.el10.vttbr_el2 ^ (1 << 48),
            ..context.el10
        },
        ..context
    };
    assert_eq!(
        restore(elsewhere, &image),
        Err(SnapError::Invalid(
            "TLB's current context is not the registers'"
        ))
    );
    // The context, its stage 2 turned off: its entries and kept
    // translations are none its walks make. (Its current tag is cleared,
    // the registers no longer its, and stage 2's registers, which a
    // context without stage 2 does not hold.)
    let mut stage_2_off = image.clone();
    stage_2_off[context_at + CONTEXT_HCR] &= !(HCR_VM as u8);
    stage_2_off[context_at + CONTEXT_VTTBR..context_at + CONTEXT_VTCR + 8].fill(0);
    stage_2_off[saved.current_at()] = 0;
    let mut unheld = stage_2_off.clone();
    unheld[context_at + CONTEXT_VTCR] = 1;
    assert_eq!(
        restore(context, &unheld),
        Err(SnapError::Invalid(
            "TLB holds a context no processor of its features makes"
        )),
        "a context without stage 2 holding a VTCR_EL2"
    );
    assert_eq!(
        restore(context, &stage_2_off),
        Err(NOT_WALKED),
        "a stage 2 entry where stage 2 translates nothing"
    );

    // With stage 2 off in the registers, no stage 2 context is current.
    let mut el10_unfound = image.clone();
    el10_unfound[saved.current_at()] = 0;
    let stage_2_off_now = TlbContext {
        el10: El10Context {
            hcr_el2: context.el10.hcr_el2 & !HCR_VM,
            ..context.el10
        }
        .normalized(),
        ..context
    };
    assert_eq!(
        restore(stage_2_off_now, &el10_unfound),
        Err(SnapError::Invalid(
            "TLB's current context is not the registers'"
        )),
        "a stage 2 context current where stage 2 translates nothing"
    );

    // The stage 2 translations kept apart from the entries, which the walks
    // of the fetch and the load made, each in stage 2's context alone
    // (`VTTBR_EL2` and `VTCR_EL2`): restored whatever the EL1&0 contexts,
    // and refused where the table of stage 2 contexts does not hold theirs
    // or where no walk makes one.
    let without_entries = |image: &[u8]| {
        let mut kept_only = image[..saved.entries_at()].to_vec();
        kept_only.extend_from_slice(&0u32.to_le_bytes());
        kept_only.extend_from_slice(&image[saved.kept_at()..]);
        kept_only
    };
    let kept_only = without_entries(&image);
    assert_ne!(saved.kept(), 0, "the walks kept IPAs");
    assert_eq!(
        image.len(),
        saved.kept_at() + 4 + saved.kept() * KEPT_BYTES,
        "the kept translations end the image"
    );
    assert_eq!(restore(context, &kept_only), Ok(kept_only.clone()));
    let stage2 = saved.stage2();
    let (s2_place, s2_at) = stage2.held[0];
    assert_eq!(stage2.held.len(), 1);
    assert_eq!(
        (s2_place, image[stage2.current_at()]),
        (0, 1),
        "the guest's stage 2 context, current"
    );
    assert_eq!(
        (saved.u64_at(s2_at), saved.u64_at(s2_at + 8)),
        (context.el10.vttbr_el2, context.el10.vtcr_el2)
    );
    // The table without it, its place empty and none current.
    let mut unheld_stage2 = kept_only[..s2_at - 1].to_vec();
    unheld_stage2.push(0);
    unheld_stage2.extend_from_slice(&kept_only[s2_at + STAGE2_CONTEXT_BYTES..]);
    unheld_stage2[stage2.current_at() - STAGE2_CONTEXT_BYTES] = 0;
    assert_eq!(restore(context, &unheld_stage2), Err(NO_CONTEXT));
    let mut asymmetric = kept_only.clone();
    // The first kept translation's permissions byte, after its IPA and PA.
    asymmetric[saved.entries_at() + 4 + 4 + 16] = 1;
    assert_eq!(restore(context, &asymmetric), Err(NOT_WALKED));

    m.ram.put64(
        S2_L2 + 8 * (S2_READ_ONLY >> 21),
        S2_READ_ONLY | AF | INNER_SHAREABLE | S2AP_R | S2AP_W | S2_NORMAL | BLOCK,
    );
    m.at(El::El2);
    assert_eq!(m.run(TLBI_VMALLS12E1), None);
    m.at(El::El1);
    m.cpu.x[2] = 0x5555;
    assert_eq!(m.run(STR), None);
    assert_eq!(m.ram.get64(S2_READ_ONLY), 0x5555);
}

/// After stage 2's table changes, `TLBI VMALLS12E1` makes the change seen
/// by every virtual address stage 1 maps to the IPA, not only by the one
/// that reached it before: a second address reaching `EL1_DATA`'s IPA
/// finds the new output.
#[test]
fn tlbi_vmalls12e1_reaches_every_address_of_a_changed_ipa() {
    let mut m = stage2();
    m.ram.put64(
        EL1_L2 + 8 * (EL1_DATA >> 21),
        EL1_DATA | AF | INNER_SHAREABLE | BLOCK,
    );
    m.at(El::El1);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x3333, "EL1_DATA's IPA is GUEST_DATA");

    m.ram.put64(
        S2_L2 + 8 * (EL1_DATA >> 21),
        EL1_DATA | AF | INNER_SHAREABLE | S2AP_R | S2AP_W | S2_NORMAL | BLOCK,
    );
    m.at(El::El2);
    assert_eq!(m.run(TLBI_VMALLS12E1), None);
    m.at(El::El1);
    m.cpu.x[1] = EL1_DATA;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x1111, "EL1_DATA's IPA is now itself");
}

/// With stage 1 of the EL1&0 regime off, the TLB holds stage 2's
/// translation of each address alone, and its permissions refuse a hit as
/// a walk's do. A snapshot keeps such an entry where stage 1 is off, and
/// refuses it where stage 1 would have walked.
#[test]
fn an_entry_without_stage_1_holds_stage_2s_permissions() {
    let mut m = stage2();
    m.cpu.sys.sctlr_el1 &= !1;
    m.at(El::El1);
    m.cpu.x[1] = S2_READ_ONLY;
    assert_eq!(
        m.run(LDR),
        None,
        "the read is permitted, and caches the page"
    );
    let refused = stage2_abort(ISV | sas(3) | srt(2) | SF | WNR | PERMISSION_L2);
    m.at(El::El1);
    assert_eq!(m.run(STR), refused, "the write, from the TLB");

    let saved = m.saved_tlb();
    let image = saved.0.clone();
    let context = m.tlb_context();
    let restore = |context: TlbContext, image: &[u8]| restored::<WithEl2>(&context, image);
    let data = saved.entry_at(S2_READ_ONLY);
    assert_eq!(image[data + ENTRY_STAGE1], 1, "stage 1 disabled");
    assert_eq!(restore(context, &image), Ok(image.clone()));
    // The context, its stage 1 turned on (its current tag cleared, the
    // registers no longer its).
    let (_, context_at) = saved.contexts()[0];
    let mut stage_1_on = image.clone();
    stage_1_on[context_at + CONTEXT_SCTLR] |= 1;
    stage_1_on[saved.current_at()] = 0;
    assert_eq!(restore(context, &stage_1_on), Err(NOT_WALKED));
}

/// A hypervisor's switch from a guest to its own EL1&0 regime and back —
/// `HCR_EL2.VM` cleared and set again, `VTTBR_EL2` its own (VMID 0, no
/// tables) and the guest's again, `VTCR_EL2` another guest's or the
/// guest's again — keeps both sides' translations: with `SHARED`'s stage 1
/// descriptor rewritten and no TLB maintenance, which leaves a TLB free to
/// go on using what it holds, each side still reads its own memory there
/// after every switch, until `TLBI VMALLS12E1` makes the guest see the
/// change. Stage 2's registers, which the hypervisor's own translations do
/// not read, never cost it its translation.
#[test]
fn a_world_switch_keeps_both_sides_translations() {
    const MSR_VTCR_EL2: u32 = 0xD51C_2140; // msr vtcr_el2, x0
    let mut m = stage2();
    let (guest_vttbr, guest_hcr) = (m.cpu.el2().vttbr, m.cpu.el2().hcr);
    let guest_vtcr = m.cpu.el2().vtcr;
    m.ram.put64(S2_READ_ONLY, 0x7777);
    let switch = |m: &mut Machine<WithEl2>, to_guest: bool, round: u64| {
        m.at(El::El2);
        m.cpu.x[0] = if to_guest { guest_vttbr } else { 0 };
        assert_eq!(m.run(MSR_VTTBR_EL2), None);
        // Another guest's T0SZ on the hypervisor's side, each round.
        m.cpu.x[0] = if to_guest {
            guest_vtcr
        } else {
            (guest_vtcr & !0x3F) | (26 + round)
        };
        assert_eq!(m.run(MSR_VTCR_EL2), None);
        m.cpu.x[0] = if to_guest {
            guest_hcr
        } else {
            guest_hcr & !HCR_VM
        };
        assert_eq!(m.run(MSR_HCR_EL2), None);
        m.at(El::El1);
        m.cpu.x[1] = SHARED;
        assert_eq!(m.run(LDR), None);
        m.cpu.x[2]
    };
    assert_eq!(switch(&mut m, true, 0), 0x3333, "the guest");
    assert_eq!(switch(&mut m, false, 0), 0x1111, "the hypervisor");
    // `SHARED` now maps `S2_READ_ONLY`'s address on both sides, which a walk
    // of either would read.
    m.ram.put64(
        EL1_L2 + 8 * (SHARED >> 21),
        S2_READ_ONLY | AF | INNER_SHAREABLE | BLOCK,
    );
    for round in 1..4 {
        assert_eq!(
            switch(&mut m, true, round),
            0x3333,
            "the guest, round {round}"
        );
        assert_eq!(
            switch(&mut m, false, round),
            0x1111,
            "the hypervisor, round {round}"
        );
    }
    assert_eq!(
        switch(&mut m, true, 4),
        0x3333,
        "the guest, before the TLBI"
    );
    m.at(El::El2);
    assert_eq!(m.run(TLBI_VMALLS12E1), None);
    m.at(El::El1);
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x7777, "the guest, after it");
}

/// The EL1&0 regime's translations carry the VMID even with stage 2
/// disabled (Arm `AArch64_TLBContextEL10`: `tlbcontext.vmid = VMID()`
/// whenever EL2 is enabled): with `SHARED`'s stage 1 descriptor rewritten
/// and no TLB maintenance, another VMID in `VTTBR_EL2` walks the tables
/// afresh and sees the change, and the first VMID, back, still finds the
/// translation it had.
#[test]
fn el1_translations_carry_the_vmid_without_stage_2() {
    let mut m = translating();
    let base = m.cpu.el2().vttbr;
    let in_vm = |m: &mut Machine<WithEl2>, vmid: u64| {
        m.at(El::El2);
        m.cpu.x[0] = base | (vmid << 48);
        assert_eq!(m.run(MSR_VTTBR_EL2), None);
        m.at(El::El1);
        m.cpu.x[1] = SHARED;
        assert_eq!(m.run(LDR), None, "VMID {vmid}");
        m.cpu.x[2]
    };
    m.ram.put64(READ_ONLY, 0x7777);
    assert_eq!(in_vm(&mut m, 0), 0x1111, "SHARED's EL1 data");
    m.ram.put64(
        EL1_L2 + 8 * (SHARED >> 21),
        READ_ONLY | AF | INNER_SHAREABLE | BLOCK,
    );
    assert_eq!(in_vm(&mut m, 1), 0x7777, "another VMID walks again");
    assert_eq!(
        in_vm(&mut m, 0),
        0x1111,
        "the first VMID's translation, kept"
    );
}

/// An entry stage 2 takes no part in holds none of stage 2's leaf, so a
/// snapshot giving one stage 2's permissions, Device memory or a level is
/// refused: `SHARED`'s EL1 entry, made with stage 2 disabled, restores as
/// saved and with any of those bytes changed is no entry a walk makes.
#[test]
fn a_snapshot_entry_without_stage_2_holds_no_stage_2_leaf() {
    let mut m = translating();
    m.at(El::El1);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None);
    let saved = m.saved_tlb();
    let image = saved.0.clone();
    let context = m.tlb_context();
    assert_eq!(restored::<WithEl2>(&context, &image), Ok(image.clone()));
    let data = saved.entry_at(SHARED);
    for (at, value) in [
        (ENTRY_S2_PERMS, 0),
        (ENTRY_S2_DEVICE, 1),
        (ENTRY_S2_LEVEL, 3),
    ] {
        let mut odd = image.clone();
        odd[data + at] = value;
        assert_eq!(
            restored::<WithEl2>(&context, &odd),
            Err(NOT_WALKED),
            "byte {at} of the entry"
        );
    }
}

/// A guest's switch between its processes — `TTBR0_EL1` written with
/// another ASID, its tables the same — keeps stage 2's translations, which
/// depend on `VTTBR_EL2` and `VTCR_EL2` alone (Arm tags them with the VMID,
/// not the ASID): with stage 2's descriptor of `EL1_DATA`'s IPA rewritten
/// and no TLB maintenance, the new process's walk of `SHARED` still reaches
/// `GUEST_DATA`, until `TLBI VMALLS12E1` makes the change seen.
#[test]
fn a_guest_process_switch_keeps_stage_2s_translations() {
    const MSR_TTBR0_EL1: u32 = 0xD518_2000; // msr ttbr0_el1, x0
    let mut m = stage2();
    m.ram.put64(S2_READ_ONLY, 0x7777);
    m.at(El::El1);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x3333, "EL1_DATA's IPA is GUEST_DATA");
    m.ram.put64(
        S2_L2 + 8 * (EL1_DATA >> 21),
        S2_READ_ONLY | AF | INNER_SHAREABLE | S2AP_R | S2AP_W | S2_NORMAL | BLOCK,
    );
    for asid in 1..4u64 {
        m.at(El::El1);
        m.cpu.x[0] = (m.cpu.sys.ttbr0_el1 & !(0xFFFF << 48)) | (asid << 48);
        assert_eq!(m.run(MSR_TTBR0_EL1), None);
        m.at(El::El1);
        assert_eq!(m.run(LDR), None);
        assert_eq!(
            m.cpu.x[2], 0x3333,
            "ASID {asid}: the kept stage 2 translation"
        );
    }
    m.at(El::El2);
    assert_eq!(m.run(TLBI_VMALLS12E1), None);
    m.at(El::El1);
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x7777, "EL1_DATA's IPA is now S2_READ_ONLY");
}

/// Translation tables in the first pages of 2 MiB blocks, as a guest's
/// tables often lie, each keep their stage 2 translation: four of the
/// guest's level 2 tables at the starts of four IPA blocks, read once, are
/// read again after a process switch through what stage 2 gave the first
/// time, with stage 2's descriptors of all four blocks made invalid and no
/// TLB maintenance, until `TLBI VMALLS12E1` makes the change seen.
#[test]
fn tables_at_the_starts_of_blocks_each_keep_their_stage_2_translation() {
    const MSR_TTBR0_EL1: u32 = 0xD518_2000; // msr ttbr0_el1, x0
    const TABLES: [u64; 4] = [SHARED, S2_UNMAPPED, S2_READ_ONLY, S2_NO_EXECUTE];
    let mut m = stage2();
    m.ram.put64(
        S2_L2 + 8 * (S2_UNMAPPED >> 21),
        S2_UNMAPPED | AF | INNER_SHAREABLE | S2AP_R | S2AP_W | S2_NORMAL | BLOCK,
    );
    // The `k`th table maps the sixth 2 MiB block of the `k`th GiB.
    let va = |k: u64| (k << 30) | (5 << 21);
    for (k, table) in (1..).zip(TABLES) {
        m.ram.put64(EL1_L1 + 8 * k, table | TABLE);
        m.ram
            .put64(table + 8 * 5, EL1_DATA | AF | INNER_SHAREABLE | BLOCK);
    }
    let load_all = |m: &mut Machine<WithEl2>, when: &str| {
        for k in 1..=4 {
            m.at(El::El1);
            m.cpu.x[1] = va(k);
            assert_eq!(m.run(LDR), None, "table {k}, {when}");
            assert_eq!(m.cpu.x[2], 0x3333, "table {k}, {when}");
        }
    };
    load_all(&mut m, "first read");
    for table in TABLES {
        m.ram.put64(S2_L2 + 8 * (table >> 21), 0);
    }
    m.at(El::El1);
    m.cpu.x[0] = (m.cpu.sys.ttbr0_el1 & !(0xFFFF << 48)) | (1 << 48);
    assert_eq!(m.run(MSR_TTBR0_EL1), None);
    load_all(&mut m, "another process, the kept stage 2 translations");
    m.at(El::El2);
    assert_eq!(m.run(TLBI_VMALLS12E1), None);
    m.at(El::El1);
    m.cpu.x[1] = va(1);
    assert_eq!(
        m.run(LDR),
        stage2_abort(S1PTW | TRANSLATION_L2),
        "after the TLBI, stage 2 maps the table's block no more"
    );
}

/// The TLB keeps the entries of at most fifteen contexts: a sixteenth takes
/// the place of the one found least recently, whose entries are emptied,
/// while one found again since it was made keeps its place; translations
/// in every context, the one replaced included, stay right.
#[test]
fn a_sixteenth_context_takes_the_least_recently_found_ones_place() {
    let mut m = stage2();
    let base = m.cpu.el2().vttbr;
    let in_vm = |m: &mut Machine<WithEl2>, vmid: u64, va: u64| {
        m.at(El::El2);
        m.cpu.x[0] = base | (vmid << 48);
        assert_eq!(m.run(MSR_VTTBR_EL2), None);
        m.at(El::El1);
        m.cpu.x[1] = va;
        assert_eq!(m.run(LDR), None, "VMID {vmid}");
    };
    // A page each of the first two VMIDs alone reads, whose page numbers'
    // low bits no other entry's share, so that no other context's entry
    // takes its slot: the code and SHARED pages are at multiples of 64.
    let (vmid0_page, vmid1_page) = (S2_READ_ONLY + 0x5000, S2_NO_EXECUTE + 0x7000);
    in_vm(&mut m, 0, vmid0_page);
    in_vm(&mut m, 1, vmid1_page);
    for vmid in 2..15 {
        in_vm(&mut m, vmid, SHARED);
        assert_eq!(m.cpu.x[2], 0x3333, "VMID {vmid}");
    }
    let saved = m.saved_tlb();
    assert!(saved.has_entry(vmid0_page) && saved.has_entry(vmid1_page));
    in_vm(&mut m, 0, SHARED);
    assert_eq!(
        m.saved_tlb().current(),
        1,
        "VMID 0 found again, in its place"
    );
    in_vm(&mut m, 15, SHARED);
    let saved = m.saved_tlb();
    assert_eq!(saved.contexts().len(), 15);
    assert_eq!(saved.current(), 2, "the sixteenth, in VMID 1's place");
    assert_eq!(
        (saved.generation(0), saved.generation(1)),
        (0, 1),
        "VMID 1's place in its next generation"
    );
    assert!(saved.has_entry(vmid0_page), "VMID 0's entries stay");
    assert!(
        !saved.has_entry(vmid1_page),
        "VMID 1's, of the place's last generation, match no more"
    );
    assert_eq!(
        restored::<WithEl2>(&m.tlb_context(), &saved.0),
        Ok(saved.0.clone())
    );
    in_vm(&mut m, 1, SHARED);
    assert_eq!(m.cpu.x[2], 0x3333, "VMID 1 again, in a place of its own");
    assert_eq!(m.saved_tlb().current(), 3, "VMID 2's, the least recent now");
}

/// A place's generation counts to 127 and then starts again from 0, when
/// the entries made in the place, of every generation, are emptied: none
/// made 128 generations before matches the context that then has it — of
/// the table of EL1&0 contexts and, each VMID a stage 2 context too, of
/// the table of stage 2 contexts.
#[test]
fn a_places_generation_starts_again_with_its_entries_emptied() {
    let mut m = stage2();
    let base = m.cpu.el2().vttbr;
    let in_vm = |m: &mut Machine<WithEl2>, vmid: u64, va: u64| {
        m.at(El::El2);
        m.cpu.x[0] = base | (vmid << 48);
        assert_eq!(m.run(MSR_VTTBR_EL2), None);
        m.at(El::El1);
        m.cpu.x[1] = va;
        assert_eq!(m.run(LDR), None, "VMID {vmid}");
    };
    // A page no other context reads, whose slot no other entry takes, nor
    // its kept stage 2 translation's slot another kept translation: every
    // other context's are those of the code, `SHARED`'s IPA and the two
    // tables' pages, each turned by its place.
    let first_page = S2_READ_ONLY + 0x25000;
    in_vm(&mut m, 0, first_page);
    let saved = m.saved_tlb();
    assert_eq!(saved.generation(0), 0);
    assert!(
        saved.kept_pages().contains(&(first_page >> 12)),
        "its IPA's stage 2 translation, kept"
    );
    let mut seen_last = false;
    for new in 1u64.. {
        // Every VMID new to the table, each taking the least recent place.
        in_vm(&mut m, new % 255 + 1, SHARED);
        let saved = m.saved_tlb();
        match saved.generation(0) {
            0 if seen_last => {
                assert!(
                    !saved.has_entry(first_page),
                    "the first context's entry, of generation 0 too, is gone"
                );
                assert!(
                    !saved.kept_pages().contains(&(first_page >> 12)),
                    "and so is its stage 2 translation"
                );
                return;
            }
            127 => seen_last = true,
            _ => {}
        }
        assert!(new < 15 * 129, "place 0 is taken every 15 new contexts");
    }
}

/// FEAT_E0PD is stage 1's walk's: with stage 1 disabled under stage 2, an
/// EL0 access in a half `TCR_EL1.E0PD0` refuses EL0 meets none of it. An
/// aligned load is permitted and cached; a misaligned one to the
/// Device-nGnRnE memory stage 1 then leaves takes stage 1's alignment
/// fault, from the TLB as from a walk, not E0PD's level 0 Translation
/// fault.
#[test]
fn e0pd_does_not_reach_an_access_stage_1_leaves_alone() {
    const TCR_E0PD0: u64 = 1 << 55;
    let mut m = stage2_on(Implementation::EVERY_FEATURE);
    m.cpu.sys.sctlr_el1 &= !1;
    m.cpu.sys.tcr_el1 |= TCR_E0PD0;
    m.cpu.flush_tlb();
    m.at(El::El0);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None, "an aligned load, permitted");
    assert!(m.saved_tlb().has_entry(SHARED), "and cached");
    let misaligned = sync(El::El1, LOWER_A64, EC_DATA_ABORT_LOWER_EL, ALIGNMENT_FAULT);
    for from in ["the TLB", "a walk"] {
        m.at(El::El0);
        m.cpu.x[1] = SHARED + 1;
        assert_eq!(m.run(LDR), misaligned, "from {from}");
        m.cpu.flush_tlb();
    }
}

/// Only the access of a load or store of one general-purpose register
/// leaves its stage 2 abort room for an instruction syndrome: offered one
/// later, the abort of any other access reports none (`ISV` 0).
#[test]
fn only_a_single_register_access_takes_an_instruction_syndrome() {
    let mut m = stage2();
    m.at(El::El1);
    let syndrome = AccessSyndrome::load(3, Extend::Zero, 2);
    for (acc, reported) in [
        (DataAccess::NORMAL, false),
        (DataAccess::NORMAL.of_one_register(), true),
    ] {
        let Err(fault) = m.cpu.read_mem(&mut m.ram, S2_UNMAPPED, 3, acc) else {
            panic!("stage 2 maps nothing at S2_UNMAPPED");
        };
        let Fault::Exception(e) = fault.reporting(Some(syndrome)) else {
            panic!("a stage 2 abort, not a stop");
        };
        assert_eq!(u64::from(e.iss) & ISV != 0, reported, "{acc:?}");
    }
}

/// A host's write of a translation register empties the TLB whatever the
/// value, as `VirtMachine::sysreg_write` promises: rewriting `TTBR0_EL1`
/// with its own value after changing a table makes the change seen, where
/// a guest's `MSR` of the same value would leave the TLB's entries in use.
#[test]
fn a_host_translation_register_write_empties_the_tlb() {
    let mut m = stage2();
    m.at(El::El1);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x3333);
    m.ram
        .put64(EL1_L2 + 8, S2_READ_ONLY | AF | INNER_SHAREABLE | BLOCK);
    m.ram.put64(S2_READ_ONLY, 0x7777);
    let ttbr0 = m.cpu.sys.ttbr0_el1;
    assert!(m.cpu.host_sysreg_write(SysReg::TTBR0_EL1, ttbr0).is_ok());
    m.at(El::El1);
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x7777, "SHARED by the changed table");
}

/// A register value no translation reads makes no context of its own: with
/// `SHARED`'s stage 1 descriptor rewritten and no TLB maintenance, an
/// `SCTLR_EL1` bit the access reads, not the walk (`UCI`), leaves the
/// regime in the context it was in, its translation still in use; a
/// `MAIR_EL1` attribute no descriptor names, which a walk reads, makes a
/// context whose walk sees the change.
#[test]
fn a_value_no_translation_reads_makes_no_new_context() {
    const MSR_SCTLR_EL1: u32 = 0xD518_1000; // msr sctlr_el1, x0
    const MSR_MAIR_EL1: u32 = 0xD518_A200; // msr mair_el1, x0
    let mut m = stage2();
    m.ram.put64(S2_READ_ONLY, 0x7777);
    m.at(El::El1);
    m.cpu.x[1] = SHARED;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x3333);
    m.ram.put64(
        EL1_L2 + 8 * (SHARED >> 21),
        S2_READ_ONLY | AF | INNER_SHAREABLE | BLOCK,
    );
    m.at(El::El1);
    m.cpu.x[0] = m.cpu.sys.sctlr_el1 ^ (1 << 26);
    assert_eq!(m.run(MSR_SCTLR_EL1), None);
    m.at(El::El1);
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x3333, "UCI flipped: the translation stays");
    m.at(El::El1);
    m.cpu.x[0] = m.cpu.sys.mair_el1 | (0x44 << 56);
    assert_eq!(m.run(MSR_MAIR_EL1), None);
    m.at(El::El1);
    assert_eq!(m.run(LDR), None);
    assert_eq!(
        m.cpu.x[2], 0x7777,
        "MAIR_EL1 changed: the walk is made again"
    );
}

// ----- FEAT_ECV_POFF: the physical count's offset -----
//
// The words are GNU as 2.42's with `.arch armv8.6-a`, `CNTPOFF_EL2` written
// as `s3_4_c14_c0_6`, which its objdump names `cntpoff_el2`.

const MRS_CNTPCT: u32 = 0xD53B_E020; // mrs x0, cntpct_el0
const MRS_CNTPCTSS: u32 = 0xD53B_E0A0; // mrs x0, cntpctss_el0
const MRS_CNTPOFF: u32 = 0xD53C_E0C0; // mrs x0, cntpoff_el2
const MSR_CNTPOFF: u32 = 0xD51C_E0C0; // msr cntpoff_el2, x0
const MRS_CNTHCTL: u32 = 0xD53C_E100; // mrs x0, cnthctl_el2
const MSR_CNTHCTL: u32 = 0xD51C_E100; // msr cnthctl_el2, x0
const MRS_CNTP_CTL: u32 = 0xD53B_E220; // mrs x0, cntp_ctl_el0
const MSR_CNTP_CTL: u32 = 0xD51B_E220; // msr cntp_ctl_el0, x0
const MRS_CNTP_CVAL: u32 = 0xD53B_E240; // mrs x0, cntp_cval_el0
const MRS_CNTP_TVAL: u32 = 0xD53B_E200; // mrs x0, cntp_tval_el0
const MSR_CNTP_TVAL: u32 = 0xD51B_E200; // msr cntp_tval_el0, x0
const MSR_CNTV_CTL: u32 = 0xD51B_E320; // msr cntv_ctl_el0, x0
const MSR_CNTVOFF: u32 = 0xD51C_E060; // msr cntvoff_el2, x0
const MSR_CNTHP_CTL: u32 = 0xD51C_E220; // msr cnthp_ctl_el2, x0
const MSR_CNTHP_CVAL: u32 = 0xD51C_E240; // msr cnthp_cval_el2, x0
/// `CNTKCTL_EL1.EL0PCTEN`: EL0's reads of the physical count enabled.
const EL0PCTEN: u64 = 1 << 0;
/// `CNTKCTL_EL1.EL0PTEN`: EL0's accesses to the EL1 physical timer enabled.
const EL0PTEN: u64 = 1 << 9;
/// `CNTx_CTL.ENABLE` and `ISTATUS`.
const TIMER_ENABLE: u64 = 1 << 0;
const TIMER_ISTATUS: u64 = 1 << 2;

/// Every feature but FEAT_ECV_POFF: FEAT_ECV alone.
fn ecv_without_poff() -> Implementation {
    Implementation::every_feature_but(&[Feature::EcvPoff])
}

/// A processor with FEAT_ECV_POFF at the tick 12,345 (its counter runs at
/// one count a tick), `CNTPOFF_EL2` holding `offset` and `CNTHCTL_EL2.ECV`
/// as `ecv` says, each written by an `MSR` at EL2, and EL0's reads of the
/// physical count enabled.
fn with_physical_offset(offset: u64, ecv: bool) -> Machine<WithEl2> {
    let mut m = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    m.cpu.wait_until(VmInstant::from_ticks(12_345));
    m.cpu.sys.cntkctl_el1 |= EL0PCTEN;
    m.at(El::El2);
    m.cpu.x[0] = offset;
    assert_eq!(m.run(MSR_CNTPOFF), None);
    m.at(El::El2);
    m.cpu.x[0] = m.cpu.el2().cnthctl | if ecv { CNTHCTL_ECV_POFF } else { 0 };
    assert_eq!(m.run(MSR_CNTHCTL), None);
    m
}

/// Run `insn` at `el`: what it reads into X0, and the physical count it ran
/// at.
fn read_at(m: &mut Machine<WithEl2>, el: El, insn: u32) -> (u64, u64) {
    m.at(el);
    let count = m.cpu.counter().get();
    assert_eq!(m.run(insn), None, "{insn:#010x} at {el:?}");
    (m.cpu.x[0], count)
}

/// The timers as a machine holds them between its samples of them
/// (`VirtMachine::sync_events`): the levels it drives the timer PPIs of
/// the GIC with, and the tick it next samples them at. It samples them
/// afresh where the processor says a register write moved them
/// ([`Cpu::take_timer_dirty`]).
#[derive(Debug, PartialEq, Eq)]
struct HeldTimers {
    lines: TimerLines,
    deadline: Option<VmInstant>,
}

impl HeldTimers {
    /// The timers as they are.
    fn of(cpu: &Cpu<WithEl2>) -> Self {
        Self {
            lines: cpu.timer_lines(),
            deadline: cpu.next_timer_deadline(),
        }
    }

    /// Sample the timers afresh if the processor says a write since the
    /// last sample moved them: whether it did.
    fn sync(&mut self, cpu: &mut Cpu<WithEl2>) -> bool {
        let moved = cpu.take_timer_dirty();
        if moved {
            *self = Self::of(cpu);
        }
        moved
    }
}

/// FEAT_ECV_POFF: with `CNTHCTL_EL2.ECV` set, EL0 and EL1 read the physical
/// count less `CNTPOFF_EL2`, modulo 2^64, through `CNTPCT_EL0` and
/// `CNTPCTSS_EL0`, and EL2 the count itself (Arm's accessors of both); with
/// `ECV` clear the offset takes no part. An offset above the count wraps
/// the count EL1 reads. `CNTPOFF_EL2` holds all 64 bits, read back at EL2.
#[test]
fn cntpoff_offsets_the_physical_count_below_el2() {
    for (ecv, offset) in [(false, 1_000), (true, 1_000), (true, 1 << 40)] {
        let mut m = with_physical_offset(offset, ecv);
        for insn in [MRS_CNTPCT, MRS_CNTPCTSS] {
            for el in [El::El0, El::El1] {
                let (read, count) = read_at(&mut m, el, insn);
                let want = if ecv {
                    count.wrapping_sub(offset)
                } else {
                    count
                };
                assert_eq!(
                    read, want,
                    "{insn:#010x} at {el:?}, ECV {ecv}, offset {offset:#x}"
                );
            }
            let (read, count) = read_at(&mut m, El::El2, insn);
            assert_eq!(read, count, "{insn:#010x} at EL2, ECV {ecv}");
        }
    }
    let mut m = with_physical_offset(u64::MAX - 7, false);
    assert_eq!(read_at(&mut m, El::El2, MRS_CNTPOFF).0, u64::MAX - 7);
}

/// The EL1 physical timer compares the count EL1 reads, the physical count
/// less `CNTPOFF_EL2` under `CNTHCTL_EL2.ECV`, whatever the level
/// (`AArch64_CheckTimerConditions`): a `TVAL` of 100 written at EL1 sets a
/// `CVAL` 900 below the physical count and fires 100 ticks later. EL1's
/// `TVAL` reads against the offset count, EL2's against the physical count
/// itself (the accessor offsets EL0's and EL1's alone). Clearing `ECV`
/// raises the line at once, the physical count being past `CVAL`, and has
/// the timer lines sampled again.
#[test]
fn the_physical_timer_compares_the_offset_count() {
    const OFFSET: u64 = 1_000;
    let mut m = with_physical_offset(OFFSET, true);
    m.at(El::El1);
    let written_at = m.cpu.counter().get();
    m.cpu.x[0] = 100;
    assert_eq!(m.run(MSR_CNTP_TVAL), None);
    m.at(El::El1);
    m.cpu.x[0] = TIMER_ENABLE;
    assert_eq!(m.run(MSR_CNTP_CTL), None);
    let cval = written_at - OFFSET + 100;
    assert_eq!(read_at(&mut m, El::El1, MRS_CNTP_CVAL).0, cval);
    let (tval, count) = read_at(&mut m, El::El1, MRS_CNTP_TVAL);
    assert_eq!(
        tval,
        cval - (count - OFFSET),
        "EL1: against the offset count"
    );
    m.cpu.sys.cntkctl_el1 |= EL0PTEN;
    let (tval, count) = read_at(&mut m, El::El0, MRS_CNTP_TVAL);
    assert_eq!(
        tval,
        cval - (count - OFFSET),
        "EL0: against the offset count"
    );
    let (tval, count) = read_at(&mut m, El::El2, MRS_CNTP_TVAL);
    assert_eq!(
        tval,
        cval.wrapping_sub(count) & 0xFFFF_FFFF,
        "EL2: against the physical count"
    );
    assert!(!m.cpu.timer_lines().physical);
    assert_eq!(
        m.cpu.next_timer_deadline().map(VmInstant::ticks),
        Some(written_at + 100)
    );
    m.cpu.wait_until(VmInstant::from_ticks(written_at + 99));
    assert!(!m.cpu.timer_lines().physical, "one tick short");
    // `ISTATUS` reports the offset count's condition at EL2 too, though
    // the physical count is long past `CVAL`.
    assert_eq!(
        read_at(&mut m, El::El2, MRS_CNTP_CTL).0,
        TIMER_ENABLE,
        "EL2, one tick short: ENABLE, no ISTATUS"
    );
    assert!(
        m.cpu.timer_lines().physical,
        "the offset count reaches CVAL"
    );
    assert_eq!(
        read_at(&mut m, El::El2, MRS_CNTP_CTL).0,
        TIMER_ENABLE | TIMER_ISTATUS,
        "EL2: ENABLE and ISTATUS"
    );

    let mut m = with_physical_offset(OFFSET, true);
    m.cpu.sys.cntp_cval_el0 = m.cpu.counter().get() - OFFSET + 100;
    m.cpu.sys.cntp_ctl_el0 = TIMER_ENABLE;
    let mut held = HeldTimers::of(&m.cpu);
    assert!(held.sync(&mut m.cpu), "written since reset");
    assert!(!held.lines.physical);
    m.at(El::El2);
    m.cpu.x[0] = m.cpu.el2().cnthctl & !CNTHCTL_ECV_POFF;
    assert_eq!(m.run(MSR_CNTHCTL), None);
    assert!(held.sync(&mut m.cpu), "CNTHCTL_EL2 written");
    assert!(
        held.lines.physical,
        "ECV clear: the physical count is past CVAL, and the PPI high"
    );
}

/// A write of `CNTP_TVAL_EL0` sets `CVAL` to the count the writing level
/// reads plus the value: under `CNTHCTL_EL2.ECV`, the physical count less
/// `CNTPOFF_EL2` at EL0 and EL1, and the physical count itself at EL2
/// (Arm's `CNTP_TVAL_EL0` accessors).
#[test]
fn cntp_tval_is_written_against_the_count_its_level_reads() {
    const OFFSET: u64 = 1_000;
    let mut m = with_physical_offset(OFFSET, true);
    m.cpu.sys.cntkctl_el1 |= EL0PTEN;
    for (el, offset) in [(El::El0, OFFSET), (El::El1, OFFSET), (El::El2, 0)] {
        m.at(el);
        let count = m.cpu.counter().get();
        m.cpu.x[0] = 100;
        assert_eq!(m.run(MSR_CNTP_TVAL), None, "{el:?}");
        assert_eq!(
            read_at(&mut m, El::El1, MRS_CNTP_CVAL).0,
            count - offset + 100,
            "written at {el:?}"
        );
    }
}

/// `HCR_EL2.TGE` leaves EL0 outside a host while `HCR_EL2.E2H` is 0, as it
/// is without FEAT_VHE (Arm `EffectivelyAtEL0NotInHost`), so EL0's reads of
/// `CNTPCT_EL0` and `CNTP_TVAL_EL0` under `CNTHCTL_EL2.ECV` are of the
/// physical count less `CNTPOFF_EL2` with TGE set too.
#[test]
fn el0_reads_the_offset_count_under_tge() {
    const OFFSET: u64 = 1_000;
    let mut m = with_physical_offset(OFFSET, true);
    m.hcr(HCR_TGE);
    m.cpu.sys.cntkctl_el1 |= EL0PTEN;
    let cval = m.cpu.counter().get() - OFFSET + 100;
    m.cpu.sys.cntp_cval_el0 = cval;
    m.cpu.sys.cntp_ctl_el0 = TIMER_ENABLE;
    let (read, count) = read_at(&mut m, El::El0, MRS_CNTPCT);
    assert_eq!(read, count - OFFSET, "CNTPCT_EL0");
    let (tval, count) = read_at(&mut m, El::El0, MRS_CNTP_TVAL);
    assert_eq!(tval, cval - (count - OFFSET), "CNTP_TVAL_EL0");
}

/// A write of `CNTHCTL_EL2` or `CNTPOFF_EL2` that changes the physical
/// offset moves the EL1 physical timer's count, and one of `CNTVOFF_EL2`
/// that changes it the virtual timer's, and with it the tick the timer's
/// line next rises at: the machine holds that tick after the write, so the
/// timer's PPI rises when its count reaches `CVAL`; so too a write of
/// `CNTHP_CVAL_EL2` that moves the EL2 timer's. One that leaves them as
/// they were moves nothing, and the machine samples nothing afresh, so the
/// writes of these registers a hypervisor commonly makes at each world
/// switch cost no sample.
#[test]
fn the_machine_holds_the_timers_where_an_el2_write_moves_them() {
    /// Make each write at EL2, the timers held as the machine holds them:
    /// whether it moves them, and with that whether it has them sampled
    /// afresh.
    fn writes(m: &mut Machine<WithEl2>, held: &mut HeldTimers, rows: &[(&str, u32, u64, bool)]) {
        for &(what, insn, value, moves) in rows {
            let before = HeldTimers::of(&m.cpu);
            m.at(El::El2);
            m.cpu.x[0] = value;
            assert_eq!(m.run(insn), None, "{what}");
            let after = HeldTimers::of(&m.cpu);
            assert_eq!(after != before, moves, "{what}: the timer's deadline");
            assert_eq!(held.sync(&mut m.cpu), moves, "{what}: sampled afresh");
            assert_eq!(*held, after, "{what}: as the machine holds it");
        }
    }
    let mut m = with_physical_offset(1_000, false);
    // The physical timer alone, enabled 500 counts ahead, which every
    // offset here leaves ahead.
    m.cpu.sys.cntp_cval_el0 = m.cpu.counter().get() + 500;
    m.cpu.sys.cntp_ctl_el0 = TIMER_ENABLE;
    let mut held = HeldTimers::of(&m.cpu);
    assert!(held.sync(&mut m.cpu), "written since reset");
    let cnthctl = m.cpu.el2().cnthctl;
    writes(
        &mut m,
        &mut held,
        &[
            ("CNTPOFF_EL2 under ECV clear", MSR_CNTPOFF, 2_000, false),
            ("CNTHCTL_EL2 as it was", MSR_CNTHCTL, cnthctl, false),
            ("ECV set", MSR_CNTHCTL, cnthctl | CNTHCTL_ECV_POFF, true),
            ("CNTPOFF_EL2 under ECV", MSR_CNTPOFF, 3_000, true),
            ("CNTPOFF_EL2 as it was", MSR_CNTPOFF, 3_000, false),
            ("ECV clear", MSR_CNTHCTL, cnthctl, true),
        ],
    );
    // The virtual timer alone, likewise.
    m.at(El::El1);
    m.cpu.x[0] = 0;
    assert_eq!(m.run(MSR_CNTP_CTL), None);
    m.cpu.sys.cntv_cval_el0 = m.cpu.virtual_count().get() + 500;
    m.at(El::El1);
    m.cpu.x[0] = TIMER_ENABLE;
    assert_eq!(m.run(MSR_CNTV_CTL), None);
    assert!(held.sync(&mut m.cpu), "the timers' controls written");
    writes(
        &mut m,
        &mut held,
        &[
            ("CNTVOFF_EL2", MSR_CNTVOFF, 100, true),
            ("CNTVOFF_EL2 as it was", MSR_CNTVOFF, 100, false),
        ],
    );
    // The EL2 timer alone, its compare value written the same way.
    m.at(El::El1);
    m.cpu.x[0] = 0;
    assert_eq!(m.run(MSR_CNTV_CTL), None);
    m.at(El::El2);
    m.cpu.x[0] = TIMER_ENABLE;
    assert_eq!(m.run(MSR_CNTHP_CTL), None);
    assert!(held.sync(&mut m.cpu), "the timers' controls written");
    let ahead = m.cpu.counter().get() + 500;
    writes(
        &mut m,
        &mut held,
        &[
            ("CNTHP_CVAL_EL2", MSR_CNTHP_CVAL, ahead, true),
            ("CNTHP_CVAL_EL2 as it was", MSR_CNTHP_CVAL, ahead, false),
        ],
    );
}

/// Only a register the timers read ([`El2Reg::feeds_timers`]) changes what
/// they read of the EL2 registers: a write of any other, whatever its
/// value, leaves every timer where it stood, so its `MSR` has nothing
/// sampled again.
#[test]
fn only_the_registers_the_timers_read_move_them() {
    let mut m = with_physical_offset(1_000, true);
    let features = m.cpu.imp.features;
    for reg in El2Reg::ALL.into_iter().filter(|reg| !reg.feeds_timers()) {
        for value in [u64::MAX, 0, 0x5A5A_5A5A_5A5A_5A5A] {
            let before = m.cpu.el2_timer_inputs();
            m.cpu.el2_state.regs.write(reg, features, u64::MAX, value);
            assert_eq!(
                m.cpu.el2_timer_inputs(),
                before,
                "{reg:?} written with {value:#x}"
            );
        }
    }
}

/// Where `CNTPOFF_EL2` lies above the physical count, the offset count has
/// wrapped below zero, so it stands above a small `CVAL` and the timer's
/// line is high; the line falls where the physical count reaches the offset
/// and the offset count wraps to zero (`IsTimerConditionMet` compares
/// unsigned), and rises again `CVAL` ticks later.
#[test]
fn the_physical_timer_line_falls_where_the_offset_count_wraps() {
    let mut m = with_physical_offset(0, true);
    let offset = m.cpu.counter().get() + 50;
    m.at(El::El2);
    m.cpu.x[0] = offset;
    assert_eq!(m.run(MSR_CNTPOFF), None);
    m.cpu.sys.cntp_cval_el0 = 10;
    m.cpu.sys.cntp_ctl_el0 = TIMER_ENABLE;
    assert!(
        m.cpu.timer_lines().physical,
        "the wrapped count is past CVAL"
    );
    assert_eq!(
        m.cpu.next_timer_deadline().map(VmInstant::ticks),
        Some(offset)
    );
    m.cpu.wait_until(VmInstant::from_ticks(offset));
    assert!(!m.cpu.timer_lines().physical, "the offset count wraps to 0");
    assert_eq!(
        m.cpu.next_timer_deadline().map(VmInstant::ticks),
        Some(offset + 10)
    );
    m.cpu.wait_until(VmInstant::from_ticks(offset + 10));
    assert!(m.cpu.timer_lines().physical);
}

/// Only a processor with FEAT_ECV_POFF keeps `CNTHCTL_EL2.ECV`: one with
/// FEAT_ECV alone, or the Cortex-A57 with neither, reads it back clear, and
/// EL1 reads the physical count itself, even where the bit is held set.
#[test]
fn cnthctl_el2_ecv_is_kept_only_with_feat_ecv_poff() {
    for (name, imp, kept) in [
        (
            "every feature",
            Implementation::EVERY_FEATURE,
            CNTHCTL_ECV_POFF,
        ),
        ("FEAT_ECV alone", ecv_without_poff(), 0),
        ("Cortex-A57", Implementation::CORTEX_A57, 0),
    ] {
        let mut m = Machine::<WithEl2>::new(imp);
        m.at(El::El2);
        m.cpu.x[0] = m.cpu.el2().cnthctl | CNTHCTL_ECV_POFF;
        m.cpu.el2_state.regs.cntpoff = 1_000;
        assert_eq!(m.run(MSR_CNTHCTL), None, "{name}");
        assert_eq!(
            read_at(&mut m, El::El2, MRS_CNTHCTL).0 & CNTHCTL_ECV_POFF,
            kept,
            "{name}"
        );
        let (read, count) = read_at(&mut m, El::El1, MRS_CNTPCT);
        let want = if kept != 0 {
            count.wrapping_sub(1_000)
        } else {
            count
        };
        assert_eq!(read, want, "{name}");
        // Without the feature the offset is not valid, `ECV` or no `ECV`
        // (`PhysicalOffsetIsValid`), whatever state holds it.
        m.cpu.el2_state.regs.cnthctl |= CNTHCTL_ECV_POFF;
        let (read, count) = read_at(&mut m, El::El1, MRS_CNTPCT);
        let want = if kept != 0 {
            count.wrapping_sub(1_000)
        } else {
            count
        };
        assert_eq!(read, want, "{name}, ECV held as set");
    }
}

/// The test RAM with fetches that are stable, as the board's RAM is: the
/// trace cache, and with it the counter poll's fast-forward, runs over it.
/// Only the test writes it, between runs.
struct StableRam<'r>(&'r mut Ram);

impl PhysBus for StableRam<'_> {
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        clock: VmClock,
    ) -> Result<u64, BusFault> {
        self.0.read(addr, width, clock)
    }

    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        clock: VmClock,
    ) -> Result<(), BusFault> {
        self.0.write(addr, width, value, clock)
    }

    fn fetch_source(&self, _addr: PhysAddr) -> FetchSource {
        FetchSource::Stable(CodeEpoch::new(0))
    }

    fn devices_touched(&self) -> bool {
        false
    }
}

/// A poll of the physical count at EL1 under `CNTPOFF_EL2` ends at the
/// instruction running every instruction ends it at, whether it is
/// fast-forwarded or not: the fast-forward reads the count as EL1 does,
/// less the offset. An offset just below 2^64 puts EL1's count 1000 ahead
/// of the physical count, and a loop that waits while it is below a target
/// is fast-forwarded to the same end; an offset just above the physical
/// count has EL1's count wrap to 0, and a loop that waits while it is at or
/// above 0x1000 ends just after the wrap, no skip crossing it. The words
/// are GNU as 2.42's (`.arch armv8.6-a`).
#[test]
fn a_poll_of_the_offset_physical_count_ends_where_running_every_instruction_ends_it() {
    const WHILE_BELOW: [u32; 5] = [
        0xD53B_E020, // loop: mrs x0, cntpct_el0
        0xEB13_001F, // cmp x0, x19
        0x54FF_FFC3, // b.lo loop
        0xD280_0022, // mov x2, #1
        0x1400_0000, // b .
    ];
    const WHILE_AT_OR_ABOVE: [u32; 5] = [
        0xD53B_E020, // loop: mrs x0, cntpct_el0
        0xEB13_001F, // cmp x0, x19
        0x54FF_FFC2, // b.hs loop
        0xD280_0022, // mov x2, #1
        0x1400_0000, // b .
    ];
    const DONE: u64 = CODE + 12;
    // The tick the loop ends at, and how many runs it took: with the trace
    // cache, which fast-forwards a poll, or one instruction at a time.
    let ends_at = |program: [u32; 5], offset: u64, target: fn(u64) -> u64, skip: bool| {
        let mut m = with_physical_offset(offset, true);
        for (i, word) in program.into_iter().enumerate() {
            let at = (CODE + 4 * i as u64) as usize;
            m.ram.0[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        m.at(El::El1);
        m.cpu.x[19] = target(m.cpu.physical_count_read(El::El1).get());
        let start = m.cpu.icount;
        let mut runs = 0u64;
        while m.cpu.pc != DONE {
            assert!(m.cpu.icount - start < 1 << 22, "the poll ends");
            let mut bus = StableRam(&mut m.ram);
            if skip {
                m.cpu.bound_run(1 << 20);
                assert_eq!(
                    m.cpu
                        .run_trace(&mut bus, &mut NoInstrumentation, 1 << 20, &[DONE]),
                    Step::Retired
                );
            } else {
                assert_eq!(m.cpu.step(&mut bus, &mut NoInstrumentation), Step::Retired);
            }
            runs += 1;
        }
        (m.cpu.icount, runs)
    };
    let ahead = 0u64.wrapping_sub(1000);
    let below = |count: u64| count + 50_000;
    let (stepped, steps) = ends_at(WHILE_BELOW, ahead, below, false);
    let (skipped, runs) = ends_at(WHILE_BELOW, ahead, below, true);
    assert_eq!(skipped, stepped, "the same end, fast-forwarded");
    assert!(runs * 100 < steps, "{runs} runs for {steps} instructions");

    // An offset 0x8000 above the physical count the loop starts at, where
    // `with_physical_offset` leaves it. The loop reads the count once in
    // three instructions, so the first read past the wrap is 0 to 2 ticks
    // past it, and the loop ends 3 instructions after that read.
    let above = with_physical_offset(0, true).cpu.counter().get() + 0x8000;
    let wrapping = |_: u64| 0x1000;
    let (stepped, steps) = ends_at(WHILE_AT_OR_ABOVE, above, wrapping, false);
    let (skipped, runs) = ends_at(WHILE_AT_OR_ABOVE, above, wrapping, true);
    assert!(
        (above + 3..above + 6).contains(&stepped),
        "the first iteration to read past the wrap ends at {stepped}"
    );
    assert_eq!(skipped, stepped, "the same end, just past the wrap");
    // Nothing is skipped: until it wraps, the offset count stands at 2^62
    // and above, where `poll.rs` refuses a skip, so no run goes further than
    // once around the three-instruction loop.
    assert!(runs * 3 >= steps, "{runs} runs for {steps} instructions");
}

/// A snapshot keeps `CNTPOFF_EL2` and `CNTHCTL_EL2.ECV`: restored, EL1
/// reads the offset count, and the EL1 physical timer's line and deadline
/// are where they stood. An image whose `CNTPOFF_EL2` holds a value, or
/// whose `CNTHCTL_EL2.ECV` is set, is refused by a processor without
/// FEAT_ECV_POFF, which keeps neither.
#[test]
fn a_snapshot_keeps_the_physical_offset_where_the_processor_has_it() {
    const OFFSET: u64 = 0x1234;
    let mut saved = with_physical_offset(OFFSET, true);
    saved.cpu.sys.cntp_cval_el0 = saved.cpu.counter().get() - OFFSET + 100;
    saved.cpu.sys.cntp_ctl_el0 = TIMER_ENABLE;
    let mut image = Vec::new();
    saved.cpu.save(&mut image).expect("save");
    let mut restored = Machine::<WithEl2>::new(Implementation::EVERY_FEATURE);
    assert_eq!(restored.cpu.restore(&mut image.as_slice()), Ok(()));
    assert_eq!(
        HeldTimers::of(&restored.cpu),
        HeldTimers::of(&saved.cpu),
        "the timers where they stood"
    );
    assert_eq!(read_at(&mut restored, El::El2, MRS_CNTPOFF).0, OFFSET);
    let (read, count) = read_at(&mut restored, El::El1, MRS_CNTPCT);
    assert_eq!(read, count - OFFSET, "EL1 reads the offset count");

    /// Whether a processor without FEAT_ECV_POFF refuses the image of one
    /// with what `set` sets.
    fn refused_without_poff(set: fn(&mut Machine<WithEl2>)) -> bool {
        let mut saved = Machine::<WithEl2>::new(ecv_without_poff());
        set(&mut saved);
        let mut image = Vec::new();
        saved.cpu.save(&mut image).expect("save");
        Machine::<WithEl2>::new(ecv_without_poff())
            .cpu
            .restore(&mut image.as_slice())
            .is_err()
    }
    assert!(
        refused_without_poff(|m| m.cpu.el2_state.regs.cntpoff = 1),
        "CNTPOFF_EL2"
    );
    assert!(
        refused_without_poff(|m| m.cpu.el2_state.regs.cnthctl |= CNTHCTL_ECV_POFF),
        "CNTHCTL_EL2.ECV"
    );
}

/// A host reaches `CNTPOFF_EL2` by its name ([`SysReg::CNTPOFF_EL2`]) where
/// the processor has FEAT_ECV_POFF, as an `MSR` at EL2 does: what it writes
/// is what EL1's physical count is offset by under `CNTHCTL_EL2.ECV`, and a
/// write that moves the offset has the timers sampled afresh. A processor
/// without FEAT_ECV_POFF has no such register.
#[test]
fn a_host_reaches_cntpoff_el2_by_its_name() {
    let mut m = with_physical_offset(0, true);
    assert_eq!(
        m.cpu.host_sysreg_write(SysReg::CNTPOFF_EL2, 1_000),
        Ok(WriteEffect::Timer)
    );
    assert_eq!(m.cpu.sysreg(SysReg::CNTPOFF_EL2), Ok(1_000));
    let (read, count) = read_at(&mut m, El::El1, MRS_CNTPCT);
    assert_eq!(read, count - 1_000, "EL1 reads the offset count");
    assert_eq!(
        m.cpu.host_sysreg_write(SysReg::CNTPOFF_EL2, 1_000),
        Ok(WriteEffect::None),
        "the offset as it was"
    );
    for imp in [ecv_without_poff(), Implementation::CORTEX_A57] {
        let mut cpu = Machine::<WithEl2>::new(imp).cpu;
        assert_eq!(
            cpu.host_sysreg_write(SysReg::CNTPOFF_EL2, 1_000),
            Err(SysRegRefusal::Absent)
        );
        assert_eq!(cpu.sysreg(SysReg::CNTPOFF_EL2), Err(SysRegRefusal::Absent));
    }
}
