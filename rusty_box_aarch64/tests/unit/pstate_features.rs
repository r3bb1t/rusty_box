//! FEAT_PAN, FEAT_PAN2, FEAT_PAN3, FEAT_UAO, FEAT_DIT, FEAT_SSBS and
//! FEAT_SSBS2, and the encodings of FEAT_FlagM, FEAT_FlagM2 and FEAT_SB on
//! processors without them, as a guest sees them (R9). What FlagM, FlagM2
//! and SB compute is in QEMU's vectors (`tests/vectors/qemu_max.txt`, the
//! `flagm_*` and `sb` families); this file holds what those vectors cannot
//! reach: EL1, the MMU, exceptions, and the processors without a feature.
//!
//! The rules are the Arm ARM's pseudocode: `AArch64_S1DirectBasePermissions`
//! (PAN and EPAN), `CreateAccDescGPR`, `CreateAccDescDCZero` and
//! `CreateAccDescAT` (which accesses PAN applies to),
//! `AArch64_IsUnprivAccessPriv` (UAO), `AArch64_TakeException`,
//! `GetPSRFromPSTATE` and `SetPSTATEFromPSR`, the pages of MSR (immediate)
//! and of the PAN, UAO, DIT and SSBS registers, and `AArch64_TakeReset`.
//!
//! The words are GNU as's (`-march=armv8.7-a+sb+ssbs+flagm`, objdump -d).

use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::features::{Feature, Features, Implementation};
use super::{id, Cpu, CpuConfig, El, Pstate, Step};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const LDR: u32 = 0xF940_0022; // ldr x2, [x1]
const STR: u32 = 0xF900_0022; // str x2, [x1]
const LDTR: u32 = 0xF840_0822; // ldtr x2, [x1]
const STTR: u32 = 0xF800_0822; // sttr x2, [x1]
const LDADD: u32 = 0xF823_0022; // ldadd x3, x2, [x1]
const LDXR: u32 = 0xC85F_7C22; // ldxr x2, [x1]
const DC_ZVA: u32 = 0xD50B_7421; // dc zva, x1
const MSR_PAN_1: u32 = 0xD500_419F;
const MSR_PAN_0: u32 = 0xD500_409F;
const MSR_UAO_1: u32 = 0xD500_417F;
const MSR_UAO_0: u32 = 0xD500_407F;
const MSR_DIT_1: u32 = 0xD503_415F;
const MSR_DIT_0: u32 = 0xD503_405F;
const MSR_SSBS_1: u32 = 0xD503_413F;
const MSR_SSBS_0: u32 = 0xD503_403F;
const MRS_PAN: u32 = 0xD538_4260; // mrs x0, pan
const MSR_PAN_X0: u32 = 0xD518_4260; // msr pan, x0
const MRS_UAO: u32 = 0xD538_4280;
const MSR_UAO_X0: u32 = 0xD518_4280;
const MRS_DIT: u32 = 0xD53B_42A0;
const MSR_DIT_X0: u32 = 0xD51B_42A0;
const MRS_SSBS: u32 = 0xD53B_42C0;
const MSR_SSBS_X0: u32 = 0xD51B_42C0;
const AT_S1E1R: u32 = 0xD508_7800; // at s1e1r, x0
const AT_S1E1W: u32 = 0xD508_7820;
const AT_S1E1RP: u32 = 0xD508_7900;
const AT_S1E1WP: u32 = 0xD508_7920;
const CFINV: u32 = 0xD500_401F;
const XAFLAG: u32 = 0xD500_403F;
const AXFLAG: u32 = 0xD500_405F;
const RMIF: u32 = 0xBA01_842A; // rmif x1, #3, #10
const SETF8: u32 = 0x3A00_082D; // setf8 w1
const SETF16: u32 = 0x3A00_482D;
const SB: u32 = 0xD503_30FF;
const SVC: u32 = 0xD400_0001;
const ERET: u32 = 0xD69F_03E0;
const MSR_SPSR: u32 = 0xD518_4000; // msr spsr_el1, x0
const MRS_SPSR: u32 = 0xD538_4000;

