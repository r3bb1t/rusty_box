//! A guest's access to fw_cfg that QEMU's `virt` rejects with a decode error
//! (`fw_cfg_ctl_mem_valid`, `fw_cfg_dma_mem_valid`) is a synchronous
//! external abort, with the syndrome Arm's `AArch64_FaultSyndrome` gives one
//! taken to EL1 without a second stage (R9: each test asserts what the guest
//! sees): exception class 0x25, `ISV` 0 (`AArch64_InstructionSyndromeValid`
//! holds only for a second-stage fault), `EA` 0 (QEMU's `arm_extabort_type`
//! for a decode error), `S1PTW` 0, `WnR` the access's direction, DFSC
//! 0b010000 (`EncodeLDFSC`, `Fault_SyncExternal`), `FAR_EL1` the address and
//! `ELR_EL1` the instruction, whose destination stays unwritten. An access
//! QEMU accepts completes.
//!
//! Guest code is hand-assembled (GNU as for aarch64); each snippet's source
//! is beside its words.

use rusty_box_aarch64::board::{FW_CFG_BASE, RAM_BASE};
use rusty_box_aarch64::{
    LentStorage, MachineBuilder, MemoryLoan, RamSize, Reg, RunBudget, StopReason, SysReg, VirtConfig,
    VirtMachine,
};

const RAM_LEN: usize = 4 << 20;

const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
const ISV: u64 = 1 << 24;
const EA: u64 = 1 << 9;
const S1PTW: u64 = 1 << 7;
const WNR: u64 = 1 << 6;
const DFSC_SYNC_EXTERNAL: u64 = 0b01_0000;

/// `ldrh w2, [x1, #8]; b .` — a read of the selector, which is write-only.
const LOAD_SELECTOR: [u32; 2] = [0x7940_1022, 0x1400_0000];
/// `strb w2, [x1, #16]; b .` — a byte written to the DMA address register,
/// which takes words and doublewords.
const STORE_BYTE_TO_DMA: [u32; 2] = [0x3900_4022, 0x1400_0000];
/// `ldr x2, [x1, #16]; b .` — the DMA register read whole.
const LOAD_DMA_SIGNATURE: [u32; 2] = [0xF940_0822, 0x1400_0000];

/// The value left in x2 before each snippet, to see whether it is written.
const UNTOUCHED: u64 = 0x5555_5555_5555_5555;

fn machine_running<'r>(ram: &'r mut [u8], code: &[u32]) -> VirtMachine<LentStorage<'r>> {
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds");
    for (i, word) in code.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("code is in RAM");
    }
    m.reg_write(Reg::X1, FW_CFG_BASE);
    m.reg_write(Reg::X2, UNTOUCHED);
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    m
}

/// The Data Abort the instruction at `RAM_BASE` took at `address`.
fn assert_external_abort(m: &VirtMachine<LentStorage<'_>>, address: u64, write: bool) {
    let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1");
    assert_eq!(esr >> 26, EC_DATA_ABORT_SAME_EL, "ESR {esr:#x}: a Data Abort from EL1");
    assert_eq!(esr & ISV, 0, "ESR {esr:#x}: no instruction syndrome without a second stage");
    assert_eq!(esr & EA, 0, "ESR {esr:#x}: a decode error");
    assert_eq!(esr & S1PTW, 0, "ESR {esr:#x}: not on a walk");
    assert_eq!(esr & WNR != 0, write, "ESR {esr:#x}: the access's direction");
    assert_eq!(esr & 0x3F, DFSC_SYNC_EXTERNAL, "ESR {esr:#x}: a synchronous external abort");
    assert_eq!(m.sysreg_read(SysReg::FAR_EL1), Ok(address), "the faulting address");
    assert_eq!(m.sysreg_read(SysReg::ELR_EL1), Ok(RAM_BASE), "the instruction is returned to");
}

#[test]
fn a_read_of_the_write_only_selector_is_a_synchronous_external_abort() {
    let mut ram = vec![0u8; RAM_LEN];
    let m = machine_running(&mut ram, &LOAD_SELECTOR);
    assert_external_abort(&m, FW_CFG_BASE + 8, false);
    assert_eq!(m.reg_read(Reg::X2), UNTOUCHED, "the load wrote nothing");
}

#[test]
fn a_byte_written_to_the_dma_register_is_a_synchronous_external_abort() {
    let mut ram = vec![0u8; RAM_LEN];
    let m = machine_running(&mut ram, &STORE_BYTE_TO_DMA);
    assert_external_abort(&m, FW_CFG_BASE + 16, true);
}

#[test]
fn a_doubleword_read_of_the_dma_register_reads_its_signature() {
    let mut ram = vec![0u8; RAM_LEN];
    let m = machine_running(&mut ram, &LOAD_DMA_SIGNATURE);
    assert_eq!(m.reg_read(Reg::X2), u64::from_le_bytes(*b"QEMU CFG"));
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 4, "the load completed");
}
