//! Alignment faults as the guest sees them (R9: each test asserts the
//! syndrome, the fault address and what memory holds afterwards).
//!
//! Arm's `Mem` judges a single register's access aligned by the size of the
//! whole access (`aligned = IsAlignedSize(address, bytes)`), so a 16-byte
//! SIMD&FP access to Device memory must be 16-byte aligned, and with stage
//! 1 disabled every data access is to Device memory. In the stage 1 walk the
//! alignment fault a Device page raises comes before its permission fault
//! (`AArch64.S1Translate`: `AArch64.S1HasAlignmentFault` is tested before
//! `AArch64.S1HasPermissionsFault`). QEMU 11.1.2 does the same
//! (`target/arm/ptw.c`, the Device check ahead of the permission check,
//! citing R_XCHFJ).
//!
//! A pair is judged by its elements: `LDP`, `LDPSW` and `STP`,
//! general-purpose and SIMD&FP alike, make one `Mem{2 * datasize}` access
//! with `ispair`, which makes `aligned` the element size's, and make it as
//! the two elements' accesses. (The single access FEAT_LSE2 makes of a pair
//! in one 16-byte granule is not this processor's: `ID_AA64MMFR2_EL1.AT`
//! is 0.) A pair of words at 4 mod 8 is aligned, on Device memory and with
//! SCTLR_EL1.A set; a pair of doublewords there is not. A pair of Q
//! registers at an address 8-byte aligned is four doubleword accesses,
//! each passed `aligned = TRUE`, so Device memory takes it at 8 mod 16,
//! where one Q register faults; at 4 mod 16 it faults.
//!
//! Guest code is hand-assembled (GNU as for aarch64); each snippet's source
//! is beside its words.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    GuestStorage, Instrumentation, LentStorage, MachineBuilder, MemoryLoan, RamSize, Reg,
    RunBudget, StopReason, SysReg, VReg, VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 4 << 20;

/// `ldr q0, [x1]; b .`
const LOAD_Q: [u32; 2] = [0x3DC0_0020, 0x1400_0000];
/// `str q0, [x1]; b .`
const STORE_Q: [u32; 2] = [0x3D80_0020, 0x1400_0000];
/// `ldp q0, q1, [x1]; b .`
const LOAD_PAIR_Q: [u32; 2] = [0xAD40_0420, 0x1400_0000];
/// `ldp w3, w4, [x1]; b .`
const LOAD_PAIR_W: [u32; 2] = [0x2940_1023, 0x1400_0000];
/// `stp w2, w3, [x1]; b .`
const STORE_PAIR_W: [u32; 2] = [0x2900_0C22, 0x1400_0000];
/// `ldpsw x3, x4, [x1]; b .`
const LOAD_PAIR_SW: [u32; 2] = [0x6940_1023, 0x1400_0000];
/// `ldp x3, x4, [x1]; b .`
const LOAD_PAIR_X: [u32; 2] = [0xA940_1023, 0x1400_0000];
/// `ldp s0, s1, [x1]; b .`
const LOAD_PAIR_S: [u32; 2] = [0x2D40_0420, 0x1400_0000];
/// `stp s0, s1, [x1]; b .`
const STORE_PAIR_S: [u32; 2] = [0x2D00_0420, 0x1400_0000];
/// `ldp d0, d1, [x1]; b .`
const LOAD_PAIR_D: [u32; 2] = [0x6D40_0420, 0x1400_0000];
/// `stp d0, d1, [x1]; b .`
const STORE_PAIR_D: [u32; 2] = [0x6D00_0420, 0x1400_0000];
/// `str w2, [x1]; b .`
const STORE_W: [u32; 2] = [0xB900_0022, 0x1400_0000];
/// `str x2, [x1]; b .`
const STORE_X: [u32; 2] = [0xF900_0022, 0x1400_0000];

// Two accesses to one page: an aligned load, which fills the page's TLB
// entry, then the access under test, which hits that entry.

