//! System register accesses as a guest sees them (R9): what an `MRS`
//! reads, what an `MSR` keeps, and the exception an access takes, as
//! `ESR_EL1` reports it.

use rusty_box_core::SnapError;

use super::super::features::{Feature, Implementation};
use super::super::step::{exec, processor, Taken, CODE, UNDEFINED};
use super::super::{Cpu, El, Pstate};

/// The words are GNU as's (`-march=armv9.2-a+predres`, objdump -d).
const MRS_MIDR: u32 = 0xD538_0000; // mrs x0, midr_el1
const MRS_ID_AA64ISAR2: u32 = 0xD538_0640; // mrs x0, id_aa64isar2_el1
const MRS_S3_1_C0_C0_3: u32 = 0xD539_0060; // unallocated, op1 = 1
const MRS_S3_3_C0_C0_0: u32 = 0xD53B_0000; // unallocated, op1 = 3
const MRS_S3_0_C0_C3_7: u32 = 0xD538_03E0; // reserved: RAZ
const MSR_MIDR: u32 = 0xD518_0000; // msr midr_el1, x0
const MRS_CTR: u32 = 0xD53B_0020; // mrs x0, ctr_el0
const MRS_SCTLR: u32 = 0xD538_1000; // mrs x0, sctlr_el1
const MRS_CNTPCTSS: u32 = 0xD53B_E0A0; // mrs x0, cntpctss_el0
const MRS_CNTVCTSS: u32 = 0xD53B_E0C0; // mrs x0, cntvctss_el0
const CFP_RCTX: u32 = 0xD50B_7380; // cfp rctx, x0
const DVP_RCTX: u32 = 0xD50B_73A0; // dvp rctx, x0
const CPP_RCTX: u32 = 0xD50B_73E0; // cpp rctx, x0

/// The words are GNU as's (`-march=armv9.2-a+ras`, objdump -d).
const ESB: u32 = 0xD503_221F;
const MRS_ERRIDR: u32 = 0xD538_5300; // mrs x0, erridr_el1
const MSR_ERRIDR: u32 = 0xD518_5300;
const MRS_ERRSELR: u32 = 0xD538_5320;
const MSR_ERRSELR: u32 = 0xD518_5320;
const MRS_ERXSTATUS: u32 = 0xD538_5440;
const MSR_ERXSTATUS: u32 = 0xD518_5440;
const MRS_ERXMISC2: u32 = 0xD538_5540;
const MRS_ERXPFGF: u32 = 0xD538_5480;
const MRS_DISR: u32 = 0xD538_C120; // mrs x0, disr_el1
const MSR_DISR: u32 = 0xD518_C120;
const MRS_DBGAUTHSTATUS: u32 = 0xD530_7EC0; // mrs x0, dbgauthstatus_el1
const MSR_DBGAUTHSTATUS: u32 = 0xD510_7EC0;
const MRS_MDRAR: u32 = 0xD530_1000; // mrs x0, mdrar_el1
const MSR_MDRAR: u32 = 0xD510_1000;
const MRS_RVBAR: u32 = 0xD538_C020; // mrs x0, rvbar_el1
const MSR_RVBAR: u32 = 0xD518_C020;
const MRS_RMR: u32 = 0xD538_C040; // mrs x0, rmr_el1
const MSR_RMR: u32 = 0xD518_C040;
const MRS_DBGCLAIMSET: u32 = 0xD530_78C0; // mrs x0, dbgclaimset_el1
const MSR_DBGCLAIMSET: u32 = 0xD510_78C0;
const MRS_DBGCLAIMCLR: u32 = 0xD530_79C0; // mrs x0, dbgclaimclr_el1
const MSR_DBGCLAIMCLR: u32 = 0xD510_79C0;
const MRS_OSDLR: u32 = 0xD530_1380; // mrs x0, osdlr_el1
const MSR_OSDLR: u32 = 0xD510_1380;
const MRS_ID_AA64DFR0: u32 = 0xD538_0500; // mrs x0, id_aa64dfr0_el1
const LDR: u32 = 0xF940_0020; // ldr x0, [x1]