/// The translation tables: a level-1 table whose first entry is a level-2
/// table of four 2 MiB blocks, identity mapped, Normal memory: EL1's alone,
/// EL0 read/write, EL0 execute-only (no EL0 data access, EL0 may execute),
/// and EL0 read-only.
const L1: u64 = 0x1000;
const L2: u64 = 0x2000;
const VBAR: u64 = 0x8000;
/// Where an EL1 instruction executes, and EL1's own data.
const CODE: u64 = 0x4_0000;
const EL1_DATA: u64 = 0x5_0000;
const EL0_RW: u64 = 0x20_0100;
/// Where an EL0 instruction executes, in the execute-only block.
const EL0_CODE: u64 = 0x40_0000;
const EL0_XO: u64 = 0x40_1000;
const EL0_RO: u64 = 0x60_0100;
const RAM_BYTES: usize = 0x80_0000;

/// `SCTLR_EL1.SPAN`, `DSSBS` and `EPAN`.
const SPAN: u64 = 1 << 23;
const DSSBS: u64 = 1 << 44;
const EPAN: u64 = 1 << 57;

/// The classes this file's exceptions are taken with.
const UNKNOWN: u64 = 0x00;
const DATA_ABORT_SAME_EL: u64 = 0x25;
/// DFSC: a permission fault at level 2, where the blocks are.
const PERMISSION_L2: u64 = 0b00_1110;
const WNR: u64 = 1 << 6;

struct Ram(Vec<u8>);

impl PhysBus for Ram {
    fn read(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        _clock: VmClock,
    ) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        _clock: VmClock,
    ) -> Result<(), BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get_mut(a..a + n).ok_or(BusFault::Unmapped)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

/// A taken exception: `ESR_EL1`'s class and ISS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    ec: u64,
    iss: u64,
}

const fn permission_fault(write: bool) -> Option<Taken> {
    Some(Taken {
        ec: DATA_ABORT_SAME_EL,
        iss: PERMISSION_L2 | if write { WNR } else { 0 },
    })
}

const UNDEFINED: Option<Taken> = Some(Taken {
    ec: UNKNOWN,
    iss: 0,
});

struct Machine {
    cpu: Cpu,
    ram: Ram,
}

impl Machine {
    /// A processor implementing `imp` at EL1 with the MMU on over the
    /// tables above.
    fn new(imp: Implementation) -> Self {
        let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
        let mut cpu = Cpu::implementing(
            CpuConfig {
                ips,
                reset_pc: CODE,
            },
            Implementation {
                counter_hz: ips,
                ..imp
            },
        );
        let mut ram = Ram(std::vec![0; RAM_BYTES]);
        const AF: u64 = 1 << 10;
        const INNER_SHAREABLE: u64 = 3 << 8;
        const BLOCK: u64 = 0b01;
        const UXN: u64 = 1 << 54;
        const PXN: u64 = 1 << 53;
        const EL0_READ_WRITE: u64 = 0b01 << 6;
        const READ_ONLY_BOTH: u64 = 0b11 << 6;
        let attrs = AF | INNER_SHAREABLE | BLOCK;
        let descriptors = [
            (L1, L2 | 0b11),
            (L2, attrs | UXN),
            (L2 + 8, 0x20_0000 | attrs | EL0_READ_WRITE | UXN | PXN),
            (L2 + 16, 0x40_0000 | attrs),
            (L2 + 24, 0x60_0000 | attrs | READ_ONLY_BOTH | UXN | PXN),
        ];
        for (at, desc) in descriptors {
            let at = at as usize;
            ram.0[at..at + 8].copy_from_slice(&desc.to_le_bytes());
        }
        cpu.sys.mair_el1 = 0xFF;
        // T0SZ = 25, walks from level 1; TTBR1 walks disabled (EPD1).
        cpu.sys.tcr_el1 = 25 | (1 << 23);
        cpu.sys.ttbr0_el1 = L1;
        cpu.sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1;
        cpu.sys.vbar_el1 = VBAR;
        Self { cpu, ram }
    }

    /// Execute `insn` once, at EL0's code when the processor is at EL0 and
    /// at EL1's otherwise: the exception it took, if it took one. After an
    /// exception the processor is put back in the state it executed in.
    fn run(&mut self, insn: u32) -> Option<Taken> {
        let pstate = self.cpu.pstate;
        let at = if pstate.el == El::El0 { EL0_CODE } else { CODE };
        let taken = self.step_raw(at, insn);
        if taken.is_some() {
            self.cpu.pstate = pstate;
        }
        taken
    }

