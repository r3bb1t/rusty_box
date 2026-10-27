//! FEAT_HPDS, FEAT_TTCNP, FEAT_E0PD, FEAT_TLBIOS, FEAT_TLBIRANGE, FEAT_XS,
//! FEAT_DPB, FEAT_DPB2 and FEAT_LOR, and the cache maintenance instructions
//! by address every processor has, as a guest sees them (R9): at EL0 and
//! EL1, through the MMU, on the processors with each feature and without.
//!
//! The rules are the Arm ARM's pseudocode: `AArch64_S1TTWParamsEL10` and
//! `AArch64_S1NextWalkStateTable` (HPD), `AArch64_S1Translate` (E0PD),
//! `AArch64_DC`, `AArch64_IC`, `AArch64_S1CheckPermissions` and
//! `AArch64_FaultSyndrome` (cache maintenance by address), the `TLBI`, `DC`
//! and LORegion register pages of Arm's `Registers.json`, and the pages of
//! DSB, LDLAR and STLLR.
//!
//! The words are GNU as's (`-march=armv9.2-a+xs`, objdump -d).

use std::vec::Vec;

use rusty_box_core::{ClockHz, VmClock};

use super::features::{Feature, Features, Implementation, VecLen};
use super::{id, Cpu, CpuConfig, El, NoEl2, Step, Tlb};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};
use crate::instrumentation::NoInstrumentation;

const LDR: u32 = 0xF940_0022; // ldr x2, [x1]
const LDTR: u32 = 0xF840_0822; // ldtr x2, [x1]
const LDAR: u32 = 0xC8DF_FC22; // ldar x2, [x1]
const LDLAR: u32 = 0xC8DF_7C22; // ldlar x2, [x1]
const STLLR: u32 = 0xC89F_7C22; // stllr x2, [x1]
const LDLARB: u32 = 0x08DF_7C22; // ldlarb w2, [x1]
const STLLRH: u32 = 0x489F_7C22; // stllrh w2, [x1]
const DC_CIVAC: u32 = 0xD50B_7E21; // dc civac, x1
const DC_CVAC: u32 = 0xD50B_7A21;
const DC_CVAU: u32 = 0xD50B_7B21;
const DC_CVAP: u32 = 0xD50B_7C21;
const DC_CVADP: u32 = 0xD50B_7D21;
const IC_IVAU: u32 = 0xD50B_7521;
const DC_IVAC: u32 = 0xD508_7621;
const TLBI_VMALLE1OS: u32 = 0xD508_811F;
const TLBI_VAE1OS: u32 = 0xD508_8121; // tlbi vae1os, x1
const TLBI_VAE1IS: u32 = 0xD508_8321;
const TLBI_VAE1: u32 = 0xD508_8721;
const TLBI_RVAE1: u32 = 0xD508_8621;
const TLBI_RVAE1IS: u32 = 0xD508_8221;
const TLBI_RVAE1OS: u32 = 0xD508_8521;
const TLBI_VMALLE1NXS: u32 = 0xD508_971F;
const TLBI_VAE1OSNXS: u32 = 0xD508_9121;
const TLBI_RVAE1OSNXS: u32 = 0xD508_9521;
const DSB_OSHNXS: u32 = 0xD503_323F;
const DSB_SYNXS: u32 = 0xD503_3E3F;
/// `op2 = 001` with `CRm<1:0> = 11`: unallocated, FEAT_XS or not.
const DSB_NXS_RESERVED: u32 = 0xD503_333F;
const MRS_LORID: u32 = 0xD538_A4E0; // mrs x0, lorid_el1
const MSR_LORID: u32 = 0xD518_A4E0;
const MRS_LORSA: u32 = 0xD538_A400;
const MSR_LORSA: u32 = 0xD518_A400;
const MRS_LOREA: u32 = 0xD538_A420;
const MSR_LOREA: u32 = 0xD518_A420;
const MRS_LORN: u32 = 0xD538_A440;
const MSR_LORN: u32 = 0xD518_A440;
const MRS_LORC: u32 = 0xD538_A460;
const MSR_LORC: u32 = 0xD518_A460;
const AT_S1E0R: u32 = 0xD508_7840; // at s1e0r, x0
const AT_S1E1R: u32 = 0xD508_7800;
const MRS_PAR: u32 = 0xD538_7400;
const MRS_TTBR0: u32 = 0xD538_2000;

