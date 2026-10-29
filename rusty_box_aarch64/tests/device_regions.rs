//! The extent of the GIC's register regions, as the guest sees it (R9: each
//! test asserts a loaded value or a syndrome).
//!
//! QEMU 11.1.2's `virt` reserves 64 KiB windows for the GIC distributor and
//! CPU interface (`base_memmap`), but the GICv2 decodes only 0x1000 bytes of
//! the first and 0x2000 of the second (hw/intc/arm_gic_common.c
//! `gic_init_irqs_and_mmio`). The rest of each window is unassigned, so an
//! access there is a decode error, which target/arm reports as a
//! synchronous external abort.
//!
//! Guest code is hand-assembled (GNU as for aarch64); each word's source is
//! beside it.

use rusty_box_aarch64::board::{GIC_CPU_BASE, GIC_DIST_BASE, RAM_BASE};
use rusty_box_aarch64::{
    GuestStorage, Instrumentation, LentStorage, MachineBuilder, MemoryLoan, NoEl2, RamSize, Reg,
    RunBudget, StopReason, SysReg, VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 4 << 20;

/// `ldr w2, [x1]; b .`
const LOAD: [u32; 2] = [0xB940_0022, 0x1400_0000];
/// `str w2, [x1]; b .`
const STORE: [u32; 2] = [0xB900_0022, 0x1400_0000];

const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
const DFSC_SYNC_EXTERNAL: u64 = 0x10;
const WNR: u64 = 1 << 6;

fn bare(ram: &mut [u8]) -> VirtMachine<LentStorage<'_>, NoEl2> {
    MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

/// Run one access of `code` at `address` with the MMU off.
fn access<S: GuestStorage, I: Instrumentation>(
    m: &mut VirtMachine<S, NoEl2, I>,
    code: &[u32],
    address: u64,
) {
    for (i, word) in code.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("code is in RAM");
    }
    m.reg_write(Reg::X1, address);
    m.reg_write(Reg::X2, 0);
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
}

fn assert_external_abort<S: GuestStorage, I: Instrumentation>(
    m: &VirtMachine<S, NoEl2, I>,
    address: u64,
    write: bool,
) {
    let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1");
    assert_eq!(esr >> 26, EC_DATA_ABORT_SAME_EL, "ESR {esr:#x}");
    assert_eq!(esr & 0x3F, DFSC_SYNC_EXTERNAL, "ESR {esr:#x}");
    assert_eq!(esr & WNR != 0, write, "ESR {esr:#x}");
    assert_eq!(m.sysreg_read(SysReg::FAR_EL1), Ok(address));
}

#[test]
fn the_distributor_decodes_its_first_4_kib_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    // GICD_IIDR: Arm's JEP106 identity.
    access(&mut m, &LOAD, GIC_DIST_BASE + 0x008);
    assert_eq!(m.reg_read(Reg::X2), 0x43B);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 4, "no abort");

    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    access(&mut m, &LOAD, GIC_DIST_BASE + 0x1000);
    assert_external_abort(&m, GIC_DIST_BASE + 0x1000, false);
}

#[test]
fn the_cpu_interface_decodes_its_first_8_kib_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    // GICC_DIR, the last register in the region.
    access(&mut m, &STORE, GIC_CPU_BASE + 0x1000);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 4, "no abort");

    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    access(&mut m, &STORE, GIC_CPU_BASE + 0x2000);
    assert_external_abort(&m, GIC_CPU_BASE + 0x2000, true);
}