    /// Execute `insn` at `at`, leaving the processor as the step left it.
    fn step_raw(&mut self, at: u64, insn: u32) -> Option<Taken> {
        let a = at as usize;
        self.ram.0[a..a + 4].copy_from_slice(&insn.to_le_bytes());
        self.cpu.pc = at;
        self.cpu.sys.esr_el1 = 0x3F << 26;
        assert_eq!(
            self.cpu.step(&mut self.ram, &mut NoInstrumentation),
            Step::Retired
        );
        let esr = self.cpu.sys.esr_el1;
        if esr >> 26 == 0x3F {
            return None;
        }
        assert_eq!(
            self.cpu.pc & !0x7FF,
            VBAR,
            "{insn:#010x} took an exception to the vector table"
        );
        Some(Taken {
            ec: esr >> 26,
            iss: esr & 0x01FF_FFFF,
        })
    }

    fn at_el0(&mut self) {
        self.cpu.pstate.el = El::El0;
        self.cpu.pstate.sp_sel = false;
    }
}

/// The Cortex-A57's features and `extra`.
fn a57_with(extra: &[Feature]) -> Implementation {
    let mut features: Vec<Feature> = [
        Feature::Base,
        Feature::Fp,
        Feature::AdvSimd,
        Feature::Aes,
        Feature::Sha1,
        Feature::Sha256,
        Feature::Crc32,
    ]
    .to_vec();
    features.extend_from_slice(extra);
    Implementation {
        features: Features::of(&features),
        ..Implementation::CORTEX_A57
    }
}

/// With PSTATE.PAN set, an EL1 data access to memory EL0 can read or write
/// takes a permission fault: a load, a store, an atomic, an exclusive and
/// DC ZVA alike (each one's access descriptor has `pan` set). An atomic is
/// refused its read first, so its fault reports WnR clear. Memory only EL1
/// can reach is unaffected, and with PAN clear so is the rest.
#[test]
fn pan_refuses_el1_data_accesses_to_memory_el0_can_reach() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.pstate.pan = true;
    for (insn, write) in [
        (LDR, false),
        (STR, true),
        (LDADD, false),
        (LDXR, false),
        (DC_ZVA, true),
    ] {
        for (address, refused) in [(EL0_RW, true), (EL0_RO, true), (EL1_DATA, false)] {
            m.cpu.x[1] = address;
            let want = if refused {
                permission_fault(write)
            } else {
                None
            };
            assert_eq!(
                m.run(insn),
                want,
                "{insn:#010x} at {address:#x} with PAN set"
            );
        }
    }
    m.cpu.pstate.pan = false;
    for insn in [LDR, STR, LDADD, LDXR, DC_ZVA] {
        m.cpu.x[1] = EL0_RW;
        assert_eq!(m.run(insn), None, "{insn:#010x} with PAN clear");
    }
}

/// Memory EL0 can execute but not read or write counts as memory EL0 can
/// reach only with FEAT_PAN3 and SCTLR_EL1.EPAN set.
#[test]
fn epan_counts_memory_el0_can_execute() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.pstate.pan = true;
    m.cpu.x[1] = EL0_XO;
    assert_eq!(m.run(LDR), None, "EPAN clear");
    m.cpu.sys.sctlr_el1 |= EPAN;
    assert_eq!(m.run(LDR), permission_fault(false), "EPAN set");

    let mut pan2 = Machine::new(a57_with(&[Feature::Pan, Feature::Pan2]));
    pan2.cpu.pstate.pan = true;
    pan2.cpu.sys.sctlr_el1 |= EPAN;
    pan2.cpu.x[1] = EL0_XO;
    assert_eq!(pan2.run(LDR), None, "EPAN is RES0 without FEAT_PAN3");
}

