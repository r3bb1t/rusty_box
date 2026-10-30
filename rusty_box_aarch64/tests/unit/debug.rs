//! Self-hosted debug as a guest sees it (R9): Software Step, Breakpoint and
//! Watchpoint exceptions, set up by the guest's own `MSR` and `ERET`.
//!
//! The rules are the Arm ARM's D2 and pseudocode: `CheckSoftwareStep`,
//! `SSAdvance`, `DebugExceptionReturnSS`, `AArch64_CheckBreakpoint`,
//! `AArch64_BreakpointMatch`, `AArch64_BreakpointValueMatch`,
//! `AArch64_StateMatch`, `AArch64_CheckWatchpoint`,
//! `AArch64_WatchpointMatch`, `AArch64_WatchpointByteMatch` and
//! `AArch64_FaultSyndrome`.
//!
//! The words are GNU as's.

use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::features::Implementation;
use super::{id, Cpu, CpuConfig, El, Step};
use crate::bus::{AccessWidth, BusFault, CodeEpoch, FetchSource, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const CODE: u64 = 0x1000;
const VBAR: u64 = 0x2000;
const DATA: u64 = 0x3000;
/// Where [`Machine::msr`] runs its `MSR`.
const SETUP: u64 = 0x1F00;
const L1: u64 = 0x8000;
const RAM_BYTES: usize = 0x1_0000;

const NOP: u32 = 0xD503_201F;
const ERET: u32 = 0xD69F_03E0;
const SVC: u32 = 0xD400_0001;
const UNDEFINED_WORD: u32 = 0;
const LDXR: u32 = 0xC85F_7C20; // ldxr x0, [x1]
const LDR: u32 = 0xF940_0022; // ldr x2, [x1]
const LDRB: u32 = 0x3940_0022; // ldrb w2, [x1]
const STR: u32 = 0xF900_0022; // str x2, [x1]
const LDP: u32 = 0xA940_0C22; // ldp x2, x3, [x1]
const STP: u32 = 0xA900_0C22; // stp x2, x3, [x1]
const LDTR: u32 = 0xF840_0822; // ldtr x2, [x1]
const LDADD: u32 = 0xF822_0023; // ldadd x2, x3, [x1]
const DC_ZVA: u32 = 0xD50B_7421; // dc zva, x1
const DC_IVAC: u32 = 0xD508_7621; // dc ivac, x1
const DC_CVAC: u32 = 0xD50B_7A21; // dc cvac, x1
const ADD_X0: u32 = 0x9100_0400; // add x0, x0, #1
const B_BACK_4: u32 = 0x17FF_FFFF; // b .-4
/// `msr daifclr, #8`: clear PSTATE.D.
const CLEAR_D: u32 = 0xD503_48FF;
/// `msr daifset, #8`: set PSTATE.D.
const SET_D: u32 = 0xD503_48DF;
const STXR: u32 = 0xC803_7C22; // stxr w3, x2, [x1]
const ST1_16B: u32 = 0x4C00_7020; // st1 {v0.16b}, [x1]
const LDR_D0: u32 = 0xFD40_0020; // ldr d0, [x1]
const LDR_Z0: u32 = 0x8580_4020; // ldr z0, [x1]
const STR_Z0: u32 = 0xE580_4020; // str z0, [x1]
const PTRUE_P0: u32 = 0x2518_E3E0; // ptrue p0.b
const LD1B_Z0: u32 = 0xA400_A020; // ld1b {z0.b}, p0/z, [x1]
const LDR_ZA: u32 = 0xE100_0020; // ldr za[w12, 0], [x1]
const SMSTART: u32 = 0xD503_477F;
/// `CPACR_EL1.FPEN` and `SMEN`: EL0 and EL1 may use them.
const FP_ENABLED: u64 = 3 << 20;
const SME_ENABLED: u64 = 3 << 24;
/// `msr mdscr_el1, x0`.
const MSR_MDSCR: u32 = 0xD510_0240;

/// `MSR` encodings: op0, op1, CRn, CRm, op2.
type Key = (u32, u32, u32, u32, u32);
const MDSCR_EL1: Key = (2, 0, 0, 2, 2);
const OSLAR_EL1: Key = (2, 0, 1, 0, 4);
const SPSR_EL1: Key = (3, 0, 4, 0, 0);
const ELR_EL1: Key = (3, 0, 4, 0, 1);
const CONTEXTIDR_EL1: Key = (3, 0, 13, 0, 1);
const fn dbgbvr(n: u32) -> Key {
    (2, 0, 0, n, 4)
}
const fn dbgbcr(n: u32) -> Key {
    (2, 0, 0, n, 5)
}
const fn dbgwvr(n: u32) -> Key {
    (2, 0, 0, n, 6)
}
const fn dbgwcr(n: u32) -> Key {
    (2, 0, 0, n, 7)
}

/// `MDSCR_EL1.SS`, `KDE` and `MDE`.
const SS: u64 = 1;
const KDE: u64 = 1 << 13;
const MDE: u64 = 1 << 15;
/// `SPSR_ELx.SS`, and M for EL0t and EL1h.
const SPSR_SS: u64 = 1 << 21;
const EL0T: u64 = 0;
const EL1H: u64 = 0b0101;

/// The exception classes of D2.
const BREAKPOINT_LOWER: u64 = 0x30;
const BREAKPOINT_SAME: u64 = 0x31;
const STEP_LOWER: u64 = 0x32;
const STEP_SAME: u64 = 0x33;
const WATCHPOINT_LOWER: u64 = 0x34;
const WATCHPOINT_SAME: u64 = 0x35;
const SVC_CLASS: u64 = 0x15;
const UNDEFINED_CLASS: u64 = 0x00;
const DATA_ABORT_SAME: u64 = 0x25;

/// IFSC and DFSC: a debug exception. A permission fault at level 1.
const DEBUG: u64 = 0b10_0010;
const PERMISSION_L1: u64 = 0b00_1101;
/// `ISS.ISV` and `EX` of a Software Step exception.
const ISV: u64 = 1 << 24;
const EX: u64 = 1 << 6;
/// `ISS.WnR` and `CM` of a Watchpoint exception.
const WNR: u64 = 1 << 6;
const CM: u64 = 1 << 8;
/// `ISS.WPF`, `WPTV` of a Watchpoint exception, and WPT's first bit.
const WPF: u64 = 1 << 16;
const WPTV: u64 = 1 << 17;
const WPT: u32 = 18;

/// `DBGBCR<n>_EL1` of a breakpoint of type `bt`, linked to `lbn`, at the
/// ELs `pmc` names.
const fn breakpoint(bt: u64, lbn: u64, pmc: u64) -> u64 {
    1 | (pmc << 1) | (0xF << 5) | (lbn << 16) | (bt << 20)
}
const fn watchpoint(lsc: u64, bas: u64, pmc: u64) -> u64 {
    1 | (pmc << 1) | (lsc << 3) | (bas << 5)
}
/// BT: an address match, linked or not, a CONTEXTIDR_EL1 match, and that
/// with linking enabled.
const ADDRESS: u64 = 0b0000;
const LINKED_ADDRESS: u64 = 0b0001;
const CONTEXT: u64 = 0b0010;
const LINKING_CONTEXT: u64 = 0b0011;
/// PMC and PAC: EL1, EL0 and both.
const EL1_ONLY: u64 = 0b01;
const EL0_ONLY: u64 = 0b10;
const BOTH_ELS: u64 = 0b11;
/// LSC: loads, stores and both.
const LOADS: u64 = 0b01;
const STORES: u64 = 0b10;
const ALL: u64 = 0b11;
/// BAS: every byte of the doubleword.
const ALL_BYTES: u64 = 0xFF;

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

    fn fetch_source(&self, _addr: PhysAddr) -> FetchSource {
        FetchSource::Stable(CodeEpoch::new(0))
    }

    fn devices_touched(&self) -> bool {
        false
    }
}

