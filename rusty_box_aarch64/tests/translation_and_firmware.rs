//! Translation configuration, address translation instructions, the
//! firmware's PSCI and SMCCC calls and store-exclusive ordering, as the
//! guest sees them (R9: each test asserts a register, a syndrome or
//! `PAR_EL1`).
//!
//! The references are the Arm documents first and QEMU 11.1.2 second:
//!
//! - an out-of-range T0SZ/T1SZ takes a level 0 Translation fault, one of
//!   the two outcomes the Arm ARM leaves IMPLEMENTATION DEFINED
//!   (`AArch64.S1InvalidTxSZ`; `AArch64.GetS1TTWParams`, the ImpDef
//!   booleans "Fault on TxSZ value above maximum" and "... below
//!   minimum"), and the walk's output addresses must fit the size
//!   `TCR_EL1.IPS` selects, bounded by PARange (both in target/arm/ptw.c
//!   `get_phys_addr_lpae`);
//! - `AT` takes a synchronous external abort on its walk as a Data Abort
//!   (Arm ARM `AArch64.AT`; target/arm/tcg/cpregs-at.c `do_ats_write`);
//! - PSCI `CPU_ON` checks its target, then its entry point, and
//!   `AFFINITY_INFO` names an instance at its affinity level (Arm DEN 0022D
//!   §5.6, §5.7), with the function identifier and an SMC32 call's
//!   arguments in W registers (SMC Calling Convention, Arm DEN 0028B §2.6,
//!   §2.7, §3.1 Table 3-1);
//! - the firmware is PSCI 1.2 (Arm DEN 0022E) and SMCCC 1.5 (Arm DEN
//!   0028F): `SMCCC_VERSION`, discovered through `PSCI_FEATURES`,
//!   `SMCCC_ARCH_FEATURES`, a function identifier's SVE hint bit taken as
//!   0 (§2.5.1), and the speculation workarounds as each `SpectreReport`
//!   answers and implements them (§7.5 to §7.7);
//! - a store-exclusive to an address the monitor does not hold fails
//!   without translating it (Arm ARM `AArch64.ExclusiveMonitorsPass`, whose
//!   IMPLEMENTATION DEFINED `AArch64.IsExclusiveVA` is permitted to return
//!   FALSE before `AArch64.TranslateAddress`, and here does;
//!   target/arm/tcg/translate-a64.c `gen_store_exclusive`).
//!
//! Guest code is hand-assembled (GNU as for aarch64); each word's source is
//! beside it.

use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{
    GuestStorage, Instrumentation, LentStorage, MachineBuilder, MemoryLoan, NoEl2, RamSize, Reg,
    RunBudget, SpectreReport, StopReason, SysReg, VirtConfig, VirtMachine,
};

const RAM_LEN: usize = 4 << 20;

/// `at s1e1r, x1; b .`
const AT_S1E1R: [u32; 2] = [0xD508_7801, 0x1400_0000];
/// `hvc #0; b .`
const HVC: [u32; 2] = [0xD400_0002, 0x1400_0000];
/// `stxr w3, x2, [x1]; b .`
const STXR: [u32; 2] = [0xC803_7C22, 0x1400_0000];
/// `ldxr x2, [x1]; stxr w3, x2, [x1]; b .`
const LDXR_STXR: [u32; 3] = [0xC85F_7C22, 0xC803_7C22, 0x1400_0000];

/// The upper half of the address space under T1SZ = 25.
const UPPER_VA: u64 = 0xFFFF_FF80_0000_0000;
const AF: u64 = 1 << 10;
const BLOCK: u64 = 0b01;
/// PAR_EL1: F, and bit 11 RES1.
const PAR_F: u64 = 1;
const PAR_RES1: u64 = 1 << 11;

/// ESR_EL1 for a Data Abort at the same EL.
const EC_DATA_ABORT_SAME_EL: u64 = 0x25;

