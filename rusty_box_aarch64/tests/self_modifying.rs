//! Code that changes after it ran runs as it is now (R9: each test asserts
//! what the guest computed). The processor keeps the instructions it decoded
//! (its trace cache), so each way code can change is driven here, after the
//! code has run at least once: the guest's own store, ahead in the very
//! trace that makes it and through a second mapping of the page; a device
//! writing the code into RAM by DMA; the cache maintenance a guest issues
//! before running new code; and a flash bank leaving read-array mode under
//! code that runs from it.
//!
//! Guest code is hand-assembled (GNU as for aarch64); each snippet's source
//! is beside its words.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::cpu::ExceptionClass;
use rusty_box_aarch64::instrumentation::{ExceptionKind, ExceptionTaken};
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

/// `adr x1, target; ldr w2, newinsn; str w2, [x1];
/// target: mov x0, #1; b .; newinsn: mov x0, #2` — the store rewrites the
/// next instruction but one, which is in the same trace as the store.
const REWRITE_AHEAD: [u32; 6] = [0x1000_0061, 0x1800_0082, 0xB900_0022, 0xD280_0020, 0x1400_0000, 0xD280_0040];

#[test]
fn an_instruction_rewritten_just_ahead_runs_as_rewritten() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, RAM_BASE, &REWRITE_AHEAD);
    let out = m.step(RunBudget::Instructions(50));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X0), 2, "the rewritten mov ran");
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 0x10);
}

/// `bl func; mov x19, x0; ldr w2, patch; str w2, [x3]; bl func; b .;
/// func: mov x0, #1; ret; patch: mov x0, #7` — `func` runs, is rewritten
/// through the address in `x3`, and runs again.
const REWRITE_FUNCTION: [u32; 9] = [
    0x9400_0006, 0xAA00_03F3, 0x1800_00C2, 0xB900_0062, 0x9400_0002, 0x1400_0000, 0xD280_0020, 0xD65F_03C0, 0xD280_00E0,
];
const FUNC_OFFSET: u64 = 0x18;

#[test]
fn a_function_rewritten_after_it_ran_runs_its_new_body() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, RAM_BASE, &REWRITE_FUNCTION);
    m.reg_write(Reg::X3, RAM_BASE + FUNC_OFFSET);
    let out = m.step(RunBudget::Instructions(50));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X19), 1, "the first call ran the old body");
    assert_eq!(m.reg_read(Reg::X0), 7, "the second call ran the new one");
}

/// Page tables: VA 0x4000_0000 maps 1 GiB of RAM identically, and the page
/// at VA 0x8000_0000 is a second mapping of the RAM's first page.
fn map_code_twice<S: rusty_box_aarch64::GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>) {
    const AF: u64 = 1 << 10;
    const INNER_SHAREABLE: u64 = 3 << 8;
    let l1 = RAM_BASE + 0x1_0000;
    let l2 = RAM_BASE + 0x1_1000;
    let l3 = RAM_BASE + 0x1_2000;
    for (table, index, desc) in [
        (l1, 1, RAM_BASE | AF | INNER_SHAREABLE | 0b01),
        (l1, 2, l2 | 0b11),
        (l2, 0, l3 | 0b11),
        (l3, 0, RAM_BASE | AF | INNER_SHAREABLE | 0b11),
    ] {
        m.mem_write(table + 8 * index, &desc.to_le_bytes()).expect("tables are in RAM");
    }
    m.sysreg_write(SysReg::MAIR_EL1, 0xFF).expect("MAIR");
    m.sysreg_write(SysReg::TCR_EL1, 25 | (1 << 23)).expect("TCR");
    m.sysreg_write(SysReg::TTBR0_EL1, l1).expect("TTBR0");
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR");
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr | 1).expect("SCTLR");
}

#[test]
fn a_store_through_a_second_mapping_of_the_code_is_seen() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, RAM_BASE, &REWRITE_FUNCTION);
    map_code_twice(&mut m);
    // `func` is written at its alias; the code runs at the identity map.
    m.reg_write(Reg::X3, 0x8000_0000 + FUNC_OFFSET);
    let out = m.step(RunBudget::Instructions(50));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X19), 1);
    assert_eq!(m.reg_read(Reg::X0), 7, "the write through the alias changed the code");
}

