//! The hardware update of stage 1's Access flag and dirty state (FEAT_HAF
//! and FEAT_HAFDBS), as a guest sees it (R9): the descriptor its own load
//! reads back after an access, the Data Abort an access takes, and
//! `ID_AA64MMFR1_EL1.HAFDBS`. Stage 2's are `tests/unit/el2.rs`'s, the
//! one processor with EL2 being the Cortex-A57, which has neither.
//!
//! The rules are the Arm ARM's (D8.5, "Hardware updates to the
//! translation tables") and its pseudocode: `AArch64_S1Walk` faults on a
//! clear Access flag only where `TCR_EL1.HA` is clear;
//! `AArch64_S1DirectBasePermissions` takes a leaf's `AP[2]` as clear where
//! `TCR_EL1.HD` is set and its `DBM` is; `AArch64_S1Translate` sets the
//! flag (`AArch64_SetAccessFlag`) and clears `AP[2]` on a write
//! (`AArch64_SetDirtyState`) once the access's checks pass, and `HD` has no
//! effect without `HA` (`AArch64_S1TTWParamsEL10`). Where the Arm ARM
//! leaves a choice open this core takes no update: after a Permission
//! fault (`Unpredictable_AFUPDATE`), and for an address translation
//! instruction ("AT updates AF"); cache maintenance, which faults on a
//! clear flag here ("Generate access flag fault on IC/DC operations"),
//! sets it. The Cortex-A57, an ARMv8.0 processor, has neither feature:
//! `TCR_EL1.HA` and `HD` are RES0 there.
//!
//! Guest code is hand-assembled (GNU as for aarch64); each word's source is
//! beside it.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    CpuModel, LentStorage, MachineBuilder, MemoryLoan, NoEl2, RamSize, Reg, RunBudget, StopReason,
    SysReg, VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 4 << 20;

/// `ldr x2, [x1]`, the load under test.
const LDR_X2: u32 = 0xF940_0022;
/// `str x2, [x1]`, the store under test.
const STR_X2: u32 = 0xF900_0022;
/// `at s1e1r, x1`.
const AT_S1E1R: u32 = 0xD508_7801;
/// `dc civac, x1`.
const DC_CIVAC: u32 = 0xD50B_7E21;
/// `ldr x3, [x4]`: the guest reads the descriptor back.
const LDR_X3_X4: u32 = 0xF940_0083;
/// `mrs x0, id_aa64mmfr1_el1`.
const MRS_MMFR1: u32 = 0xD538_0720;
/// `b .`
const SPIN: u32 = 0x1400_0000;

/// The tables: a level 1 table whose second entry leads to a level 2
/// table, whose first entry maps the first 2 MiB of RAM to themselves (the
/// code, the tables and the vectors) and whose second leads to a level 3
/// table, whose first entry, the descriptor under test, maps [`DATA_VA`].
const L1: u64 = RAM_BASE + 0x1_0000;
const L2: u64 = RAM_BASE + 0x2_0000;
const L3: u64 = RAM_BASE + 0x3_0000;
/// The descriptor under test, at an address the first 2 MiB map to itself.
const DESC: u64 = L3;
const DATA_VA: u64 = 0x4020_0000;
const DATA_PA: u64 = RAM_BASE + 0x30_0000;
/// The vector base: the synchronous exception from the current level with
/// `SP_EL1` is at `VBAR + 0x200`.
const VBAR: u64 = RAM_BASE + 0x800;
const SYNC_VECTOR: u64 = VBAR + 0x200;

const BLOCK: u64 = 0b01;
const TABLE: u64 = 0b11;
const PAGE: u64 = 0b11;
const AF: u64 = 1 << 10;
const INNER_SHAREABLE: u64 = 3 << 8;
/// `AP[2]`: read-only.
const AP2: u64 = 1 << 7;
/// `DBM`: a candidate for the hardware update of dirty state.
const DBM: u64 = 1 << 51;
/// The page under test as each test varies it: its output address, Normal
/// memory (`AttrIndx` 0), Inner Shareable.
const PAGE_BASE: u64 = DATA_PA | INNER_SHAREABLE | PAGE;