/// The translation tables, shared by both halves of the address space
/// (TTBR0_EL1 and TTBR1_EL1 both name `L1`, and T0SZ = T1SZ = 25): `L1`'s
/// first entry is a level-2 table of four 2 MiB blocks, identity mapped,
/// Normal memory — EL1's alone, EL0 read/write, EL0 execute-only, and
/// read-only to both — and its second (VA 1 GiB) a level-2 table, `L2_HIER`,
/// whose table descriptor takes EL0's access away and execution from both
/// ELs (APTable = 01, UXNTable, PXNTable), mapping VA 1 GiB to the EL0
/// read/write block and VA 1 GiB + 2 MiB to the first.
const L1: u64 = 0x1000;
const L2: u64 = 0x2000;
const L2_HIER: u64 = 0x3000;
const VBAR: u64 = 0x8000;
/// Where EL1's instructions execute, and its own data.
const CODE: u64 = 0x4_0000;
const EL1_DATA: u64 = 0x5_0000;
const EL0_RW: u64 = 0x20_0100;
/// Where EL0's instructions execute, in the execute-only block.
const EL0_CODE: u64 = 0x40_0000;
const READ_ONLY: u64 = 0x60_0100;
/// Nothing maps VA 2 GiB.
const UNMAPPED: u64 = 0x8000_0000;
/// VA 1 GiB + `EL0_RW`'s offset: the EL0 read/write block, under the
/// restrictive table descriptor.
const HIER_EL0_RW: u64 = 0x4000_0100;
/// VA 1 GiB + 2 MiB + `CODE`: EL1's code, under the same descriptor.
const HIER_CODE: u64 = 0x4020_0000 + CODE;
/// The upper half's alias of physical address 0, with T1SZ = 25.
const UPPER: u64 = 0xFFFF_FF80_0000_0000;
const RAM_BYTES: usize = 0x80_0000;

/// `TCR_EL1.HPD0` and `HPD1`, `E0PD0` and `E0PD1`, `TBI0`.
const HPD0: u64 = 1 << 41;
const HPD1: u64 = 1 << 42;
const E0PD0: u64 = 1 << 55;
const E0PD1: u64 = 1 << 56;
const TBI0: u64 = 1 << 37;
/// `SCTLR_EL1.UCI`.
const UCI: u64 = 1 << 26;

/// The classes this file's exceptions are taken with.
const UNKNOWN: u64 = 0x00;
const SYS_TRAP: u64 = 0x18;
const INSTRUCTION_ABORT_SAME_EL: u64 = 0x21;
const DATA_ABORT_LOWER_EL: u64 = 0x24;
const DATA_ABORT_SAME_EL: u64 = 0x25;
/// DFSC/IFSC: a translation fault at level 0, and at level 1 (VA 2 GiB has
/// no level-1 entry); a permission fault at level 2, where the blocks are.
const ADDRESS_SIZE_L0: u64 = 0b00_0000;
const TRANSLATION_L0: u64 = 0b00_0100;
const TRANSLATION_L1: u64 = 0b00_0101;
const TRANSLATION_L2: u64 = 0b00_0110;
const PERMISSION_L2: u64 = 0b00_1110;
const ALIGNMENT: u64 = 0b10_0001;
const WNR: u64 = 1 << 6;
/// `ISS.CM`: the fault is a cache maintenance instruction's.
const CM: u64 = 1 << 8;

struct Ram(Vec<u8>);

impl PhysBus for Ram {
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _clock: VmClock) -> Result<u64, BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get(a..a + n).ok_or(BusFault::Unmapped)?;
        Ok(bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b)))
    }

    fn write(&mut self, addr: PhysAddr, width: AccessWidth, value: u64, _clock: VmClock) -> Result<(), BusFault> {
        let a = addr.get() as usize;
        let n = width.bytes() as usize;
        let bytes = self.0.get_mut(a..a + n).ok_or(BusFault::Unmapped)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

/// A taken exception: `ESR_EL1`'s class and ISS, and `FAR_EL1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    ec: u64,
    iss: u64,
    far: u64,
}

struct Machine {
    cpu: Cpu<NoEl2>,
    ram: Ram,
}