/// `SCTLR_EL1.UCT` and `EnRCTX`; `CNTKCTL_EL1.EL0PCTEN` and `EL0VCTEN`.
const UCT: u64 = 1 << 15;
const ENRCTX: u64 = 1 << 10;
const EL0PCTEN: u64 = 1 << 0;
const EL0VCTEN: u64 = 1 << 1;

/// A trapped `MRS`/`MSR`/`SYS`: EC 0x18 and the ISS naming the encoding.
const fn sysreg_trap(iss: u64) -> Taken {
    Taken { ec: 0x18, iss }
}

/// `MRS X0, MDCCSR_EL0`, `MRS X0, DBGDTR_EL0`, `MRS X0, DBGDTRRX_EL0`.
const MRS_MDCCSR: u32 = 0xD533_0100;
const MRS_DBGDTR: u32 = 0xD533_0400;
const MRS_DBGDTRRX: u32 = 0xD533_0500;
/// `MSR DBGDTR_EL0, X0`, `MSR DBGDTRTX_EL0, X0`.
const MSR_DBGDTR: u32 = 0xD513_0400;
const MSR_DBGDTRTX: u32 = 0xD513_0500;
/// The `MSR` form of `MDCCSR_EL0`'s encoding, which is read-only.
const MSR_MDCCSR: u32 = 0xD513_0100;

fn at_el0(cpu: &mut Cpu) {
    cpu.pstate = Pstate {
        el: El::El0,
        sp_sel: false,
        ..Pstate::RESET
    };
}

/// The Debug Communications Channel with no external debugger on its far
/// end (Arm ARM, DDI 0487, chapter H4, and the `MDCCSR_EL0`, `DBGDTR_EL0`,
/// `DBGDTRRX_EL0` and `DBGDTRTX_EL0` descriptions): nothing fills DTRRX, so
/// with RXfull clear `DBGDTR_EL0` and `DBGDTRRX_EL0` read UNKNOWN, zero
/// here, and `MDCCSR_EL0` reads 0 until a word written to `DBGDTR_EL0` or
/// `DBGDTRTX_EL0` fills DTRTX and sets TXfull, which nothing empties. EL0
/// reaches all four unless `MDSCR_EL1.TDCC` is set, which traps EL0's
/// accesses to EL1 (EC 0x18, the ISS naming the register); EL1's go
/// through. `MDCCSR_EL0` cannot be written.
#[test]
fn the_debug_communications_channel_has_no_far_end_and_tdcc_traps_el0() {
    // ISS: op0 2, op2 0, op1 3, CRn 0, Rt 0, CRm and the direction.
    let reads = [
        (MRS_MDCCSR, 0x0020_C003),
        (MRS_DBGDTR, 0x0020_C009),
        (MRS_DBGDTRRX, 0x0020_C00B),
    ];
    let writes = [(MSR_DBGDTR, 0x0020_C008), (MSR_DBGDTRTX, 0x0020_C00A)];
    const TXFULL: u64 = 1 << 29;
    for el0 in [false, true] {
        let mut cpu = processor(Implementation::CORTEX_A57);
        if el0 {
            at_el0(&mut cpu);
        }
        for (insn, _) in reads {
            cpu.x[0] = 0xDEAD;
            assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}, EL0 {el0}");
            assert_eq!(cpu.x[0], 0, "{insn:#010x}, EL0 {el0}");
        }
        for (insn, _) in writes {
            cpu.x[0] = 0x1234_5678;
            assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}, EL0 {el0}");
            cpu.x[0] = 0xDEAD;
            assert_eq!(exec(&mut cpu, MRS_MDCCSR), None, "EL0 {el0}");
            assert_eq!(cpu.x[0], TXFULL, "TXfull after {insn:#010x}, EL0 {el0}");
        }
        assert_eq!(exec(&mut cpu, MSR_MDCCSR), Some(UNDEFINED), "EL0 {el0}");
    }

    let mut cpu = processor(Implementation::CORTEX_A57);
    at_el0(&mut cpu);
    cpu.sys.mdscr_el1 = 1 << 12;
    for (insn, iss) in reads.into_iter().chain(writes) {
        assert_eq!(
            exec(&mut cpu, insn),
            Some(Taken { ec: 0x18, iss }),
            "{insn:#010x} at EL0"
        );
    }
    cpu.pstate = Pstate::RESET;
    for (insn, _) in reads.into_iter().chain(writes) {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL1");
    }
}

