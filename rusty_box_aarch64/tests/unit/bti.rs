//! FEAT_BTI as a guest sees it, on a processor that implements it
//! ([`Implementation::EVERY_FEATURE`]) and on the Cortex-A57, which does
//! not (R9).
//!
//! The rules are the Arm ARM's pseudocode: `BranchTargetCheck`,
//! `BTypeCompatible_BTI`, `BTypeCompatible_PAC`, `EndOfDecode` (the check
//! before an UNDEFINED decode), `AArch64_S1CheckPermissions`
//! (`InGuardedPage`), `AArch64_S1DisabledOutput` (no page guarded with
//! stage 1 off), `AArch64_TakeException` (which exceptions keep BTYPE in
//! SPSR) and `SetPSTATEFromPSR`, and the pages of BR, BLR, RET, BTI and
//! their authenticating forms.
//!
//! The words are GNU as's (`-march=armv8.5-a+pauth`, objdump -d).

use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::features::Implementation;
use super::{id, Btype, Cpu, CpuConfig, El, Pstate, Reg, Step, Tlb};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

/// `br x1`, `br x16`, `br x17`, `blr x1`, `ret x1`.
const BR_X1: u32 = 0xD61F_0020;
const BR_X16: u32 = 0xD61F_0200;
const BR_X17: u32 = 0xD61F_0220;
const BLR_X1: u32 = 0xD63F_0020;
const RET_X1: u32 = 0xD65F_0020;
/// `braa x1, x2`, `blraaz x1`.
const BRAA_X1: u32 = 0xD71F_0822;
const BLRAAZ_X1: u32 = 0xD63F_083F;
/// `bti`, `bti c`, `bti j`, `bti jc`.
const BTI: u32 = 0xD503_241F;
const BTI_C: u32 = 0xD503_245F;
const BTI_J: u32 = 0xD503_249F;
const BTI_JC: u32 = 0xD503_24DF;
/// `nop`, `paciasp`, `pacibsp`, `brk #0x1234`, `svc #0`, `eret`, `udf #0`.
const NOP: u32 = 0xD503_201F;
const PACIASP: u32 = 0xD503_233F;
const PACIBSP: u32 = 0xD503_237F;
const BRK: u32 = 0xD422_4680;
const SVC: u32 = 0xD400_0001;
const ERET: u32 = 0xD69F_03E0;
const UDF: u32 = 0;
/// `msr spsr_el1, x0`, `mrs x0, spsr_el1`.
const MSR_SPSR: u32 = 0xD518_4000;
const MRS_SPSR: u32 = 0xD538_4000;

/// The translation tables: a level-1 table whose first entry is a level-2
/// table of three 2 MiB blocks, identity mapped, Normal memory: EL1-only and
/// not guarded, EL1-only and guarded, read-only to both ELs and guarded.
const L1: u64 = 0x1000;
const L2: u64 = 0x2000;
const VBAR: u64 = 0x1_0000;
const UNGUARDED: u64 = 0x4_0000;
const GUARDED: u64 = 0x20_0000;
const GUARDED_EL0: u64 = 0x40_0000;
/// Where a branch lands, in the guarded block, and outside RAM.
const TARGET: u64 = GUARDED + 0x1000;
const UNMAPPED: u64 = 0x7FFF_F000;
const RAM_BYTES: usize = 0x60_0000;

/// `SCTLR_EL1.BT0` and `BT1`.
const BT0: u64 = 1 << 35;
const BT1: u64 = 1 << 36;

/// The Branch Target exception's class.
const BRANCH_TARGET: u64 = 0x0D;

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

/// A taken exception: `ESR_EL1`'s class and ISS, and the `SPSR_EL1.BTYPE`
/// it saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    ec: u64,
    iss: u64,
    saved_btype: Btype,
}

struct Machine {
    cpu: Cpu,
    ram: Ram,
}