/// An exception taken: `ESR_EL1`'s class and ISS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    ec: u64,
    iss: u64,
}

struct Machine {
    cpu: Cpu,
    ram: Ram,
}

impl Machine {
    /// A processor implementing `imp` at EL1 with the MMU off, every debug
    /// exception masked by PSTATE.D, and the OS Lock unlocked.
    fn new(imp: Implementation) -> Self {
        let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
        let cpu = Cpu::implementing(
            CpuConfig {
                ips,
                reset_pc: CODE,
            },
            Implementation {
                counter_hz: ips,
                ..imp
            },
        );
        let mut m = Self {
            cpu,
            ram: Ram(std::vec![0; RAM_BYTES]),
        };
        m.cpu.sys.vbar_el1 = VBAR;
        m.msr(OSLAR_EL1, 0);
        m
    }

    /// Set the MMU on over one identity 1 GiB block of Normal Write-Back
    /// Inner Shareable memory, EL1 read/write and executable.
    fn mmu_on(&mut self) {
        const AF: u64 = 1 << 10;
        const INNER_SHAREABLE: u64 = 3 << 8;
        let block = AF | INNER_SHAREABLE | 0b01;
        let at = L1 as usize;
        self.ram.0[at..at + 8].copy_from_slice(&block.to_le_bytes());
        self.cpu.sys.mair_el1 = 0xFF;
        self.cpu.sys.tcr_el1 = 25;
        self.cpu.sys.ttbr0_el1 = L1;
        self.cpu.sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1 | (1 << 2);
    }

    fn put(&mut self, at: u64, word: u32) {
        let at = at as usize;
        self.ram.0[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }

    fn step(&mut self) {
        assert_eq!(
            self.cpu.step(&mut self.ram, &mut NoInstrumentation),
            Step::Retired
        );
    }

    /// Run `word` at `pc`.
    fn run(&mut self, pc: u64, word: u32) {
        self.put(pc, word);
        self.cpu.pc = pc;
        self.step();
    }

    /// Run `word` at `CODE`, returning the exception it took, as `ESR_EL1`
    /// reports it. The exception level it ran at is put back.
    fn exec(&mut self, word: u32) -> Option<Taken> {
        let pstate = self.cpu.pstate;
        self.cpu.sys.esr_el1 = 0x3F << 26;
        self.run(CODE, word);
        let taken = self.taken();
        if taken.is_some() {
            self.cpu.pstate = pstate;
        }
        taken
    }

    fn taken(&self) -> Option<Taken> {
        let esr = self.cpu.sys.esr_el1;
        (esr >> 26 != 0x3F).then_some(Taken {
            ec: esr >> 26,
            iss: esr & 0x01FF_FFFF,
        })
    }

    /// `MSR` at EL1 of `value` to the register `key` names.
    fn msr(&mut self, key: Key, value: u64) {
        let (op0, op1, crn, crm, op2) = key;
        let word = 0xD500_0000 | (op0 << 19) | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5);
        let saved = (self.cpu.pc, self.cpu.x[0]);
        let pstate = self.cpu.pstate;
        self.cpu.pstate.el = El::El1;
        self.cpu.pstate.d = true;
        self.cpu.x[0] = value;
        self.run(SETUP, word);
        self.cpu.pstate = pstate;
        (self.cpu.pc, self.cpu.x[0]) = saved;
    }

    /// Execute `ERET` at EL1, with debug exceptions masked, into `spsr` at
    /// `target`.
    fn eret(&mut self, spsr: u64, target: u64) {
        self.msr(SPSR_EL1, spsr);
        self.msr(ELR_EL1, target);
        self.cpu.sys.esr_el1 = 0x3F << 26;
        self.run(SETUP, ERET);
        assert_eq!(self.cpu.pc, target);
    }

    fn at_el0(&mut self) {
        self.cpu.pstate.el = El::El0;
        self.cpu.pstate.sp_sel = false;
        self.cpu.pstate.d = false;
    }

    /// At EL1, debug exceptions unmasked (`MDSCR_EL1.KDE`, PSTATE.D).
    fn at_el1_debuggable(&mut self, mdscr: u64) {
        self.msr(MDSCR_EL1, mdscr | KDE);
        self.cpu.pstate.d = false;
    }

    /// `n` instructions by [`Cpu::run_trace`], as a machine runs them, the
    /// timer outputs just evaluated, and how many retired.
    fn run_traces(&mut self, n: u64) -> u64 {
        let retired = self.cpu.retired;
        self.cpu.timer_dirty = false;
        self.cpu.bound_run(n);
        assert_eq!(
            self.cpu
                .run_trace(&mut self.ram, &mut NoInstrumentation, n, &[]),
            Step::Retired
        );
        self.cpu.retired - retired
    }
}

fn v8() -> Machine {
    Machine::new(Implementation::CORTEX_A57)
}

fn v9() -> Machine {
    Machine::new(Implementation::ARMV9P2)
}

// ----- Software Step -----

/// An `ERET` with `MDSCR_EL1.SS` and `SPSR_EL1.SS` set runs one instruction
/// at EL0, then takes a Software Step exception to the debugger, whose
/// preferred return address is the instruction not yet run, with the
/// syndrome valid, since an instruction was stepped.
#[test]
fn a_debugger_steps_one_instruction() {
    let mut m = v8();
    m.msr(MDSCR_EL1, SS);
    m.eret(EL0T | SPSR_SS, CODE);
    assert!(m.cpu.pstate.ss, "active-not-pending");
    m.run(CODE, ADD_X0);
    assert_eq!(m.cpu.x[0], 1);
    assert!(!m.cpu.pstate.ss, "active-pending");
    assert_eq!(m.cpu.pc, CODE + 4);
    assert_eq!(m.taken(), None);

    m.put(CODE + 4, ADD_X0);
    m.step();
    assert_eq!(m.cpu.x[0], 1, "the next instruction does not run");
    assert_eq!(
        m.taken(),
        Some(Taken {
            ec: STEP_LOWER,
            iss: ISV | DEBUG,
        })
    );
    assert_eq!(m.cpu.sys.elr_el1, CODE + 4);
    assert_eq!(m.cpu.sys.spsr_el1 & SPSR_SS, 0);
    assert_eq!(m.cpu.pc, VBAR + 0x400);
    assert_eq!(m.cpu.pstate.el, El::El1);
    assert!(!m.cpu.pstate.ss);
}