impl Machine {
    /// A processor implementing `imp` at EL1 with the MMU on over the
    /// tables above, in both halves of the address space.
    fn new(imp: Implementation) -> Self {
        let ips = ClockHz::new(1_000_000).unwrap_or(ClockHz::BOCHS_DEFAULT);
        let mut cpu = Cpu::<NoEl2>::implementing(
            CpuConfig {
                ips,
                counter_hz: ips,
                reset_pc: CODE,
            },
            imp,
        );
        let mut ram = Ram(std::vec![0; RAM_BYTES]);
        const AF: u64 = 1 << 10;
        const INNER_SHAREABLE: u64 = 3 << 8;
        const BLOCK: u64 = 0b01;
        const TABLE: u64 = 0b11;
        const UXN: u64 = 1 << 54;
        const PXN: u64 = 1 << 53;
        const EL0_READ_WRITE: u64 = 0b01 << 6;
        const READ_ONLY_BOTH: u64 = 0b11 << 6;
        /// APTable = 01 (no EL0 access below), UXNTable, PXNTable.
        const RESTRICTIVE_TABLE: u64 = (0b01 << 61) | (1 << 60) | (1 << 59);
        let attrs = AF | INNER_SHAREABLE | BLOCK;
        let descriptors = [
            (L1, L2 | TABLE),
            (L1 + 8, L2_HIER | TABLE | RESTRICTIVE_TABLE),
            (L2, attrs | UXN),
            (L2 + 8, 0x20_0000 | attrs | EL0_READ_WRITE | UXN | PXN),
            (L2 + 16, 0x40_0000 | attrs),
            (L2 + 24, 0x60_0000 | attrs | READ_ONLY_BOTH | UXN | PXN),
            (L2_HIER, 0x20_0000 | attrs | EL0_READ_WRITE | UXN),
            (L2_HIER + 8, attrs | UXN),
        ];
        for (at, desc) in descriptors {
            let at = at as usize;
            ram.0[at..at + 8].copy_from_slice(&desc.to_le_bytes());
        }
        cpu.sys.mair_el1 = 0xFF;
        // T0SZ = T1SZ = 25: both halves walk from level 1.
        cpu.sys.tcr_el1 = 25 | (25 << 16);
        cpu.sys.ttbr0_el1 = L1;
        cpu.sys.ttbr1_el1 = L1;
        cpu.sys.sctlr_el1 = id::SCTLR_EL1_RESET | 1 | UCI;
        cpu.sys.vbar_el1 = VBAR;
        Self { cpu, ram }
    }

    /// Execute `insn` once at EL1's code, or at EL0's when the processor is
    /// at EL0: the exception it took, if it took one. After an exception
    /// the processor is put back at the exception level it executed at.
    fn run(&mut self, insn: u32) -> Option<Taken> {
        let at = if self.cpu.pstate.el == El::El0 { EL0_CODE } else { CODE };
        self.run_at(at, at, insn)
    }

    /// Execute `insn`, placed at physical address `pa`, from virtual
    /// address `va`.
    fn run_at(&mut self, va: u64, pa: u64, insn: u32) -> Option<Taken> {
        let pstate = self.cpu.pstate;
        let a = pa as usize;
        self.ram.0[a..a + 4].copy_from_slice(&insn.to_le_bytes());
        self.cpu.pc = va;
        self.cpu.sys.esr_el1 = 0x3F << 26;
        assert_eq!(self.cpu.step(&mut self.ram, &mut NoInstrumentation), Step::Retired);
        let esr = self.cpu.sys.esr_el1;
        if esr >> 26 == 0x3F {
            return None;
        }
        assert_eq!(self.cpu.pc & !0x7FF, VBAR, "{insn:#010x} took an exception to the vector table");
        self.cpu.pstate = pstate;
        Some(Taken {
            ec: esr >> 26,
            iss: esr & 0x01FF_FFFF,
            far: self.cpu.sys.far_el1,
        })
    }

    fn at_el0(&mut self) {
        self.cpu.pstate.el = El::El0;
        self.cpu.pstate.sp_sel = false;
    }

    /// Write descriptor `desc` at physical address `at`.
    fn set_descriptor(&mut self, at: u64, desc: u64) {
        let at = at as usize;
        self.ram.0[at..at + 8].copy_from_slice(&desc.to_le_bytes());
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
        max_svl: VecLen::MIN,
    }
}

/// A data abort taken from `el` with fault status `fsc` at `far`.
fn data_abort(el: El, fsc: u64, far: u64) -> Option<Taken> {
    let ec = if el == El::El0 { DATA_ABORT_LOWER_EL } else { DATA_ABORT_SAME_EL };
    Some(Taken { ec, iss: fsc, far })
}

/// The Undefined Instruction exception, taken with `FAR_EL1` as `m` left
/// it.
fn undefined(m: &Machine) -> Option<Taken> {
    Some(Taken {
        ec: UNKNOWN,
        iss: 0,
        far: m.cpu.sys.far_el1,
    })
}