/// LDTR and STTR at EL1 are checked against EL0's permissions, unless
/// PSTATE.UAO is set, when they are EL1's, which PAN applies to as to any
/// other access.
#[test]
fn uao_gives_ldtr_and_sttr_el1_permissions() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    for (insn, write) in [(LDTR, false), (STTR, true)] {
        m.cpu.pstate.uao = false;
        m.cpu.x[1] = EL1_DATA;
        assert_eq!(
            m.run(insn),
            permission_fault(write),
            "{insn:#010x}: EL0 cannot reach EL1's data"
        );
        m.cpu.x[1] = EL0_RW;
        assert_eq!(m.run(insn), None, "{insn:#010x}: EL0 can reach its own");
        m.cpu.pstate.pan = true;
        assert_eq!(
            m.run(insn),
            None,
            "{insn:#010x}: PAN takes no part in an EL0 check"
        );

        m.cpu.pstate.uao = true;
        assert_eq!(
            m.run(insn),
            permission_fault(write),
            "{insn:#010x}: UAO, then PAN"
        );
        m.cpu.pstate.pan = false;
        assert_eq!(m.run(insn), None, "{insn:#010x}: UAO without PAN");
        m.cpu.x[1] = EL1_DATA;
        assert_eq!(m.run(insn), None, "{insn:#010x}: UAO reaches EL1's data");
    }
}

/// MSR (immediate) writes PAN and UAO at EL1, DIT and SSBS at EL0 too, and
/// the registers of each read and write it in its SPSR bit. At EL0, MSR and
/// MRS of PAN and UAO are UNDEFINED.
#[test]
fn msr_and_mrs_reach_the_pstate_fields() {
    /// One PSTATE field's MSR (immediate) words, register accessors, SPSR
    /// bit, and how to read it.
    struct Field {
        set: u32,
        clear: u32,
        read: u32,
        write: u32,
        bit: u64,
        field: fn(&Pstate) -> bool,
    }
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    let fields = [
        Field {
            set: MSR_PAN_1,
            clear: MSR_PAN_0,
            read: MRS_PAN,
            write: MSR_PAN_X0,
            bit: 1 << 22,
            field: |p| p.pan,
        },
        Field {
            set: MSR_UAO_1,
            clear: MSR_UAO_0,
            read: MRS_UAO,
            write: MSR_UAO_X0,
            bit: 1 << 23,
            field: |p| p.uao,
        },
        Field {
            set: MSR_DIT_1,
            clear: MSR_DIT_0,
            read: MRS_DIT,
            write: MSR_DIT_X0,
            bit: 1 << 24,
            field: |p| p.dit,
        },
        Field {
            set: MSR_SSBS_1,
            clear: MSR_SSBS_0,
            read: MRS_SSBS,
            write: MSR_SSBS_X0,
            bit: 1 << 12,
            field: |p| p.ssbs,
        },
    ];
    for Field {
        set,
        clear,
        read,
        write,
        bit,
        field,
    } in fields
    {
        assert_eq!(m.run(set), None, "{set:#010x}");
        assert!(field(&m.cpu.pstate), "{set:#010x}");
        assert_eq!(m.run(read), None);
        assert_eq!(
            m.cpu.x[0], bit,
            "{read:#010x} reads the field alone, in its SPSR bit"
        );
        assert_eq!(m.run(clear), None);
        assert!(!field(&m.cpu.pstate), "{clear:#010x}");
        m.cpu.x[0] = u64::MAX;
        assert_eq!(m.run(write), None);
        assert!(field(&m.cpu.pstate), "{write:#010x}");
        m.cpu.x[0] = !bit;
        assert_eq!(m.run(write), None);
        assert!(
            !field(&m.cpu.pstate),
            "{write:#010x} reads only the field's bit"
        );
    }

    let mut el0 = Machine::new(Implementation::EVERY_FEATURE);
    el0.at_el0();
    for insn in [
        MSR_PAN_1, MSR_UAO_1, MRS_PAN, MSR_PAN_X0, MRS_UAO, MSR_UAO_X0,
    ] {
        assert_eq!(el0.run(insn), UNDEFINED, "{insn:#010x} at EL0");
    }
    assert_eq!(el0.run(MSR_DIT_1), None);
    assert_eq!(el0.run(MSR_SSBS_1), None);
    assert!(el0.cpu.pstate.dit && el0.cpu.pstate.ssbs);
    assert_eq!(el0.run(MRS_DIT), None);
    assert_eq!(el0.cpu.x[0], 1 << 24);
    assert_eq!(el0.run(MRS_SSBS), None);
    assert_eq!(el0.cpu.x[0], 1 << 12);
}

