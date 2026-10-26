//! WFI and WFE trap from EL0 only when they would wait, and the Debug
//! Communications Channel registers a guest cannot probe for exist (R9: each
//! test asserts what the guest computed or which exception it took).
//!
//! Arm ARM `Hint_WFI`: a pending interrupt, masked or not, ends WFI before
//! the trap check. `Hint_WFE`: a set Event Register is consumed before it,
//! and `SEV`, `SEVL`, every exception return (`SendEventLocal`) and the
//! virtual event stream (`TestEventCNTV`) set the register. QEMU 11.1.2
//! (target/arm/tcg/op_helper.c `HELPER(wfi)`, `HELPER(wfe)`) checks the
//! same way; the DCC registers are RAZ/WI, trapped from EL0 by
//! `MDSCR_EL1.TDCC` (target/arm/debug_helper.c `debug_cp_reginfo`).
//!
//! Guest code is hand-assembled (GNU as for aarch64). An EL1 prologue
//! optionally raises the virtual timer's interrupt (`x10` non-zero), sets
//! `CNTKCTL_EL1` from `x11` and `MDSCR_EL1` from `x12`, and returns to EL0
//! at `x13` with IRQs masked:
//!
//! ```text
//! _start: cbz x10, 1f
//!         ldr x1, =0x08000000 ; mov w2, #1 ; str w2, [x1]          // GICD_CTLR
//!         mov w2, #(1 << 27) ; str w2, [x1, #0x100]                // ISENABLER0
//!         ldr x3, =0x08010000 ; mov w2, #1 ; str w2, [x3]          // GICC_CTLR
//!         mov w2, #0xff ; str w2, [x3, #4]                         // GICC_PMR
//!         msr cntv_cval_el0, xzr ; mov x4, #1 ; msr cntv_ctl_el0, x4
//! 1:      msr cntkctl_el1, x11 ; msr mdscr_el1, x12 ; msr elr_el1, x13
//!         mov x0, #0x80 ; msr spsr_el1, x0 ; isb ; eret
//! 0x100:  wfi ; mov x5, #1 ; b .                                   // WFI_EL0
//! 0x140:  wfe ; mov x5, #1 ; wfe ; mov x6, #1 ; b .                // WFE_TWICE
//! 0x180:  wfe ; sevl ; wfe ; mov x5, #1 ; b .                      // SEVL_WFE
//! 0x1c0:  wfe ; mov x7, #4096 ; 2: subs x7, x7, #1 ; b.ne 2b ;
//!         wfe ; mov x5, #1 ; b .                                   // STREAM_WFE
//! 0x200:  msr dbgdtrtx_el0, x0 ; mov x5, #1 ; b .                  // DCC_EL0
//! 0x240:  mov x1, #7 ; msr dbgdtrtx_el0, x0 ; mrs x1, dbgdtrrx_el0 ;
//!         mrs x2, mdccsr_el0 ; msr osdtrtx_el1, x0 ;
//!         mrs x3, osdtrrx_el1 ; mov x5, #1 ; b .                   // DCC_EL1
//! 0xc00:  b .                                   // VBAR + 0x400: a trap from EL0
//! ```

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    GuestStorage, Instrumentation, LentStorage, MachineBuilder, MemoryLoan, RamSize, Reg, RunBudget, StopReason,
    SysReg, VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 1 << 20;