/// FEAT_HPDS: with `TCR_EL1.HPD0` or `HPD1` set, a table descriptor's
/// APTable, UXNTable and PXNTable take no part in the walk of that half of
/// the address space; clear, they take EL0's access and both ELs'
/// execution away from everything below. Without FEAT_HPDS the bits are
/// RES0, and the table's restrictions apply whatever they hold.
#[test]
fn hpd_turns_off_the_hierarchical_permissions_of_its_half() {
    for (half, hpd) in [(0, HPD0), (UPPER, HPD1)] {
        let mut m = Machine::new(Implementation::EVERY_FEATURE);
        m.at_el0();
        m.cpu.x[1] = half + HIER_EL0_RW;
        let refused = data_abort(El::El0, PERMISSION_L2, half + HIER_EL0_RW);
        assert_eq!(m.run(LDR), refused, "{half:#x}: APTable takes EL0's access away");
        m.cpu.sys.tcr_el1 |= hpd ^ (HPD0 | HPD1);
        m.cpu.tlb.flush();
        assert_eq!(m.run(LDR), refused, "{half:#x}: the other half's HPD");
        m.cpu.sys.tcr_el1 ^= HPD0 | HPD1;
        m.cpu.tlb.flush();
        assert_eq!(m.run(LDR), None, "{half:#x}: this half's HPD");

        // EL1 executes its code through the table's PXNTable only under
        // HPD.
        m.cpu.pstate.el = El::El1;
        m.cpu.x[0] = 0;
        assert_eq!(m.run_at(half + HIER_CODE, CODE, 0xD280_0020), None, "{half:#x}: mov x0, #1 under HPD");
        assert_eq!(m.cpu.x[0], 1);
        m.cpu.sys.tcr_el1 &= !(HPD0 | HPD1);
        m.cpu.tlb.flush();
        let taken = m.run_at(half + HIER_CODE, CODE, 0xD280_0020);
        assert_eq!(
            taken,
            Some(Taken {
                ec: INSTRUCTION_ABORT_SAME_EL,
                iss: PERMISSION_L2,
                far: half + HIER_CODE
            }),
            "{half:#x}: PXNTable"
        );
    }

    let mut a57 = Machine::new(a57_with(&[]));
    a57.cpu.sys.tcr_el1 |= HPD0 | HPD1;
    a57.at_el0();
    a57.cpu.x[1] = HIER_EL0_RW;
    assert_eq!(a57.run(LDR), data_abort(El::El0, PERMISSION_L2, HIER_EL0_RW), "HPD0 is RES0 without FEAT_HPDS");
}

