//! Load and store pairs act as their two elements' accesses made one after
//! the other (R9: each test asserts what the guest computed or saw): the
//! second element of a store pair that rewrites the next instruction is
//! run as rewritten, a load pair whose second element runs off the end of
//! RAM takes the second element's external abort and writes neither
//! register, and a load pair within a page reads both elements. The
//! second element's store is seen on its own: rewriting the next
//! instruction where the first element touches no decoded code, and, for a
//! Q register's high half, clearing the exclusive monitor that only it
//! overlaps. A pair within one page whose second element alone reaches an
//! address nothing answers takes that element's external abort after the
//! first element's access, as Arm ARM `LDP`/`STP` order them: a load writes
//! neither register, a store has made its first element's write, and
//! neither writes the base back.
//!
//! Guest code is hand-assembled (GNU as for aarch64); each snippet's source
//! is beside its words.

use rusty_box_aarch64::board::{FW_CFG_BASE, RAM_BASE};
use rusty_box_aarch64::instrumentation::{Direction, MemoryAccess};
use rusty_box_aarch64::{
    AccessWidth, Instrumentation, LentStorage, MachineBuilder, MemoryLoan, PhysAddr, RamSize, Reg, RunBudget,
    StopReason, SysReg, VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 4 << 20;

fn bare(ram: &mut [u8]) -> VirtMachine<LentStorage<'_>> {
    MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

fn observed<I: Instrumentation>(ram: &mut [u8], observer: I) -> VirtMachine<LentStorage<'_>, I> {
    MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .instrumentation(observer)
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

fn load<S: rusty_box_aarch64::GuestStorage, I: Instrumentation>(machine: &mut VirtMachine<S, I>, at: u64, words: &[u32]) {
    for (i, word) in words.iter().enumerate() {
        machine.mem_write(at + 4 * i as u64, &word.to_le_bytes()).expect("code is in RAM");
    }
}

/// `adr x1, 3f; ldr w2, 3f; ldr w3, 5f; 3: stp w2, w3, [x1]; mov x0, #1;
/// b .; 5: mov x0, #2` — the pair's second word rewrites the instruction
/// after the store, which is in the store's own trace.
const PAIR_REWRITES_NEXT: [u32; 7] = [0x1000_0061, 0x1800_0042, 0x1800_0083, 0x2900_0C22, 0xD280_0020, 0x1400_0000, 0xD280_0040];

#[test]
fn a_store_pair_whose_second_word_rewrites_the_next_instruction_runs_it_rewritten() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, RAM_BASE, &PAIR_REWRITES_NEXT);
    let out = m.step(RunBudget::Instructions(20));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X0), 2, "the rewritten mov ran");
}

/// `mov x0, #2`, the word the store pair below puts over the `mov x0, #1`
/// that follows it.
const MOV_X0_2: u64 = 0xD280_0040;

/// `stp x2, x3, [x1]; mov x0, #1; b .`
const PAIR_X_THEN_MOV: [u32; 3] = [0xA900_0C22, 0xD280_0020, 0x1400_0000];

#[test]
fn a_store_pair_whose_second_doubleword_alone_rewrites_the_next_instruction_runs_it_rewritten() {
    // The pair starts the trace, at the start of a 128-byte line, and its
    // first element is the doubleword just below, in the line before: only
    // the second element, which holds the pair's own word and then
    // `mov x0, #2`, writes where the trace was decoded from.
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    let code = RAM_BASE + 0x80;
    load(&mut m, code, &PAIR_X_THEN_MOV);
    m.reg_write(Reg::Pc, code);
    m.reg_write(Reg::X1, code - 8);
    m.reg_write(Reg::X2, 0x5555);
    m.reg_write(Reg::X3, (MOV_X0_2 << 32) | u64::from(PAIR_X_THEN_MOV[0]));
    let out = m.step(RunBudget::Instructions(20));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X0), 2, "the rewritten mov ran");
}

/// `ldxr x5, [x2]; str q3, [x1]; stxr w6, x5, [x2]; b .`
const Q_STORE_UNDER_MONITOR: [u32; 4] = [0xC85F_7C45, 0x3D80_0023, 0xC806_7C45, 0x1400_0000];

#[test]
fn a_q_register_store_whose_high_half_alone_overlaps_the_monitored_doubleword_clears_the_monitor() {
    // Every store this processor makes that overlaps the monitored bytes
    // clears the local monitor (Arm ARM B2.9.5, the IMPLEMENTATION DEFINED
    // choice), so the store-exclusive after it fails and writes 1.
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, RAM_BASE, &Q_STORE_UNDER_MONITOR);
    m.sysreg_write(SysReg::CPACR_EL1, 3 << 20).expect("CPACR_EL1 is writable");
    let data = RAM_BASE + 0x1000;
    m.reg_write(Reg::X1, data);
    m.reg_write(Reg::X2, data + 8);
    m.reg_write(Reg::X6, 0x5555);
    let out = m.step(RunBudget::Instructions(3));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 12, "no instruction faulted");
    assert_eq!(m.reg_read(Reg::X6), 1, "the store-exclusive failed");
}

/// `ldp x3, x4, [x1]; b .`
const LOAD_PAIR: [u32; 2] = [0xA940_1023, 0x1400_0000];

