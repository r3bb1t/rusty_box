//! WFI and WFE trap from EL0 only when they would wait, and the Debug
//! Communications Channel behaves as one with no external debugger attached
//! (R9: each test asserts what the guest computed or which exception it
//! took).
//!
//! Arm ARM `Hint_WFI`: a pending interrupt, masked or not, ends WFI before
//! the trap check. `Hint_WFE`: a set Event Register is consumed before it,
//! and `SEV`, `SEVL`, every exception return (`SendEventLocal`) and the
//! virtual event stream (`TestEventCNTV`) set the register. QEMU 11.1.2
//! (target/arm/tcg/op_helper.c `HELPER(wfi)`, `HELPER(wfe)`) checks the
//! same way.
//!
//! Arm ARM (DDI 0487) chapter H4 and the `DBGDTRTX_EL0`, `DBGDTRRX_EL0`,
//! `DBGDTR_EL0`, `OSDTRRX_EL1`, `OSDTRTX_EL1`, `MDCCSR_EL0` and `MDSCR_EL1`
//! descriptions: a write to the transmit register fills DTRTX and sets
//! TXfull, which only a debugger's read clears; a read of the receive
//! register returns DTRRX while RXfull is set and clears RXfull; the
//! `OSDTR` registers read and write DTRRX and DTRTX and leave the flags as
//! they are; `MDSCR_EL1`'s TXfull and RXfull, like its other save/restore
//! fields, are read-only while the OS Lock is unlocked and restore the
//! flags while it is locked; `MDSCR_EL1.TDCC` traps EL0's accesses. QEMU
//! 11.1.2 (target/arm/debug_helper.c `debug_cp_reginfo`) reads the status
//! and the data registers as zero.
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
//! 0x280:  mrs x1, mdccsr_el0 ; mov x0, #10 ; msr dbgdtrtx_el0, x0 ;
//!         mov x7, #1000                                         // DCC_PROBE
//! 1:      mrs x2, mdccsr_el0 ; tbz x2, #29, 2f ; subs x7, x7, #1 ; b.ne 1b
//!         mov x5, #1 ; msr dbgdtrtx_el0, x0 ; mrs x3, mdccsr_el0 ;
//!         mrs x4, dbgdtrrx_el0 ; mrs x6, mdccsr_el0 ; b .
//! 2:      mov x5, #2 ; b .
//! 0x300:  msr oslar_el1, xzr ; msr dbgdtr_el0, x0 ; mrs x1, mdscr_el1 ;
//!         msr mdscr_el1, xzr ; mrs x2, mdscr_el1 ; mrs x3, mdccsr_el0 ;
//!         mov x4, #1 ; msr oslar_el1, x4 ; msr mdscr_el1, x9 ;
//!         mrs x6, mdccsr_el0 ; mrs x7, dbgdtrrx_el0 ; mrs x8, mdccsr_el0 ;
//!         msr mdscr_el1, x12 ; mrs x10, dbgdtr_el0 ; mrs x11, mdscr_el1 ;
//!         mov x5, #1 ; b .                                      // DCC_VIEW
//! 0x380:  msr oslar_el1, xzr ; msr mdscr_el1, x9 ; mrs x1, mdscr_el1 ;
//!         mov x4, #1 ; msr oslar_el1, x4 ; msr mdscr_el1, x9 ;
//!         mrs x2, mdscr_el1 ; msr oslar_el1, xzr ; msr mdscr_el1, xzr ;
//!         mrs x3, mdscr_el1 ; mov x5, #1 ; b .                  // MDSCR_LOCK
//! 0x400:  msr oslar_el1, x10 ; msr osdtrrx_el1, x0 ; msr osdtrtx_el1, x1 ;
//!         mrs x2, mdccsr_el0 ; mrs x3, osdtrrx_el1 ; mrs x4, osdtrtx_el1 ;
//!         msr dbgdtrtx_el0, x6 ; mrs x7, osdtrtx_el1 ; mrs x11, osdtrrx_el1 ;
//!         msr dbgdtr_el0, x8 ; mrs x9, osdtrtx_el1 ; mrs x12, osdtrrx_el1 ;
//!         mrs x13, mdccsr_el0 ; mov x5, #1 ; b .                // DCC_DATA
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
    (0x280, 0xD533_0101),
    (0x284, 0xD280_0140),
    (0x288, 0xD513_0500),
    (0x28C, 0xD280_7D07),
    (0x290, 0xD533_0102),
    (0x294, 0x36E8_0122),
    (0x298, 0xF100_04E7),
    (0x29C, 0x54FF_FFA1),
    (0x2A0, 0xD280_0025),
    (0x2A4, 0xD513_0500),
    (0x2A8, 0xD533_0103),
    (0x2AC, 0xD533_0504),
    (0x2B0, 0xD533_0106),
    (0x2B4, 0x1400_0000),
    (0x2B8, 0xD280_0045),
    (0x2BC, 0x1400_0000),
    (0x300, 0xD510_109F),
    (0x304, 0xD513_0400),
    (0x308, 0xD530_0241),
    (0x30C, 0xD510_025F),
    (0x310, 0xD530_0242),
    (0x314, 0xD533_0103),
    (0x318, 0xD280_0024),
    (0x31C, 0xD510_1084),
    (0x320, 0xD510_0249),
    (0x324, 0xD533_0106),
    (0x328, 0xD533_0507),
    (0x32C, 0xD533_0108),
    (0x330, 0xD510_024C),
    (0x334, 0xD533_040A),
    (0x338, 0xD530_024B),
    (0x33C, 0xD280_0025),
    (0x340, 0x1400_0000),
    (0x380, 0xD510_109F),
    (0x384, 0xD510_0249),
    (0x388, 0xD530_0241),
    (0x38C, 0xD280_0024),
    (0x390, 0xD510_1084),
    (0x394, 0xD510_0249),
    (0x398, 0xD530_0242),
    (0x39C, 0xD510_109F),
    (0x3A0, 0xD510_025F),
    (0x3A4, 0xD530_0243),
    (0x3A8, 0xD280_0025),
    (0x3AC, 0x1400_0000),
    (0x400, 0xD510_108A),
    (0x404, 0xD510_0040),
    (0x408, 0xD510_0341),
    (0x40C, 0xD533_0102),
    (0x410, 0xD530_0043),
    (0x414, 0xD530_0344),
    (0x418, 0xD513_0506),
    (0x41C, 0xD530_0347),
    (0x420, 0xD530_004B),
    (0x424, 0xD513_0408),
    (0x428, 0xD530_0349),
    (0x42C, 0xD530_004C),
    (0x430, 0xD533_010D),
    (0x434, 0xD280_0025),
    (0x438, 0x1400_0000),
    (0xC00, 0x1400_0000),
];