/// FEAT_E0PD: with `TCR_EL1.E0PD0` or `E0PD1` set, every EL0 access to
/// that half of the address space takes a level 0 translation fault ahead
/// of the walk — a load, LDTR at EL1 (an EL0 access), an instruction fetch,
/// a cache maintenance instruction and AT S1E0R alike — on memory EL0 has
/// a permission for, and on EL1's own memory through the translation an
/// EL1 access left in the TLB, ahead of a permission or Device alignment
/// fault. EL1's own accesses, PSTATE.PAN's refusals among them, and the
/// other half's, are unaffected. Without FEAT_E0PD the bits are RES0.
#[test]
fn e0pd_faults_el0_accesses_to_its_half() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.sys.tcr_el1 |= E0PD1;
    for address in [UPPER + EL0_RW, UPPER + EL1_DATA] {
        m.cpu.pstate.el = El::El1;
        m.cpu.x[1] = address;
        assert_eq!(m.run(LDR), None, "EL1 at {address:#x}, which caches EL1's own memory");
        let refused = data_abort(El::El1, TRANSLATION_L0, address);
        assert_eq!(m.run(LDTR), refused, "LDTR at EL1 is an EL0 access, at {address:#x}");
        m.cpu.pstate.uao = true;
        let ldtr_el1 = m.run(LDTR);
        assert_eq!(ldtr_el1, None, "LDTR under UAO is an EL1 access, at {address:#x}");
        m.cpu.pstate.uao = false;
        m.at_el0();
        let refused = data_abort(El::El0, TRANSLATION_L0, address);
        assert_eq!(m.run(LDR), refused, "EL0 at {address:#x}");
        let cm = refused.map(|t| Taken { iss: t.iss | CM | WNR, ..t });
        for insn in [DC_CVAC, IC_IVAU] {
            assert_eq!(m.run(insn), cm, "{insn:#010x} at EL0 at {address:#x}");
        }
    }
    m.cpu.x[1] = EL0_RW;
    assert_eq!(m.run(LDR), None, "the lower half");

    // PSTATE.PAN still sees what EL0 may do in a half E0PD refuses it.
    m.cpu.pstate.el = El::El1;
    m.cpu.pstate.pan = true;
    m.cpu.x[1] = UPPER + EL0_RW;
    assert_eq!(m.run(LDR), data_abort(El::El1, PERMISSION_L2, UPPER + EL0_RW), "PAN");
    m.cpu.pstate.pan = false;

    // E0PD's fault comes ahead of Device memory's alignment fault, without
    // the TLB and through it: every block Device memory, a misaligned load.
    m.cpu.sys.mair_el1 = 0;
    m.cpu.tlb.flush();
    let misaligned = UPPER + EL1_DATA + 1;
    m.cpu.x[1] = misaligned;
    m.at_el0();
    let refused = data_abort(El::El0, TRANSLATION_L0, misaligned);
    assert_eq!(m.run(LDR), refused, "Device memory, nothing in the TLB");
    m.cpu.pstate.el = El::El1;
    assert_eq!(m.run(LDR), data_abort(El::El1, ALIGNMENT, misaligned), "EL1 fills the TLB");
    m.at_el0();
    assert_eq!(m.run(LDR), refused, "Device memory, through the TLB");
    m.cpu.sys.mair_el1 = 0xFF;
    m.cpu.tlb.flush();

    // An EL0 fetch from the lower half under E0PD0, run from where EL0's
    // code is in the upper half. The TLB is flushed, as an MSR to TCR_EL1
    // flushes it.
    m.cpu.sys.tcr_el1 ^= E0PD0 | E0PD1;
    m.cpu.tlb.flush();
    let fetched = m.run(LDR);
    assert_eq!(
        fetched,
        Some(Taken {
            ec: 0x20,
            iss: TRANSLATION_L0,
            far: EL0_CODE
        }),
        "an EL0 fetch"
    );
    m.cpu.x[1] = EL0_RW;
    assert_eq!(m.run_at(UPPER + EL0_CODE, EL0_CODE, LDR), data_abort(El::El0, TRANSLATION_L0, EL0_RW));
    m.cpu.x[1] = UPPER + EL0_RW;
    assert_eq!(m.run_at(UPPER + EL0_CODE, EL0_CODE, LDR), None, "the upper half under E0PD0");

    // AT S1E0R reports the fault in PAR_EL1; AT S1E1R translates.
    m.cpu.pstate.el = El::El1;
    for (at, faults) in [(AT_S1E0R, true), (AT_S1E1R, false)] {
        m.cpu.x[0] = EL0_RW;
        assert_eq!(m.run(at), None);
        assert_eq!(m.run(MRS_PAR), None);
        let par = m.cpu.x[0];
        if faults {
            assert_eq!(par & 0x7F, (TRANSLATION_L0 << 1) | 1, "{at:#010x}: F, and FST level 0 translation");
        } else {
            assert_eq!((par & 1, par & 0x0000_FFFF_FFFF_F000), (0, EL0_RW & !0xFFF), "{at:#010x}");
        }
    }

    let mut a57 = Machine::new(a57_with(&[]));
    a57.cpu.sys.tcr_el1 |= E0PD0 | E0PD1;
    a57.at_el0();
    a57.cpu.x[1] = EL0_RW;
    assert_eq!(a57.run(LDR), None, "E0PD0 is RES0 without FEAT_E0PD");
}

/// A TLB entry giving EL0 a permission in a half FEAT_E0PD refuses EL0 is
/// one the TLB never holds, so a snapshot holding one is refused.
#[test]
fn a_snapshot_entry_e0pd_refuses_is_refused() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.x[1] = UPPER + EL0_RW;
    assert_eq!(m.run(LDR), None, "E0PD clear: the page is cached");
    let mut image = Vec::new();
    assert!(m.cpu.tlb.save(&mut image).is_ok());
    let tcr = m.cpu.sys.tcr_el1;
    let restored =
        |features: Features, tcr: u64| Tlb::new().restore(&mut image.as_slice(), features, tcr).is_ok();
    let every = Implementation::EVERY_FEATURE.features;
    assert!(restored(every, tcr));
    assert!(!restored(every, tcr | E0PD1), "E0PD1 refuses EL0 the upper half");
    assert!(restored(every, tcr | E0PD0), "E0PD0 refuses EL0 the lower half alone");
    assert!(restored(a57_with(&[]).features, tcr | E0PD1), "E0PD1 is RES0 without FEAT_E0PD");
}