/// `ldr w3, [x1]; ldur w3, [x1, #1]; b .`
const WARM_THEN_MISALIGNED_LOAD: [u32; 3] = [0xB940_0023, 0xB840_1023, 0x1400_0000];
/// `ldr w3, [x1]; stur w2, [x1, #1]; b .`
const WARM_THEN_MISALIGNED_STORE: [u32; 3] = [0xB940_0023, 0xB800_1022, 0x1400_0000];
/// `ldr w3, [x1]; str w2, [x1]; b .`
const WARM_THEN_STORE: [u32; 3] = [0xB940_0023, 0xB900_0022, 0x1400_0000];
/// `ldr w3, [x2]; ldar w3, [x1]; b .`
const WARM_THEN_LOAD_ACQUIRE: [u32; 3] = [0xB940_0043, 0x88DF_FC23, 0x1400_0000];
/// `ldr w3, [x1]; ldtr w3, [x1]; b .`
const WARM_THEN_UNPRIVILEGED_LOAD: [u32; 3] = [0xB940_0023, 0xB840_0823, 0x1400_0000];
/// `ldr w3, [x1]; sttr w2, [x1]; b .`
const WARM_THEN_UNPRIVILEGED_STORE: [u32; 3] = [0xB940_0023, 0xB800_0822, 0x1400_0000];

/// ESR_EL1.EC of a Data Abort taken without a change of EL.
const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
/// DFSC values (Arm ARM `ESR_ELx.ISS` for Data Aborts).
const DFSC_ALIGNMENT: u64 = 0x21;
const DFSC_TRANSLATION_L3: u64 = 0x07;
const DFSC_PERMISSION_L3: u64 = 0x0F;
/// ESR_EL1.ISS.WnR.
const WNR: u64 = 1 << 6;
/// SCTLR_EL1.A: every misaligned data access faults.
const SCTLR_A: u64 = 1 << 1;

const Q_VALUE: u128 = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF;
const FILL: [u8; 16] = [0x5A; 16];

/// A pair's elements: words (the first negative, for `LDPSW`) and
/// doublewords.
const FIRST_WORD: u32 = 0x8899_AABB;
const SECOND_WORD: u32 = 0x1122_3344;
const FIRST_DWORD: u64 = 0x8899_AABB_CCDD_EEFF;
const SECOND_DWORD: u64 = 0x0011_2233_4455_6677;
/// What a register holds before a load that must not write it.
const UNTOUCHED: u64 = 0x5555;

fn bare(ram: &mut [u8]) -> VirtMachine<LentStorage<'_>> {
    MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

fn load<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>, words: &[u32]) {
    for (i, word) in words.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("code is in RAM");
    }
    m.sysreg_write(SysReg::CPACR_EL1, 3 << 20)
        .expect("CPACR_EL1 is writable");
}

/// Run the one instruction at `RAM_BASE` and return what it reported:
/// `(ESR_EL1, FAR_EL1)` after the Data Abort it must take.
fn fault<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>) -> (u64, u64) {
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1");
    let far = m.sysreg_read(SysReg::FAR_EL1).expect("FAR_EL1");
    assert_eq!(
        esr >> 26,
        EC_DATA_ABORT_SAME_EL,
        "a data abort, ESR {esr:#x}"
    );
    (esr, far)
}

/// Run the warming load at `RAM_BASE`, which must complete, then the access
/// after it, and return what that one reported, as [`fault`] does. The
/// tests that use it access VA 0x8000_1000 or 0x8000_2000, never
/// 0x8000_0000, whose entry in this core's direct-mapped TLB is the code
/// page's: the fetch between the two accesses would evict it.
fn fault_on_a_warm_tlb<S: GuestStorage, I: Instrumentation>(
    m: &mut VirtMachine<S, I>,
) -> (u64, u64) {
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(
        m.reg_read(Reg::Pc),
        RAM_BASE + 4,
        "the warming load completes"
    );
    fault(m)
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
        m.mem_write(table + 8 * index, &desc.to_le_bytes())
            .expect("tables are in RAM");
    }
    // Attribute 0 Normal write-back, attribute 1 Device-nGnRnE.
    m.sysreg_write(SysReg::MAIR_EL1, 0x00FF).expect("MAIR");
    m.sysreg_write(SysReg::TCR_EL1, 25 | (1 << 23))
        .expect("TCR");
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
fn a_q_register_pair_load_from_device_memory_at_a_doubleword_boundary_loads() {
    const SECOND_Q: u128 = 0xFFEE_DDCC_BBAA_9988_7766_5544_3322_1100;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_Q);
    let at = RAM_BASE + 0x1008;
    m.mem_write(at, &Q_VALUE.to_le_bytes()).expect("in RAM");
    m.mem_write(at + 16, &SECOND_Q.to_le_bytes())
        .expect("in RAM");
    m.reg_write(Reg::X1, at);
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.vreg_read(v(0)), Q_VALUE);
    assert_eq!(m.vreg_read(v(1)), SECOND_Q);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 4);
}