/// At VA 0x8000_0000: `str x2, [x1]; ldr x4, [x3]; mov x0, #1; b .` — the
/// store points the code page's own descriptor at another page, without a
/// TLBI, and the load, through an address in the same TLB slot, evicts the
/// code page's entry. The next fetch walks the tables again and finds the
/// other page, which holds `mov x0, #2; b .` at the same offsets.
const REMAP_SELF: [u32; 4] = [0xF900_0022, 0xF940_0064, 0xD280_0020, 0x1400_0000];
const REMAPPED: [u32; 2] = [0xD280_0040, 0x1400_0000];

#[test]
fn code_whose_mapping_changed_runs_from_the_new_page_once_its_tlb_entry_is_gone() {
    const AF: u64 = 1 << 10;
    const INNER_SHAREABLE: u64 = 3 << 8;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, RAM_BASE, &REMAP_SELF);
    load(&mut m, RAM_BASE + 0x2008, &REMAPPED);
    map_code_twice(&mut m);
    // VA 0x8040_0000, in the same slot of the direct-mapped TLB as VA
    // 0x8000_0000, maps 2 MiB of RAM.
    let l2 = RAM_BASE + 0x1_1000;
    m.mem_write(l2 + 8 * 2, &(RAM_BASE | AF | INNER_SHAREABLE | 0b01).to_le_bytes()).expect("tables are in RAM");
    m.reg_write(Reg::X1, RAM_BASE + 0x1_2000);
    m.reg_write(Reg::X2, (RAM_BASE + 0x2000) | AF | INNER_SHAREABLE | 0b11);
    m.reg_write(Reg::X3, 0x8040_0000);
    m.reg_write(Reg::Pc, 0x8000_0000);
    let out = m.step(RunBudget::Instructions(10));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X0), 2, "the instruction after the load came from the new page");
}

/// `bl func; mov x19, x0; ldr w2, patch; str w2, [x3]; dc cvau, x1;
/// dsb ish; ic ivau, x1; dsb ish; isb; bl func; mov x20, x0;
/// ldr w2, patch2; str w2, [x3]; ic iallu; dsb ish; isb; bl func; b .;
/// func: mov x0, #1; ret; patch: mov x0, #7; patch2: mov x0, #9` — new
/// code made visible the way the Arm ARM asks (B2.4.4), by line and then
/// whole.
const MAINTAINED: [u32; 22] = [
    0x9400_0012, 0xAA00_03F3, 0x1800_0242, 0xB900_0062, 0xD50B_7B21, 0xD503_3B9F, 0xD50B_7521, 0xD503_3B9F,
    0xD503_3FDF, 0x9400_0009, 0xAA00_03F4, 0x1800_0142, 0xB900_0062, 0xD508_751F, 0xD503_3B9F, 0xD503_3FDF,
    0x9400_0002, 0x1400_0000, 0xD280_0020, 0xD65F_03C0, 0xD280_00E0, 0xD280_0120,
];
const MAINTAINED_FUNC: u64 = 0x48;

/// With the MMU off; and with the MMU on and the line's address in the
/// TLB, or not, through the identity mapping or the alias. (`IC IVAU`
/// translates the address it names, as Arm `AArch64_IC` does, and takes a
/// Data Abort where it has no translation: `memory_system`'s tests cover
/// that.)
#[test]
fn code_made_visible_by_cache_maintenance_runs() {
    for (mmu, store_at, ivau_at) in [
        (false, RAM_BASE + MAINTAINED_FUNC, RAM_BASE + MAINTAINED_FUNC),
        (true, RAM_BASE + MAINTAINED_FUNC, RAM_BASE + MAINTAINED_FUNC),
        (true, 0x8000_0000 + MAINTAINED_FUNC, 0x8000_0000 + MAINTAINED_FUNC),
        (true, RAM_BASE + MAINTAINED_FUNC, 0x8000_0000 + MAINTAINED_FUNC),
    ] {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = bare(&mut ram);
        load(&mut m, RAM_BASE, &MAINTAINED);
        if mmu {
            map_code_twice(&mut m);
        }
        m.reg_write(Reg::X3, store_at);
        m.reg_write(Reg::X1, ivau_at);
        let out = m.step(RunBudget::Instructions(60));
        assert_eq!(out.stop, StopReason::BudgetExhausted, "mmu {mmu}, ivau at {ivau_at:#x}");
        let got = [m.reg_read(Reg::X19), m.reg_read(Reg::X20), m.reg_read(Reg::X0)];
        assert_eq!(got, [1, 7, 9], "mmu {mmu}, store at {store_at:#x}, ivau at {ivau_at:#x}");
    }
}

