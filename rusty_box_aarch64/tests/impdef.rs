//! What a guest on the board reads of the Cortex-A57's IMPLEMENTATION
//! DEFINED registers that describe the board (R9): `CBAR_EL1` reports the
//! PERIPHBASE where the board's device layout has its GIC, the GIC-400's
//! base on the board's own layout and QEMU's `VIRT_CPUPERIPHS` on QEMU's
//! (DDI 0488H, 4.3.70 and 8.2.4), and `L2CTLR_EL1` one processor with none
//! of the L2's register slices (4.3.58), with and without EL2. The Armv9.2
//! model has neither register: its `MRS` is UNDEFINED.
//!
//! Each guest is a few instructions at the base of RAM, where a bare
//! machine starts, encoded by hand; the comments give their assembly.

use rusty_box_aarch64::board::{GIC_DIST_BASE, RAM_BASE};
use rusty_box_aarch64::{
    CpuModel, DeviceLayout, LentStorage, Levels, MachineBuilder, MemoryLoan, NoEl2, RamSize, Reg,
    RunBudget, StopReason, SysReg, SysRegRefusal, VirtConfig, VirtMachine, WithEl2,
};

#[path = "support/qemu_virt.rs"]
mod qemu_virt;

const RAM_LEN: usize = 1 << 20;

/// `mrs x1, s3_1_c11_c0_2` (`L2CTLR_EL1`); `mrs x2, s3_1_c15_c3_0`
/// (`CBAR_EL1`); `b .`.
const READ_BOARD_REGISTERS: [u32; 3] = [0xD539_B041, 0xD539_F302, SPIN];
/// `b .`
const SPIN: u32 = 0x1400_0000;
/// The vector table the guest's exceptions are taken to, and its entry for
/// a synchronous exception from the current level on `SP_ELx`.
const VECTORS: u64 = RAM_BASE + 0x1_0000;
const CURRENT_SPX_SYNC: u64 = VECTORS + 0x200;

/// `CBAR_EL1` as a host names it.
fn cbar() -> SysReg {
    SysReg::new(3, 1, 15, 3, 0).expect("in the system register space")
}

/// The PERIPHBASE each layout ties the processor's input to: on the board's
/// own, the GIC-400's base, its distributor 0x1000 above it (DDI 0471B
/// Table 3-1); on QEMU's, `VIRT_CPUPERIPHS`, where its distributor starts.
fn periphbase(layout: DeviceLayout) -> u64 {
    match layout {
        DeviceLayout::Board => GIC_DIST_BASE - 0x1000,
        DeviceLayout::QemuVirt => 0x0800_0000,
    }
}

fn config(cpu: CpuModel, layout: DeviceLayout) -> VirtConfig {
    let config = VirtConfig {
        cpu,
        ..VirtConfig::DEFAULT
    };
    match layout {
        DeviceLayout::Board => config,
        DeviceLayout::QemuVirt => qemu_virt::qemu_virt(config),
    }
}

/// What the guest did.
#[derive(Debug, PartialEq, Eq)]
enum Ran {
    /// It reached its `b .`, X1 and X2 the two registers.
    Read { l2ctlr: u64, cbar: u64 },
    /// It took a synchronous exception to its own level, with this `ESR`
    /// and `ELR`.
    Took { esr: u64, elr: u64 },
}

/// Run [`READ_BOARD_REGISTERS`] at the level the machine starts at, its
/// vector base [`VECTORS`].
fn run<L: Levels>(machine: &mut VirtMachine<LentStorage<'_>, L>) -> Ran {
    for (i, word) in READ_BOARD_REGISTERS.iter().enumerate() {
        machine
            .mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("the program is in RAM");
    }
    machine
        .mem_write(CURRENT_SPX_SYNC, &SPIN.to_le_bytes())
        .expect("the vectors are in RAM");
    let (vbar, esr, elr) = if L::EL2 {
        (SysReg::VBAR_EL2, SysReg::ESR_EL2, SysReg::ELR_EL2)
    } else {
        (SysReg::VBAR_EL1, SysReg::ESR_EL1, SysReg::ELR_EL1)
    };
    machine
        .sysreg_write(vbar, VECTORS)
        .expect("the vector base is writable");
    let out = machine.step(RunBudget::Instructions(1_000));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    let read = |reg| machine.sysreg_read(reg).expect("the register is readable");
    match machine.reg_read(Reg::Pc) {
        pc if pc == RAM_BASE + 8 => Ran::Read {
            l2ctlr: machine.reg_read(Reg::X1),
            cbar: machine.reg_read(Reg::X2),
        },
        CURRENT_SPX_SYNC => Ran::Took {
            esr: read(esr),
            elr: read(elr),
        },
        pc => panic!("the guest stopped at {pc:#x}"),
    }
}

fn without_el2(
    ram: &mut [u8],
    cpu: CpuModel,
    layout: DeviceLayout,
) -> VirtMachine<LentStorage<'_>, NoEl2> {
    MachineBuilder::new(config(cpu, layout))
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("the test board builds")
}

fn with_el2(ram: &mut [u8], layout: DeviceLayout) -> VirtMachine<LentStorage<'_>, WithEl2> {
    MachineBuilder::new(config(CpuModel::CortexA57, layout))
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .with_el2()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("the test board builds")
}

/// On the Cortex-A57, at EL1 and at EL2, `CBAR_EL1` reads the PERIPHBASE of
/// the board's layout and `L2CTLR_EL1` 0: one processor (25:24 0b00), no
/// L1 ECC (22), no register slices (13:10), L2RSTDISABLE low (31), and the
/// RAMs' setup and latency fields at their reset values. A host reads the
/// same `CBAR_EL1`.
#[test]
fn cbar_and_l2ctlr_describe_the_board() {
    for layout in DeviceLayout::ALL {
        let want = Ran::Read {
            l2ctlr: 0,
            cbar: periphbase(layout),
        };
        let mut ram = vec![0u8; RAM_LEN];
        let mut machine = without_el2(&mut ram, CpuModel::CortexA57, layout);
        assert_eq!(run(&mut machine), want, "{layout}");
        assert_eq!(
            machine.sysreg_read(cbar()),
            Ok(periphbase(layout)),
            "{layout}"
        );
        let mut ram = vec![0u8; RAM_LEN];
        let mut machine = with_el2(&mut ram, layout);
        assert_eq!(run(&mut machine), want, "{layout}, with EL2");
    }
}

/// On the Armv9.2 model the same program's first `MRS` is UNDEFINED (EC 0,
/// IL), and a host finds no `CBAR_EL1`.
#[test]
fn the_armv9p2_model_reads_neither() {
    for layout in DeviceLayout::ALL {
        let mut ram = vec![0u8; RAM_LEN];
        let mut machine = without_el2(&mut ram, CpuModel::Armv9p2, layout);
        let undefined = Ran::Took {
            esr: 0x0200_0000,
            elr: RAM_BASE,
        };
        assert_eq!(run(&mut machine), undefined, "{layout}");
        assert_eq!(
            machine.sysreg_read(cbar()),
            Err(SysRegRefusal::Absent),
            "{layout}"
        );
    }
}