/// FEAT_TTCNP: `TTBRn_EL1.CnP`, bit 0, is kept as written and leaves the
/// walk's base address alone.
#[test]
fn cnp_is_kept_and_does_not_move_the_table() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.sys.ttbr0_el1 = L1 | 1;
    m.cpu.x[1] = EL1_DATA;
    m.ram.0[EL1_DATA as usize] = 0x5A;
    assert_eq!(m.run(LDR), None);
    assert_eq!(m.cpu.x[2], 0x5A);
    assert_eq!(m.run(MRS_TTBR0), None);
    assert_eq!(m.cpu.x[0], L1 | 1);
}

/// The data and instruction cache maintenance instructions by address
/// translate it, every processor's (`AArch64_DC`, `AArch64_IC`): a
/// translation fault is a data abort at the register's address, with CM
/// and WnR set. DC IVAC needs write permission; a clean, or a clean and
/// invalidate, needs read permission at EL0 and none at EL1; IC IVAU needs
/// none. PSTATE.PAN takes no part. Device memory refuses none of them.
#[test]
fn cache_maintenance_by_address_translates_and_checks_its_own_permissions() {
    let cm = |el: El, fsc: u64, far: u64| data_abort(el, fsc | CM | WNR, far);
    let cleans = [DC_CVAC, DC_CVAU, DC_CVAP, DC_CVADP, DC_CIVAC];
    for imp in [Implementation::EVERY_FEATURE, a57_with(&[Feature::Dpb, Feature::Dpb2])] {
        let mut m = Machine::new(imp);
        m.cpu.pstate.pan = imp.has(Feature::Pan);
        for insn in cleans.into_iter().chain([IC_IVAU, DC_IVAC]) {
            m.cpu.x[1] = UNMAPPED + 0x123;
            assert_eq!(m.run(insn), cm(El::El1, TRANSLATION_L1, UNMAPPED + 0x123), "{insn:#010x}: unmapped");
            m.cpu.x[1] = READ_ONLY;
            let want = if insn == DC_IVAC { cm(El::El1, PERMISSION_L2, READ_ONLY) } else { None };
            assert_eq!(m.run(insn), want, "{insn:#010x}: read-only at EL1");
            m.cpu.x[1] = EL0_RW;
            assert_eq!(m.run(insn), None, "{insn:#010x}: memory EL0 can reach, PAN or not");
        }

        m.at_el0();
        for insn in cleans.into_iter().chain([IC_IVAU]) {
            m.cpu.x[1] = EL1_DATA;
            let want = if insn == IC_IVAU { None } else { cm(El::El0, PERMISSION_L2, EL1_DATA) };
            assert_eq!(m.run(insn), want, "{insn:#010x}: EL1's data at EL0");
            m.cpu.x[1] = READ_ONLY;
            assert_eq!(m.run(insn), None, "{insn:#010x}: read-only at EL0");
            m.cpu.x[1] = UNMAPPED;
            assert_eq!(m.run(insn), cm(El::El0, TRANSLATION_L1, UNMAPPED), "{insn:#010x}: unmapped at EL0");
        }
        assert_eq!(m.run(DC_IVAC), undefined(&m), "DC IVAC is EL1's");

        // SCTLR_EL1.UCI clear traps EL0's, before the translation.
        m.cpu.sys.sctlr_el1 &= !UCI;
        for insn in cleans.into_iter().chain([IC_IVAU]) {
            let taken = m.run(insn).map(|t| t.ec);
            assert_eq!(taken, Some(SYS_TRAP), "{insn:#010x} with UCI clear");
        }
    }

    // Device memory, every block of it here (MAIR_EL1 attribute 0 =
    // Device-nGnRnE), refuses none of them, at an address of any alignment.
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.sys.mair_el1 = 0;
    m.cpu.tlb.flush();
    for insn in cleans.into_iter().chain([IC_IVAU, DC_IVAC]) {
        m.cpu.x[1] = EL1_DATA + 3;
        assert_eq!(m.run(insn), None, "{insn:#010x} on Device memory");
    }

    // With stage 1 disabled, an address wider than the physical address
    // size is an address size fault at level 0, after TBI0 drops the tag.
    m.cpu.sys.sctlr_el1 &= !1;
    let tagged = 0x5A00_0000_0000_0000 | EL1_DATA;
    for insn in cleans.into_iter().chain([IC_IVAU, DC_IVAC]) {
        for (address, faults) in [(EL1_DATA, false), (1 << id::PA_BITS, true), (tagged, true)] {
            m.cpu.x[1] = address;
            let want = if faults { cm(El::El1, ADDRESS_SIZE_L0, address) } else { None };
            assert_eq!(m.run(insn), want, "{insn:#010x} at {address:#x}, MMU off");
        }
        m.cpu.sys.tcr_el1 |= TBI0;
        m.cpu.x[1] = tagged;
        assert_eq!(m.run(insn), None, "{insn:#010x} at {tagged:#x}, MMU off under TBI0");
        m.cpu.sys.tcr_el1 &= !TBI0;
    }
}