#[test]
fn a_q_register_pair_load_from_device_memory_at_a_word_boundary_faults() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_Q);
    let at = RAM_BASE + 0x1004;
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
    assert_eq!(
        esr & 0x3F,
        DFSC_ALIGNMENT,
        "alignment before permission, ESR {esr:#x}"
    );
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

/// The bytes the warm-TLB tests put at the start of the tested page, and
/// the word an aligned load of them gives.
const PAGE_BYTES: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
const ALIGNED_WORD: u64 = 0x4433_2211;

#[test]
fn a_misaligned_load_from_a_device_page_already_in_the_tlb_is_an_alignment_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &WARM_THEN_MISALIGNED_LOAD);
    map_test_pages(&mut m);
    m.mem_write(RAM_BASE + 0x4000, &PAGE_BYTES).expect("in RAM");
    m.reg_write(Reg::X1, 0x8000_2000);
    let (esr, far) = fault_on_a_warm_tlb(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(esr & WNR, 0);
    assert_eq!(far, 0x8000_2001);
    assert_eq!(
        m.reg_read(Reg::X3),
        ALIGNED_WORD,
        "only the warming load loaded"
    );
}

#[test]
fn a_misaligned_store_to_a_read_only_device_page_already_in_the_tlb_is_an_alignment_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &WARM_THEN_MISALIGNED_STORE);
    map_test_pages(&mut m);
    m.mem_write(RAM_BASE + 0x4000, &FILL).expect("in RAM");
    m.reg_write(Reg::X1, 0x8000_2000);
    m.reg_write(Reg::X2, u64::MAX);
    let (esr, far) = fault_on_a_warm_tlb(&mut m);
    assert_eq!(
        esr & 0x3F,
        DFSC_ALIGNMENT,
        "alignment before permission, ESR {esr:#x}"
    );
    assert_eq!(esr & WNR, WNR);
    assert_eq!(far, 0x8000_2001);
    assert_eq!(bytes_at(&m, RAM_BASE + 0x4000), FILL, "nothing is written");
}

#[test]
fn an_aligned_store_to_a_read_only_page_already_in_the_tlb_is_a_permission_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &WARM_THEN_STORE);
    map_test_pages(&mut m);
    m.mem_write(RAM_BASE + 0x4000, &FILL).expect("in RAM");
    m.reg_write(Reg::X1, 0x8000_2000);
    m.reg_write(Reg::X2, u64::MAX);
    let (esr, far) = fault_on_a_warm_tlb(&mut m);
    assert_eq!(esr & 0x3F, DFSC_PERMISSION_L3, "ESR {esr:#x}");
    assert_eq!(esr & WNR, WNR);
    assert_eq!(far, 0x8000_2000);
    assert_eq!(bytes_at(&m, RAM_BASE + 0x4000), FILL, "nothing is written");
}

#[test]
fn a_misaligned_load_from_normal_memory_already_in_the_tlb_loads() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &WARM_THEN_MISALIGNED_LOAD);
    map_test_pages(&mut m);
    m.mem_write(RAM_BASE + 0x5000, &PAGE_BYTES).expect("in RAM");
    m.reg_write(Reg::X1, 0x8000_1000);
    let out = m.step(RunBudget::Instructions(2));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 8);
    assert_eq!(m.reg_read(Reg::X3), 0x5544_3322);
}

#[test]
fn with_sctlr_a_set_a_misaligned_load_from_normal_memory_already_in_the_tlb_faults() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &WARM_THEN_MISALIGNED_LOAD);
    map_test_pages(&mut m);
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR");
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr | SCTLR_A)
        .expect("SCTLR");
    m.mem_write(RAM_BASE + 0x5000, &PAGE_BYTES).expect("in RAM");
    m.reg_write(Reg::X1, 0x8000_1000);
    let (esr, far) = fault_on_a_warm_tlb(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(esr & WNR, 0);
    assert_eq!(far, 0x8000_1001);
    assert_eq!(
        m.reg_read(Reg::X3),
        ALIGNED_WORD,
        "only the warming load loaded"
    );
}