/// `TCR_EL1`: T0SZ 25 (39-bit addresses, a walk from level 1), the walks
/// Inner and Outer Write-Back Inner Shareable, EPD1 (no upper range), IPS
/// 44 bits; and the hardware update controls.
const TCR_BASE: u64 = 25 | (1 << 8) | (1 << 10) | (3 << 12) | (1 << 23) | (0b100 << 32);
const TCR_HA: u64 = 1 << 39;
const TCR_HD: u64 = 1 << 40;

/// `ESR_EL1`: a Data Abort from the current level, its fault status code
/// and `WnR`.
const EC_DATA_ABORT_SAME_EL: u64 = 0x25;
const DFSC_ACCESS_FLAG_L3: u64 = 0b00_1011;
const DFSC_PERMISSION_L3: u64 = 0b00_1111;
const WNR: u64 = 1 << 6;

/// A bare machine of `cpu` with the MMU on over the tables, `TCR_EL1` the
/// base and `hw` (`HA` and `HD` as the test sets them), the page under
/// test `page`, and the program `access` then `ldr x3, [x4]` from the
/// base of RAM, with `x1` the page's address and `x4` its descriptor's.
fn machine(
    cpu: CpuModel,
    ram: &mut [u8],
    hw: u64,
    page: u64,
    access: u32,
) -> VirtMachine<LentStorage<'_>, NoEl2> {
    let config = VirtConfig {
        cpu,
        ..VirtConfig::DEFAULT
    };
    let mut m = MachineBuilder::new(config)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds");
    let words = [
        (RAM_BASE, access),
        (RAM_BASE + 4, LDR_X3_X4),
        (RAM_BASE + 8, SPIN),
    ];
    for (at, word) in words.into_iter().chain([(SYNC_VECTOR, SPIN)]) {
        m.mem_write(at, &word.to_le_bytes())
            .expect("code is in RAM");
    }
    for (at, desc) in [
        (L1 + 8, L2 | TABLE),
        (L2, RAM_BASE | AF | INNER_SHAREABLE | BLOCK),
        (L2 + 8, L3 | TABLE),
        (DESC, page),
    ] {
        m.mem_write(at, &desc.to_le_bytes())
            .expect("tables are in RAM");
    }
    m.mem_write(DATA_PA, &0x1234_5678u64.to_le_bytes())
        .expect("data is in RAM");
    m.sysreg_write(SysReg::MAIR_EL1, 0xFF).expect("MAIR_EL1");
    m.sysreg_write(SysReg::TCR_EL1, TCR_BASE | hw)
        .expect("TCR_EL1");
    m.sysreg_write(SysReg::TTBR0_EL1, L1).expect("TTBR0_EL1");
    m.sysreg_write(SysReg::VBAR_EL1, VBAR).expect("VBAR_EL1");
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR_EL1");
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr | 1)
        .expect("SCTLR_EL1");
    m.reg_write(Reg::X1, DATA_VA);
    m.reg_write(Reg::X2, 0xD1D7);
    m.reg_write(Reg::X4, DESC);
    m
}

fn step(m: &mut VirtMachine<LentStorage<'_>, NoEl2>) {
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
}

/// What the guest saw of its access: the descriptor it read back after
/// it, or the Data Abort's syndrome and fault address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    Descriptor(u64),
    Abort { esr: u64, far: u64 },
}

/// Run the access, and the descriptor's read back if it completed.
fn run(m: &mut VirtMachine<LentStorage<'_>, NoEl2>) -> Seen {
    step(m);
    if m.reg_read(Reg::Pc) == SYNC_VECTOR {
        let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1");
        let far = m.sysreg_read(SysReg::FAR_EL1).expect("FAR_EL1");
        return Seen::Abort { esr, far };
    }
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 4, "the access completed");
    step(m);
    Seen::Descriptor(m.reg_read(Reg::X3))
}

/// A Data Abort at [`DATA_VA`] with fault status `dfsc` and `WnR` as
/// `write` holds it ([`WNR`] or 0).
fn abort(dfsc: u64, write: u64) -> Seen {
    Seen::Abort {
        esr: (EC_DATA_ABORT_SAME_EL << 26) | (1 << 25) | write | dfsc,
        far: DATA_VA,
    }
}

/// The page's data as memory holds it.
fn data(m: &VirtMachine<LentStorage<'_>, NoEl2>) -> u64 {
    let mut bytes = [0; 8];
    m.mem_read(DATA_PA, &mut bytes).expect("data is in RAM");
    u64::from_le_bytes(bytes)
}