const PROGRAM: &[(u64, u32)] = &[
    (0x00, 0xB400_01CA),
    (0x04, 0x5800_02A1),
    (0x08, 0x5280_0022),
    (0x0C, 0xB900_0022),
    (0x10, 0x52A1_0002),
    (0x14, 0xB901_0022),
    (0x18, 0x5800_0243),
    (0x1C, 0x5280_0022),
    (0x20, 0xB900_0062),
    (0x24, 0x5280_1FE2),
    (0x28, 0xB900_0462),
    (0x2C, 0xD51B_E35F),
    (0x30, 0xD280_0024),
    (0x34, 0xD51B_E324),
    (0x38, 0xD518_E10B),
    (0x3C, 0xD510_024C),
    (0x40, 0xD518_402D),
    (0x44, 0xD280_1000),
    (0x48, 0xD518_4000),
    (0x4C, 0xD503_3FDF),
    (0x50, 0xD69F_03E0),
    (0x58, 0x0800_0000),
    (0x60, 0x0801_0000),
    (0x100, 0xD503_207F),
    (0x104, 0xD280_0025),
    (0x108, 0x1400_0000),
    (0x140, 0xD503_205F),
    (0x144, 0xD280_0025),
    (0x148, 0xD503_205F),
    (0x14C, 0xD280_0026),
    (0x150, 0x1400_0000),
    (0x180, 0xD503_205F),
    (0x184, 0xD503_20BF),
    (0x188, 0xD503_205F),
    (0x18C, 0xD280_0025),
    (0x190, 0x1400_0000),
    (0x1C0, 0xD503_205F),
    (0x1C4, 0xD282_0007),
    (0x1C8, 0xF100_04E7),
    (0x1CC, 0x54FF_FFE1),
    (0x1D0, 0xD503_205F),
    (0x1D4, 0xD280_0025),
    (0x1D8, 0x1400_0000),
    (0x200, 0xD513_0500),
    (0x204, 0xD280_0025),
    (0x208, 0x1400_0000),
    (0x240, 0xD280_00E1),
    (0x244, 0xD513_0500),
    (0x248, 0xD533_0501),
    (0x24C, 0xD533_0102),
    (0x250, 0xD510_0340),
    (0x254, 0xD530_0043),
    (0x258, 0xD280_0025),
    (0x25C, 0x1400_0000),
    (0xC00, 0x1400_0000),
];

const WFI_EL0: u64 = 0x100;
const WFE_TWICE: u64 = 0x140;
const SEVL_WFE: u64 = 0x180;
const STREAM_WFE: u64 = 0x1C0;
const DCC_EL0: u64 = 0x200;
const DCC_EL1: u64 = 0x240;
const VBAR: u64 = RAM_BASE + 0x800;
/// Where a synchronous exception from EL0 lands, and stays.
const TRAPPED: u64 = VBAR + 0x400;

/// `SCTLR_EL1.nTWI` and `nTWE`: both clear, so a waiting WFx traps from EL0.
const SCTLR_NTWI: u64 = 1 << 16;
const SCTLR_NTWE: u64 = 1 << 18;
/// `CNTKCTL_EL1`: the virtual event stream on bit 10's rising edge.
const EVENT_STREAM: u64 = (1 << 2) | (10 << 4);
const MDSCR_TDCC: u64 = 1 << 12;

/// ESR_EL1 classes.
const EC_WFX: u64 = 0x01;
const EC_SYSREG: u64 = 0x18;

struct Setup {
    raise_irq: bool,
    cntkctl: u64,
    mdscr: u64,
    el0_code: u64,
}

fn run(ram: &mut [u8], setup: Setup, instructions: u64) -> VirtMachine<LentStorage<'_>> {
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds");
    for (offset, word) in PROGRAM {
        m.mem_write(RAM_BASE + offset, &word.to_le_bytes()).expect("the program is in RAM");
    }
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR");
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr & !(SCTLR_NTWI | SCTLR_NTWE)).expect("SCTLR");
    m.sysreg_write(SysReg::VBAR_EL1, VBAR).expect("VBAR");
    m.reg_write(Reg::X10, u64::from(setup.raise_irq));
    m.reg_write(Reg::X11, setup.cntkctl);
    m.reg_write(Reg::X12, setup.mdscr);
    m.reg_write(Reg::X13, RAM_BASE + setup.el0_code);
    let out = m.step(RunBudget::Instructions(instructions));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    m
}

fn esr<S: GuestStorage, I: Instrumentation>(m: &VirtMachine<S, I>) -> u64 {
    m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1")
}

fn el0(code: u64) -> Setup {
    Setup {
        raise_irq: false,
        cntkctl: 0,
        mdscr: 0,
        el0_code: code,
    }
}

#[test]
fn wfi_with_no_interrupt_pending_traps_from_el0() {
    let mut ram = vec![0u8; RAM_LEN];
    let m = run(&mut ram, el0(WFI_EL0), 60);
    assert_eq!(m.reg_read(Reg::Pc), TRAPPED);
    assert_eq!(esr(&m) >> 26, EC_WFX);
    assert_eq!(esr(&m) & 1, 0, "TI: WFI");
    assert_eq!(m.reg_read(Reg::X5), 0);
}