/// FEAT_IDST: a read of the feature ID space that would be UNDEFINED traps
/// with EC 0x18 instead, the ISS naming the register: EL0's reads of the
/// ID registers (each accessor's `AArch64_SystemAccessTraptoEL1orEL2`), and
/// an unallocated encoding at any EL (`UnimplementedIDRegister`). The
/// reserved encodings EL1 reads as zero stay so, a write stays UNDEFINED,
/// and so does a read outside the space. Without FEAT_IDST every such read
/// is UNDEFINED.
#[test]
fn idst_traps_the_reads_of_the_id_space_that_would_be_undefined() {
    for imp in [Implementation::EVERY_FEATURE, Implementation::CORTEX_A57] {
        let idst = imp.has(Feature::Idst);
        let refused = |iss| {
            if idst {
                Some(sysreg_trap(iss))
            } else {
                Some(UNDEFINED)
            }
        };
        let mut cpu = processor(imp);
        cpu.sys.sctlr_el1 |= UCT;

        assert_eq!(
            exec(&mut cpu, MRS_S3_1_C0_C0_3),
            refused(0x0036_4001),
            "IDST {idst}"
        );
        assert_eq!(
            exec(&mut cpu, MRS_S3_3_C0_C0_0),
            refused(0x0030_C001),
            "IDST {idst}"
        );
        cpu.x[0] = 0xDEAD;
        assert_eq!(exec(&mut cpu, MRS_S3_0_C0_C3_7), None);
        assert_eq!(cpu.x[0], 0, "a reserved ID register is RAZ at EL1");
        assert_eq!(exec(&mut cpu, MRS_ID_AA64ISAR2), None);
        assert_eq!(
            cpu.x[0] & 0xF,
            if imp.has(Feature::WfxT) { 2 } else { 0 },
            "ISAR2.WFxT"
        );
        assert_eq!(
            exec(&mut cpu, MSR_MIDR),
            Some(UNDEFINED),
            "IDST {idst}: writes are not trapped"
        );

        at_el0(&mut cpu);
        assert_eq!(
            exec(&mut cpu, MRS_MIDR),
            refused(0x0030_0001),
            "IDST {idst}"
        );
        assert_eq!(
            exec(&mut cpu, MRS_ID_AA64ISAR2),
            refused(0x0034_000D),
            "IDST {idst}"
        );
        assert_eq!(
            exec(&mut cpu, MRS_S3_3_C0_C0_0),
            refused(0x0030_C001),
            "IDST {idst}"
        );
        assert_eq!(
            exec(&mut cpu, MRS_SCTLR),
            Some(UNDEFINED),
            "IDST {idst}: outside the ID space"
        );
        assert_eq!(exec(&mut cpu, MSR_MIDR), Some(UNDEFINED), "IDST {idst}");
        assert_eq!(
            exec(&mut cpu, MRS_CTR),
            None,
            "IDST {idst}: EL0 may read CTR_EL0 with UCT"
        );
    }
}

