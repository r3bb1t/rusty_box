//! Alignment faults as the guest sees them (R9: each test asserts the
//! syndrome, the fault address and what memory holds afterwards).
//!
//! Arm ARM `Mem[]` checks alignment against the size of the whole access
//! (`AArch64.CheckAlignment(address, size, ...)`), so a 16-byte SIMD&FP
//! access to Device memory must be 16-byte aligned, and with stage 1
//! disabled every data access is to Device memory. In the stage 1 walk the
//! alignment fault a Device page raises comes before its permission fault
//! (`AArch64.S1Translate`: `AArch64.S1HasAlignmentFault` is tested before
//! `AArch64.S1HasPermissionsFault`). QEMU 11.1.2 does the same
//! (`target/arm/ptw.c`, the Device check ahead of the permission check,
//! citing R_XCHFJ).
//!
//! Guest code is hand-assembled (GNU as for aarch64); each snippet's source
//! is beside its words.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    GuestStorage, Instrumentation, LentStorage, MachineBuilder, MemoryLoan, RamSize, Reg, RunBudget, StopReason,
    SysReg, VReg, VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 4 << 20;

/// `ldr q0, [x1]; b .`
const LOAD_Q: [u32; 2] = [0x3DC0_0020, 0x1400_0000];
/// `str q0, [x1]; b .`
const STORE_Q: [u32; 2] = [0x3D80_0020, 0x1400_0000];
/// `ldp q0, q1, [x1]; b .`
const LOAD_PAIR_Q: [u32; 2] = [0xAD40_0420, 0x1400_0000];
/// `str w2, [x1]; b .`
const STORE_W: [u32; 2] = [0xB900_0022, 0x1400_0000];
/// `str x2, [x1]; b .`
const STORE_X: [u32; 2] = [0xF900_0022, 0x1400_0000];

/// ESR_EL1.EC of a Data Abort taken without a change of EL.
const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
/// DFSC values (Arm ARM `ESR_ELx.ISS` for Data Aborts).
const DFSC_ALIGNMENT: u64 = 0x21;
const DFSC_TRANSLATION_L3: u64 = 0x07;
const DFSC_PERMISSION_L3: u64 = 0x0F;
/// ESR_EL1.ISS.WnR.
const WNR: u64 = 1 << 6;

const Q_VALUE: u128 = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF;
const FILL: [u8; 16] = [0x5A; 16];

fn bare(ram: &mut [u8]) -> VirtMachine<LentStorage<'_>> {
    MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

fn load<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>, words: &[u32]) {
    for (i, word) in words.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes()).expect("code is in RAM");
    }
    m.sysreg_write(SysReg::CPACR_EL1, 3 << 20).expect("CPACR_EL1 is writable");
}

/// Run the one instruction at `RAM_BASE` and return what it reported:
/// `(ESR_EL1, FAR_EL1)` after the Data Abort it must take.
fn fault<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>) -> (u64, u64) {
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1");
    let far = m.sysreg_read(SysReg::FAR_EL1).expect("FAR_EL1");
    assert_eq!(esr >> 26, EC_DATA_ABORT_SAME_EL, "a data abort, ESR {esr:#x}");
    (esr, far)
}

fn bytes_at<S: GuestStorage, I: Instrumentation>(m: &VirtMachine<S, I>, addr: u64) -> [u8; 16] {
    let mut buf = [0u8; 16];
    m.mem_read(addr, &mut buf).expect("in RAM");
    buf
}