#[test]
fn wfi_with_a_masked_interrupt_pending_completes_at_el0() {
    let mut ram = vec![0u8; RAM_LEN];
    let setup = Setup {
        raise_irq: true,
        ..el0(WFI_EL0)
    };
    let m = run(&mut ram, setup, 60);
    assert_eq!(m.reg_read(Reg::X5), 1, "WFI completed, ESR {:#x}", esr(&m));
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + WFI_EL0 + 8);
}

#[test]
fn the_exception_return_sets_the_event_register_for_one_wfe() {
    let mut ram = vec![0u8; RAM_LEN];
    let m = run(&mut ram, el0(WFE_TWICE), 60);
    assert_eq!(m.reg_read(Reg::X5), 1, "the first WFE consumed the ERET's event");
    assert_eq!(m.reg_read(Reg::X6), 0, "the second WFE trapped");
    assert_eq!(m.reg_read(Reg::Pc), TRAPPED);
    assert_eq!(esr(&m) >> 26, EC_WFX);
    assert_eq!(esr(&m) & 1, 1, "TI: WFE");
    let elr = m.sysreg_read(SysReg::ELR_EL1).expect("ELR_EL1");
    assert_eq!(elr, RAM_BASE + WFE_TWICE + 8, "the second WFE");
}

#[test]
fn sevl_sets_the_event_register() {
    let mut ram = vec![0u8; RAM_LEN];
    let m = run(&mut ram, el0(SEVL_WFE), 60);
    assert_eq!(m.reg_read(Reg::X5), 1, "ESR {:#x}", esr(&m));
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + SEVL_WFE + 16);
}

#[test]
fn the_event_stream_sets_the_event_register() {
    for (cntkctl, completes) in [(EVENT_STREAM, true), (0, false)] {
        let mut ram = vec![0u8; RAM_LEN];
        let setup = Setup {
            cntkctl,
            ..el0(STREAM_WFE)
        };
        let m = run(&mut ram, setup, 9_000);
        assert_eq!(m.reg_read(Reg::X5), u64::from(completes), "CNTKCTL_EL1 {cntkctl:#x}, ESR {:#x}", esr(&m));
        if !completes {
            assert_eq!(m.reg_read(Reg::Pc), TRAPPED);
        }
    }
}

#[test]
fn the_debug_communications_channel_reads_as_zero_and_ignores_writes() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("a bare machine builds");
    for (offset, word) in PROGRAM {
        m.mem_write(RAM_BASE + offset, &word.to_le_bytes()).expect("the program is in RAM");
    }
    m.reg_write(Reg::Pc, RAM_BASE + DCC_EL1);
    m.reg_write(Reg::X2, 9);
    m.reg_write(Reg::X3, 9);
    let out = m.step(RunBudget::Instructions(20));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X5), 1, "every access completed");
    assert_eq!([m.reg_read(Reg::X1), m.reg_read(Reg::X2), m.reg_read(Reg::X3)], [0, 0, 0]);
}

#[test]
fn the_debug_communications_channel_is_reachable_from_el0_unless_mdscr_tdcc_traps_it() {
    for (mdscr, completes) in [(0, true), (MDSCR_TDCC, false)] {
        let mut ram = vec![0u8; RAM_LEN];
        let setup = Setup {
            mdscr,
            ..el0(DCC_EL0)
        };
        let m = run(&mut ram, setup, 60);
        assert_eq!(m.reg_read(Reg::X5), u64::from(completes), "MDSCR_EL1 {mdscr:#x}");
        if !completes {
            assert_eq!(esr(&m) >> 26, EC_SYSREG);
        }
    }
}

#[test]
fn sctlr_el1_memory_tagging_controls_read_as_zero() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan {
            ram: &mut ram,
            vars: &mut [],
        })
        .expect("a bare machine builds");
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR");
    // ITFSB, TCF0, TCF, ATA0, ATA, TCSO0, TCSO.
    let mte = (1u64 << 37) | (3 << 38) | (3 << 40) | (1 << 42) | (1 << 43) | (1 << 58) | (1 << 59);
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr | mte | (1 << 44)).expect("SCTLR");
    assert_eq!(m.sysreg_read(SysReg::SCTLR_EL1), Ok(sctlr | (1 << 44)), "only the MTE fields read as zero");
}