fn bare(ram: &mut [u8]) -> VirtMachine<LentStorage<'_>, NoEl2> {
    bare_with(VirtConfig::DEFAULT, ram)
}

/// [`bare`] built with `config`.
fn bare_with(config: VirtConfig, ram: &mut [u8]) -> VirtMachine<LentStorage<'_>, NoEl2> {
    MachineBuilder::new(config)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

fn load<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, NoEl2, I>, words: &[u32]) {
    for (i, word) in words.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("code is in RAM");
    }
}

fn write_desc<S: GuestStorage, I: Instrumentation>(
    m: &mut VirtMachine<S, NoEl2, I>,
    at: u64,
    desc: u64,
) {
    m.mem_write(at, &desc.to_le_bytes())
        .expect("tables are in RAM");
}

/// The MMU on, with TTBR0 mapping RAM identically at VA 0x4000_0000 (so the
/// code runs) and TTBR1, T1SZ and IPS as each test needs them.
fn mmu_on<S: GuestStorage, I: Instrumentation>(
    m: &mut VirtMachine<S, NoEl2, I>,
    t1sz: u64,
    ips: u64,
    ttbr1: u64,
) {
    let l1 = RAM_BASE + 0x1_0000;
    write_desc(m, l1 + 8, RAM_BASE | AF | BLOCK);
    m.sysreg_write(SysReg::MAIR_EL1, 0xFF).expect("MAIR");
    m.sysreg_write(SysReg::TCR_EL1, 25 | (t1sz << 16) | (ips << 32))
        .expect("TCR");
    m.sysreg_write(SysReg::TTBR0_EL1, l1).expect("TTBR0");
    m.sysreg_write(SysReg::TTBR1_EL1, ttbr1).expect("TTBR1");
    let sctlr = m.sysreg_read(SysReg::SCTLR_EL1).expect("SCTLR");
    m.sysreg_write(SysReg::SCTLR_EL1, sctlr | 1).expect("SCTLR");
}

fn step1<S: GuestStorage, I: Instrumentation>(m: &mut VirtMachine<S, NoEl2, I>) {
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
}

fn par<S: GuestStorage, I: Instrumentation>(m: &VirtMachine<S, NoEl2, I>) -> u64 {
    m.sysreg_read(SysReg::PAR_EL1).expect("PAR_EL1")
}

/// `PAR_EL1` for a failed translation with fault status `fst`.
const fn par_fault(fst: u64) -> u64 {
    PAR_RES1 | (fst << 1) | PAR_F
}

#[test]
fn a_t1sz_above_39_is_a_level_0_translation_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &AT_S1E1R);
    // An empty table: a walk with T1SZ taken as 39 would start at level 2,
    // where this address (in range for 39) would fault.
    mmu_on(&mut m, 40, 4, RAM_BASE + 0x2_0000);
    m.reg_write(Reg::X1, 0xFFFF_FFFF_FE00_0000);
    step1(&mut m);
    assert_eq!(par(&m), par_fault(0x04), "Translation fault, level 0");
}

#[test]
fn a_block_beyond_the_ips_output_size_is_an_address_size_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &AT_S1E1R);
    let l1 = RAM_BASE + 0x2_0000;
    write_desc(&mut m, l1, 0x1_0000_0000 | AF | BLOCK);
    // IPS 0: a 32-bit output size, so a 4 GiB block address does not fit.
    mmu_on(&mut m, 25, 0, l1);
    m.reg_write(Reg::X1, UPPER_VA);
    step1(&mut m);
    assert_eq!(par(&m), par_fault(0x01), "Address size fault, level 1");
}

#[test]
fn the_same_block_within_the_ips_output_size_translates() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &AT_S1E1R);
    let l1 = RAM_BASE + 0x2_0000;
    write_desc(&mut m, l1, 0x1_0000_0000 | AF | BLOCK);
    mmu_on(&mut m, 25, 4, l1);
    m.reg_write(Reg::X1, UPPER_VA);
    step1(&mut m);
    assert_eq!(par(&m) & PAR_F, 0, "PAR {:#x}", par(&m));
    assert_eq!(par(&m) & 0x0000_FFFF_FFFF_F000, 0x1_0000_0000);
}