/// FEAT_SPECRES: `CFP`, `DVP` and `CPP RCTX` complete at EL1 and, with
/// `SCTLR_EL1.EnRCTX`, at EL0; without it EL0's trap with EC 0x18, the ISS
/// naming the operation as a write. Without FEAT_SPECRES they are
/// UNDEFINED.
#[test]
fn the_prediction_restriction_instructions_need_feat_specres_and_enrctx_at_el0() {
    let ops = [
        (CFP_RCTX, 0x0018_DC06),
        (DVP_RCTX, 0x001A_DC06),
        (CPP_RCTX, 0x001E_DC06),
    ];
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for (insn, _) in ops {
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL1");
    }
    at_el0(&mut cpu);
    for (insn, iss) in ops {
        assert_eq!(
            exec(&mut cpu, insn),
            Some(sysreg_trap(iss)),
            "{insn:#010x} at EL0"
        );
    }
    cpu.sys.sctlr_el1 |= ENRCTX;
    for (insn, _) in ops {
        assert_eq!(
            exec(&mut cpu, insn),
            None,
            "{insn:#010x} at EL0 with EnRCTX"
        );
    }
    let mut v80 = processor(Implementation::CORTEX_A57);
    for (insn, _) in ops {
        assert_eq!(
            exec(&mut v80, insn),
            Some(UNDEFINED),
            "{insn:#010x} without FEAT_SPECRES"
        );
    }
}

/// FEAT_ECV's `CNTPCTSS_EL0` and `CNTVCTSS_EL0` read the count as
/// `CNTPCT_EL0` and `CNTVCT_EL0` do, under the same EL0 enables; without
/// FEAT_ECV they are UNDEFINED.
#[test]
fn the_self_synchronized_counter_views_need_feat_ecv() {
    let views = [
        (MRS_CNTPCTSS, EL0PCTEN, 0x003A_F801),
        (MRS_CNTVCTSS, EL0VCTEN, 0x003C_F801),
    ];
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.wait_until(rusty_box_core::VmInstant::from_ticks(12_345));
    for (insn, _, _) in views {
        let count = cpu.counter().get();
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL1");
        assert_eq!(cpu.x[0], count);
    }
    at_el0(&mut cpu);
    for (insn, enable, iss) in views {
        cpu.sys.cntkctl_el1 = 0;
        assert_eq!(
            exec(&mut cpu, insn),
            Some(sysreg_trap(iss)),
            "{insn:#010x} at EL0"
        );
        cpu.sys.cntkctl_el1 = enable;
        let count = cpu.counter().get();
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x} at EL0, enabled");
        assert_eq!(cpu.x[0], count);
    }
    let mut v80 = processor(Implementation::CORTEX_A57);
    for (insn, _, _) in views {
        assert_eq!(
            exec(&mut v80, insn),
            Some(UNDEFINED),
            "{insn:#010x} without FEAT_ECV"
        );
    }
}