/// `bl code; mov x19, x0; loop: ldp w4, w5, [x1], #8; cmn w4, #1;
/// b.eq done; str w5, [x3, x4]; b loop; done: bl code; b .`, with `code`
/// (`mov x0, #1; ret`) at +0x200: run `code`, then write the virtio-mmio
/// registers a table at `x1` lists into the transport at `x3`, which sets
/// up a virtio-blk queue and asks for sector 0 to be read over `code`, and
/// run `code` again.
#[cfg(feature = "alloc")]
const DISK_LOAD: [u32; 9] = [
    0x9400_0080, 0xAA00_03F3, 0x28C1_1424, 0x3100_049F, 0x5400_0060, 0xB824_6865, 0x17FF_FFFC, 0x9400_0079, 0x1400_0000,
];
#[cfg(feature = "alloc")]
const DISK_CODE: u64 = RAM_BASE + 0x200;
/// `mov x0, #1; ret`
#[cfg(feature = "alloc")]
const OLD_CODE: [u32; 2] = [0xD280_0020, 0xD65F_03C0];
/// `mov x0, #7; ret`
#[cfg(feature = "alloc")]
const NEW_CODE: [u32; 2] = [0xD280_00E0, 0xD65F_03C0];

/// A disk needs an allocator.
#[cfg(feature = "alloc")]
#[test]
fn code_a_disk_read_puts_over_code_that_ran_runs_as_read() {
    use rusty_box_aarch64::board::VIRTIO_MMIO_BASE;
    use rusty_box_aarch64::Disk;

    const DESC: u64 = RAM_BASE + 0x1_0000;
    const AVAIL: u64 = RAM_BASE + 0x1_1000;
    const USED: u64 = RAM_BASE + 0x1_2000;
    const HEADER: u64 = RAM_BASE + 0x1_3000;
    const STATUS_BYTE: u64 = RAM_BASE + 0x1_3100;
    const TABLE: u64 = RAM_BASE + 0x1_4000;
    // The only device takes the highest transport.
    const TRANSPORT: u64 = VIRTIO_MMIO_BASE + 31 * 0x200;
    const NEXT: u16 = 1;
    const WRITE: u16 = 2;

    let mut sector = vec![0u8; 512];
    for (i, word) in NEW_CODE.iter().enumerate() {
        sector[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    let mut image = sector;
    image.resize(64 * 1024, 0);
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .virtio_disk(Disk::memory(image))
        .build()
        .expect("a bare machine with a disk builds");
    load(&mut m, RAM_BASE, &DISK_LOAD);
    load(&mut m, DISK_CODE, &OLD_CODE);

    // A read of sector 0: the request header, the code's 512 bytes, and the
    // status byte, as one chain made available in a queue of 8.
    let descriptor = |addr: u64, len: u32, flags: u16, next: u16| {
        let mut d = [0u8; 16];
        d[..8].copy_from_slice(&addr.to_le_bytes());
        d[8..12].copy_from_slice(&len.to_le_bytes());
        d[12..14].copy_from_slice(&flags.to_le_bytes());
        d[14..].copy_from_slice(&next.to_le_bytes());
        d
    };
    m.mem_write(DESC, &descriptor(HEADER, 16, NEXT, 1)).expect("RAM");
    m.mem_write(DESC + 16, &descriptor(DISK_CODE, 512, WRITE | NEXT, 2)).expect("RAM");
    m.mem_write(DESC + 32, &descriptor(STATUS_BYTE, 1, WRITE, 0)).expect("RAM");
    m.mem_write(HEADER, &[0u8; 16]).expect("RAM"); // VIRTIO_BLK_T_IN, sector 0
    m.mem_write(STATUS_BYTE, &[0xFF]).expect("RAM");
    m.mem_write(AVAIL, &[0, 0, 1, 0, 0, 0]).expect("RAM"); // flags 0, idx 1, ring[0] = 0

    // Linux's virtio_dev_probe and vm_setup_vq, then the notification.
    let registers: [(u32, u32); 20] = [
        (0x070, 0),
        (0x070, 1),
        (0x070, 3),
        (0x024, 0),
        (0x020, 0),
        (0x024, 1),
        (0x020, 1), // VIRTIO_F_VERSION_1
        (0x070, 11),
        (0x030, 0),
        (0x038, 8),
        (0x080, DESC as u32),
        (0x084, 0),
        (0x090, AVAIL as u32),
        (0x094, 0),
        (0x0A0, USED as u32),
        (0x0A4, 0),
        (0x044, 1),
        (0x070, 15),
        (0x050, 0),
        (u32::MAX, 0),
    ];
    let mut table = Vec::new();
    for (offset, value) in registers {
        table.extend_from_slice(&offset.to_le_bytes());
        table.extend_from_slice(&value.to_le_bytes());
    }
    m.mem_write(TABLE, &table).expect("RAM");
    m.reg_write(Reg::X1, TABLE);
    m.reg_write(Reg::X3, TRANSPORT);

    let out = m.step(RunBudget::Instructions(500));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    let mut status = [0u8];
    m.mem_read(STATUS_BYTE, &mut status).expect("RAM");
    assert_eq!(status, [0], "the read completed (VIRTIO_BLK_S_OK)");
    assert_eq!(m.reg_read(Reg::X19), 1, "the old code ran first");
    assert_eq!(m.reg_read(Reg::X0), 7, "the code the disk read in ran after");
}

/// Flash bank 0's reset code: `ldr x0, =0x40001000; br x0`.
const TO_RAM: [u32; 4] = [0x5800_0040, 0xD61F_0000, 0x4000_1000, 0];
/// In RAM at 0x4000_1000: `ldr x5, =0x04000000; blr x5; mov x19, x0;
/// ldr x6, =0x04010000; mov w1, #0x20; str w1, [x6]; mov w1, #0xd0;
/// str w1, [x6]; mov w1, #0xff; str w1, [x6]; blr x5; b .` — call the
/// variable store's code, erase the 256 KiB block it is in with a command
/// written to another page of the block, back to read array, call it
/// again.
const ERASE_AND_CALL: [u32; 16] = [
    0x5800_0185, 0xD63F_00A0, 0xAA00_03F3, 0x5800_0166, 0x5280_0401, 0xB900_00C1, 0x5280_1A01, 0xB900_00C1,
    0x5280_1FE1, 0xB900_00C1, 0xD63F_00A0, 0x1400_0000, 0x0400_0000, 0, 0x0401_0000, 0,
];
/// The synchronous exception handler: `mrs x21, esr_el1; mrs x22, elr_el1;
/// b .`.
const RECORD: [u32; 3] = [0xD538_5215, 0xD538_4036, 0x1400_0000];

/// A block erase changes every word of the block, while the command that
/// asks for it touches one word of another page: the code that ran there
/// is fetched as erased flash afterwards, which is no instruction.
#[test]
fn code_in_flash_erased_by_a_command_elsewhere_in_its_block_is_fetched_erased() {
    let mut flash0 = vec![0u8; 16];
    for (i, word) in TO_RAM.iter().enumerate() {
        flash0[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    let mut vars = vec![0u8; 0x200];
    for (i, word) in FLASH_FUNC.iter().enumerate() {
        vars[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    let mut ram = vec![0u8; RAM_LEN];
    let mut vars_bank = vec![0xFFu8; 1 << 20];
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .firmware(&flash0)
        .variables(&vars)
        .instrumentation(Exceptions::default())
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut vars_bank })
        .expect("a firmware machine builds");
    load(&mut m, RAM_BASE + 0x1000, &ERASE_AND_CALL);
    let vectors = RAM_BASE + 0x10_0000;
    load(&mut m, vectors + 0x200, &RECORD);
    m.sysreg_write(SysReg::VBAR_EL1, vectors).expect("VBAR");

    let out = m.step(RunBudget::Instructions(40));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X19), 1, "the code in flash ran before the erase");
    let taken = &m.instrumentation().0;
    assert_eq!(taken.len(), 1, "{taken:?}");
    assert_eq!(taken[0].kind, ExceptionKind::Synchronous(ExceptionClass::Unknown));
    assert_eq!(m.reg_read(Reg::X22), 0x0400_0000, "the erased word faulted where the code was");
}

/// The exceptions an observer saw.
#[derive(Default)]
struct Exceptions(Vec<ExceptionTaken>);

impl Instrumentation for Exceptions {
    fn exception(&mut self, taken: ExceptionTaken) {
        self.0.push(taken);
    }
}

/// Flash bank 0: `bl func; mov x19, x0; mov x0, #0; mov w1, #0x70;
/// mov x2, #0x100; str w1, [x2]; bl func; mov x20, x0; b .`, with `func`
/// (`mov x0, #1; ret`) at 0x100 — code that runs from the flash in
/// read-array mode, and whose store of the CFI Read Status command leaves
/// that mode under it.
const FLASH_CODE: [u32; 9] = [
    0x9400_0040, 0xAA00_03F3, 0xD280_0000, 0x5280_0E01, 0xD280_2002, 0xB900_0041, 0x9400_003A, 0xAA00_03F4, 0x1400_0000,
];
const FLASH_FUNC: [u32; 2] = [0xD280_0020, 0xD65F_03C0];
/// The synchronous exception handler, in RAM: `mov w1, #0xff;
/// mov x2, #0x100; str w1, [x2]; mrs x21, esr_el1; eret` — back to
/// read-array mode, and return to the instruction that faulted.
const RESTORE_ARRAY: [u32; 5] = [0x5280_1FE1, 0xD280_2002, 0xB900_0041, 0xD538_5215, 0xD69F_03E0];

#[test]
fn code_in_flash_that_left_read_array_mode_is_fetched_as_flash_answers() {
    let mut flash = vec![0u8; 0x200];
    for (i, word) in FLASH_CODE.iter().enumerate() {
        flash[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
    }
    for (i, word) in FLASH_FUNC.iter().enumerate() {
        flash[0x100 + 4 * i..0x104 + 4 * i].copy_from_slice(&word.to_le_bytes());
    }
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .firmware(&flash)
        .instrumentation(Exceptions::default())
        .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })
        .expect("a firmware machine builds");
    let vectors = RAM_BASE + 0x10_0000;
    load(&mut m, vectors + 0x200, &RESTORE_ARRAY);
    m.sysreg_write(SysReg::VBAR_EL1, vectors).expect("VBAR");

    let out = m.step(RunBudget::Instructions(40));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X19), 1, "func ran from the flash's array");
    // In Read Status mode a fetch reads the status register — 0x80, ready,
    // in each half of the word — which is no instruction: the instruction
    // right after the store, which ran from the same trace's flash, is
    // fetched as that.
    let taken = &m.instrumentation().0;
    assert_eq!(taken.len(), 1, "{taken:?}");
    assert_eq!(taken[0].kind, ExceptionKind::Synchronous(ExceptionClass::Unknown));
    assert_eq!(taken[0].return_address, 0x18, "the fetch after the store faulted");
    assert_eq!(m.reg_read(Reg::X21) >> 26, 0, "ESR_EL1.EC: unknown reason");
    assert_eq!(m.reg_read(Reg::X20), 1, "back in read-array mode, the code ran on and called func");
}