/// Stepping a Load-Exclusive sets `ISS.EX`.
#[test]
fn stepping_a_load_exclusive_reports_it() {
    let mut m = v8();
    m.cpu.x[1] = DATA;
    m.msr(MDSCR_EL1, SS);
    m.eret(EL0T | SPSR_SS, CODE);
    m.run(CODE, LDXR);
    m.step();
    assert_eq!(
        m.taken(),
        Some(Taken {
            ec: STEP_LOWER,
            iss: ISV | EX | DEBUG,
        })
    );
}

/// An SVC completes the step: its exception saves `SPSR_EL1.SS` 0, so the
/// return from it takes the Software Step exception. In the handler,
/// PSTATE.D masks the step.
#[test]
fn an_svc_completes_the_step() {
    let mut m = v8();
    m.msr(MDSCR_EL1, SS);
    m.eret(EL0T | SPSR_SS, CODE);
    m.run(CODE, SVC);
    assert_eq!(m.taken().map(|t| t.ec), Some(SVC_CLASS));
    assert_eq!(m.cpu.sys.elr_el1, CODE + 4);
    assert_eq!(m.cpu.sys.spsr_el1 & SPSR_SS, 0);
    m.run(VBAR + 0x400, ADD_X0);
    assert_eq!(m.cpu.x[0], 1, "the handler runs, masked");
    assert_eq!(m.taken().map(|t| t.ec), Some(SVC_CLASS));
}

/// Any other exception leaves the instruction unstepped: `SPSR_EL1.SS` is
/// 1, and the return runs it again.
#[test]
fn an_exception_in_place_of_the_stepped_instruction_saves_ss() {
    let mut m = v8();
    m.msr(MDSCR_EL1, SS);
    m.eret(EL0T | SPSR_SS, CODE);
    m.run(CODE, UNDEFINED_WORD);
    assert_eq!(m.taken().map(|t| t.ec), Some(UNDEFINED_CLASS));
    assert_eq!(m.cpu.sys.elr_el1, CODE);
    assert_ne!(m.cpu.sys.spsr_el1 & SPSR_SS, 0);
    assert!(!m.cpu.pstate.ss);
}

/// An interrupt taken before the stepped instruction saves `SPSR_EL1.SS` 1
/// as well.
#[test]
fn an_interrupt_before_the_stepped_instruction_saves_ss() {
    let mut m = v8();
    m.msr(MDSCR_EL1, SS);
    m.eret(EL0T | SPSR_SS, CODE);
    m.cpu.take_irq();
    assert_ne!(m.cpu.sys.spsr_el1 & SPSR_SS, 0);
    assert_eq!(m.cpu.sys.elr_el1, CODE);
}

/// An `ERET` with `SPSR_EL1.SS` 0 enters active-pending: the exception comes
/// before the instruction at the target runs, with no syndrome, since
/// nothing was stepped.
#[test]
fn a_return_with_ss_clear_takes_the_exception_at_once() {
    let mut m = v8();
    m.msr(MDSCR_EL1, SS);
    m.eret(EL0T, CODE);
    m.put(CODE, ADD_X0);
    m.step();
    assert_eq!(m.cpu.x[0], 0);
    assert_eq!(
        m.taken(),
        Some(Taken {
            ec: STEP_LOWER,
            iss: DEBUG,
        })
    );
    assert_eq!(m.cpu.sys.elr_el1, CODE);
}

/// With `MDSCR_EL1.KDE` set and PSTATE.D clear, EL1 is stepped too, and the
/// exception is the same-level class.
#[test]
fn el1_is_stepped_with_kde_and_d_clear() {
    let mut m = v8();
    m.msr(MDSCR_EL1, SS | KDE);
    m.eret(EL1H | SPSR_SS, CODE);
    assert!(!m.cpu.pstate.d);
    m.run(CODE, NOP);
    m.step();
    assert_eq!(m.taken().map(|t| t.ec), Some(STEP_SAME));
}

/// The step state machine also steps code a machine runs from a trace: one
/// instruction, and the exception.
#[test]
fn a_stepped_instruction_runs_alone_from_a_trace() {
    let mut m = v8();
    m.put(CODE, ADD_X0);
    m.put(CODE + 4, ADD_X0);
    m.put(CODE + 8, B_BACK_4);
    m.cpu.pc = CODE;
    assert_eq!(m.run_traces(3), 3, "the trace runs");
    m.cpu.x[0] = 0;
    m.msr(MDSCR_EL1, SS);
    m.eret(EL0T | SPSR_SS, CODE);
    assert_eq!(m.run_traces(100), 1);
    assert_eq!(m.cpu.x[0], 1);
    assert_eq!(m.taken(), None);
    assert_eq!(m.run_traces(100), 1);
    assert_eq!(m.cpu.x[0], 1);
    assert_eq!(m.taken().map(|t| t.ec), Some(STEP_LOWER));
}

/// A snapshot taken between the step and its exception keeps the syndrome.
#[test]
fn a_snapshot_carries_what_was_stepped() {
    let mut m = v8();
    m.cpu.x[1] = DATA;
    m.msr(MDSCR_EL1, SS);
    m.eret(EL0T | SPSR_SS, CODE);
    m.run(CODE, LDXR);
    let mut image = Vec::new();
    assert!(m.cpu.save(&mut image).is_ok());
    let mut restored = v8();
    let mut bytes = image.as_slice();
    assert!(restored.cpu.restore(&mut bytes).is_ok());
    restored.ram = m.ram;
    restored.step();
    assert_eq!(
        restored.taken(),
        Some(Taken {
            ec: STEP_LOWER,
            iss: ISV | EX | DEBUG,
        })
    );
}

/// A stepped instruction that masks debug exceptions leaves Software Step
/// inactive: the unmasking that follows enters active-pending from there,
/// with no syndrome to record (Arm ARM D2.11.6, D2.11.9.1).
#[test]
fn a_step_that_masks_debug_exceptions_records_no_syndrome() {
    let mut m = v8();
    m.msr(MDSCR_EL1, SS | KDE);
    m.eret(EL1H | SPSR_SS, CODE);
    m.run(CODE, SET_D);
    m.run(CODE + 4, NOP);
    assert_eq!(m.taken(), None, "masked, the step is inactive");
    m.run(CODE + 8, CLEAR_D);
    m.put(CODE + 12, ADD_X0);
    m.step();
    assert_eq!(m.cpu.x[0], 0);
    assert_eq!(
        m.taken(),
        Some(Taken {
            ec: STEP_SAME,
            iss: DEBUG,
        })
    );
}