#[test]
fn a_table_base_beyond_the_ips_output_size_is_a_level_0_address_size_fault() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &AT_S1E1R);
    mmu_on(&mut m, 25, 0, 0x1_0000_0000);
    m.reg_write(Reg::X1, UPPER_VA);
    step1(&mut m);
    assert_eq!(par(&m), par_fault(0x00), "Address size fault, level 0");
}

#[test]
fn an_external_abort_on_the_walk_of_an_at_is_a_data_abort() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &AT_S1E1R);
    // The table base is past the end of RAM: the walk's first read aborts.
    mmu_on(&mut m, 25, 4, RAM_BASE + RAM_LEN as u64);
    m.reg_write(Reg::X1, UPPER_VA);
    let par_before = par(&m);
    step1(&mut m);
    let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1");
    assert_eq!(esr >> 26, EC_DATA_ABORT_SAME_EL, "ESR {esr:#x}");
    assert_eq!(
        esr & 0x3F,
        0x15,
        "synchronous external abort on the walk, level 1"
    );
    assert_eq!(
        esr & (1 << 8),
        1 << 8,
        "CM: an address translation instruction"
    );
    assert_eq!(esr & (1 << 6), 1 << 6, "WnR");
    assert_eq!(m.sysreg_read(SysReg::FAR_EL1), Ok(UPPER_VA));
    assert_eq!(par(&m), par_before, "PAR_EL1 is not written");
}

/// PSCI function identifiers and return codes (DEN 0022D §5.1, Table 6).
mod psci {
    pub const VERSION: u64 = 0x8400_0000;
    pub const FEATURES: u64 = 0x8400_000A;
    pub const CPU_ON_32: u64 = 0x8400_0003;
    pub const CPU_ON_64: u64 = 0xC400_0003;
    pub const AFFINITY_INFO_32: u64 = 0x8400_0004;
    pub const AFFINITY_INFO_64: u64 = 0xC400_0004;
    pub const SUCCESS: u64 = 0;
    pub const INVALID_PARAMETERS: u64 = (-2i64) as u64;
    pub const ALREADY_ON: u64 = (-4i64) as u64;
    pub const INVALID_ADDRESS: u64 = (-9i64) as u64;
    pub const ON: u64 = 0;
    pub const NOT_SUPPORTED: u64 = (-1i64) as u64;
    pub const VERSION_1_2: u64 = 0x0001_0002;
}

/// The SMC Calling Convention's Arm Architecture Calls (DEN 0028F §7).
mod smccc {
    pub const VERSION: u64 = 0x8000_0000;
    pub const ARCH_FEATURES: u64 = 0x8000_0001;
    pub const ARCH_SOC_ID: u64 = 0x8000_0002;
    pub const ARCH_WORKAROUND_1: u64 = 0x8000_8000;
    pub const ARCH_WORKAROUND_2: u64 = 0x8000_7FFF;
    pub const ARCH_WORKAROUND_3: u64 = 0x8000_3FFF;
    pub const NOT_REQUIRED: u64 = (-2i64) as u64;
    pub const VERSION_1_5: u64 = 0x0001_0005;
}

/// X0 after the guest issues `hvc #0` with X0, X1 and X2 as given.
fn psci_call(x0: u64, x1: u64, x2: u64) -> u64 {
    firmware_call(SpectreReport::DEFAULT, [x0, x1, x2, 0])[0]
}