const WFI_EL0: u64 = 0x100;
const WFE_TWICE: u64 = 0x140;
const SEVL_WFE: u64 = 0x180;
const STREAM_WFE: u64 = 0x1C0;
const DCC_EL0: u64 = 0x200;
const DCC_EL1: u64 = 0x240;
const DCC_PROBE: u64 = 0x280;
const DCC_VIEW: u64 = 0x300;
const MDSCR_LOCK: u64 = 0x380;
const DCC_DATA: u64 = 0x400;
const VBAR: u64 = RAM_BASE + 0x800;
/// Where a synchronous exception from EL0 lands, and stays.
const TRAPPED: u64 = VBAR + 0x400;

/// `SCTLR_EL1.nTWI` and `nTWE`: both clear, so a waiting WFx traps from EL0.
const SCTLR_NTWI: u64 = 1 << 16;
const SCTLR_NTWE: u64 = 1 << 18;
/// `CNTKCTL_EL1`: the virtual event stream on bit 10's rising edge.
const EVENT_STREAM: u64 = (1 << 2) | (10 << 4);
const MDSCR_TDCC: u64 = 1 << 12;
/// `MDCCSR_EL0` and `MDSCR_EL1`: DTRTX full and DTRRX full.
const TXFULL: u64 = 1 << 29;
const RXFULL: u64 = 1 << 30;

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