/// The descriptor as memory holds it, for an access that faulted.
fn descriptor(m: &VirtMachine<LentStorage<'_>, NoEl2>) -> u64 {
    let mut bytes = [0; 8];
    m.mem_read(DESC, &mut bytes).expect("tables are in RAM");
    u64::from_le_bytes(bytes)
}

/// `ID_AA64MMFR1_EL1.HAFDBS` as a guest reads it: 2 on the Armv9.2 model
/// (FEAT_HAF and FEAT_HAFDBS, not FEAT_HAFT), 0 on the Cortex-A57.
#[test]
fn hafdbs_is_2_on_the_armv9_2_model_and_0_on_the_cortex_a57() {
    for (cpu, hafdbs) in [(CpuModel::Armv9p2, 2), (CpuModel::CortexA57, 0)] {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = machine(cpu, &mut ram, 0, PAGE_BASE | AF, MRS_MMFR1);
        step(&mut m);
        assert_eq!(m.reg_read(Reg::X0) & 0xF, hafdbs, "{cpu:?}");
    }
}

/// With `TCR_EL1.HA` a load through a leaf whose Access flag is clear
/// completes, and the guest finds the flag set and nothing else of the
/// descriptor changed; without it the load takes a level 3 Access flag
/// fault and the descriptor stays as it was.
#[test]
fn ha_has_a_load_set_the_access_flag_where_without_it_faults() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::Armv9p2, &mut ram, TCR_HA, PAGE_BASE, LDR_X2);
    assert_eq!(run(&mut m), Seen::Descriptor(PAGE_BASE | AF));
    assert_eq!(m.reg_read(Reg::X2), 0x1234_5678, "the load read the page");

    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::Armv9p2, &mut ram, 0, PAGE_BASE, LDR_X2);
    assert_eq!(run(&mut m), abort(DFSC_ACCESS_FLAG_L3, 0));
    assert_eq!(descriptor(&m), PAGE_BASE);
}

/// With `HA` and `HD` a store to a writable-clean page (`DBM` set,
/// `AP[2]` set) completes and makes it writable-dirty: the guest finds
/// `AP[2]` clear. A clear Access flag is set by the same store.
#[test]
fn hd_has_a_store_make_a_writable_clean_page_dirty() {
    for flag in [AF, 0] {
        let page = PAGE_BASE | flag | DBM | AP2;
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = machine(CpuModel::Armv9p2, &mut ram, TCR_HA | TCR_HD, page, STR_X2);
        assert_eq!(
            run(&mut m),
            Seen::Descriptor(PAGE_BASE | AF | DBM),
            "Access flag {flag:#x}"
        );
        assert_eq!(data(&m), 0xD1D7, "the store wrote the page");
    }
}

/// A load leaves a writable-clean page clean, and the translation it
/// leaves in the TLB does not let the store that follows past the update:
/// the store still makes the page dirty.
#[test]
fn a_load_leaves_a_writable_clean_page_clean_for_a_later_store_to_dirty() {
    const STR_X2_THEN_LDR: [u32; 3] = [STR_X2, LDR_X3_X4, SPIN];
    let page = PAGE_BASE | AF | DBM | AP2;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::Armv9p2, &mut ram, TCR_HA | TCR_HD, page, LDR_X2);
    assert_eq!(
        run(&mut m),
        Seen::Descriptor(page),
        "a load leaves it clean"
    );
    for (i, word) in STR_X2_THEN_LDR.into_iter().enumerate() {
        m.mem_write(RAM_BASE + 0xC + 4 * i as u64, &word.to_le_bytes())
            .expect("code is in RAM");
    }
    m.reg_write(Reg::Pc, RAM_BASE + 0xC);
    m.reg_write(Reg::X2, 0xD1D7);
    step(&mut m);
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 0x10, "the store completed");
    step(&mut m);
    assert_eq!(
        m.reg_read(Reg::X3),
        PAGE_BASE | AF | DBM,
        "the store dirtied it"
    );
    assert_eq!(data(&m), 0xD1D7);
}