#[test]
fn a_load_pair_running_off_the_end_of_ram_faults_on_its_second_word() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, RAM_BASE, &LOAD_PAIR);
    let end = RAM_BASE + RAM_LEN as u64;
    m.reg_write(Reg::X1, end - 8);
    m.reg_write(Reg::X3, 0x5555);
    m.reg_write(Reg::X4, 0x5555);
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.sysreg_read(SysReg::FAR_EL1).expect("FAR"), end, "the second word faulted");
    let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR");
    assert_eq!(esr >> 26, 0x25, "a data abort taken from the same EL");
    assert_eq!(esr & 0x3F, 0x10, "a synchronous external abort");
    assert_eq!(m.reg_read(Reg::X3), 0x5555, "the first register is not written");
    assert_eq!(m.reg_read(Reg::X4), 0x5555, "the second register is not written");
}

#[test]
fn a_load_pair_within_a_page_reads_both_words() {
    const FIRST: u64 = 0x0123_4567_89AB_CDEF;
    const SECOND: u64 = 0xFEDC_BA98_7654_3210;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, RAM_BASE, &LOAD_PAIR);
    let data = RAM_BASE + 0x1000;
    m.mem_write(data, &FIRST.to_le_bytes()).expect("data is in RAM");
    m.mem_write(data + 8, &SECOND.to_le_bytes()).expect("data is in RAM");
    m.reg_write(Reg::X1, data);
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X3), FIRST);
    assert_eq!(m.reg_read(Reg::X4), SECOND);
}

// A hole inside a page of the board's address map: fw_cfg decodes 0x18
// bytes (QEMU 11.1.2 hw/arm/virt.c `base_memmap`, `VIRT_FW_CFG`), the
// last of them the low word of its DMA address register, and the rest of
// its page is unassigned. A word pair at FW_CFG_BASE + 0x14, which is on
// Device memory with stage 1 disabled and aligned there, reaches the
// register with its first element and the hole with its second.

/// `ldp w3, w4, [x1, #20]!; b .`
const LOAD_PAIR_INTO_HOLE: [u32; 2] = [0x29C2_9023, 0x1400_0000];
/// `stp w2, w3, [x1, #20]!; b .`
const STORE_PAIR_INTO_HOLE: [u32; 2] = [0x2982_8C22, 0x1400_0000];

const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
const DFSC_SYNC_EXTERNAL: u64 = 0x10;
const WNR: u64 = 1 << 6;

/// Run the pair at `RAM_BASE` with `x1 = FW_CFG_BASE` and check the Data
/// Abort it must take: the second element's synchronous external abort,
/// with the pair's own address to return to and its base not written back.
fn second_element_aborts<S: rusty_box_aarch64::GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>, write: bool) {
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1");
    assert_eq!(esr >> 26, EC_DATA_ABORT_SAME_EL, "ESR {esr:#x}");
    assert_eq!(esr & 0x3F, DFSC_SYNC_EXTERNAL, "ESR {esr:#x}");
    assert_eq!(esr & WNR != 0, write, "ESR {esr:#x}");
    assert_eq!(m.sysreg_read(SysReg::FAR_EL1), Ok(FW_CFG_BASE + 0x18), "the second element's address");
    assert_eq!(m.sysreg_read(SysReg::ELR_EL1), Ok(RAM_BASE), "the pair is where it returns to");
    assert_eq!(m.reg_read(Reg::X1), FW_CFG_BASE, "the base is not written back");
}

#[test]
fn a_load_pair_within_a_page_whose_second_word_alone_reaches_a_hole_loads_neither_register() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, RAM_BASE, &LOAD_PAIR_INTO_HOLE);
    m.reg_write(Reg::X1, FW_CFG_BASE);
    m.reg_write(Reg::X3, 0x5555);
    m.reg_write(Reg::X4, 0x5555);
    second_element_aborts(&mut m, false);
    assert_eq!(m.reg_read(Reg::X3), 0x5555, "the first register is not written");
    assert_eq!(m.reg_read(Reg::X4), 0x5555, "the second register is not written");
}

/// Every data access that reaches the bus, in order.
#[derive(Default)]
struct BusLog(Vec<MemoryAccess>);

impl Instrumentation for BusLog {
    fn memory_access(&mut self, access: MemoryAccess) {
        self.0.push(access);
    }
}

#[test]
fn a_store_pair_within_a_page_whose_second_word_alone_reaches_a_hole_writes_its_first_word_only() {
    // fw_cfg ignores the write, so the bus is where the first element is
    // seen to land.
    const FIRST: u32 = 0x1234_5678;
    const SECOND: u32 = 0x9ABC_DEF0;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = observed(&mut ram, BusLog::default());
    load(&mut m, RAM_BASE, &STORE_PAIR_INTO_HOLE);
    m.reg_write(Reg::X1, FW_CFG_BASE);
    m.reg_write(Reg::X2, u64::from(FIRST));
    m.reg_write(Reg::X3, u64::from(SECOND));
    second_element_aborts(&mut m, true);
    let first_store = MemoryAccess {
        address: PhysAddr::new(FW_CFG_BASE + 0x14),
        width: AccessWidth::Word,
        direction: Direction::Write,
        value: u64::from(FIRST),
    };
    assert_eq!(m.instrumentation().0, [first_store], "the first word is written, the second is not");
}