/// DC CVAP is FEAT_DPB's and DC CVADP FEAT_DPB2's: UNDEFINED on a
/// processor without, at either EL, ahead of the UCI trap.
#[test]
fn dc_cvap_and_cvadp_need_their_features() {
    for (imp, cvap, cvadp) in [
        (a57_with(&[]), false, false),
        (a57_with(&[Feature::Dpb]), true, false),
        (a57_with(&[Feature::Dpb, Feature::Dpb2]), true, true),
    ] {
        let mut m = Machine::new(imp);
        m.cpu.x[1] = EL0_RW;
        for (insn, implemented) in [(DC_CVAP, cvap), (DC_CVADP, cvadp)] {
            let want = if implemented { None } else { undefined(&m) };
            assert_eq!(m.run(insn), want, "{insn:#010x} on {imp:?}");
            m.at_el0();
            m.cpu.sys.sctlr_el1 &= !UCI;
            let want = if implemented { Some(SYS_TRAP) } else { Some(UNKNOWN) };
            assert_eq!(m.run(insn).map(|t| t.ec), want, "{insn:#010x} at EL0 on {imp:?}");
            m.cpu.sys.sctlr_el1 |= UCI;
            m.cpu.pstate.el = El::El1;
        }
    }
}

/// The TLB maintenance instructions of FEAT_TLBIOS, FEAT_TLBIRANGE and
/// FEAT_XS invalidate as the ARMv8.0 ones do, and are UNDEFINED on a
/// processor without their features, and at EL0.
#[test]
fn the_later_tlbi_forms_need_their_features_and_invalidate() {
    /// Each TLBI, and what it needs besides the ARMv8.0 set.
    const FORMS: [(u32, &[Feature]); 10] = [
        (TLBI_VAE1, &[]),
        (TLBI_VAE1IS, &[]),
        (TLBI_VMALLE1OS, &[Feature::TlbiOs]),
        (TLBI_VAE1OS, &[Feature::TlbiOs]),
        (TLBI_RVAE1, &[Feature::TlbiRange]),
        (TLBI_RVAE1IS, &[Feature::TlbiRange]),
        (TLBI_RVAE1OS, &[Feature::TlbiOs, Feature::TlbiRange]),
        (TLBI_VMALLE1NXS, &[Feature::Xs]),
        (TLBI_VAE1OSNXS, &[Feature::TlbiOs, Feature::Xs]),
        (TLBI_RVAE1OSNXS, &[Feature::TlbiOs, Feature::TlbiRange, Feature::Xs]),
    ];
    let offered: [&[Feature]; 5] = [
        &[],
        &[Feature::TlbiOs],
        &[Feature::TlbiOs, Feature::TlbiRange],
        &[Feature::Xs],
        &[Feature::TlbiOs, Feature::TlbiRange, Feature::Xs],
    ];
    for extra in offered {
        for (insn, needs) in FORMS {
            let mut m = Machine::new(a57_with(extra));
            let implemented = needs.iter().all(|f| extra.contains(f));
            // Load through the read-only block, so the TLB holds it, then
            // take the block away.
            m.cpu.x[1] = READ_ONLY;
            assert_eq!(m.run(LDR), None);
            m.set_descriptor(L2 + 24, 0);
            assert_eq!(m.run(LDR), None, "the TLB still holds the block");
            if !implemented {
                assert_eq!(m.run(insn), undefined(&m), "{insn:#010x} with {extra:?}");
                continue;
            }
            m.at_el0();
            assert_eq!(m.run(insn), undefined(&m), "{insn:#010x} at EL0");
            m.cpu.pstate.el = El::El1;
            m.cpu.x[1] = READ_ONLY >> 12;
            assert_eq!(m.run(insn), None, "{insn:#010x} with {extra:?}");
            m.cpu.x[1] = READ_ONLY;
            assert_eq!(m.run(LDR), data_abort(El::El1, TRANSLATION_L2, READ_ONLY), "{insn:#010x} invalidated it");
        }
    }
}