/// The program run at EL1 from `code`, as reset leaves the processor (the
/// OS Lock locked), with `regs` set first.
fn run_el1<'a>(ram: &'a mut [u8], code: u64, regs: &[(Reg, u64)], instructions: u64) -> VirtMachine<LentStorage<'a>> {
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds");
    for (offset, word) in PROGRAM {
        m.mem_write(RAM_BASE + offset, &word.to_le_bytes()).expect("the program is in RAM");
    }
    m.reg_write(Reg::Pc, RAM_BASE + code);
    for &(reg, value) in regs {
        m.reg_write(reg, value);
    }
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
fn with_no_debugger_the_channel_receives_nothing_and_its_transmit_stays_full() {
    let mut ram = vec![0u8; RAM_LEN];
    let m = run_el1(&mut ram, DCC_EL1, &[(Reg::X2, 9), (Reg::X3, 9)], 20);
    assert_eq!(m.reg_read(Reg::X5), 1, "every access completed");
    assert_eq!(m.reg_read(Reg::X1), 0, "DBGDTRRX_EL0 with RXfull clear (UNKNOWN; this model reads zero)");
    assert_eq!(m.reg_read(Reg::X2), TXFULL, "MDCCSR_EL0 after a write: TXfull, and RXfull clear");
    assert_eq!(m.reg_read(Reg::X3), 0, "OSDTRRX_EL1");
}

/// Linux's `hvc_dcc_check` (drivers/tty/hvc/hvc_dcc.c): write a character,
/// then poll `MDCCSR_EL0` for TXfull to clear. With no debugger to read it,
/// it never does, so the probe gives up and concludes no channel is there.
#[test]
fn a_probe_from_el0_finds_its_character_never_read() {
    let mut ram = vec![0u8; RAM_LEN];
    let m = run(&mut ram, el0(DCC_PROBE), 5_000);
    assert_eq!(m.reg_read(Reg::X5), 1, "the probe timed out, PC {:#x}", m.reg_read(Reg::Pc));
    assert_eq!(m.reg_read(Reg::X1), 0, "MDCCSR_EL0 before any write: both flags clear");
    assert_eq!(m.reg_read(Reg::X2), TXFULL, "MDCCSR_EL0 on the last poll");
    assert_eq!(m.reg_read(Reg::X3), TXFULL, "a write while TXfull is set leaves it set");
    assert_eq!(m.reg_read(Reg::X4), 0, "DBGDTRRX_EL0 with RXfull clear");
    assert_eq!(m.reg_read(Reg::X6), TXFULL, "reading DTRRX leaves TXfull alone");
}

#[test]
fn mdscr_el1_views_the_flags_and_restores_them_only_under_the_os_lock() {
    let mut ram = vec![0u8; RAM_LEN];
    let regs = [
        (Reg::X0, 0x0123_4567_89AB_CDEF),
        (Reg::X9, RXFULL | MDSCR_TDCC),
        (Reg::X12, RXFULL | TXFULL | MDSCR_TDCC),
    ];
    let m = run_el1(&mut ram, DCC_VIEW, &regs, 40);
    assert_eq!(m.reg_read(Reg::X5), 1, "every access completed, PC {:#x}", m.reg_read(Reg::Pc));
    assert_eq!(m.reg_read(Reg::X1), TXFULL, "MDSCR_EL1 after a DBGDTR_EL0 write");
    assert_eq!(m.reg_read(Reg::X2), TXFULL, "with the OS Lock unlocked, writing zero leaves TXfull");
    assert_eq!(m.reg_read(Reg::X3), TXFULL, "MDCCSR_EL0 agrees");
    assert_eq!(m.reg_read(Reg::X6), RXFULL, "with the OS Lock locked, MDSCR_EL1 restores both flags");
    assert_eq!(m.reg_read(Reg::X7), 0x0123_4567, "DBGDTRRX_EL0 with RXfull set: the DTRRX DBGDTR_EL0's high word filled");
    assert_eq!(m.reg_read(Reg::X8), 0, "reading DBGDTRRX_EL0 cleared RXfull");
    assert_eq!(
        m.reg_read(Reg::X10),
        0x89AB_CDEF_0123_4567,
        "DBGDTR_EL0 with RXfull set: DTRTX in bits [63:32], DTRRX in [31:0]"
    );
    assert_eq!(
        m.reg_read(Reg::X11),
        TXFULL | MDSCR_TDCC,
        "with both flags restored, reading DBGDTR_EL0 cleared RXfull, and only RXfull"
    );
}

