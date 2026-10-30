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
/// `ISS.WPTV` of a Watchpoint exception, and WPT's first bit.
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