/// FEAT_RAS on a processor with no error records and no source of SError
/// interrupts: ESB completes with nothing to defer; `ERRIDR_EL1` reports no
/// records (NUM = 0) and cannot be written, and with none `ERRSELR_EL1`
/// and the `ERX*` registers are UNDEFINED (QEMU's `minimal_ras_reginfo`);
/// `DISR_EL1` keeps A, IDS and bits 23:0. EL0 reaches none of them, and a
/// processor without FEAT_RAS has none, ESB being a NOP there.
#[test]
fn ras_has_no_error_records_and_a_deferred_error_register() {
    for imp in [Implementation::EVERY_FEATURE, Implementation::CORTEX_A57] {
        let ras = imp.has(Feature::Ras);
        let mut cpu = processor(imp);
        cpu.x[0] = 0x1234;
        assert_eq!(exec(&mut cpu, ESB), None, "RAS {ras}");
        assert_eq!(cpu.x[0], 0x1234);

        cpu.x[0] = 0xDEAD;
        let erridr = exec(&mut cpu, MRS_ERRIDR);
        assert_eq!(erridr, if ras { None } else { Some(UNDEFINED) });
        if ras {
            assert_eq!(cpu.x[0], 0, "ERRIDR_EL1.NUM = 0");
        }
        for insn in [
            MSR_ERRIDR,
            MRS_ERRSELR,
            MSR_ERRSELR,
            MRS_ERXSTATUS,
            MSR_ERXSTATUS,
            MRS_ERXMISC2,
            MRS_ERXPFGF,
        ] {
            assert_eq!(
                exec(&mut cpu, insn),
                Some(UNDEFINED),
                "{insn:#010x}, RAS {ras}"
            );
        }

        cpu.x[0] = u64::MAX;
        assert_eq!(
            exec(&mut cpu, MSR_DISR),
            if ras { None } else { Some(UNDEFINED) }
        );
        cpu.x[0] = 0;
        assert_eq!(
            exec(&mut cpu, MRS_DISR),
            if ras { None } else { Some(UNDEFINED) }
        );
        if ras {
            assert_eq!(cpu.x[0], 0x81FF_FFFF, "A, IDS and bits 23:0");
            let mut image = std::vec::Vec::new();
            cpu.save(&mut image).expect("save");
            let mut restored = processor(imp);
            restored.restore(&mut image.as_slice()).expect("restore");
            assert_eq!(restored.sys.disr_el1, 0x81FF_FFFF);
            let mut v80 = processor(Implementation::CORTEX_A57);
            assert_eq!(
                v80.restore(&mut image.as_slice()),
                Err(SnapError::Invalid(
                    "snapshot of a processor implementing other features"
                ))
            );
        }

        at_el0(&mut cpu);
        assert_eq!(exec(&mut cpu, ESB), None, "ESB at EL0");
        for insn in [MRS_ERRIDR, MRS_DISR, MSR_DISR] {
            assert_eq!(
                exec(&mut cpu, insn),
                Some(UNDEFINED),
                "{insn:#010x} at EL0, RAS {ras}"
            );
        }
    }

    // A processor without FEAT_RAS refuses a snapshot holding DISR_EL1 bits.
    let mut v80 = processor(Implementation::CORTEX_A57);
    v80.sys.disr_el1 = 1 << 31;
    let mut image = std::vec::Vec::new();
    v80.save(&mut image).expect("save");
    assert_eq!(
        processor(Implementation::CORTEX_A57).restore(&mut image.as_slice()),
        Err(SnapError::Invalid(
            "snapshot's DISR_EL1 holds bits this processor cannot set"
        ))
    );
}

/// A synchronous external abort (here a load from no memory, the MMU off)
/// reports the same syndrome with FEAT_RAS as without: SET, which FEAT_RAS
/// adds, names the Recoverable state (0b00) this model chooses.
#[test]
fn an_external_abort_reports_a_recoverable_error_state() {
    for imp in [Implementation::EVERY_FEATURE, Implementation::CORTEX_A57] {
        let mut cpu = processor(imp);
        cpu.x[1] = 0x5_0000;
        assert_eq!(
            exec(&mut cpu, LDR),
            Some(Taken {
                ec: 0x25,
                iss: 0x10
            }),
            "FSC 0x10 and SET 0b00, RAS {}",
            imp.has(Feature::Ras)
        );
    }
}

/// `DBGAUTHSTATUS_EL1` as Arm's `UpdateDbgAuthStatus` sets it on a
/// Non-secure-only processor with the external debugger's `DBGEN` and
/// `NIDEN` low: Non-secure invasive and non-invasive debug implemented and
/// disabled (0x0A), non-invasive enabled with FEAT_Debugv8p4 (0x0E), no
/// Secure, Realm or Root debug. It cannot be written, and EL0 cannot read
/// it.
#[test]
fn dbgauthstatus_reports_non_secure_debug_implemented() {
    for (imp, value) in [
        (Implementation::CORTEX_A57, 0x0A),
        (Implementation::EVERY_FEATURE, 0x0E),
    ] {
        let mut cpu = processor(imp);
        assert_eq!(exec(&mut cpu, MRS_DBGAUTHSTATUS), None);
        assert_eq!(
            cpu.x[0],
            value,
            "FEAT_Debugv8p4 {}",
            imp.has(Feature::Debugv8p4)
        );
        assert_eq!(exec(&mut cpu, MSR_DBGAUTHSTATUS), Some(UNDEFINED));
        at_el0(&mut cpu);
        assert_eq!(exec(&mut cpu, MRS_DBGAUTHSTATUS), Some(UNDEFINED));
    }
}