/// Arm ARM `OSDTRRX_EL1` and `OSDTRTX_EL1`: the save/restore views read and
/// write DTRRX and DTRTX, bits [31:0], and leave RXfull and TXfull as they
/// are, whether the OS Lock is locked or not (Arm deprecates their use while
/// it is unlocked). `DBGDTRTX_EL0` fills DTRTX alone; a write while TXfull
/// is set writes UNKNOWN values, which in this model leaves both registers
/// as they were. A snapshot carries both registers.
#[test]
fn the_os_save_restore_registers_reach_the_channel_data_registers() {
    let mdccsr = SysReg::new(2, 3, 0, 1, 0).expect("MDCCSR_EL0");
    let osdtrrx = SysReg::new(2, 0, 0, 0, 2).expect("OSDTRRX_EL1");
    let osdtrtx = SysReg::new(2, 0, 0, 3, 2).expect("OSDTRTX_EL1");
    for oslk in [0, 1] {
        let mut ram = vec![0u8; RAM_LEN];
        let regs = [
            (Reg::X10, oslk),
            (Reg::X0, 0xFFFF_FFFF_1111_2222),
            (Reg::X1, 0xAAAA_BBBB_3333_4444),
            (Reg::X6, 0x5555_6666),
            (Reg::X8, 0x7777_8888_9999_AAAA),
        ];
        let m = run_el1(&mut ram, DCC_DATA, &regs, 30);
        assert_eq!(m.reg_read(Reg::X5), 1, "OSLK {oslk}: every access completed, PC {:#x}", m.reg_read(Reg::Pc));
        assert_eq!(m.reg_read(Reg::X2), 0, "OSLK {oslk}: writing the OSDTR registers sets no flag");
        assert_eq!(m.reg_read(Reg::X3), 0x1111_2222, "OSLK {oslk}: OSDTRRX_EL1 reads DTRRX back");
        assert_eq!(m.reg_read(Reg::X4), 0x3333_4444, "OSLK {oslk}: OSDTRTX_EL1 reads DTRTX back");
        assert_eq!(m.reg_read(Reg::X7), 0x5555_6666, "OSLK {oslk}: DBGDTRTX_EL0 filled DTRTX");
        assert_eq!(m.reg_read(Reg::X11), 0x1111_2222, "OSLK {oslk}: and left DTRRX alone");
        assert_eq!(m.reg_read(Reg::X9), 0x5555_6666, "OSLK {oslk}: a write with TXfull set left DTRTX as it was");
        assert_eq!(m.reg_read(Reg::X12), 0x1111_2222, "OSLK {oslk}: a write with TXfull set left DTRRX as it was");
        assert_eq!(m.reg_read(Reg::X13), TXFULL, "OSLK {oslk}: MDCCSR_EL0");

        let mut image = Vec::new();
        m.save_snapshot(&mut image).expect("a snapshot saves");
        drop(m);
        let mut fresh_ram = vec![0u8; RAM_LEN];
        let mut fresh = MachineBuilder::new(VirtConfig::DEFAULT)
            .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
            .bare()
            .build_in(MemoryLoan {
                ram: &mut fresh_ram,
                vars: &mut [],
            })
            .expect("a bare machine builds");
        fresh.restore_snapshot(&mut image.as_slice()).expect("the image restores");
        assert_eq!(fresh.sysreg_read(osdtrrx), Ok(0x1111_2222), "OSLK {oslk}: DTRRX restored");
        assert_eq!(fresh.sysreg_read(osdtrtx), Ok(0x5555_6666), "OSLK {oslk}: DTRTX restored");
        assert_eq!(fresh.sysreg_read(mdccsr), Ok(TXFULL));
    }
}

