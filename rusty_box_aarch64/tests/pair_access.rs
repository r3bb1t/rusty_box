//! Load and store pairs act as their two elements' accesses made one after
//! the other (R9: each test asserts what the guest computed or saw): the
//! second element of a store pair that rewrites the next instruction is
//! run as rewritten, a load pair whose second element runs off the end of
//! RAM takes the second element's external abort and writes neither
//! register, and a load pair within a page reads both elements.
//!
//! Guest code is hand-assembled (GNU as for aarch64); each snippet's source
//! is beside its words.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    Instrumentation, LentStorage, MachineBuilder, MemoryLoan, RamSize, Reg, RunBudget, StopReason, SysReg, VirtConfig,
    VirtMachine,
};

const RAM_LEN: usize = 4 << 20;

fn bare(ram: &mut [u8]) -> VirtMachine<LentStorage<'_>> {
    MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
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