impl Machine {
    /// A processor implementing `imp` at EL1 with the MMU on over the
    /// tables above.
    fn new(imp: Implementation) -> Self {
        let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
        let mut cpu = Cpu::implementing(
            CpuConfig {
                ips,
                reset_pc: UNGUARDED,
            },
            Implementation { counter_hz: ips, ..imp },
        );
        let mut ram = Ram(std::vec![0; RAM_BYTES]);
        const AF: u64 = 1 << 10;
        const INNER_SHAREABLE: u64 = 3 << 8;
        const BLOCK: u64 = 0b01;
        const GP: u64 = 1 << 50;
        const READ_ONLY_BOTH: u64 = 0b11 << 6;
        let descriptors = [
            (L1, L2 | 0b11),
            (L2, AF | INNER_SHAREABLE | BLOCK),
            (L2 + 8, GUARDED | AF | INNER_SHAREABLE | GP | BLOCK),
            (L2 + 16, GUARDED_EL0 | AF | INNER_SHAREABLE | READ_ONLY_BOTH | GP | BLOCK),
        ];
        for (at, desc) in descriptors {
            let at = at as usize;
            ram.0[at..at + 8].copy_from_slice(&desc.to_le_bytes());
        }
        cpu.sys.mair_el1 = 0xFF;
        // T0SZ = 25, walks from level 1; TTBR1 walks disabled (EPD1).
        cpu.sys.tcr_el1 = 25 | (1 << 23);
        cpu.sys.ttbr0_el1 = L1;
        cpu.sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1;
        cpu.sys.vbar_el1 = VBAR;
        Self { cpu, ram }
    }

    fn put(&mut self, at: u64, insn: u32) {
        let at = at as usize;
        self.ram.0[at..at + 4].copy_from_slice(&insn.to_le_bytes());
    }

    /// Step the word at `at`: the exception it took, if it took one. The
    /// processor is put back at the exception level it executed at.
    fn step_at(&mut self, at: u64) -> Option<Taken> {
        let pstate = self.cpu.pstate;
        self.cpu.pc = at;
        self.cpu.sys.esr_el1 = 0x3F << 26;
        assert_eq!(self.cpu.step(&mut self.ram, &mut NoInstrumentation), Step::Retired);
        let esr = self.cpu.sys.esr_el1;
        if esr >> 26 == 0x3F {
            return None;
        }
        assert_eq!(self.cpu.pc & !0x7FF, VBAR, "{at:#x} took an exception to the vector table");
        assert_eq!(self.cpu.pstate.btype, Btype::None, "an exception leaves BTYPE 00");
        self.cpu.pstate = Pstate { btype: Btype::None, ..pstate };
        Some(Taken {
            ec: esr >> 26,
            iss: esr & 0x01FF_FFFF,
            saved_btype: Btype::from_bits(self.cpu.sys.spsr_el1 >> 10),
        })
    }

    /// Execute `branch` at `from` to `target`, which holds `landing`
    /// (X1 = X16 = X17 = `target`), and step the landing instruction: the
    /// BTYPE the branch left, and the exception the landing took.
    fn branch(&mut self, from: u64, branch: u32, target: u64, landing: u32) -> (Btype, Option<Taken>) {
        self.put(from, branch);
        self.put(target, landing);
        self.cpu.x[1] = target;
        self.cpu.x[16] = target;
        self.cpu.x[17] = target;
        assert_eq!(self.step_at(from), None, "the branch at {from:#x}");
        assert_eq!(self.cpu.pc, target, "the branch at {from:#x}");
        let btype = self.cpu.pstate.btype;
        (btype, self.step_at(target))
    }
}

fn branch_target(btype: Btype) -> Option<Taken> {
    Some(Taken {
        ec: BRANCH_TARGET,
        iss: btype.bits(),
        saved_btype: btype,
    })
}