/// Arm ARM `MDSCR_EL1` and Arm's ASL `__set_MDSCR_EL1`: the save/restore
/// fields — RXfull, TXfull, RXO, TXU, INTdis, TDA, HDE and ERR — take an
/// `MSR` only while `OSLSR_EL1.OSLK` is 1, and keep their value while it is
/// 0; the other fields take every write.
#[test]
fn mdscr_el1_save_restore_fields_are_written_only_under_the_os_lock() {
    const SAVE_RESTORE: u64 = RXFULL | TXFULL | (1 << 27) | (1 << 26) | (0b11 << 22) | (1 << 21) | (1 << 14) | (1 << 6);
    /// MDE, KDE and TDCC.
    const OTHERS: u64 = (1 << 15) | (1 << 13) | MDSCR_TDCC;
    let mut ram = vec![0u8; RAM_LEN];
    let m = run_el1(&mut ram, MDSCR_LOCK, &[(Reg::X9, SAVE_RESTORE | OTHERS)], 20);
    assert_eq!(m.reg_read(Reg::X5), 1, "every access completed, PC {:#x}", m.reg_read(Reg::Pc));
    assert_eq!(m.reg_read(Reg::X1), OTHERS, "unlocked: the save/restore fields keep their reset value");
    assert_eq!(m.reg_read(Reg::X2), SAVE_RESTORE | OTHERS, "locked: every field takes the write");
    assert_eq!(m.reg_read(Reg::X3), SAVE_RESTORE, "unlocked: writing zero clears only the other fields");
}

/// Arm ARM `MDSCR_EL1` for ARMv8.0: the bits that are not one of its fields
/// — the save/restore fields, SS, TDCC, KDE and MDE — are RES0, and read as
/// zero whatever is written to them, locked or not.
#[test]
fn mdscr_el1_res0_bits_read_as_zero() {
    const SAVE_RESTORE: u64 = RXFULL | TXFULL | (1 << 27) | (1 << 26) | (0b11 << 22) | (1 << 21) | (1 << 14) | (1 << 6);
    /// SS, MDE, KDE and TDCC.
    const OTHERS: u64 = 1 | (1 << 15) | (1 << 13) | MDSCR_TDCC;
    let mut ram = vec![0u8; RAM_LEN];
    let m = run_el1(&mut ram, MDSCR_LOCK, &[(Reg::X9, u64::MAX)], 20);
    assert_eq!(m.reg_read(Reg::X5), 1, "every access completed, PC {:#x}", m.reg_read(Reg::Pc));
    assert_eq!(m.reg_read(Reg::X1), OTHERS, "unlocked: the other fields, and no RES0 bit");
    assert_eq!(m.reg_read(Reg::X2), SAVE_RESTORE | OTHERS, "locked: every field, and no RES0 bit");
}

#[test]
fn the_debug_communications_channel_is_reachable_from_el0_unless_mdscr_tdcc_traps_it() {
    let mdccsr = SysReg::new(2, 3, 0, 1, 0).expect("MDCCSR_EL0");
    for (mdscr, completes) in [(0, true), (MDSCR_TDCC, false)] {
        let mut ram = vec![0u8; RAM_LEN];
        let setup = Setup {
            mdscr,
            ..el0(DCC_EL0)
        };
        let m = run(&mut ram, setup, 60);
        assert_eq!(m.reg_read(Reg::X5), u64::from(completes), "MDSCR_EL1 {mdscr:#x}");
        let status = m.sysreg_read(mdccsr).expect("MDCCSR_EL0");
        if completes {
            assert_eq!(status, TXFULL, "the write from EL0 filled DTRTX");
        } else {
            assert_eq!(esr(&m) >> 26, EC_SYSREG);
            assert_eq!(status, 0, "the trapped write had no effect");
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