/// EL1 is the highest EL, so it has the reset management registers (Arm's
/// data: `IsHighestEL(EL1)`): `RVBAR_EL1` reads the address the last reset
/// started execution from, and `RMR_EL1` reads AA64 as one and keeps the
/// Warm reset request RR, which resets nothing here. `MDRAR_EL1` reads no
/// ROM table (Valid 0b00). The two read-only registers cannot be written,
/// and EL0 reaches none of the three.
#[test]
fn the_reset_management_and_rom_table_registers_read_at_el1() {
    for imp in [Implementation::CORTEX_A57, Implementation::ARMV9P2] {
        let mut cpu = processor(imp);
        assert_eq!(exec(&mut cpu, MRS_RVBAR), None);
        assert_eq!(cpu.x[0], CODE, "{imp:?}: the reset address");
        assert_eq!(exec(&mut cpu, MRS_RMR), None);
        assert_eq!(cpu.x[0], 0b01, "{imp:?}: AA64");
        cpu.x[0] = !0;
        assert_eq!(exec(&mut cpu, MSR_RMR), None, "{imp:?}");
        assert_eq!(exec(&mut cpu, MRS_RMR), None);
        assert_eq!(cpu.x[0], 0b11, "{imp:?}: AA64 and RR, nothing else");
        assert_eq!(exec(&mut cpu, MRS_MDRAR), None);
        assert_eq!(cpu.x[0], 0, "{imp:?}: no ROM table");
        assert_eq!(exec(&mut cpu, MSR_RVBAR), Some(UNDEFINED));
        assert_eq!(exec(&mut cpu, MSR_MDRAR), Some(UNDEFINED));
        at_el0(&mut cpu);
        for insn in [MRS_RVBAR, MRS_RMR, MSR_RMR, MRS_MDRAR] {
            assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
        }
    }
}

/// `DBGCLAIMSET_EL1` reads as one for each of the eight claim tags, all of
/// them implemented, whatever the tags hold; `DBGCLAIMCLR_EL1` reads the
/// tags, which ones written to the first set and to the second clear.
#[test]
fn the_claim_tag_set_register_reads_as_ones() {
    let mut cpu = processor(Implementation::CORTEX_A57);
    // The write, the value written, and the tags after it: bits above the
    // eight tags are RAZ/WI.
    for (insn, value, tags) in [
        (MSR_DBGCLAIMSET, 0xFF_FFA5, 0xA5),
        (MSR_DBGCLAIMCLR, 0xFF, 0x00),
    ] {
        cpu.x[0] = value;
        assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
        assert_eq!(exec(&mut cpu, MRS_DBGCLAIMCLR), None);
        assert_eq!(cpu.x[0], tags, "the tags after {insn:#010x}");
        assert_eq!(exec(&mut cpu, MRS_DBGCLAIMSET), None);
        assert_eq!(cpu.x[0], 0xFF, "RAO after {insn:#010x}");
    }
}

/// FEAT_DoubleLock is the Cortex-A57's and not the Armv9.2 model's, as
/// Armv9.0 forbids it (`v9Ap0 ==> !FEAT_DoubleLock`):
/// `ID_AA64DFR0_EL1.DoubleLock` reads 0 and 0b1111, and `OSDLR_EL1.DLK`
/// keeps a one written to it only with the feature, being RES0 without.
#[test]
fn only_the_cortex_a57_has_the_os_double_lock() {
    for (imp, double_lock) in [
        (Implementation::CORTEX_A57, true),
        (Implementation::ARMV9P2, false),
    ] {
        let mut cpu = processor(imp);
        assert_eq!(exec(&mut cpu, MRS_ID_AA64DFR0), None);
        let field = (cpu.x[0] >> 36) & 0xF;
        assert_eq!(field, if double_lock { 0 } else { 0xF }, "{imp:?}");
        cpu.x[0] = !0;
        assert_eq!(exec(&mut cpu, MSR_OSDLR), None);
        assert_eq!(exec(&mut cpu, MRS_OSDLR), None);
        assert_eq!(cpu.x[0], u64::from(double_lock), "{imp:?}: DLK");
    }
}