/// The BTYPE a branch leaves (the instruction pages): BR in a guarded page
/// 11 (01 through X16 or X17), BR from a page that is not guarded 01, BLR
/// 10 wherever it is, RET 00; the authenticating forms alike. A guarded
/// target accepts it only with a BTI that names it
/// (`BTypeCompatible_BTI`): `BTI c` 01 and 10, `BTI j` 01 and 11, `BTI jc`
/// all, `BTI` and any other instruction none.
#[test]
fn an_indirect_branch_into_a_guarded_page_needs_a_bti_that_accepts_it() {
    use Btype::{Call, Jump, JumpOrCall};
    let cases = [
        (GUARDED, BR_X1, Jump),
        (GUARDED, BR_X16, JumpOrCall),
        (GUARDED, BR_X17, JumpOrCall),
        (UNGUARDED, BR_X1, JumpOrCall),
        (GUARDED, BLR_X1, Call),
        (UNGUARDED, BLR_X1, Call),
        (GUARDED, BRAA_X1, Jump),
        (GUARDED, BLRAAZ_X1, Call),
    ];
    // SCTLR_EL1 at reset enables no key: BRAA and BLRAAZ branch to X1.
    for (from, branch, btype) in cases {
        for landing in [NOP, BTI, BTI_C, BTI_J, BTI_JC] {
            let mut m = Machine::new(Implementation::EVERY_FEATURE);
            let (left, taken) = m.branch(from, branch, TARGET, landing);
            assert_eq!(left, btype, "{branch:#010x} at {from:#x}");
            let accepted = match landing {
                BTI_C => btype != Jump,
                BTI_J => btype != Call,
                BTI_JC => true,
                _ => false,
            };
            let want = if accepted { None } else { branch_target(btype) };
            assert_eq!(taken, want, "{branch:#010x} at {from:#x} to {landing:#010x}");
            if accepted {
                assert_eq!(m.cpu.pstate.btype, Btype::None, "the landing leaves BTYPE 00");
            } else {
                assert_eq!(m.cpu.sys.elr_el1, TARGET, "the exception returns to the landing");
            }
        }
    }
}

/// No check is made outside a guarded page, after RET or a direct
/// branch, or with stage 1 disabled, where no page is guarded.
#[test]
fn an_unguarded_target_ret_and_the_mmu_off_need_no_bti() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    let (btype, taken) = m.branch(GUARDED, BR_X1, UNGUARDED + 0x100, NOP);
    assert_eq!((btype, taken), (Btype::Jump, None));
    assert_eq!(m.cpu.pstate.btype, Btype::None);

    let (btype, taken) = m.branch(GUARDED, RET_X1, TARGET, NOP);
    assert_eq!((btype, taken), (Btype::None, None));

    let mut off = Machine::new(Implementation::EVERY_FEATURE);
    off.cpu.sys.sctlr_el1 &= !1;
    let (btype, taken) = off.branch(GUARDED, BR_X1, TARGET, NOP);
    assert_eq!((btype, taken), (Btype::JumpOrCall, None), "the MMU off: BR from a page not guarded");
}

/// PACIASP and PACIBSP accept a BLR, and a BR from X16 or X17, and a BR
/// from another register unless SCTLR_EL1.BT1 (BT0 at EL0) is set
/// (`BTypeCompatible_PAC`).
#[test]
fn paciasp_accepts_a_jump_unless_bt_is_set() {
    for landing in [PACIASP, PACIBSP] {
        for (branch, bt1, accepted) in
            [(BR_X1, false, true), (BR_X1, true, false), (BR_X16, true, true), (BLR_X1, true, true)]
        {
            let mut m = Machine::new(Implementation::EVERY_FEATURE);
            if bt1 {
                m.cpu.sys.sctlr_el1 |= BT1;
            }
            let (btype, taken) = m.branch(GUARDED, branch, TARGET, landing);
            let want = if accepted { None } else { branch_target(btype) };
            assert_eq!(taken, want, "{branch:#010x} to {landing:#010x}, BT1 {bt1}");
        }
    }
    // At EL0 BT0 decides, and BT1 does not.
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.sys.sctlr_el1 |= BT1;
    m.cpu.pstate.el = El::El0;
    m.cpu.pstate.sp_sel = false;
    let (btype, taken) = m.branch(GUARDED_EL0, BR_X1, GUARDED_EL0 + 0x100, PACIASP);
    assert_eq!((btype, taken), (Btype::Jump, None));
    m.cpu.sys.sctlr_el1 |= BT0;
    let (btype, taken) = m.branch(GUARDED_EL0, BR_X1, GUARDED_EL0 + 0x100, PACIASP);
    assert_eq!(taken, branch_target(btype));
}