/// Without `HD`, or with `HD` but not `HA`, which it needs, `DBM` gives
/// no write: the store takes a level 3 Permission fault, and the page
/// stays clean.
#[test]
fn without_hd_or_ha_a_store_to_a_writable_clean_page_faults() {
    let page = PAGE_BASE | AF | DBM | AP2;
    for hw in [TCR_HA, TCR_HD, 0] {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = machine(CpuModel::Armv9p2, &mut ram, hw, page, STR_X2);
        assert_eq!(
            run(&mut m),
            abort(DFSC_PERMISSION_L3, WNR),
            "TCR_EL1 HA/HD {hw:#x}"
        );
        assert_eq!(descriptor(&m), page, "TCR_EL1 HA/HD {hw:#x}");
        assert_eq!(data(&m), 0x1234_5678);
    }
}

/// `HD` without `HA` leaves the Access flag unmanaged: a clear one still
/// faults.
#[test]
fn hd_without_ha_leaves_a_clear_access_flag_to_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::Armv9p2, &mut ram, TCR_HD, PAGE_BASE, LDR_X2);
    assert_eq!(run(&mut m), abort(DFSC_ACCESS_FLAG_L3, 0));
}

/// An access stage 1's checks refuse sets no Access flag, this core's
/// choice of the outcomes the Arm ARM leaves open: a store to a read-only
/// page whose flag is clear takes its Permission fault, and the flag stays
/// clear, until a load, which the page permits, sets it.
#[test]
fn an_access_its_permissions_refuse_sets_no_access_flag() {
    let page = PAGE_BASE | AP2;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::Armv9p2, &mut ram, TCR_HA | TCR_HD, page, STR_X2);
    assert_eq!(run(&mut m), abort(DFSC_PERMISSION_L3, WNR));
    assert_eq!(descriptor(&m), page, "the flag stays clear");
    m.mem_write(RAM_BASE, &LDR_X2.to_le_bytes())
        .expect("code is in RAM");
    m.reg_write(Reg::Pc, RAM_BASE);
    assert_eq!(run(&mut m), Seen::Descriptor(page | AF));
}

/// An address translation instruction reports a leaf whose Access flag
/// `HA` manages as translating, and leaves the flag as it was (the Arm ARM
/// permits either; this core does not update it).
#[test]
fn at_reports_the_translation_and_leaves_the_access_flag_clear() {
    const PAR_F: u64 = 1;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::Armv9p2, &mut ram, TCR_HA, PAGE_BASE, AT_S1E1R);
    assert_eq!(run(&mut m), Seen::Descriptor(PAGE_BASE));
    let par = m.sysreg_read(SysReg::PAR_EL1).expect("PAR_EL1");
    assert_eq!(par & PAR_F, 0, "PAR_EL1 {par:#x}");
    assert_eq!(par & 0x0000_FFFF_FFFF_F000, DATA_PA);
}

/// Cache maintenance by address, which faults on a clear Access flag on
/// this core, sets it where `HA` manages it.
#[test]
fn cache_maintenance_sets_a_managed_access_flag() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::Armv9p2, &mut ram, TCR_HA, PAGE_BASE, DC_CIVAC);
    assert_eq!(run(&mut m), Seen::Descriptor(PAGE_BASE | AF));

    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::Armv9p2, &mut ram, 0, PAGE_BASE, DC_CIVAC);
    let Seen::Abort { esr, far } = run(&mut m) else {
        panic!("DC CIVAC on a clear Access flag completed");
    };
    assert_eq!(esr & 0x3F, DFSC_ACCESS_FLAG_L3, "ESR_EL1 {esr:#x}");
    assert_eq!(far, DATA_VA);
}

/// On the Cortex-A57, which has neither FEAT_HAF nor FEAT_HAFDBS, `HA` and
/// `HD` are RES0: a clear Access flag faults, and a writable-clean page
/// refuses a store, whatever they hold.
#[test]
fn the_cortex_a57_ignores_ha_and_hd() {
    let hw = TCR_HA | TCR_HD;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::CortexA57, &mut ram, hw, PAGE_BASE, LDR_X2);
    assert_eq!(run(&mut m), abort(DFSC_ACCESS_FLAG_L3, 0));
    assert_eq!(descriptor(&m), PAGE_BASE);

    let page = PAGE_BASE | AF | DBM | AP2;
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = machine(CpuModel::CortexA57, &mut ram, hw, page, STR_X2);
    assert_eq!(run(&mut m), abort(DFSC_PERMISSION_L3, WNR));
    assert_eq!(descriptor(&m), page);
}
