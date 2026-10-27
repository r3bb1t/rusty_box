//! FEAT_BTI on the path a machine runs its guest: pre-decoded traces, some
//! of them linked. A branch through a register sets PSTATE.BTYPE and ends
//! its trace, and the instruction it reaches is checked before it runs
//! (Arm ARM `BranchTargetCheck`, `BTypeCompatible_BTI`): `BR` from a
//! guarded page sets 11, which `BTI j` accepts and a `NOP` does not; `BLR`
//! sets 10, which `BTI c` accepts and `BTI j` does not; `RET` leaves 00,
//! which needs no landing pad. A refused landing takes a Branch Target
//! exception (EC 0x0D) whose ISS is the BTYPE, returning to the landing.
//! The Cortex-A57, without FEAT_BTI, guards no page (R9).

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    CpuModel, LentStorage, MachineBuilder, MemoryLoan, RamSize, Reg, StopReason, SysReg,
    VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 4 << 20;

/// `adr x1, .+0x40`, `adr x1, .+0x38`, `br x1`, `blr x1`, `ret x1`,
/// `add x0, x0, #1`, `nop`, `bti c`, `bti j`, `b .`, `mrs x2, esr_el1`,
/// `mrs x3, elr_el1`.
const ADR_X1_40: u32 = 0x1000_0201;
const ADR_X1_38: u32 = 0x1000_01C1;
const BR_X1: u32 = 0xD61F_0020;
const BLR_X1: u32 = 0xD63F_0020;
const RET_X1: u32 = 0xD65F_0020;
const ADD_X0_1: u32 = 0x9100_0400;
const NOP: u32 = 0xD503_201F;
const BTI_C: u32 = 0xD503_245F;
const BTI_J: u32 = 0xD503_249F;
const SPIN: u32 = 0x1400_0000;
const MRS_X2_ESR: u32 = 0xD538_5202;
const MRS_X3_ELR: u32 = 0xD538_4023;

/// The exception vectors, and the synchronous exception's handler, at
/// EL1 with SP_EL1: it reads `ESR_EL1` and `ELR_EL1` and spins.
const VBAR: u64 = RAM_BASE + 0x800;
const SYNC_HANDLER: u64 = VBAR + 0x200;

/// What a program left: X0, `ESR_EL1` and `ELR_EL1` as the handler read
/// them (zero if no exception was taken).
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    x0: u64,
    esr: u64,
    elr: u64,
}

/// Run `code` (offsets from `RAM_BASE`) on `cpu` with the MMU on, every
/// page of RAM mapped as one guarded 1 GiB block, for 200 instructions.
fn run(cpu: CpuModel, code: &[(u64, u32)]) -> Outcome {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m: VirtMachine<LentStorage<'_>> = MachineBuilder::new(VirtConfig {
        cpu,
        ..VirtConfig::DEFAULT
    })
    .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
    .bare()
    .build_in(MemoryLoan {
        ram: &mut ram,
        vars: &mut [],
    })
    .expect("a bare machine builds");
    for &(offset, word) in code.iter().chain(&[
        (SYNC_HANDLER - RAM_BASE, MRS_X2_ESR),
        (SYNC_HANDLER - RAM_BASE + 4, MRS_X3_ELR),
        (SYNC_HANDLER - RAM_BASE + 8, SPIN),
    ]) {
        m.mem_write(RAM_BASE + offset, &word.to_le_bytes())
            .expect("code is in RAM");
    }
    const AF: u64 = 1 << 10;
    const INNER_SHAREABLE: u64 = 3 << 8;
    const GP: u64 = 1 << 50;
    let l1 = RAM_BASE + 0x1_0000;
    let block = RAM_BASE | GP | AF | INNER_SHAREABLE | 0b01;
    m.mem_write(l1 + 8, &block.to_le_bytes())
        .expect("tables are in RAM");
    m.sysreg_write(SysReg::MAIR_EL1, 0xFF).expect("MAIR");
    // T0SZ 25: the walk starts at level 1; EPD1: no upper half.
    m.sysreg_write(SysReg::TCR_EL1, 25 | (1 << 23))
        .expect("TCR");
    m.sysreg_write(SysReg::TTBR0_EL1, l1).expect("TTBR0");
    m.sysreg_write(SysReg::VBAR_EL1, VBAR).expect("VBAR");
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR");
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr | 1).expect("SCTLR");
    let out = m.emu_start(RAM_BASE, None, Some(200));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    Outcome {
        x0: m.reg_read(Reg::X0),
        esr: m.reg_read(Reg::X2),
        elr: m.reg_read(Reg::X3),
    }
}

/// A Branch Target exception for `btype`, returning to `landing`.
fn branch_target(x0: u64, btype: u64, landing: u64) -> Outcome {
    Outcome {
        x0,
        esr: (0x0D << 26) | (1 << 25) | btype,
        elr: RAM_BASE + landing,
    }
}

/// `BR` from a guarded page sets BTYPE 11: `BTI j` accepts it, a `NOP` does
/// not.
#[test]
fn br_needs_a_jump_landing_pad() {
    let code = [
        (0x00, ADR_X1_40),
        (0x04, BR_X1),
        (0x40, BTI_J),
        (0x44, ADD_X0_1),
        (0x48, ADR_X1_38),
        (0x4C, BR_X1),
        (0x80, NOP),
        (0x84, SPIN),
    ];
    assert_eq!(run(CpuModel::Armv9p2, &code), branch_target(1, 0b11, 0x80));
    assert_eq!(
        run(CpuModel::CortexA57, &code),
        Outcome {
            x0: 1,
            esr: 0,
            elr: 0
        },
        "no page is guarded"
    );
}

/// `BLR` sets BTYPE 10: `BTI c` accepts it, `BTI j` does not.
#[test]
fn blr_needs_a_call_landing_pad() {
    let code = [
        (0x00, ADR_X1_40),
        (0x04, BLR_X1),
        (0x40, BTI_C),
        (0x44, ADD_X0_1),
        (0x48, ADR_X1_38),
        (0x4C, BLR_X1),
        (0x80, BTI_J),
        (0x84, SPIN),
    ];
    assert_eq!(run(CpuModel::Armv9p2, &code), branch_target(1, 0b10, 0x80));
    assert_eq!(
        run(CpuModel::CortexA57, &code),
        Outcome {
            x0: 1,
            esr: 0,
            elr: 0
        },
        "no page is guarded"
    );
}

/// `RET` leaves BTYPE 00: any instruction may follow it.
#[test]
fn ret_needs_no_landing_pad() {
    let code = [
        (0x00, ADR_X1_40),
        (0x04, RET_X1),
        (0x40, NOP),
        (0x44, ADD_X0_1),
        (0x48, SPIN),
    ];
    assert_eq!(
        run(CpuModel::Armv9p2, &code),
        Outcome {
            x0: 1,
            esr: 0,
            elr: 0
        }
    );
}