/// X0 to X3 after the guest issues `hvc #0` with X0 to X3 as given, on a
/// board whose firmware gives `spectre`.
fn firmware_call(spectre: SpectreReport, args: [u64; 4]) -> [u64; 4] {
    let mut ram = vec![0u8; RAM_LEN];
    let config = VirtConfig {
        spectre,
        ..VirtConfig::DEFAULT
    };
    let mut m = bare_with(config, &mut ram);
    load(&mut m, &HVC);
    for (reg, value) in [Reg::X0, Reg::X1, Reg::X2, Reg::X3].into_iter().zip(args) {
        m.reg_write(reg, value);
    }
    step1(&mut m);
    [Reg::X0, Reg::X1, Reg::X2, Reg::X3].map(|reg| m.reg_read(reg))
}

/// The firmware is PSCI 1.2 (DEN 0022E) and SMCCC 1.5 (DEN 0028F), and
/// says so the ways each specification asks: `SMCCC_VERSION` is discovered
/// through `PSCI_FEATURES` (DEN 0022E §5.15.2), `SMCCC_ARCH_FEATURES`
/// answers SUCCESS for itself and `SMCCC_VERSION` (DEN 0028F §7.3.6), and a
/// function it does not implement, `SMCCC_ARCH_SOC_ID`, or an ID outside
/// the Arm Architecture ranges, is NOT_SUPPORTED.
#[test]
fn the_firmware_is_psci_1_2_and_smccc_1_5() {
    for (x0, x1, expected, what) in [
        (psci::VERSION, 0, psci::VERSION_1_2, "PSCI_VERSION"),
        (smccc::VERSION, 0, smccc::VERSION_1_5, "SMCCC_VERSION"),
        (
            psci::FEATURES,
            smccc::VERSION,
            psci::SUCCESS,
            "PSCI_FEATURES of SMCCC_VERSION",
        ),
        (
            psci::FEATURES,
            smccc::ARCH_FEATURES,
            psci::NOT_SUPPORTED,
            "not a PSCI function",
        ),
        (
            smccc::ARCH_FEATURES,
            smccc::VERSION,
            psci::SUCCESS,
            "of SMCCC_VERSION",
        ),
        (
            smccc::ARCH_FEATURES,
            smccc::ARCH_FEATURES,
            psci::SUCCESS,
            "of itself",
        ),
        (
            smccc::ARCH_FEATURES,
            smccc::ARCH_SOC_ID,
            psci::NOT_SUPPORTED,
            "of SOC_ID",
        ),
        (
            smccc::ARCH_FEATURES,
            psci::VERSION,
            psci::NOT_SUPPORTED,
            "of a PSCI function",
        ),
        (
            smccc::ARCH_SOC_ID,
            0,
            psci::NOT_SUPPORTED,
            "SMCCC_ARCH_SOC_ID",
        ),
    ] {
        assert_eq!(psci_call(x0, x1, 0), expected, "{what}");
    }
}

/// A Fast Call's bit 16, from SMCCC 1.3 the hint that the caller holds no
/// SVE live state, "is not part of the function identification" (DEN
/// 0028F §2.5.1): a call with it set is the call without it, and so is a
/// function ID with it that `PSCI_FEATURES` or `SMCCC_ARCH_FEATURES` asks
/// about.
#[test]
fn the_sve_hint_bit_is_no_part_of_a_function_identifier() {
    const HINT: u64 = 1 << 16;
    for (x0, x1, expected, what) in [
        (HINT | psci::VERSION, 0, psci::VERSION_1_2, "PSCI_VERSION"),
        (
            HINT | smccc::VERSION,
            0,
            smccc::VERSION_1_5,
            "SMCCC_VERSION",
        ),
        (
            HINT | psci::FEATURES,
            HINT | smccc::VERSION,
            psci::SUCCESS,
            "PSCI_FEATURES",
        ),
        (
            HINT | smccc::ARCH_FEATURES,
            HINT | smccc::ARCH_WORKAROUND_1,
            1,
            "SMCCC_ARCH_FEATURES",
        ),
    ] {
        assert_eq!(psci_call(x0, x1, 0), expected, "{what}");
    }
}