/// So does a stepped write that clears `MDSCR_EL1.KDE`.
#[test]
fn a_step_that_clears_kde_records_no_syndrome() {
    let mut m = v8();
    m.msr(MDSCR_EL1, SS | KDE);
    m.eret(EL1H | SPSR_SS, CODE);
    m.cpu.x[0] = SS;
    m.run(CODE, MSR_MDSCR);
    m.msr(MDSCR_EL1, SS | KDE);
    m.put(CODE + 4, ADD_X0);
    m.step();
    assert_eq!(
        m.taken(),
        Some(Taken {
            ec: STEP_SAME,
            iss: DEBUG,
        })
    );
}

/// An ERET that is itself stepped writes PSTATE.SS, so Software Step does
/// not advance past it, and the exception that follows it records no
/// syndrome, which D2.11.9.1 leaves optional.
#[test]
fn a_stepped_eret_records_no_syndrome() {
    const TARGET: u64 = CODE + 0x40;
    let mut m = v8();
    m.msr(MDSCR_EL1, SS | KDE);
    m.eret(EL1H | SPSR_SS, CODE);
    m.msr(SPSR_EL1, EL1H);
    m.msr(ELR_EL1, TARGET);
    m.run(CODE, ERET);
    assert_eq!(m.cpu.pc, TARGET);
    assert!(!m.cpu.pstate.ss);
    m.put(TARGET, ADD_X0);
    m.step();
    assert_eq!(m.cpu.x[0], 0);
    assert_eq!(
        m.taken(),
        Some(Taken {
            ec: STEP_SAME,
            iss: DEBUG,
        })
    );
    assert_eq!(m.cpu.sys.elr_el1, TARGET);
}

/// An illegal exception return's destination is the current level
/// (`DebugExceptionReturnSS`, D2.11.10.1). With SPSR_EL1.SS set the
/// Illegal Execution state exception comes first and saves SS 1; with it
/// clear Software Step comes first and saves IL 1.
#[test]
fn an_illegal_return_under_software_step() {
    const ILLEGAL_EL2H: u64 = 0b01001;
    const SPSR_IL: u64 = 1 << 20;
    const ILLEGAL_STATE: u64 = 0x0E;

    let mut m = v8();
    m.msr(MDSCR_EL1, SS | KDE);
    m.eret(ILLEGAL_EL2H | SPSR_SS, CODE);
    assert!(m.cpu.pstate.il && m.cpu.pstate.ss);
    m.put(CODE, ADD_X0);
    m.step();
    assert_eq!(m.taken().map(|t| t.ec), Some(ILLEGAL_STATE));
    assert_ne!(m.cpu.sys.spsr_el1 & SPSR_SS, 0);

    let mut m = v8();
    m.msr(MDSCR_EL1, SS | KDE);
    m.eret(ILLEGAL_EL2H, CODE);
    assert!(m.cpu.pstate.il && !m.cpu.pstate.ss);
    m.put(CODE, ADD_X0);
    m.step();
    assert_eq!(
        m.taken(),
        Some(Taken {
            ec: STEP_SAME,
            iss: DEBUG,
        })
    );
    assert_ne!(m.cpu.sys.spsr_el1 & SPSR_IL, 0);
}

// ----- Breakpoints -----

/// A breakpoint on an instruction's address fires before it runs, at EL0
/// with `MDSCR_EL1.MDE` set; the preferred return address is the
/// instruction's.
#[test]
fn an_address_breakpoint_fires_before_the_instruction() {
    let mut m = v8();
    m.msr(dbgbvr(0), CODE);
    m.msr(dbgbcr(0), breakpoint(ADDRESS, 0, EL0_ONLY));
    m.msr(MDSCR_EL1, MDE);
    m.at_el0();
    assert_eq!(
        m.exec(ADD_X0),
        Some(Taken {
            ec: BREAKPOINT_LOWER,
            iss: DEBUG,
        })
    );
    assert_eq!(m.cpu.x[0], 0);
    assert_eq!(m.cpu.sys.elr_el1, CODE);
    m.run(CODE + 4, ADD_X0);
    assert_eq!(m.cpu.x[0], 1, "another address runs");
}

/// `MDSCR_EL1.MDE` enables breakpoints, the OS Lock disables every debug
/// exception, and at EL1 `KDE` and PSTATE.D decide.
#[test]
fn breakpoints_need_their_enables() {
    let mut m = v8();
    m.msr(dbgbvr(0), CODE);
    m.msr(dbgbcr(0), breakpoint(ADDRESS, 0, BOTH_ELS));
    m.at_el0();
    assert_eq!(m.exec(NOP), None, "MDE clear");
    m.msr(MDSCR_EL1, MDE);
    m.msr(OSLAR_EL1, 1);
    assert_eq!(m.exec(NOP), None, "OS Lock locked");
    m.msr(OSLAR_EL1, 0);
    assert_eq!(m.exec(NOP).map(|t| t.ec), Some(BREAKPOINT_LOWER));

    let mut m = v8();
    m.msr(dbgbvr(0), CODE);
    m.msr(dbgbcr(0), breakpoint(ADDRESS, 0, BOTH_ELS));
    m.msr(MDSCR_EL1, MDE);
    assert_eq!(m.exec(NOP), None, "EL1 with KDE clear and D set");
    m.at_el1_debuggable(MDE);
    assert_eq!(m.exec(NOP).map(|t| t.ec), Some(BREAKPOINT_SAME));
    m.cpu.pstate.d = true;
    assert_eq!(m.exec(NOP), None, "PSTATE.D masks it");
}

/// PMC names the ELs a breakpoint fires at; 0b00, and any HMC or SSC on a
/// processor with no EL2 or EL3, is reserved, which is disabled here.
#[test]
fn pmc_selects_the_exception_level() {
    let mut m = v8();
    m.msr(dbgbvr(0), CODE);
    m.at_el1_debuggable(MDE);
    for (pmc, fires) in [
        (EL1_ONLY, true),
        (EL0_ONLY, false),
        (BOTH_ELS, true),
        (0, false),
    ] {
        m.msr(dbgbcr(0), breakpoint(ADDRESS, 0, pmc));
        assert_eq!(m.exec(NOP).is_some(), fires, "PMC {pmc:#04b}");
    }
    for reserved in [1 << 13, 1 << 14, 2 << 14] {
        m.msr(dbgbcr(0), breakpoint(ADDRESS, 0, BOTH_ELS) | reserved);
        assert_eq!(m.exec(NOP), None, "{reserved:#x}");
    }
}