#[test]
fn a_misaligned_load_acquire_from_normal_memory_already_in_the_tlb_faults() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &WARM_THEN_LOAD_ACQUIRE);
    map_test_pages(&mut m);
    m.mem_write(RAM_BASE + 0x5000, &PAGE_BYTES).expect("in RAM");
    m.reg_write(Reg::X2, 0x8000_1000);
    m.reg_write(Reg::X1, 0x8000_1001);
    let (esr, far) = fault_on_a_warm_tlb(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(esr & WNR, 0);
    assert_eq!(far, 0x8000_1001);
    assert_eq!(
        m.reg_read(Reg::X3),
        ALIGNED_WORD,
        "only the warming load loaded"
    );
}

#[test]
fn an_unprivileged_load_from_an_el1_only_page_already_in_the_tlb_is_a_permission_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &WARM_THEN_UNPRIVILEGED_LOAD);
    map_test_pages(&mut m);
    m.mem_write(RAM_BASE + 0x5000, &PAGE_BYTES).expect("in RAM");
    m.reg_write(Reg::X1, 0x8000_1000);
    let (esr, far) = fault_on_a_warm_tlb(&mut m);
    assert_eq!(esr & 0x3F, DFSC_PERMISSION_L3, "ESR {esr:#x}");
    assert_eq!(esr & WNR, 0);
    assert_eq!(far, 0x8000_1000);
}

#[test]
fn an_unprivileged_store_to_an_el1_only_page_already_in_the_tlb_is_a_permission_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &WARM_THEN_UNPRIVILEGED_STORE);
    map_test_pages(&mut m);
    m.mem_write(RAM_BASE + 0x5000, &FILL).expect("in RAM");
    m.reg_write(Reg::X1, 0x8000_1000);
    m.reg_write(Reg::X2, u64::MAX);
    let (esr, far) = fault_on_a_warm_tlb(&mut m);
    assert_eq!(esr & 0x3F, DFSC_PERMISSION_L3, "ESR {esr:#x}");
    assert_eq!(esr & WNR, WNR);
    assert_eq!(far, 0x8000_1000);
    assert_eq!(bytes_at(&m, RAM_BASE + 0x5000), FILL, "nothing is written");
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
    assert_eq!(
        far, 0x8000_2000,
        "the first byte that reaches the Device page"
    );
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
    assert_eq!(
        bytes_at(&m, RAM_BASE + 0x6FF0),
        FILL,
        "the first page is untouched"
    );
}

// Pairs whose elements are aligned and the pair is not: words at 4 mod 8,
// doublewords at 8 mod 16. With stage 1 disabled the data is on Device
// memory at RAM_BASE + 0x1000; with it enabled, on the Normal page at VA
// 0x8000_1000 (RAM + 0x5000) with SCTLR_EL1.A set.

/// Device memory: a word pair's address, and a doubleword pair's.
const DEVICE_WORDS: u64 = RAM_BASE + 0x1004;
const DEVICE_DWORDS: u64 = RAM_BASE + 0x1008;
/// Normal memory under SCTLR_EL1.A: the same, as VA and as PA.
const NORMAL_WORDS: u64 = 0x8000_1004;
const NORMAL_WORDS_PA: u64 = RAM_BASE + 0x5004;
const NORMAL_DWORDS: u64 = 0x8000_1008;
const NORMAL_DWORDS_PA: u64 = RAM_BASE + 0x5008;

/// Run the one instruction at `RAM_BASE`, which must complete.
fn completes<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>) {
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 4, "no fault");
}

/// The pages of [`map_test_pages`], with SCTLR_EL1.A set.
fn map_test_pages_strictly<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>) {
    map_test_pages(m);
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR");
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr | SCTLR_A)
        .expect("SCTLR");
}

fn put_words<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>, pa: u64) {
    m.mem_write(pa, &FIRST_WORD.to_le_bytes()).expect("in RAM");
    m.mem_write(pa + 4, &SECOND_WORD.to_le_bytes())
        .expect("in RAM");
}

fn put_dwords<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, I>, pa: u64) {
    m.mem_write(pa, &FIRST_DWORD.to_le_bytes()).expect("in RAM");
    m.mem_write(pa + 8, &SECOND_DWORD.to_le_bytes())
        .expect("in RAM");
}

/// The 16 bytes from 4 below a stored word pair: the fill on either side.
fn words_stored() -> [u8; 16] {
    let mut bytes = FILL;
    bytes[4..8].copy_from_slice(&FIRST_WORD.to_le_bytes());
    bytes[8..12].copy_from_slice(&SECOND_WORD.to_le_bytes());
    bytes
}