/// Each [`SpectreReport`] answers `SMCCC_ARCH_FEATURES` for the three
/// workarounds as DEN 0028F §7.5.2, §7.6.2 and §7.7.2 define the answer,
/// and a workaround it implements is a call with no effect, every register
/// as it was; one it does not is an unknown function.
#[test]
fn each_spectre_report_answers_and_implements_the_workarounds() {
    use smccc::{ARCH_WORKAROUND_1 as WA1, ARCH_WORKAROUND_2 as WA2, ARCH_WORKAROUND_3 as WA3};
    let cases = [
        (SpectreReport::NotRequired, [1, smccc::NOT_REQUIRED, 1]),
        (SpectreReport::Mitigated, [0, 0, 0]),
        (SpectreReport::Unreported, [psci::NOT_SUPPORTED; 3]),
    ];
    for (report, discovery) in cases {
        for (workaround, answer) in [WA1, WA2, WA3].into_iter().zip(discovery) {
            assert_eq!(
                firmware_call(report, [smccc::ARCH_FEATURES, workaround, 0, 0])[0],
                answer,
                "{report:?}: discovery of {workaround:#x}"
            );
            let args = [workaround, 1, 0x2222, 0x3333];
            let after = firmware_call(report, args);
            if (answer as i64) >= 0 {
                assert_eq!(after, args, "{report:?}: {workaround:#x} has no effect");
            } else {
                assert_eq!(
                    after,
                    [psci::NOT_SUPPORTED, 1, 0x2222, 0x3333],
                    "{report:?}: {workaround:#x} is not implemented"
                );
            }
        }
    }
    assert_eq!(SpectreReport::DEFAULT, SpectreReport::NotRequired);
}

/// Each report's name reads back as that report, and a name none has is
/// refused with the names there are.
#[test]
fn spectre_reports_are_named() {
    for report in SpectreReport::ALL {
        assert_eq!(report.name().parse(), Ok(report));
    }
    let refused = "affected"
        .parse::<SpectreReport>()
        .expect_err("no such report");
    assert_eq!(
        refused.to_string(),
        "no speculation report has that name; the reports are \"not-required\", \
         \"mitigated\", \"unreported\""
    );
}

/// `CPU_ON` against this processor, whose affinity value is 0
/// (DEN 0022D §5.6.2): an invalid target is INVALID_PARAMETERS, then an
/// entry point that cannot be executed is INVALID_ADDRESS, and the valid
/// call finds the processor ALREADY_ON.
#[test]
fn psci_cpu_on_checks_the_target_then_the_entry_point() {
    for (x1, x2, expected) in [
        (0, RAM_BASE, psci::ALREADY_ON),
        (0, RAM_BASE + 2, psci::INVALID_ADDRESS),
        (1, RAM_BASE + 2, psci::INVALID_PARAMETERS),
        (0x8000_0000, RAM_BASE, psci::INVALID_PARAMETERS),
        (1, RAM_BASE, psci::INVALID_PARAMETERS),
    ] {
        assert_eq!(
            psci_call(psci::CPU_ON_64, x1, x2),
            expected,
            "x1 {x1:#x}, x2 {x2:#x}"
        );
    }
}

/// `AFFINITY_INFO` (DEN 0022D §5.1.5, §5.7): level `n` ignores Aff0 to
/// Aff(`n`-1), an instance this processor is in is ON, and any other
/// instance, or a level above 3, is not present: INVALID_PARAMETERS. The
/// must-be-zero bits 31:24 (§5.1.4) count at every level.
#[test]
fn psci_affinity_info_names_an_instance_at_its_level() {
    for (x1, level, expected) in [
        (0, 0, psci::ON),
        (1, 0, psci::INVALID_PARAMETERS),
        (0x8000_0000, 0, psci::INVALID_PARAMETERS),
        (0x8000_0000, 1, psci::INVALID_PARAMETERS),
        (0xFF, 1, psci::ON),
        (0x100, 1, psci::INVALID_PARAMETERS),
        (0xFFFF, 2, psci::ON),
        (0x1_0000, 2, psci::INVALID_PARAMETERS),
        (0xFF_FFFF, 3, psci::ON),
        (0x0100_0000, 3, psci::INVALID_PARAMETERS),
        (0x1_0000_0000, 3, psci::INVALID_PARAMETERS),
        (0, 4, psci::INVALID_PARAMETERS),
    ] {
        assert_eq!(
            psci_call(psci::AFFINITY_INFO_64, x1, level),
            expected,
            "x1 {x1:#x}, level {level}"
        );
    }
}