/// A breakpoint's enable, E, and its type decide: BT values this processor
/// has no such breakpoint for are reserved, which is disabled here.
#[test]
fn a_disabled_or_reserved_breakpoint_is_silent() {
    let mut m = v8();
    m.msr(dbgbvr(0), CODE);
    m.at_el1_debuggable(MDE);
    m.msr(dbgbcr(0), breakpoint(ADDRESS, 0, BOTH_ELS) & !1);
    assert_eq!(m.exec(NOP), None, "E clear");
    for bt in [0b0100, 0b0101, 0b1000, 0b1111] {
        m.msr(dbgbcr(0), breakpoint(bt, 0, BOTH_ELS));
        assert_eq!(m.exec(NOP), None, "BT {bt:#06b}");
    }
}

/// Breakpoint 1 is the context-aware one (`ID_AA64DFR0_EL1.CTX_CMPs` 0): it
/// matches `CONTEXTIDR_EL1`, where the same type on breakpoint 0 is
/// reserved.
#[test]
fn only_the_highest_breakpoint_matches_a_context() {
    let mut m = v8();
    m.msr(CONTEXTIDR_EL1, 0x1234);
    m.at_el1_debuggable(MDE);
    m.msr(dbgbvr(1), 0x1234);
    m.msr(dbgbcr(1), breakpoint(CONTEXT, 0, BOTH_ELS));
    assert_eq!(m.exec(NOP).map(|t| t.ec), Some(BREAKPOINT_SAME));
    m.msr(dbgbvr(1), 0x1235);
    assert_eq!(m.exec(NOP), None, "another context");
    m.msr(dbgbcr(1), 0);
    m.msr(dbgbvr(0), 0x1234);
    m.msr(dbgbcr(0), breakpoint(CONTEXT, 0, BOTH_ELS));
    assert_eq!(m.exec(NOP), None, "breakpoint 0 is not context-aware");
}

/// A linked address breakpoint fires only in the context of the breakpoint
/// it links to, and that breakpoint, with linking enabled, never fires on
/// its own.
#[test]
fn a_linked_breakpoint_needs_its_context() {
    let mut m = v8();
    m.msr(CONTEXTIDR_EL1, 0x55);
    m.at_el1_debuggable(MDE);
    m.msr(dbgbvr(1), 0x55);
    m.msr(dbgbcr(1), breakpoint(LINKING_CONTEXT, 0, BOTH_ELS));
    assert_eq!(m.exec(NOP), None, "linking enabled fires on no event");
    m.msr(dbgbvr(0), CODE);
    m.msr(dbgbcr(0), breakpoint(LINKED_ADDRESS, 1, BOTH_ELS));
    assert_eq!(m.exec(NOP).map(|t| t.ec), Some(BREAKPOINT_SAME));
    m.msr(CONTEXTIDR_EL1, 0x56);
    assert_eq!(m.exec(NOP), None, "another context");
    m.msr(CONTEXTIDR_EL1, 0x55);
    m.msr(dbgbcr(1), breakpoint(CONTEXT, 0, BOTH_ELS) & !1);
    assert_eq!(m.exec(NOP), None, "the linked breakpoint is disabled");
}

/// A link to anything but a context-aware breakpoint with linking enabled
/// matches nothing, Arm's `Unpredictable_BPNOTLINKING` and
/// `Unpredictable_BPLINKEDADDRMATCH` defaults: itself, an address
/// breakpoint, or a breakpoint this processor does not have.
#[test]
fn a_link_to_anything_else_matches_nothing() {
    let mut m = v8();
    m.msr(dbgbvr(0), CODE);
    m.msr(dbgbvr(1), CODE);
    m.at_el1_debuggable(MDE);
    m.msr(dbgbcr(1), breakpoint(ADDRESS, 0, EL0_ONLY));
    for lbn in [0, 1, 2] {
        m.msr(dbgbcr(0), breakpoint(LINKED_ADDRESS, lbn, BOTH_ELS));
        assert_eq!(m.exec(NOP), None, "LBN {lbn}");
    }
}

/// A breakpoint fires in the middle of code that runs from a trace.
#[test]
fn a_breakpoint_fires_inside_a_trace() {
    let mut m = v8();
    m.put(CODE, ADD_X0);
    m.put(CODE + 4, ADD_X0);
    m.put(CODE + 8, B_BACK_4);
    m.cpu.pc = CODE;
    assert_eq!(m.run_traces(3), 3, "the trace runs");
    m.msr(dbgbvr(0), CODE + 4);
    m.msr(dbgbcr(0), breakpoint(ADDRESS, 0, BOTH_ELS));
    m.at_el1_debuggable(MDE);
    m.cpu.x[0] = 0;
    m.cpu.pc = CODE;
    m.cpu.sys.esr_el1 = 0x3F << 26;
    assert_eq!(m.run_traces(100), 1);
    assert_eq!(m.run_traces(100), 1);
    assert_eq!(m.cpu.x[0], 1);
    assert_eq!(m.cpu.sys.elr_el1, CODE + 4);
    assert_eq!(m.taken().map(|t| t.ec), Some(BREAKPOINT_SAME));
}

/// A breakpoint and a watchpoint compare the address up to bit 48: the value
/// registers' bits above it are left out of the comparison, Arm's
/// `Unpredictable_DBGxVR_RESS` default, and bits 1:0 of `DBGBVR<n>` too.
#[test]
fn the_value_registers_compare_up_to_bit_48() {
    const ABOVE: u64 = (1 << 49) | (1 << 60);
    let mut m = v8();
    m.msr(dbgbvr(0), CODE | ABOVE | 3);
    m.msr(dbgbcr(0), breakpoint(ADDRESS, 0, BOTH_ELS));
    m.at_el1_debuggable(MDE);
    assert_eq!(m.exec(NOP).map(|t| t.ec), Some(BREAKPOINT_SAME));

    let mut m = watching(v8(), ALL);
    m.msr(dbgwvr(0), DATA | ABOVE);
    assert_eq!(m.exec(LDR).map(|t| t.ec), Some(WATCHPOINT_SAME));
}