fn dwords_stored() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&FIRST_DWORD.to_le_bytes());
    bytes[8..].copy_from_slice(&SECOND_DWORD.to_le_bytes());
    bytes
}

#[test]
fn a_word_pair_load_from_device_memory_needs_word_alignment_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_W);
    put_words(&mut m, DEVICE_WORDS);
    m.reg_write(Reg::X1, DEVICE_WORDS);
    completes(&mut m);
    assert_eq!(m.reg_read(Reg::X3), u64::from(FIRST_WORD));
    assert_eq!(m.reg_read(Reg::X4), u64::from(SECOND_WORD));
}

#[test]
fn a_word_pair_store_to_device_memory_needs_word_alignment_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_PAIR_W);
    m.mem_write(DEVICE_WORDS - 4, &FILL).expect("in RAM");
    m.reg_write(Reg::X1, DEVICE_WORDS);
    m.reg_write(Reg::X2, u64::from(FIRST_WORD));
    m.reg_write(Reg::X3, u64::from(SECOND_WORD));
    completes(&mut m);
    assert_eq!(bytes_at(&m, DEVICE_WORDS - 4), words_stored());
}

#[test]
fn a_signed_word_pair_load_from_device_memory_needs_word_alignment_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_SW);
    put_words(&mut m, DEVICE_WORDS);
    m.reg_write(Reg::X1, DEVICE_WORDS);
    completes(&mut m);
    assert_eq!(m.reg_read(Reg::X3), 0xFFFF_FFFF_8899_AABB, "sign-extended");
    assert_eq!(m.reg_read(Reg::X4), u64::from(SECOND_WORD));
}

#[test]
fn a_single_precision_pair_load_from_device_memory_needs_word_alignment_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_S);
    put_words(&mut m, DEVICE_WORDS);
    m.vreg_write(v(0), Q_VALUE);
    m.vreg_write(v(1), Q_VALUE);
    m.reg_write(Reg::X1, DEVICE_WORDS);
    completes(&mut m);
    assert_eq!(
        m.vreg_read(v(0)),
        u128::from(FIRST_WORD),
        "the rest of V0 is cleared"
    );
    assert_eq!(
        m.vreg_read(v(1)),
        u128::from(SECOND_WORD),
        "the rest of V1 is cleared"
    );
}

#[test]
fn a_double_precision_pair_store_to_device_memory_needs_doubleword_alignment_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_PAIR_D);
    m.vreg_write(v(0), (Q_VALUE << 64) | u128::from(FIRST_DWORD));
    m.vreg_write(v(1), (Q_VALUE << 64) | u128::from(SECOND_DWORD));
    m.reg_write(Reg::X1, DEVICE_DWORDS);
    completes(&mut m);
    assert_eq!(
        bytes_at(&m, DEVICE_DWORDS),
        dwords_stored(),
        "the low doubleword of each"
    );
}

#[test]
fn a_doubleword_pair_load_from_device_memory_at_a_word_boundary_faults() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_X);
    m.reg_write(Reg::X1, DEVICE_WORDS);
    m.reg_write(Reg::X3, UNTOUCHED);
    m.reg_write(Reg::X4, UNTOUCHED);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(esr & WNR, 0);
    assert_eq!(far, DEVICE_WORDS, "the first element's address");
    assert_eq!(m.reg_read(Reg::X3), UNTOUCHED, "X3 is not loaded");
    assert_eq!(m.reg_read(Reg::X4), UNTOUCHED, "X4 is not loaded");
}

#[test]
fn with_sctlr_a_set_a_word_pair_load_from_normal_memory_needs_word_alignment_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_W);
    map_test_pages_strictly(&mut m);
    put_words(&mut m, NORMAL_WORDS_PA);
    m.reg_write(Reg::X1, NORMAL_WORDS);
    completes(&mut m);
    assert_eq!(m.reg_read(Reg::X3), u64::from(FIRST_WORD));
    assert_eq!(m.reg_read(Reg::X4), u64::from(SECOND_WORD));
}

#[test]
fn with_sctlr_a_set_a_word_pair_store_to_normal_memory_needs_word_alignment_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_PAIR_W);
    map_test_pages_strictly(&mut m);
    m.mem_write(NORMAL_WORDS_PA - 4, &FILL).expect("in RAM");
    m.reg_write(Reg::X1, NORMAL_WORDS);
    m.reg_write(Reg::X2, u64::from(FIRST_WORD));
    m.reg_write(Reg::X3, u64::from(SECOND_WORD));
    completes(&mut m);
    assert_eq!(bytes_at(&m, NORMAL_WORDS_PA - 4), words_stored());
}