/// The check comes before decode refuses a word (`EndOfDecode`): an
/// UNDEFINED word in a guarded page takes the Branch Target exception, as
/// does SVC. BRK accepts any branch and keeps BTYPE in SPSR_EL1.
#[test]
fn the_check_comes_before_undefined_and_brk_accepts_any_branch() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    let (btype, taken) = m.branch(GUARDED, BR_X1, TARGET, UDF);
    assert_eq!(taken, branch_target(btype));
    let (btype, taken) = m.branch(GUARDED, BR_X1, TARGET, SVC);
    assert_eq!(taken, branch_target(btype));
    let (btype, taken) = m.branch(GUARDED, BR_X1, TARGET, BRK);
    assert_eq!(
        taken,
        Some(Taken {
            ec: 0x3C,
            iss: 0x1234,
            saved_btype: btype
        })
    );
}

/// An exception keeps BTYPE in SPSR_EL1 only for the classes whose return
/// can resume the check (`AArch64_TakeException`): an instruction abort
/// does, an SVC from a page that is not guarded saves 00. An IRQ before the
/// landing keeps it, and the ERET that returns restores it, so the check
/// still runs.
#[test]
fn spsr_keeps_btype_for_an_abort_an_interrupt_and_eret() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.put(GUARDED, BR_X1);
    m.cpu.x[1] = UNMAPPED;
    assert_eq!(m.step_at(GUARDED), None);
    let taken = m.step_at(UNMAPPED);
    assert_eq!(taken.map(|t| (t.ec, t.saved_btype)), Some((0x21, Btype::Jump)), "an instruction abort");

    let (btype, taken) = m.branch(GUARDED, BR_X1, UNGUARDED + 0x200, SVC);
    assert_eq!(btype, Btype::Jump);
    assert_eq!(taken.map(|t| (t.ec, t.saved_btype)), Some((0x15, Btype::None)), "an SVC");

    m.put(GUARDED, BR_X1);
    m.put(TARGET, NOP);
    m.put(VBAR + 0x280, ERET);
    m.cpu.x[1] = TARGET;
    assert_eq!(m.step_at(GUARDED), None);
    let irq = m.cpu.take_irq();
    assert_eq!((irq.return_address, irq.vector), (TARGET, VBAR + 0x280));
    assert_eq!(Btype::from_bits(m.cpu.sys.spsr_el1 >> 10), Btype::Jump, "the IRQ keeps BTYPE");
    assert_eq!(m.cpu.pstate.btype, Btype::None);
    assert_eq!(m.step_at(VBAR + 0x280), None, "ERET");
    assert_eq!((m.cpu.pc, m.cpu.pstate.btype), (TARGET, Btype::Jump));
    assert_eq!(m.step_at(TARGET), branch_target(Btype::Jump));
}