/// With FEAT_Debugv8p2, BT 0b0110 and 0b0111 match `CONTEXTIDR_EL1` as
/// 0b0010 and 0b0011 do; without it they are reserved, which is disabled
/// here.
#[test]
fn feat_debugv8p2_adds_two_context_types() {
    const CONTEXT_EL1: u64 = 0b0110;
    const LINKING_CONTEXT_EL1: u64 = 0b0111;

    let mut m = v9();
    m.msr(CONTEXTIDR_EL1, 0x1234);
    m.msr(dbgbvr(1), 0x1234);
    m.at_el1_debuggable(MDE);
    m.msr(dbgbcr(1), breakpoint(CONTEXT_EL1, 0, BOTH_ELS));
    assert_eq!(m.exec(NOP).map(|t| t.ec), Some(BREAKPOINT_SAME));
    m.msr(dbgbcr(1), breakpoint(LINKING_CONTEXT_EL1, 0, BOTH_ELS));
    assert_eq!(m.exec(NOP), None, "linking enabled fires on no event");
    m.msr(dbgbvr(0), CODE);
    m.msr(dbgbcr(0), breakpoint(LINKED_ADDRESS, 1, BOTH_ELS));
    assert_eq!(m.exec(NOP).map(|t| t.ec), Some(BREAKPOINT_SAME));

    let mut m = v8();
    m.msr(CONTEXTIDR_EL1, 0x1234);
    m.msr(dbgbvr(1), 0x1234);
    m.at_el1_debuggable(MDE);
    m.msr(dbgbcr(1), breakpoint(CONTEXT_EL1, 0, BOTH_ELS));
    assert_eq!(m.exec(NOP), None, "reserved without FEAT_Debugv8p2");
}

// ----- Watchpoints -----

/// A watchpoint at `DATA` on `lsc` for both ELs, on a processor at EL1 with
/// debug exceptions enabled.
fn watching(mut m: Machine, lsc: u64) -> Machine {
    m.msr(dbgwvr(0), DATA);
    m.msr(dbgwcr(0), watchpoint(lsc, ALL_BYTES, BOTH_ELS));
    m.at_el1_debuggable(MDE);
    m.cpu.x[1] = DATA;
    m
}

/// A load or a store of a watched address takes a Watchpoint exception,
/// with FAR the address, WnR saying which, and the instruction not
/// completed; LSC picks which watch.
#[test]
fn a_watchpoint_fires_on_the_access_it_is_set_for() {
    let mut m = watching(v8(), ALL);
    m.cpu.x[2] = 0x1111;
    assert_eq!(
        m.exec(LDR),
        Some(Taken {
            ec: WATCHPOINT_SAME,
            iss: DEBUG,
        })
    );
    assert_eq!(m.cpu.sys.far_el1, DATA);
    assert_eq!(m.cpu.x[2], 0x1111, "the load wrote nothing");
    assert_eq!(m.cpu.sys.elr_el1, CODE);
    assert_eq!(
        m.exec(STR),
        Some(Taken {
            ec: WATCHPOINT_SAME,
            iss: WNR | DEBUG,
        })
    );

    let mut m = watching(v8(), LOADS);
    assert_eq!(m.exec(STR), None);
    assert!(m.exec(LDR).is_some());
    let mut m = watching(v8(), STORES);
    assert_eq!(m.exec(LDR), None);
    assert!(m.exec(STR).is_some());
    let mut m = watching(v8(), 0);
    assert_eq!(m.exec(LDR), None, "LSC 0 watches nothing");
}

/// At EL0 the class is the lower-level one.
#[test]
fn a_watchpoint_at_el0_is_the_lower_level_class() {
    let mut m = watching(v8(), ALL);
    m.at_el0();
    assert_eq!(m.exec(LDR).map(|t| t.ec), Some(WATCHPOINT_LOWER));
}

/// `MDSCR_EL1.MDE`, the OS Lock, KDE and PSTATE.D enable watchpoints as
/// they do breakpoints.
#[test]
fn watchpoints_need_their_enables() {
    let mut m = watching(v8(), ALL);
    m.msr(OSLAR_EL1, 1);
    assert_eq!(m.exec(LDR), None, "OS Lock locked");
    m.msr(OSLAR_EL1, 0);
    assert!(m.exec(LDR).is_some());
    m.cpu.pstate.d = true;
    assert_eq!(m.exec(LDR), None, "PSTATE.D masks it");
    m.cpu.pstate.d = false;
    m.msr(MDSCR_EL1, KDE);
    assert_eq!(m.exec(LDR), None, "MDE clear");
}

/// BAS selects bytes of the watched doubleword: an access touching one
/// selected byte fires, and FAR holds the first byte that matched.
#[test]
fn bas_selects_the_watched_bytes() {
    let mut m = watching(v8(), ALL);
    m.msr(dbgwcr(0), watchpoint(ALL, 0b0000_1000, BOTH_ELS));
    m.cpu.x[1] = DATA + 2;
    assert_eq!(m.exec(LDRB), None, "byte 2 is not selected");
    m.cpu.x[1] = DATA + 3;
    assert!(m.exec(LDRB).is_some());
    m.cpu.x[1] = DATA;
    assert!(m.exec(LDR).is_some(), "the doubleword holds byte 3");
    assert_eq!(m.cpu.sys.far_el1, DATA + 3);
    m.msr(dbgwcr(0), watchpoint(ALL, 0b0101_0000, BOTH_ELS));
    assert_eq!(m.exec(LDR), None, "bytes not contiguous: disabled here");
}

/// MASK watches a power-of-two range; 1 and 2 are reserved, disabled here,
/// and so is a mask with BAS other than all ones, or with address bits set
/// inside it.
#[test]
fn mask_watches_a_range() {
    const MASK_12: u64 = 12 << 24;
    let mut m = watching(v8(), ALL);
    m.msr(dbgwvr(0), 0x3000);
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, BOTH_ELS) | MASK_12);
    m.cpu.x[1] = 0x3FF8;
    assert!(m.exec(LDR).is_some());
    m.cpu.x[1] = 0x4000;
    assert_eq!(m.exec(LDR), None, "past the range");
    m.cpu.x[1] = DATA;
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, BOTH_ELS) | (1 << 24));
    assert_eq!(m.exec(LDR), None, "reserved MASK");
    m.msr(dbgwcr(0), watchpoint(ALL, 0x0F, BOTH_ELS) | MASK_12);
    assert_eq!(m.exec(LDR), None, "MASK with BAS not all ones");
    m.msr(dbgwvr(0), 0x3010);
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, BOTH_ELS) | MASK_12);
    m.cpu.x[1] = 0x3010;
    assert_eq!(m.exec(LDR), None, "address bits set inside the mask");
}

/// Word-aligned `DBGWVR` watches the word, whose BAS bits 3:0 select.
#[test]
fn a_word_watch_uses_four_byte_selects() {
    let mut m = watching(v8(), ALL);
    m.msr(dbgwvr(0), DATA + 4);
    m.msr(dbgwcr(0), watchpoint(ALL, 0b0000_0001, BOTH_ELS));
    m.cpu.x[1] = DATA + 4;
    assert!(m.exec(LDRB).is_some());
    m.cpu.x[1] = DATA;
    assert_eq!(m.exec(LDRB), None, "the word below");
}