/// The SMC Calling Convention's registers (DEN 0028B §2.6, §2.7, §3.1
/// Table 3-1): the function identifier is W0 in both conventions and an
/// SMC32 call's arguments are W1-W3, so the upper halves of X0-X2 take no
/// part; an SMC64 call's arguments are whole X registers, except a
/// parameter DEN 0022D declares `uint32`.
#[test]
fn psci_reads_w_registers_where_the_calling_convention_passes_them() {
    const HIGH: u64 = 0xDEAD_BEEF_0000_0000;
    for (x0, x1, x2, expected) in [
        (HIGH | psci::VERSION, 0, 0, psci::VERSION_1_2),
        (psci::FEATURES, HIGH | psci::CPU_ON_64, 0, psci::SUCCESS),
        (psci::CPU_ON_32, HIGH, RAM_BASE, psci::ALREADY_ON),
        (psci::CPU_ON_64, HIGH, RAM_BASE, psci::INVALID_PARAMETERS),
        (psci::AFFINITY_INFO_32, HIGH, HIGH, psci::ON),
        (psci::AFFINITY_INFO_64, HIGH, 0, psci::INVALID_PARAMETERS),
        (psci::AFFINITY_INFO_64, 0, HIGH, psci::ON),
        (psci::AFFINITY_INFO_64, 1, HIGH | 1, psci::ON),
    ] {
        assert_eq!(
            psci_call(x0, x1, x2),
            expected,
            "x0 {x0:#x}, x1 {x1:#x}, x2 {x2:#x}"
        );
    }
}

#[test]
fn a_store_exclusive_the_monitor_does_not_hold_fails_without_translating() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STXR);
    mmu_on(&mut m, 25, 4, RAM_BASE + 0x2_0000);
    // Unmapped: the upper half's level 1 table is empty.
    m.reg_write(Reg::X1, UPPER_VA);
    m.reg_write(Reg::X3, 7);
    step1(&mut m);
    assert_eq!(m.reg_read(Reg::X3), 1, "the store failed");
    assert_eq!(m.reg_read(Reg::Pc), RAM_BASE + 4, "and took no exception");
}

#[test]
fn a_misaligned_store_exclusive_faults_whatever_the_monitor_holds() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &STXR);
    m.reg_write(Reg::X1, RAM_BASE + 0x1004);
    step1(&mut m);
    let esr = m.sysreg_read(SysReg::ESR_EL1).expect("ESR_EL1");
    assert_eq!(esr >> 26, EC_DATA_ABORT_SAME_EL, "ESR {esr:#x}");
    assert_eq!(esr & 0x3F, 0x21, "alignment fault");
}

#[test]
fn a_store_exclusive_after_its_load_exclusive_succeeds() {
    let mut ram = vec![0u8; RAM_LEN];
    let mut m = bare(&mut ram);
    load(&mut m, &LDXR_STXR);
    m.mem_write(RAM_BASE + 0x1000, &0x1234u64.to_le_bytes())
        .expect("in RAM");
    m.reg_write(Reg::X1, RAM_BASE + 0x1000);
    m.reg_write(Reg::X3, 7);
    let out = m.step(RunBudget::Instructions(2));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    assert_eq!(m.reg_read(Reg::X3), 0, "the store succeeded");
}