/// Page tables for VA 0x8000_0000 onwards, with RAM mapped identically at
/// VA 0x4000_0000 for the code:
///
/// | VA | maps | memory | EL1 access |
/// |---|---|---|---|
/// | 0x8000_0000 | RAM + 0x3000 | Normal | read/write |
/// | 0x8000_1000 | RAM + 0x5000 | Normal | read/write |
/// | 0x8000_2000 | RAM + 0x4000 | Device-nGnRnE | read-only |
/// | 0x8000_3000 | RAM + 0x6000 | Normal | read/write |
/// | 0x8000_4000 | nothing | | |
fn map_test_pages<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>) {
    const AF: u64 = 1 << 10;
    const INNER_SHAREABLE: u64 = 3 << 8;
    const NORMAL: u64 = 0 << 2;
    const DEVICE: u64 = 1 << 2;
    const READ_ONLY: u64 = 0b10 << 6;
    let l1 = RAM_BASE + 0x1_0000;
    let l2 = RAM_BASE + 0x1_1000;
    let l3 = RAM_BASE + 0x1_2000;
    let page = |pa: u64, attrs: u64| pa | AF | INNER_SHAREABLE | attrs | 0b11;
    for (table, index, desc) in [
        (l1, 1, RAM_BASE | AF | INNER_SHAREABLE | NORMAL | 0b01),
        (l1, 2, l2 | 0b11),
        (l2, 0, l3 | 0b11),
        (l3, 0, page(RAM_BASE + 0x3000, NORMAL)),
        (l3, 1, page(RAM_BASE + 0x5000, NORMAL)),
        (l3, 2, page(RAM_BASE + 0x4000, DEVICE | READ_ONLY)),
        (l3, 3, page(RAM_BASE + 0x6000, NORMAL)),
    ] {
        m.mem_write(table + 8 * index, &desc.to_le_bytes()).expect("tables are in RAM");
    }
    // Attribute 0 Normal write-back, attribute 1 Device-nGnRnE.
    m.sysreg_write(SysReg::MAIR_EL1, 0x00FF).expect("MAIR");
    m.sysreg_write(SysReg::TCR_EL1, 25 | (1 << 23)).expect("TCR");
    m.sysreg_write(SysReg::TTBR0_EL1, l1).expect("TTBR0");
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR");
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr | 1).expect("SCTLR");
}

fn v(n: u8) -> VReg {
    VReg::new(n).expect("a vector register")
}

#[test]
fn a_q_register_load_from_device_memory_needs_16_byte_alignment() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_Q);
    // With the MMU off, data accesses are to Device memory.
    let at = RAM_BASE + 0x1008;
    m.reg_write(Reg::X1, at);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(esr & WNR, 0);
    assert_eq!(far, at);
    assert_eq!(m.vreg_read(v(0)), 0, "Q0 is not loaded");
}

#[test]
fn a_q_register_load_from_device_memory_at_a_16_byte_boundary_loads() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_Q);
    let at = RAM_BASE + 0x1010;
    m.mem_write(at, &Q_VALUE.to_le_bytes()).expect("in RAM");
    m.reg_write(Reg::X1, at);
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.vreg_read(v(0)), Q_VALUE);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 4);
}

#[test]
fn a_q_register_store_to_device_memory_needs_16_byte_alignment() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_Q);
    let at = RAM_BASE + 0x1008;
    m.mem_write(at, &FILL).expect("in RAM");
    m.vreg_write(v(0), Q_VALUE);
    m.reg_write(Reg::X1, at);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(esr & WNR, WNR);
    assert_eq!(far, at);
    assert_eq!(bytes_at(&m, at), FILL, "nothing is written");
}

#[test]
fn a_q_register_pair_load_from_device_memory_needs_16_byte_alignment() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_Q);
    let at = RAM_BASE + 0x1008;
    m.reg_write(Reg::X1, at);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(far, at);
}

#[test]
fn a_misaligned_store_to_a_read_only_device_page_is_an_alignment_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_W);
    map_test_pages(&mut m);
    m.reg_write(Reg::X1, 0x8000_2001);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "alignment before permission, ESR {esr:#x}");
    assert_eq!(far, 0x8000_2001);
}

#[test]
fn an_aligned_store_to_a_read_only_device_page_is_a_permission_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_W);
    map_test_pages(&mut m);
    m.reg_write(Reg::X1, 0x8000_2000);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_PERMISSION_L3, "ESR {esr:#x}");
    assert_eq!(far, 0x8000_2000);
}

#[test]
fn a_store_running_from_normal_memory_into_a_device_page_faults_where_the_device_page_starts() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_X);
    map_test_pages(&mut m);
    m.mem_write(RAM_BASE + 0x5FF0, &FILL).expect("in RAM");
    m.reg_write(Reg::X1, 0x8000_1FFC);
    m.reg_write(Reg::X2, u64::MAX);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(far, 0x8000_2000, "the first byte that reaches the Device page");
    assert_eq!(bytes_at(&m, RAM_BASE + 0x5FF0), FILL, "nothing is written");
}

#[test]
fn a_q_register_store_running_into_an_unmapped_page_writes_nothing() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_Q);
    map_test_pages(&mut m);
    m.mem_write(RAM_BASE + 0x6FF0, &FILL).expect("in RAM");
    m.vreg_write(v(0), Q_VALUE);
    m.reg_write(Reg::X1, 0x8000_3FF8);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_TRANSLATION_L3, "ESR {esr:#x}");
    assert_eq!(esr & WNR, WNR);
    assert_eq!(far, 0x8000_4000);
    assert_eq!(bytes_at(&m, RAM_BASE + 0x6FF0), FILL, "the first page is untouched");
}