/// PAC names the ELs of the access, so an LDTR at EL1 is watched as EL0's.
#[test]
fn pac_judges_the_access_not_the_processor() {
    let mut m = watching(v8(), ALL);
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, EL0_ONLY));
    assert_eq!(m.exec(LDR), None, "EL1's load");
    assert!(m.exec(LDTR).is_some(), "EL0's permissions, EL0's watch");
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, 0));
    assert_eq!(m.exec(LDR), None, "PAC 0 is reserved");
}

/// The second element of a pair is watched as the first is, both on a
/// page and across a page boundary.
#[test]
fn a_pair_is_watched_in_both_elements() {
    let mut m = watching(v8(), ALL);
    m.msr(dbgwvr(0), DATA + 8);
    m.cpu.x[1] = DATA;
    assert!(m.exec(LDP).is_some());
    assert_eq!(m.cpu.sys.far_el1, DATA + 8);
    assert!(m.exec(STP).is_some());

    m.msr(dbgwvr(0), 0x4000);
    m.cpu.x[1] = 0x3FF8;
    assert!(m.exec(LDP).is_some(), "across pages");
    assert_eq!(m.cpu.sys.far_el1, 0x4000);
}

/// Any HMC or SSC on a processor with no EL2 or EL3 is reserved for a
/// watchpoint as for a breakpoint, which is disabled here.
#[test]
fn a_reserved_watchpoint_state_is_silent() {
    let mut m = watching(v8(), ALL);
    for reserved in [1 << 13, 1 << 14, 2 << 14] {
        m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, BOTH_ELS) | reserved);
        assert_eq!(m.exec(LDR), None, "{reserved:#x}");
    }
}

/// `ST1` makes each element an access of its own, so a watchpoint on one
/// leaves those before it written, which Arm's `Mem` allows.
#[test]
fn st1_is_watched_an_element_at_a_time() {
    let mut m = watching(v8(), STORES);
    m.cpu.sys.cpacr_el1 = FP_ENABLED;
    m.cpu.v[0] = u128::MAX;
    m.msr(dbgwvr(0), 0x4000);
    m.cpu.x[1] = 0x3FF8;
    assert_eq!(m.exec(ST1_16B).map(|t| t.ec), Some(WATCHPOINT_SAME));
    assert_eq!(m.cpu.sys.far_el1, 0x4000);
    assert_eq!(m.ram.0[0x3FFF], 0xFF, "the page before it is written");
    assert_eq!(m.ram.0[0x4000], 0, "the watched byte is not");
}

/// An access across a page boundary is watched byte by byte, and a store
/// fires before anything is written.
#[test]
fn an_access_across_pages_is_watched() {
    let mut m = watching(v8(), ALL);
    m.mmu_on();
    m.msr(dbgwvr(0), 0x4000);
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, BOTH_ELS));
    m.cpu.x[1] = 0x3FFC;
    assert!(m.exec(LDR).is_some());
    assert_eq!(m.cpu.sys.far_el1, 0x4000);
    m.cpu.x[2] = u64::MAX;
    assert!(m.exec(STR).is_some());
    assert_eq!(m.ram.0[0x3FFC], 0, "nothing written");
}

/// With the MMU on a watchpoint fires whatever the TLB holds: a page cached
/// before the watchpoint was set, and one cached while it is set.
#[test]
fn a_tlb_hit_does_not_pass_a_watchpoint() {
    let mut m = v8();
    m.mmu_on();
    m.cpu.x[1] = DATA;
    assert_eq!(m.exec(LDR), None, "the page is in the TLB");
    m.msr(dbgwvr(0), DATA);
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, BOTH_ELS));
    m.at_el1_debuggable(MDE);
    assert!(m.exec(LDR).is_some());
    m.cpu.x[1] = DATA + 8;
    assert_eq!(m.exec(LDR), None, "this load fills the TLB");
    m.cpu.x[1] = DATA;
    assert!(m.exec(LDR).is_some(), "and the next one still fires");
}

/// A permission fault comes before a watchpoint: the exception is the Data
/// Abort.
#[test]
fn a_permission_fault_comes_first() {
    let mut m = v8();
    m.mmu_on();
    let at = L1 as usize;
    let read_only: u64 = (1 << 10) | (3 << 8) | (0b10 << 6) | 0b01;
    m.ram.0[at..at + 8].copy_from_slice(&read_only.to_le_bytes());
    m.msr(dbgwvr(0), DATA);
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, BOTH_ELS));
    m.at_el1_debuggable(MDE);
    m.cpu.x[1] = DATA;
    let taken = m.exec(STR);
    assert_eq!(taken.map(|t| t.ec), Some(DATA_ABORT_SAME));
    assert_eq!(taken.map(|t| t.iss & 0x3F), Some(PERMISSION_L1));
}

/// An atomic read-modify-write reports WnR 0 if a read would have fired it,
/// and 1 if only a write does.
#[test]
fn an_atomic_reports_the_watch_that_fired() {
    let mut m = watching(v9(), ALL);
    assert_eq!(m.exec(LDADD).map(|t| t.iss & WNR), Some(0));
    let mut m = watching(v9(), STORES);
    assert_eq!(m.exec(LDADD).map(|t| t.iss & WNR), Some(WNR));
}

/// DC ZVA is a store of its block; DC IVAC is treated as one, with CM and
/// WnR set; the other maintenance instructions are not watched.
#[test]
fn dc_zva_and_dc_ivac_are_watched_as_stores() {
    let mut m = v8();
    m.mmu_on();
    let mut m = watching(m, STORES);
    m.msr(dbgwvr(0), DATA + 0x20);
    m.cpu.x[1] = DATA + 8;
    assert_eq!(
        m.exec(DC_ZVA),
        Some(Taken {
            ec: WATCHPOINT_SAME,
            iss: WNR | DEBUG,
        })
    );
    assert_eq!(m.exec(DC_CVAC), None);
    assert_eq!(
        m.exec(DC_IVAC),
        Some(Taken {
            ec: WATCHPOINT_SAME,
            iss: CM | WNR | DEBUG,
        })
    );
    let mut m = watching(v8(), LOADS);
    m.mmu_on();
    m.msr(dbgwvr(0), DATA + 0x20);
    m.cpu.x[1] = DATA + 8;
    assert_eq!(m.exec(DC_IVAC), None, "a store to LSC");
}

/// With FEAT_Debugv8p2 the syndrome names the watchpoint; without it those
/// bits are RES0. When two match, the last one is reported.
#[test]
fn the_syndrome_names_the_watchpoint_with_debugv8p2() {
    let mut m = watching(v9(), ALL);
    m.msr(dbgwvr(1), DATA);
    m.msr(dbgwcr(1), watchpoint(ALL, ALL_BYTES, BOTH_ELS));
    assert_eq!(
        m.exec(LDR).map(|t| t.iss),
        Some(WPTV | (1 << WPT) | DEBUG)
    );
    let mut m = watching(v8(), ALL);
    assert_eq!(m.exec(LDR).map(|t| t.iss), Some(DEBUG));
}