/// DSB nXS is FEAT_XS's, for `CRm<1:0> = 10` alone: a barrier with
/// nothing to wait for, and UNDEFINED without the feature.
#[test]
fn dsb_nxs_needs_feat_xs() {
    for (imp, xs) in [(a57_with(&[]), false), (a57_with(&[Feature::Xs]), true)] {
        let mut m = Machine::new(imp);
        for insn in [DSB_OSHNXS, DSB_SYNXS] {
            let want = if xs { None } else { undefined(&m) };
            assert_eq!(m.run(insn), want, "{insn:#010x}, FEAT_XS {xs}");
            m.at_el0();
            assert_eq!(m.run(insn), want, "{insn:#010x} at EL0, FEAT_XS {xs}");
            m.cpu.pstate.el = El::El1;
        }
        assert_eq!(m.run(DSB_NXS_RESERVED), undefined(&m), "CRm<1:0> = 11");
    }
}

/// LDLAR and STLLR (FEAT_LOR) load and store as LDAR and STLR do: an
/// acquire or release access, with the ordered accesses' alignment rule
/// and PSTATE.PAN's check. Without the feature they are UNDEFINED.
#[test]
fn ldlar_and_stllr_are_ordered_accesses() {
    let mut m = Machine::new(Implementation::EVERY_FEATURE);
    m.cpu.x[1] = EL1_DATA;
    m.cpu.x[2] = 0x1122_3344_5566_7788;
    assert_eq!(m.run(STLLR), None);
    m.cpu.x[2] = 0;
    assert_eq!(m.run(LDLAR), None);
    assert_eq!(m.cpu.x[2], 0x1122_3344_5566_7788);
    assert_eq!(m.run(LDLARB), None);
    assert_eq!(m.cpu.x[2], 0x88, "a byte, zero-extended");
    m.cpu.x[2] = 0xABCD;
    assert_eq!(m.run(STLLRH), None);
    assert_eq!(m.run(LDLAR), None);
    assert_eq!(m.cpu.x[2], 0x1122_3344_5566_ABCD, "a halfword store");

    // A misaligned one faults or not as LDAR does, here across a 16-byte
    // boundary with SCTLR_EL1.nAA clear.
    m.cpu.x[1] = EL1_DATA + 12;
    let ldar = m.run(LDAR);
    assert!(ldar.is_some(), "LDAR across 16 bytes");
    assert_eq!(m.run(LDLAR), ldar);

    m.cpu.pstate.pan = true;
    m.cpu.x[1] = EL0_RW;
    assert_eq!(m.run(LDLAR), data_abort(El::El1, PERMISSION_L2, EL0_RW), "PAN applies");

    let mut a57 = Machine::new(a57_with(&[]));
    a57.cpu.x[1] = EL1_DATA;
    for insn in [LDLAR, STLLR, LDLARB, STLLRH] {
        assert_eq!(a57.run(insn), undefined(&a57), "{insn:#010x} without FEAT_LOR");
    }
}

/// The LORegion registers with FEAT_LOR: LORID_EL1 reports no LORegions
/// and no descriptors, and is read-only; LORSA_EL1, LOREA_EL1, LORN_EL1
/// and LORC_EL1 hold nothing (RAZ/WI). All are EL1's. Without FEAT_LOR
/// none exists.
#[test]
fn the_loregion_registers_hold_no_regions() {
    let mut m = Machine::new(a57_with(&[Feature::Lor]));
    assert_eq!(m.run(MRS_LORID), None);
    assert_eq!(m.cpu.x[0], 0, "LR = LD = 0");
    assert_eq!(m.run(MSR_LORID), undefined(&m), "LORID_EL1 is read-only");
    for (write, read) in [(MSR_LORSA, MRS_LORSA), (MSR_LOREA, MRS_LOREA), (MSR_LORN, MRS_LORN), (MSR_LORC, MRS_LORC)] {
        m.cpu.x[0] = 0xFFFF_0001;
        assert_eq!(m.run(write), None, "{write:#010x}");
        assert_eq!(m.run(read), None, "{read:#010x}");
        assert_eq!(m.cpu.x[0], 0, "{read:#010x} reads as zero");
        m.at_el0();
        assert_eq!(m.run(read), undefined(&m), "{read:#010x} at EL0");
        assert_eq!(m.run(write), undefined(&m), "{write:#010x} at EL0");
        m.cpu.pstate.el = El::El1;
    }

    let mut a57 = Machine::new(a57_with(&[]));
    for insn in [MRS_LORID, MRS_LORC, MSR_LORC] {
        assert_eq!(a57.run(insn), undefined(&a57), "{insn:#010x} without FEAT_LOR");
    }
}