/// Every encoding of these features is UNDEFINED on a processor without
/// the feature, and so is a field MSR (immediate) names for it. The SSBS
/// register is FEAT_SSBS2's, which MSR SSBS (immediate) does not need.
#[test]
fn without_the_features_their_encodings_are_undefined() {
    let mut m = Machine::new(Implementation::CORTEX_A57);
    let words = [
        CFINV,
        XAFLAG,
        AXFLAG,
        RMIF,
        SETF8,
        SETF16,
        SB,
        MSR_PAN_1,
        MSR_UAO_1,
        MSR_DIT_1,
        MSR_SSBS_1,
        MRS_PAN,
        MSR_PAN_X0,
        MRS_UAO,
        MRS_DIT,
        MSR_DIT_X0,
        MRS_SSBS,
        MSR_SSBS_X0,
        AT_S1E1RP,
        AT_S1E1WP,
    ];
    let before = m.cpu.pstate;
    for insn in words {
        assert_eq!(m.run(insn), UNDEFINED, "{insn:#010x} on the Cortex-A57");
    }
    assert_eq!(m.cpu.pstate, before);

    let mut flagm = Machine::new(a57_with(&[Feature::FlagM]));
    assert_eq!(flagm.run(CFINV), None);
    assert_eq!(flagm.run(RMIF), None);
    assert_eq!(flagm.run(XAFLAG), UNDEFINED, "XAFLAG is FEAT_FlagM2's");
    assert_eq!(flagm.run(AXFLAG), UNDEFINED, "AXFLAG is FEAT_FlagM2's");

    let mut ssbs = Machine::new(a57_with(&[Feature::Ssbs]));
    assert_eq!(ssbs.run(MSR_SSBS_1), None);
    assert!(ssbs.cpu.pstate.ssbs);
    assert_eq!(
        ssbs.run(MRS_SSBS),
        UNDEFINED,
        "the SSBS register is FEAT_SSBS2's"
    );

    let mut pan = Machine::new(a57_with(&[Feature::Pan]));
    assert_eq!(pan.run(MSR_PAN_1), None);
    assert_eq!(pan.run(AT_S1E1RP), UNDEFINED, "AT S1E1RP is FEAT_PAN2's");
}

/// AT S1E1RP and AT S1E1WP check a translation as a data access with PAN
/// would; AT S1E1R and AT S1E1W ignore PAN (`ATAccess_Read`/`_Write`).
#[test]
fn at_s1e1rp_and_s1e1wp_take_pan_into_account() {
    const RES1: u64 = 1 << 11;
    let fault = RES1 | (PERMISSION_L2 << 1) | 1;
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.x[0] = EL0_RW;
    m.cpu.pstate.pan = true;
    for (insn, refused) in [
        (AT_S1E1R, false),
        (AT_S1E1W, false),
        (AT_S1E1RP, true),
        (AT_S1E1WP, true),
    ] {
        assert_eq!(m.run(insn), None);
        assert_eq!(
            m.cpu.sys.par_el1 == fault,
            refused,
            "{insn:#010x} with PAN set: PAR_EL1 {:#x}",
            m.cpu.sys.par_el1
        );
        if !refused {
            assert_eq!(m.cpu.sys.par_el1 & 1, 0, "{insn:#010x} translates");
            assert_eq!(m.cpu.sys.par_el1 & 0x0000_FFFF_FFFF_F000, EL0_RW & !0xFFF);
        }
    }
    m.cpu.pstate.pan = false;
    assert_eq!(m.run(AT_S1E1RP), None);
    assert_eq!(m.cpu.sys.par_el1 & 1, 0, "PAN clear");
}