/// A linked watchpoint fires only in the context of its breakpoint.
#[test]
fn a_linked_watchpoint_needs_its_context() {
    const WT: u64 = 1 << 20;
    const LBN_1: u64 = 1 << 16;
    let mut m = watching(v8(), ALL);
    m.msr(CONTEXTIDR_EL1, 7);
    m.msr(dbgbvr(1), 7);
    m.msr(dbgbcr(1), breakpoint(LINKING_CONTEXT, 0, BOTH_ELS));
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, BOTH_ELS) | WT | LBN_1);
    assert!(m.exec(LDR).is_some());
    m.msr(CONTEXTIDR_EL1, 8);
    assert_eq!(m.exec(LDR), None);
}

/// A Store-Exclusive the monitor would pass is watched as a store. One that
/// fails on the monitor's address before it translates raises nothing, the
/// IMPLEMENTATION DEFINED choice Arm ARM D2.9.6.1 leaves, and reports its
/// failure.
#[test]
fn a_store_exclusive_is_watched_only_once_it_translates() {
    let mut m = watching(v8(), STORES);
    assert_eq!(m.exec(STXR), None, "no monitor armed");
    assert_eq!(m.cpu.x[3], 1, "the store failed");
    assert_eq!(m.exec(LDXR), None, "a load, to a watch on stores");
    assert_eq!(m.exec(STXR).map(|t| t.ec), Some(WATCHPOINT_SAME));
}

/// A Store-Exclusive to the monitor's virtual address that fails on the
/// physical one, after it translates, is watched all the same: the
/// translation is where the check is. With no watchpoint it reports its
/// failure, so this is the failing path.
#[test]
fn a_store_exclusive_failing_after_it_translates_is_watched() {
    const SECOND_GIB: u64 = 1 << 30;
    const VA: u64 = SECOND_GIB + DATA;
    let map_second_gib = |m: &mut Machine, output: u64| {
        let block: u64 = output | (1 << 10) | (3 << 8) | 0b01;
        let at = (L1 + 8) as usize;
        m.ram.0[at..at + 8].copy_from_slice(&block.to_le_bytes());
        m.cpu.tlb.flush();
    };

    let mut m = v8();
    m.mmu_on();
    map_second_gib(&mut m, 0);
    m.cpu.x[1] = VA;
    assert_eq!(m.exec(LDXR), None);
    map_second_gib(&mut m, SECOND_GIB);
    assert_eq!(m.exec(STXR), None);
    assert_eq!(m.cpu.x[3], 1, "the physical address differs");

    let mut m = v8();
    m.mmu_on();
    map_second_gib(&mut m, 0);
    m.msr(dbgwvr(0), VA);
    m.msr(dbgwcr(0), watchpoint(STORES, ALL_BYTES, BOTH_ELS));
    m.at_el1_debuggable(MDE);
    m.cpu.x[1] = VA;
    assert_eq!(m.exec(LDXR), None);
    map_second_gib(&mut m, SECOND_GIB);
    assert_eq!(m.exec(STXR).map(|t| t.ec), Some(WATCHPOINT_SAME));
}

/// A processor in Streaming SVE mode on Normal memory, with a watchpoint
/// on `lsc` of the doubleword at `DATA`.
fn streaming(lsc: u64) -> Machine {
    let mut m = watching(v9(), lsc);
    m.mmu_on();
    m.cpu.sys.cpacr_el1 = FP_ENABLED | SME_ENABLED;
    assert_eq!(m.exec(SMSTART), None);
    m
}

/// A contiguous SVE access and an SME access are relaxed (Arm
/// `IsRelaxedWatchpointAccess`): each is watched out to the 16-byte
/// boundaries around it, and WPF says a match may be of a byte it did not
/// access, Arm's listed outcome of `Unpredictable_16BYTEROUNDEDDOWNACCESS`
/// and `...UPACCESS`. An ordinary access there is not.
#[test]
fn a_relaxed_access_is_watched_out_to_16_bytes() {
    let mut m = streaming(ALL);
    assert_eq!(m.exec(PTRUE_P0), None);
    m.cpu.x[1] = DATA + 8;
    for word in [LDR_Z0, LD1B_Z0, LDR_ZA] {
        assert_eq!(
            m.exec(word),
            Some(Taken {
                ec: WATCHPOINT_SAME,
                iss: WPTV | WPF | DEBUG,
            }),
            "{word:#010x}"
        );
        assert_eq!(m.cpu.sys.far_el1, DATA, "{word:#010x}");
    }
    assert_eq!(
        m.exec(STR_Z0),
        Some(Taken {
            ec: WATCHPOINT_SAME,
            iss: WPTV | WPF | WNR | DEBUG,
        })
    );
    assert_eq!(m.exec(LDR), None, "a general-purpose load is exact");
}

/// A SIMD&FP access is relaxed in Streaming SVE mode alone.
#[test]
fn a_simd_fp_access_is_relaxed_only_when_streaming() {
    let mut m = watching(v9(), ALL);
    m.mmu_on();
    m.cpu.sys.cpacr_el1 = FP_ENABLED | SME_ENABLED;
    m.cpu.x[1] = DATA + 8;
    assert_eq!(m.exec(LDR_D0), None, "not streaming");
    assert_eq!(m.exec(SMSTART), None);
    assert_eq!(
        m.exec(LDR_D0),
        Some(Taken {
            ec: WATCHPOINT_SAME,
            iss: WPTV | WPF | DEBUG,
        })
    );
}

/// Instruction fetches are never watched, nor are translation table walks.
#[test]
fn fetches_and_walks_are_not_watched() {
    let mut m = v8();
    m.mmu_on();
    m.msr(dbgwvr(0), CODE);
    m.msr(dbgwcr(0), watchpoint(ALL, ALL_BYTES, BOTH_ELS));
    m.msr(dbgwvr(1), L1);
    m.msr(dbgwcr(1), watchpoint(ALL, ALL_BYTES, BOTH_ELS));
    m.at_el1_debuggable(MDE);
    assert_eq!(m.exec(ADD_X0), None);
    assert_eq!(m.cpu.x[0], 1);
}

/// `MSR DAIFClr` that unmasks debug exceptions lets the pending step fire
/// on the next instruction.
#[test]
fn unmasking_d_arms_the_step() {
    let mut m = v8();
    m.msr(MDSCR_EL1, SS | KDE);
    m.run(CODE, CLEAR_D);
    m.put(CODE + 4, ADD_X0);
    m.step();
    assert_eq!(m.cpu.x[0], 0);
    assert_eq!(
        m.taken(),
        Some(Taken {
            ec: STEP_SAME,
            iss: DEBUG,
        })
    );
}