#[test]
fn with_sctlr_a_set_a_signed_word_pair_load_from_normal_memory_needs_word_alignment_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_SW);
    map_test_pages_strictly(&mut m);
    put_words(&mut m, NORMAL_WORDS_PA);
    m.reg_write(Reg::X1, NORMAL_WORDS);
    completes(&mut m);
    assert_eq!(m.reg_read(Reg::X3), 0xFFFF_FFFF_8899_AABB, "sign-extended");
    assert_eq!(m.reg_read(Reg::X4), u64::from(SECOND_WORD));
}

#[test]
fn with_sctlr_a_set_a_single_precision_pair_store_to_normal_memory_needs_word_alignment_only() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_PAIR_S);
    map_test_pages_strictly(&mut m);
    m.mem_write(NORMAL_WORDS_PA - 4, &FILL).expect("in RAM");
    m.vreg_write(v(0), (Q_VALUE << 32) | u128::from(FIRST_WORD));
    m.vreg_write(v(1), (Q_VALUE << 32) | u128::from(SECOND_WORD));
    m.reg_write(Reg::X1, NORMAL_WORDS);
    completes(&mut m);
    assert_eq!(
        bytes_at(&m, NORMAL_WORDS_PA - 4),
        words_stored(),
        "the low word of each"
    );
}

#[test]
fn with_sctlr_a_set_a_double_precision_pair_load_from_normal_memory_needs_doubleword_alignment_only(
) {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_D);
    map_test_pages_strictly(&mut m);
    put_dwords(&mut m, NORMAL_DWORDS_PA);
    m.reg_write(Reg::X1, NORMAL_DWORDS);
    completes(&mut m);
    assert_eq!(m.vreg_read(v(0)), u128::from(FIRST_DWORD));
    assert_eq!(m.vreg_read(v(1)), u128::from(SECOND_DWORD));
}

#[test]
fn with_sctlr_a_set_a_word_pair_load_at_a_halfword_boundary_faults() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_W);
    map_test_pages_strictly(&mut m);
    let at = NORMAL_WORDS - 2;
    m.reg_write(Reg::X1, at);
    m.reg_write(Reg::X3, UNTOUCHED);
    m.reg_write(Reg::X4, UNTOUCHED);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(esr & WNR, 0);
    assert_eq!(far, at, "the first element's address");
    assert_eq!(m.reg_read(Reg::X3), UNTOUCHED, "W3 is not loaded");
    assert_eq!(m.reg_read(Reg::X4), UNTOUCHED, "W4 is not loaded");
}

#[test]
fn with_sctlr_a_set_a_doubleword_pair_load_at_a_word_boundary_faults() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LOAD_PAIR_X);
    map_test_pages_strictly(&mut m);
    m.reg_write(Reg::X1, NORMAL_WORDS);
    m.reg_write(Reg::X3, UNTOUCHED);
    m.reg_write(Reg::X4, UNTOUCHED);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(esr & WNR, 0);
    assert_eq!(far, NORMAL_WORDS, "the first element's address");
    assert_eq!(m.reg_read(Reg::X3), UNTOUCHED, "X3 is not loaded");
    assert_eq!(m.reg_read(Reg::X4), UNTOUCHED, "X4 is not loaded");
}

#[test]
fn with_sctlr_a_set_a_double_precision_pair_store_at_a_word_boundary_faults() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STORE_PAIR_D);
    map_test_pages_strictly(&mut m);
    m.mem_write(NORMAL_WORDS_PA - 4, &FILL).expect("in RAM");
    m.vreg_write(v(0), u128::from(FIRST_DWORD));
    m.vreg_write(v(1), u128::from(SECOND_DWORD));
    m.reg_write(Reg::X1, NORMAL_WORDS);
    let (esr, far) = fault(&mut m);
    assert_eq!(esr & 0x3F, DFSC_ALIGNMENT, "ESR {esr:#x}");
    assert_eq!(esr & WNR, WNR);
    assert_eq!(far, NORMAL_WORDS, "the first element's address");
    assert_eq!(bytes_at(&m, NORMAL_WORDS_PA - 4), FILL, "nothing is stored");
}