/// Taking an exception sets PSTATE.PAN when SCTLR_EL1.SPAN is 0 and keeps
/// it otherwise, clears UAO, sets SSBS to SCTLR_EL1.DSSBS and keeps DIT;
/// SPSR_EL1 holds all four as they were. A processor without the features
/// holds none of them.
#[test]
fn taking_an_exception_sets_pan_and_ssbs_and_clears_uao() {
    for (span, dssbs) in [(false, true), (true, false)] {
        let mut m = Machine::new(Implementation::EVERY_FEATURE);
        let span_bit = if span { SPAN } else { 0 };
        let dssbs_bit = if dssbs { DSSBS } else { 0 };
        m.cpu.sys.sctlr_el1 = (m.cpu.sys.sctlr_el1 & !(SPAN | DSSBS)) | span_bit | dssbs_bit;
        m.cpu.pstate.uao = true;
        m.cpu.pstate.dit = true;
        let taken = m.step_raw(CODE, SVC);
        assert_eq!(taken.map(|t| t.ec), Some(0x15));
        let p = m.cpu.pstate;
        assert_eq!(
            (p.pan, p.uao, p.dit, p.ssbs),
            (!span, false, true, dssbs),
            "SPAN {span}, DSSBS {dssbs}"
        );
        let spsr = m.cpu.sys.spsr_el1;
        assert_eq!(
            spsr & ((1 << 22) | (1 << 23) | (1 << 24) | (1 << 12)),
            (1 << 23) | (1 << 24),
            "the SPSR keeps UAO and DIT"
        );
    }

    // With SPAN set, PAN stays as the interrupted code had it.
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.pstate.pan = true;
    m.cpu.take_irq();
    assert!(m.cpu.pstate.pan);
    assert_ne!(m.cpu.sys.spsr_el1 & (1 << 22), 0);

    let mut a57 = Machine::new(Implementation::CORTEX_A57);
    a57.cpu.sys.sctlr_el1 = (a57.cpu.sys.sctlr_el1 & !SPAN) | DSSBS;
    a57.cpu.take_irq();
    let p = a57.cpu.pstate;
    assert_eq!((p.pan, p.uao, p.dit, p.ssbs), (false, false, false, false));
}

/// ERET restores PAN, UAO, DIT and SSBS from SPSR_EL1. An illegal return
/// restores PAN too (`SetPSTATEFromPSR`) and leaves the others, whose
/// values it makes UNKNOWN, as they were.
#[test]
fn eret_restores_the_fields_and_an_illegal_return_restores_pan() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    const EL0T: u64 = 0b0000;
    let fields = (1 << 22) | (1 << 23) | (1 << 24) | (1 << 12);
    m.cpu.sys.spsr_el1 = EL0T | fields;
    m.cpu.sys.elr_el1 = EL0_CODE;
    assert_eq!(m.step_raw(CODE, ERET), None);
    let p = m.cpu.pstate;
    assert_eq!(
        (p.el, p.pan, p.uao, p.dit, p.ssbs),
        (El::El0, true, true, true, true)
    );

    // SPSR M = EL2h, which this core lacks.
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.pstate.uao = true;
    m.cpu.pstate.dit = true;
    m.cpu.sys.spsr_el1 = 0b1001 | (1 << 22);
    m.cpu.sys.elr_el1 = CODE + 0x100;
    assert_eq!(m.step_raw(CODE, ERET), None);
    let p = m.cpu.pstate;
    assert_eq!(
        (p.el, p.il, p.pan, p.uao, p.dit, p.ssbs),
        (El::El1, true, true, true, true, false)
    );
}