/// MSR writes SPSR_EL1.BTYPE with FEAT_BTI; without it the bits are RES0.
/// An ERET from an SPSR_EL1 with BTYPE set leaves it in PSTATE.
#[test]
fn spsr_btype_is_writable_only_with_bti() {
    for (imp, kept) in [(Implementation::EVERY_FEATURE, 0b11 << 10), (Implementation::CORTEX_A57, 0)] {
        let mut m = Machine::new(imp);
        m.put(UNGUARDED, MSR_SPSR);
        m.put(UNGUARDED + 4, MRS_SPSR);
        m.cpu.x[0] = 0x3C5 | (0b11 << 10);
        assert_eq!(m.step_at(UNGUARDED), None);
        assert_eq!(m.step_at(UNGUARDED + 4), None);
        assert_eq!(m.cpu.x[0], 0x3C5 | kept, "{imp:?}");
    }
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.put(UNGUARDED, ERET);
    m.cpu.sys.spsr_el1 = 0x3C5 | (0b10 << 10);
    m.cpu.sys.elr_el1 = TARGET;
    assert_eq!(m.step_at(UNGUARDED), None);
    assert_eq!(m.cpu.pstate.btype, Btype::Call);
}

/// Without FEAT_BTI no branch sets BTYPE, BTI is a NOP, and the GP bit
/// guards nothing.
#[test]
fn without_bti_nothing_is_guarded() {
    let mut m = Machine::new(Implementation::CORTEX_A57);
    for (branch, landing) in [(BR_X1, NOP), (BLR_X1, UDF), (BR_X1, BTI)] {
        let (btype, taken) = m.branch(GUARDED, branch, TARGET, landing);
        assert_eq!(btype, Btype::None, "{branch:#010x}");
        let want = (landing == UDF).then_some(Taken {
            ec: 0x00,
            iss: 0,
            saved_btype: Btype::None,
        });
        assert_eq!(taken, want, "{branch:#010x} to {landing:#010x}");
    }
}

/// A host write of the PC is no branch: it leaves BTYPE 00, so the
/// instruction it moves to is not checked.
#[test]
fn a_host_pc_write_clears_btype() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.put(GUARDED, BR_X1);
    m.put(TARGET, NOP);
    m.cpu.x[1] = UNGUARDED;
    assert_eq!(m.step_at(GUARDED), None);
    assert_eq!(m.cpu.pstate.btype, Btype::Jump);
    m.cpu.set_reg(Reg::Pc, TARGET);
    assert_eq!(m.cpu.pstate.btype, Btype::None);
    assert_eq!(m.cpu.step(&mut m.ram, &mut NoInstrumentation), Step::Retired);
    assert_eq!(m.cpu.pc, TARGET + 4);
}

/// A TLB entry of a guarded page restores only on a processor whose walk
/// can produce one.
#[test]
fn a_guarded_tlb_entry_needs_bti() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.put(GUARDED, NOP);
    assert_eq!(m.step_at(GUARDED), None);
    let mut image = Vec::new();
    assert!(m.cpu.tlb.save(&mut image).is_ok());
    let tcr = m.cpu.sys.tcr_el1;
    let restored = |features| Tlb::new().restore(&mut image.as_slice(), features, tcr).is_ok();
    assert!(restored(Implementation::EVERY_FEATURE.features));
    assert!(!restored(Implementation::CORTEX_A57.features));
}

/// A snapshot carries PSTATE.BTYPE and the TLB's guarded pages.
#[test]
fn a_snapshot_carries_btype_and_guarded_pages() {
    // The landing shares the branch's page, whose entry the branch's fetch
    // left in the TLB.
    let landing = GUARDED + 0x100;
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.put(GUARDED, BR_X1);
    m.put(landing, NOP);
    m.cpu.x[1] = landing;
    assert_eq!(m.step_at(GUARDED), None);
    let mut image = Vec::new();
    assert!(m.cpu.save(&mut image).is_ok());

    let mut restored = Machine::new(Implementation::EVERY_FEATURE);
    restored.put(landing, NOP);
    // No guarded block in the tables: that the page is guarded comes from
    // the restored TLB.
    restored.ram.0[L2 as usize + 8..L2 as usize + 16].copy_from_slice(&(GUARDED | (1 << 10) | 1).to_le_bytes());
    assert!(restored.cpu.restore(&mut image.as_slice()).is_ok());
    assert_eq!(restored.cpu.pstate.btype, Btype::Jump);
    assert_eq!(restored.step_at(landing), branch_target(Btype::Jump));
}