/// SPSR_EL1 keeps PAN, UAO, DIT and SSBS only on a processor with their
/// features; without them the bits are RES0, and a snapshot whose SPSR_EL1
/// holds them is refused, so no ERET can set them.
#[test]
fn spsr_holds_the_fields_only_with_their_features() {
    let fields = (1 << 22) | (1 << 23) | (1 << 24) | (1 << 12);
    for (imp, kept) in [
        (Implementation::EVERY_FEATURE, fields),
        (Implementation::CORTEX_A57, 0),
    ] {
        let mut m = Machine::new(imp);
        m.cpu.x[0] = 0x3C5 | fields;
        assert_eq!(m.run(MSR_SPSR), None);
        assert_eq!(m.run(MRS_SPSR), None);
        assert_eq!(m.cpu.x[0], 0x3C5 | kept, "{imp:?}");
    }

    // The Cortex-A57's own image, with the fields set in SPSR_EL1: after
    // the feature set (8 bytes), the streaming vector length (4), MIDR_EL1
    // and the counter frequency (8 each), X0-X30, SP_EL0, SP_EL1, the PC
    // and PSTATE (8 each), V0-V31 (16 each), FPCR and FPSR (8 each), the
    // seventh system register.
    let mut a57 = Machine::new(Implementation::CORTEX_A57);
    a57.cpu.sys.spsr_el1 = 0x3C5;
    let mut image = Vec::new();
    assert!(a57.cpu.save(&mut image).is_ok());
    let at = 8 + 4 + 2 * 8 + 35 * 8 + 32 * 16 + 2 * 8 + 6 * 8;
    let mut spsr = [0u8; 8];
    spsr.copy_from_slice(&image[at..at + 8]);
    assert_eq!(
        u64::from_le_bytes(spsr),
        0x3C5,
        "the layout this test assumes"
    );
    image[at..at + 8].copy_from_slice(&(0x3C5 | fields).to_le_bytes());
    let mut refusing = Machine::new(Implementation::CORTEX_A57);
    assert!(refusing.cpu.restore(&mut image.as_slice()).is_err());
    image[at..at + 8].copy_from_slice(&0x3C5u64.to_le_bytes());
    assert!(refusing.cpu.restore(&mut image.as_slice()).is_ok());
}

/// What a processor holds of PSTATE and what SPSR_EL1 can hold of it are
/// the same fields.
#[test]
fn the_held_fields_and_the_spsr_mask_agree() {
    for imp in [
        Implementation::EVERY_FEATURE,
        Implementation::CORTEX_A57,
        a57_with(&[Feature::Pan, Feature::Dit]),
    ] {
        let cpu = Machine::new(imp).cpu;
        // Every bit set, M = EL1h.
        let spsr = !0x1F | 0b0101;
        let every = Pstate::from_spsr(spsr).unwrap_or(Pstate::RESET);
        assert_eq!(cpu.held(every).to_spsr(), spsr & cpu.spsr_mask(), "{imp:?}");
    }
}

/// A snapshot carries PAN, UAO, DIT and SSBS, and a processor without
/// their features refuses an image whose PSTATE holds one.
#[test]
fn a_snapshot_carries_the_fields() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.pstate.pan = true;
    m.cpu.pstate.uao = true;
    m.cpu.pstate.dit = true;
    m.cpu.pstate.ssbs = true;
    let mut image = Vec::new();
    assert!(m.cpu.save(&mut image).is_ok());
    let mut restored = Machine::new(Implementation::EVERY_FEATURE);
    assert!(restored.cpu.restore(&mut image.as_slice()).is_ok());
    assert_eq!(restored.cpu.pstate, m.cpu.pstate);

    // The Cortex-A57's own image, with SPSR's PAN bit set in its PSTATE:
    // after the feature set (8 bytes), the streaming vector length (4),
    // MIDR_EL1 and the counter frequency (8 each), X0-X30, SP_EL0, SP_EL1
    // and the PC (8 each).
    let a57 = Machine::new(Implementation::CORTEX_A57);
    let mut image = Vec::new();
    assert!(a57.cpu.save(&mut image).is_ok());
    let at = 8 + 4 + 2 * 8 + 34 * 8;
    let mut spsr = [0u8; 8];
    spsr.copy_from_slice(&image[at..at + 8]);
    assert_eq!(
        u64::from_le_bytes(spsr),
        a57.cpu.pstate.to_spsr(),
        "the layout this test assumes"
    );
    image[at..at + 8].copy_from_slice(&(a57.cpu.pstate.to_spsr() | (1 << 22)).to_le_bytes());
    let mut refusing = Machine::new(Implementation::CORTEX_A57);
    assert!(refusing.cpu.restore(&mut image.as_slice()).is_err());
}

/// Reset leaves PAN, UAO, DIT and SSBS clear (`AArch64_TakeReset` clears
/// DIT; the rest are clear as QEMU leaves them).
#[test]
fn reset_clears_the_fields() {
    let m = Machine::new(Implementation::EVERY_FEATURE);
    let p = m.cpu.pstate;
    assert_eq!((p.pan, p.uao, p.dit, p.ssbs), (false, false, false, false));
}
