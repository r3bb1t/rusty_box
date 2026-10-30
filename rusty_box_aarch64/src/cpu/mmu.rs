//! Address translation (VMSAv8-64, Arm ARM D8, `AArch64_FullTranslate`)
//! with the 4 KiB granule, the one `ID_AA64MMFR0_EL1` advertises: stage 1
//! in the EL1&0 regime and with FEAT_EL2 the EL2 regime ([`Regime`]; their
//! walk parameters are `regime.rs`'s), and with FEAT_EL2 stage 2 for the
//! EL1&0 regime (`stage2.rs`), which also translates the addresses of the
//! descriptors stage 1's walk reads; the TLB that caches it; and the data
//! and instruction access paths every load, store, fetch and cache
//! maintenance by address goes through — alignment checks, translation,
//! permission checks, and the external abort when the bus has nothing at
//! an address.
//!
//! The TLB is direct-mapped and holds one 4 KiB page per entry, whatever the
//! size of the block that mapped it, tagged with the regime that translated
//! it (Arm `TLBContext`), so an access hits only an entry of its own regime.
//! An entry of the EL1&0 regime under stage 2 holds both stages' results:
//! each stage's permissions and level apart, and whether stage 1 maps
//! Device memory beside whether either does, so a hit raises each stage's
//! fault in Arm's order; with stage 1 disabled under stage 2 an
//! entry holds stage 2's translation of the address alone. Without stage 2
//! it holds only translations stage 1's walk made: with stage 1 disabled
//! and no stage 2 an address translates to itself, which no entry holds.
//!
//! Every TLB maintenance instruction empties the TLB whole. A write that
//! changes a register the EL2 regime's walk reads empties it too
//! (`SCTLR_EL2`, `TCR_EL2`, `TTBR0_EL2`, `MAIR_EL2`). A write that changes
//! what the EL1&0 regime's translations read of its registers (`SCTLR_EL1`,
//! `TCR_EL1`, `TTBR0/1_EL1`, `MAIR_EL1`, and with EL2 `VTTBR_EL2`,
//! `VTCR_EL2` and `HCR_EL2`'s `VM`, `PTW`, `DC` and `TGE`) empties it
//! without EL2; with EL2 it changes the context the regime's entries are
//! made and found in ([`El10Context`], [`Tlb`]), each set of those
//! registers' values its own, so no entry made under other values is used
//! and a hypervisor's return to a guest's registers finds the entries it
//! had; stage 2's translations of IPAs are kept apart, under `VTTBR_EL2`
//! and `VTCR_EL2` alone ([`Stage2Context`]). Invalidation is never narrower
//! than the architecture requires: an entry is never used under register
//! values other than those it was made under, so neither ASIDs nor VMIDs
//! need tagging.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::dp::ones;
use super::el2::{
    El2Reg, El2Regs, El2Storage, HCR_CD, HCR_DC, HCR_PTW, HCR_TGE, HCR_VM, VTTBR_VMID8,
};
use super::exception::{AccessSyndrome, AddressedClass, Exception, SyndromeClass, SyndromeSlot};
use super::exec::{Fault, StopCause};
use super::features::{Feature, Features};
use super::memattr::{self, MemAttrs};
use super::regime::{
    el10_s1_disabled_data_normal, el10_stage1_enabled, el10_stage2_enabled, Regime, S1Params,
};
use super::stage2::S2Walked;
use super::{id, sctlr, Cpu, El, Levels, SysRegs};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus, PhysSpan};

/// What an access does, for permission checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessKind {
    Read,
    Write,
    Fetch,
}

/// What a data access does at its address, which decides the WnR its
/// faults report: Arm `NoFault`'s `!accdesc.read && accdesc.write`, so a
/// read-modify-write reports an alignment or translation fault as a read,
/// and `AArch64_S1CheckPermissions`, which checks read permission before
/// write permission.
///
/// Closed (R5): the directions of a data access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataDirection {
    Read,
    Write,
    /// An atomic read-modify-write (`CreateAccDescAtomicOp`).
    ReadWrite,
}

/// Which regime an `AT` instruction translates in, and whose permissions it
/// checks the translation against.
///
/// Closed (R5): the stage 1 address translation instructions of a
/// processor with EL2 and no EL3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtCheck {
    /// `AT S1E0R`, `AT S1E0W`: EL1&0, EL0's.
    El0,
    /// `AT S1E1R`, `AT S1E1W`: EL1&0, EL1's, which PSTATE.PAN takes no part
    /// in (Arm `CreateAccDescAT`, `ATAccess_Read`/`ATAccess_Write`).
    El1,
    /// `AT S1E1RP`, `AT S1E1WP` (FEAT_PAN2): EL1&0, EL1's, with PSTATE.PAN
    /// as a data access has it (`ATAccess_ReadPAN`/`ATAccess_WritePAN`).
    El1Pan,
    /// `AT S1E2R`, `AT S1E2W` (FEAT_EL2): EL2, its own.
    El2,
}

/// Which stages an `AT` instruction translates by (Arm `TranslationStage`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtStages {
    /// `AT S1E*`: stage 1 alone, whose output may be an IPA.
    One,
    /// `AT S12E*` (FEAT_EL2): the EL1&0 regime's stage 1, then stage 2
    /// where `HCR_EL2.VM` or `DC` enables it.
    OneAndTwo,
}

/// The constraints a data access carries beyond its size, as Arm's
/// `AccessDescriptor` carries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DataAccess {
    /// `LDTR`/`STTR`: checked against EL0 permissions when executed at EL1
    /// with PSTATE.UAO clear (Arm `AArch64_IsUnprivAccessPriv`).
    pub(crate) unprivileged: bool,
    pub(crate) alignment: AlignmentRule,
    pub(crate) part: Part,
    pub(crate) syndrome: InstructionSyndrome,
}

impl DataAccess {
    pub(crate) const NORMAL: Self = Self {
        unprivileged: false,
        alignment: AlignmentRule::Any,
        part: Part::Whole,
        syndrome: InstructionSyndrome::Absent,
    };
    /// A load-acquire or store-release.
    pub(crate) const ORDERED: Self = Self {
        unprivileged: false,
        alignment: AlignmentRule::Ordered,
        part: Part::Whole,
        syndrome: InstructionSyndrome::Absent,
    };
    /// An exclusive or atomic access.
    pub(crate) const ATOMIC: Self = Self {
        unprivileged: false,
        alignment: AlignmentRule::Atomic,
        part: Part::Whole,
        syndrome: InstructionSyndrome::Absent,
    };
    pub(crate) const UNPRIVILEGED: Self = Self {
        unprivileged: true,
        alignment: AlignmentRule::Any,
        part: Part::Whole,
        syndrome: InstructionSyndrome::Absent,
    };

    /// The same access, made by a load or store of one general-purpose
    /// register ([`InstructionSyndrome::Reported`]).
    #[inline(always)]
    pub(crate) const fn of_one_register(self) -> Self {
        Self {
            syndrome: InstructionSyndrome::Reported,
            ..self
        }
    }

    /// The [`SyndromeSlot`] a stage 2 Data Abort on this access at `at`
    /// may leave open: only for an instruction with a syndrome, at the
    /// access's lowest address.
    const fn syndrome_slot(self, at: AddressInAccess) -> SyndromeSlot {
        match (self.syndrome, at) {
            (InstructionSyndrome::Reported, AddressInAccess::Lowest) => SyndromeSlot::Open,
            (InstructionSyndrome::Reported, AddressInAccess::Later)
            | (InstructionSyndrome::Absent, _) => SyndromeSlot::Closed,
        }
    }
}

/// Whether the instruction making a data access may have a syndrome a
/// stage 2 Data Abort on the access reports (`ESR_ELx.ISV`, Arm
/// `LSInstructionSyndrome`): `Reported` for a load or store of one
/// general-purpose register, whose form decides when the access faults
/// whether it gives its syndrome or none — one with writeback none — and
/// closes the abort's [`SyndromeSlot`] either way ([`Cpu::read_gpr`],
/// `op::one_register_syndrome`, [`Exception::reporting`]); `Absent` for every
/// other access, whose abort reports `ISV` 0 whatever else is done with it.
/// Every access is `Absent` unless its instruction says otherwise.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstructionSyndrome {
    Reported,
    Absent,
}

/// Where in an instruction's access a translated address lies, as Arm's
/// `accdesc.lowestaddress` records it: at the access's lowest address, or
/// at a byte after the first of an access `Mem[]` splits into bytes. Only
/// a stage 2 abort at the lowest address reports the instruction's
/// syndrome ([`SyndromeSlot`]).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddressInAccess {
    Lowest,
    Later,
}

impl AddressInAccess {
    /// The address of byte `i` of an access split into bytes.
    const fn of_byte(i: u64) -> Self {
        if i == 0 {
            Self::Lowest
        } else {
            Self::Later
        }
    }
}

/// What a data access is of the access an instruction makes, for Device
/// memory's alignment check.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    /// All of it: judged by its own size and address.
    Whole,
    /// One single-copy access of a transfer that is misaligned as a whole:
    /// an 8-byte half of a 16-byte SIMD&FP access, or a byte of an SVE
    /// `LDR`/`STR` whose base is misaligned. Arm passes the whole
    /// transfer's `aligned` to `AArch64_MemSingle` for each part, so Device
    /// memory refuses the part however it is itself aligned.
    OfMisaligned,
}

/// Which misaligned data accesses fault before translation, the part of Arm
/// `AArch64_UnalignedAccessFaults` an access's kind decides.
/// The current regime's `SCTLR_ELx.A` faults every misaligned access
/// whatever the rule, and
/// Device memory every one, after the walk and before the permissions
/// ([`OnDevice`]).
///
/// Closed (R5): the kinds of access the instruction set has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlignmentRule {
    /// An ordinary load or store: none.
    Any,
    /// An exclusive or atomic access (`accdesc.exclusive`, `atomicop`):
    /// every one, but with FEAT_LSE2 those that lie in one 16-byte granule
    /// (`AllInSCAGranule`, whose IMPLEMENTATION DEFINED quantity is 16 here).
    Atomic,
    /// A load-acquire or store-release (`acqsc`, `acqpc`, `relsc`): with
    /// FEAT_LSE2, those that cross a 16-byte boundary while `SCTLR_EL1.nAA`
    /// is 0. Without it, every one, the rule before FEAT_LSE2 that QEMU
    /// keeps (`check_ordered_align`): the pseudocode's rule reads `nAA`,
    /// which FEAT_LSE2 adds.
    Ordered,
}

/// What a cache maintenance instruction by virtual address does, which
/// decides the permission its translation needs (Arm
/// `AArch64_S1CheckPermissions`, `AccessType_DC` and `AccessType_IC`).
/// PSTATE.PAN takes no part (`CreateAccDescDC` and `CreateAccDescIC` leave
/// `pan` clear).
///
/// Closed (R5): the maintenance by address this core implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheMaintenance {
    /// `DC IVAC` (`CacheOp_Invalidate`): write permission, at any EL.
    Invalidate,
    /// `DC CVAC`, `DC CVAU`, `DC CVAP`, `DC CVADP` and `DC CIVAC`
    /// (`CacheOp_Clean`, `CacheOp_CleanInvalidate`): read permission at
    /// EL0, none at EL1.
    Clean,
    /// `IC IVAU`: none, the IMPLEMENTATION DEFINED choice "Permission fault
    /// on EL0 IC_IVAU execution" being FALSE here.
    InstructionInvalidate,
}

/// Whether Device memory refuses an access for its alignment, as Arm
/// `AArch64_S1HasAlignmentFaultDueToMemType` decides after the walk and
/// before the permission check: a misaligned access (a [`Part`] of one
/// included), and `DC ZVA`, are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnDevice {
    Refused,
    Permitted,
}

/// A fault status code (`ESR_ELx.ISS.DFSC`/`IFSC`) with its lookup level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fsc {
    AddressSize(u8),
    Translation(u8),
    AccessFlag(u8),
    Permission(u8),
    SyncExternal,
    SyncExternalOnWalk(u8),
    Alignment,
}

impl Fsc {
    const fn code(self) -> u32 {
        match self {
            Self::AddressSize(l) => l as u32,
            Self::Translation(l) => 0b00_0100 | l as u32,
            Self::AccessFlag(l) => 0b00_1000 | l as u32,
            Self::Permission(l) => 0b00_1100 | l as u32,
            Self::SyncExternal => 0b01_0000,
            Self::SyncExternalOnWalk(l) => 0b01_0100 | l as u32,
            Self::Alignment => 0b10_0001,
        }
    }
}

/// Why a translation table walk found no translation.
///
/// Closed (R5): the faults a walk can end in before the permissions of the
/// access it translates are considered. `level` is the lookup level the
/// walk had reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranslationFault {
    /// The address, or a table or output address, is wider than the
    /// configured address size.
    AddressSize { level: u8 },
    /// No valid descriptor maps it.
    Translation { level: u8 },
    /// The descriptor that maps it has its access flag clear, which this
    /// ARMv8.0 core faults on rather than sets.
    AccessFlag { level: u8 },
    /// A descriptor read reached nothing on the bus.
    ExternalAbortOnWalk { level: u8 },
    /// Stage 2 refuses a stage 1 walk the descriptor it reads: its stage 2
    /// descriptor gives no read permission, or `HCR_EL2.PTW` refuses it
    /// Device memory (`AArch64_S2CheckPermissions` for `AccessType_TTW`).
    /// `level` is the stage 2 walk's.
    Permission { level: u8 },
}

impl core::fmt::Display for TranslationFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AddressSize { level } => write!(f, "address size fault at level {level}"),
            Self::Translation { level } => write!(f, "translation fault at level {level}"),
            Self::AccessFlag { level } => write!(f, "access flag fault at level {level}"),
            Self::ExternalAbortOnWalk { level } => {
                write!(f, "external abort on the walk at level {level}")
            }
            Self::Permission { level } => write!(f, "permission fault at level {level}"),
        }
    }
}

impl core::error::Error for TranslationFault {}

impl From<TranslationFault> for Fsc {
    fn from(fault: TranslationFault) -> Self {
        match fault {
            TranslationFault::AddressSize { level } => Self::AddressSize(level),
            TranslationFault::Translation { level } => Self::Translation(level),
            TranslationFault::AccessFlag { level } => Self::AccessFlag(level),
            TranslationFault::ExternalAbortOnWalk { level } => Self::SyncExternalOnWalk(level),
            TranslationFault::Permission { level } => Self::Permission(level),
        }
    }
}

/// An intermediate physical address (IPA): what stage 1 outputs, which
/// stage 2 translates to a physical address where it translates the
/// regime (R4: neither a virtual nor a physical address).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ipa(u64);

impl Ipa {
    pub const fn new(address: u64) -> Self {
        Self(address)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    /// The physical address the IPA is where stage 2 does not translate
    /// it — stage 2 disabled, or the EL2 regime, which has none — the IPA
    /// itself (Arm `AArch64_FullTranslate`, without `AArch64_S2Translate`).
    #[inline(always)]
    pub(crate) const fn untranslated(self) -> PhysAddr {
        PhysAddr::new(self.0)
    }
}

impl core::fmt::Display for Ipa {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "IPA {:#x}", self.0)
    }
}

/// Which walk a translation fault arose in (Arm `FaultRecord.secondstage`
/// and `s2fs1walk`, with `ipaddress`).
///
/// Closed (R5): stage 1, and with FEAT_EL2 stage 2 of the EL1&0 regime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranslationStage {
    /// Stage 1, translating the virtual address.
    One,
    /// Stage 2, translating `ipa`, the virtual address's intermediate
    /// physical address.
    Two { ipa: Ipa },
    /// Stage 2, translating `ipa`, the intermediate physical address of a
    /// descriptor stage 1's walk reads (`ESR_ELx.S1PTW`).
    TwoOnWalk { ipa: Ipa },
}

/// Why a virtual address has no translation: the walk's fault, and the
/// stage it arose in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NoTranslation {
    pub stage: TranslationStage,
    pub fault: TranslationFault,
}

impl NoTranslation {
    /// A fault of stage 1.
    const fn stage1(fault: TranslationFault) -> Self {
        Self {
            stage: TranslationStage::One,
            fault,
        }
    }

    /// The abort the fault raises for an access whose `FaultRecord.write`
    /// is `write`.
    fn abort(self, write: bool) -> Abort {
        let fsc = Fsc::from(self.fault);
        let stage = match self.stage {
            TranslationStage::One => AbortStage::First,
            TranslationStage::Two { ipa } => AbortStage::Second {
                hpfar: ipa_valid(fsc, false).then_some(ipa),
            },
            TranslationStage::TwoOnWalk { ipa } => AbortStage::SecondOnWalk {
                hpfar: ipa_valid(fsc, true).then_some(ipa),
            },
        };
        Abort { fsc, stage, write }
    }
}

/// The fault and its stage in one line, so an error report shows it once:
/// `source` names nothing beneath it.
impl core::fmt::Display for NoTranslation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.stage {
            TranslationStage::One => write!(f, "stage 1 {}", self.fault),
            TranslationStage::Two { ipa } => {
                write!(f, "stage 2 {} translating {ipa}", self.fault)
            }
            TranslationStage::TwoOnWalk { ipa } => write!(
                f,
                "stage 2 {} translating {ipa}, a stage 1 descriptor's",
                self.fault
            ),
        }
    }
}

impl core::error::Error for NoTranslation {}

/// Arm `IPAValid` of a stage 2 fault whose status is `fsc`, on a stage 1
/// walk's descriptor read where `on_walk`: whether `HPFAR_EL2` reports the
/// IPA — for a Translation, Access flag or Address size fault, and on a
/// stage 1 walk for a Permission fault too.
const fn ipa_valid(fsc: Fsc, on_walk: bool) -> bool {
    match fsc {
        Fsc::AddressSize(_) | Fsc::Translation(_) | Fsc::AccessFlag(_) => true,
        Fsc::Permission(_) => on_walk,
        Fsc::SyncExternal | Fsc::SyncExternalOnWalk(_) | Fsc::Alignment => false,
    }
}

/// Where an abort arose, for its syndrome, its routing and what
/// `HPFAR_EL2` reports.
///
/// Closed (R5): stage 1, stage 2 on the access, and stage 2 on stage 1's
/// walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbortStage {
    /// Stage 1, or with stage 1 disabled the checks of its output.
    First,
    /// Stage 2, translating the access's IPA. `hpfar` is the IPA where
    /// `IPAValid`.
    Second { hpfar: Option<Ipa> },
    /// Stage 2, translating the IPA of a descriptor stage 1's walk reads.
    /// `hpfar` is that IPA where `IPAValid`.
    SecondOnWalk { hpfar: Option<Ipa> },
}

/// A translation that raises an abort: its fault status, its stage, and
/// Arm's `FaultRecord.write` — whether the access writes and reads nothing
/// (`NoFault`'s `!accdesc.read && accdesc.write`), or for a Permission
/// fault whether the permission refused was write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Abort {
    pub(crate) fsc: Fsc,
    pub(crate) stage: AbortStage,
    pub(crate) write: bool,
}

impl Abort {
    /// A fault of stage 1.
    const fn first(fsc: Fsc, write: bool) -> Self {
        Self {
            fsc,
            stage: AbortStage::First,
            write,
        }
    }

    /// A fault of stage 2 on the access whose IPA `IPAValid` does not report
    /// ([`ipa_valid`]): an Alignment or Permission fault, which the checks of
    /// a TLB entry, holding no IPA, raise as a walk's leaf's do.
    const fn second_unreported(fsc: Fsc, write: bool) -> Self {
        debug_assert!(!ipa_valid(fsc, false), "IPAValid reports this fault's IPA");
        Self {
            fsc,
            stage: AbortStage::Second { hpfar: None },
            write,
        }
    }
}

/// The permissions an access needs at each stage of its translation, as
/// Arm's access descriptor gives them to the permission checks
/// (`AArch64_S1CheckPermissions`, `AArch64_S2CheckPermissions`).
///
/// Closed (R5): the accesses this core translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Needs {
    /// A data read, a data write or an instruction fetch.
    Access(AccessKind),
    /// An atomic read-modify-write (`CreateAccDescAtomicOp`): read
    /// permission, then write permission.
    ReadWrite,
    /// A cache maintenance instruction by address.
    Maintenance(CacheMaintenance),
}

impl Needs {
    /// What the walk translates for: an instruction address for a fetch, a
    /// data address for everything else.
    const fn kind(self) -> AccessKind {
        match self {
            Self::Access(kind) => kind,
            Self::ReadWrite | Self::Maintenance(_) => AccessKind::Read,
        }
    }

    /// `FaultRecord.write` of a fault that is not a refused permission
    /// (`NoFault`): a write, and no read. A cache maintenance instruction's
    /// abort reports WnR set whatever this is ([`Cpu::maintenance_abort`]).
    const fn writes(self) -> bool {
        matches!(self, Self::Access(AccessKind::Write))
    }
}

/// The permission an access needs to maintain the cache at `va` by `op`
/// at `el` (`AArch64_S1CheckPermissions` and `AArch64_S2CheckPermissions`
/// for `AccessType_DC` and `AccessType_IC`, alike at both stages): `DC IVAC`
/// write permission, the other `DC` operations read permission at EL0 and
/// none above, and `IC IVAU` none, the IMPLEMENTATION DEFINED choice
/// "Permission fault on EL0 IC_IVAU execution" being FALSE here.
const fn maintenance_needs(op: CacheMaintenance, el: El) -> Option<AccessKind> {
    match (op, el) {
        (CacheMaintenance::Invalidate, _) => Some(AccessKind::Write),
        (CacheMaintenance::Clean, El::El0) => Some(AccessKind::Read),
        (CacheMaintenance::Clean, El::El1 | El::El2)
        | (CacheMaintenance::InstructionInvalidate, _) => None,
    }
}

/// Whether a stage's permission checks take PSTATE.PAN into a data
/// access's (`AArch64_S1CheckPermissions`) or not (`AArch64_S2CheckPermissions`).
///
/// Closed (R5): the two stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pan {
    Checked,
    Absent,
}

/// Whether stage 1's walk made a TLB entry, or the entry is of an access
/// stage 1 left as it was, being disabled (`AArch64_S1DisabledOutput`),
/// which only stage 2 translates and whose permissions stage 1 does not
/// check.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Stage1 {
    Walked = 0,
    Disabled = 1,
}

/// The stage 2 translations one translation made, under stage 2's context
/// `tag` ([`Cpu::found_stage2_tag`]), for the TLB to keep once the translation is
/// done ([`Tlb::keep`]): one for each IPA page stage 1's walk read a
/// descriptor in and one for the output, each page once, as the later
/// stage 2 translations of the same translation find them here first.
#[derive(Debug, Clone, Copy)]
struct Stage2Made {
    tag: u64,
    made: [Option<IpaEntry>; MADE],
}

/// The most IPA pages one translation under stage 2 reads: a descriptor at
/// each of the four levels stage 1's walk can read with the 4 KiB granule
/// and no FEAT_LPA2 (levels 0 to 3), and the output.
const MADE: usize = 4 + 1;

impl Stage2Made {
    const fn new(tag: u64) -> Self {
        Self {
            tag,
            made: [None; MADE],
        }
    }

    /// The key of the stage 2 translation of `ipa`'s page in the context.
    const fn key(&self, ipa: Ipa) -> u64 {
        (ipa.get() >> 12) | self.tag
    }

    /// The stage 2 translation `key` names, if this translation made it.
    fn find(&self, key: u64) -> Option<IpaEntry> {
        self.made
            .iter()
            .flatten()
            .find(|entry| entry.key == key)
            .copied()
    }

    /// Record `entry`, a page this translation had not yet made: there is
    /// room for every page one translation reads ([`MADE`]). The record is
    /// a cache, as the TLB's kept translations are: a translation it could
    /// not hold would only be walked again when next needed.
    fn record(&mut self, entry: IpaEntry) {
        let free = self.made.iter_mut().find(|slot| slot.is_none());
        debug_assert!(free.is_some(), "a translation reads at most MADE IPA pages");
        if let Some(slot) = free {
            *slot = Some(entry);
        }
    }
}

/// Where a walk's stage 2 translations come from: an access's from the
/// ones its translation made and the TLB's kept ones, a walk recording the
/// ones it makes; an address translation instruction's and a debugger's
/// from walks alone, which the TLB neither answers nor keeps (as it does
/// not their stage 1 walks).
///
/// Closed (R5).
enum Stage2Source<'a> {
    Cached(&'a mut Stage2Made),
    Walked,
}

/// What stage 1 gives a translation, as far as its output (Arm
/// `AArch64_S1Translate` before its checks): its walk's leaf with stage 1
/// enabled, or with it disabled its output address
/// (`AArch64_S1DisabledOutput`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage1Output {
    Walked(Walked),
    Disabled(Ipa),
}

impl Stage1Output {
    /// The output address: an IPA where stage 2 translates the regime, the
    /// physical address where it does not.
    const fn ipa(&self) -> Ipa {
        match self {
            Self::Walked(walked) => walked.ipa,
            Self::Disabled(ipa) => *ipa,
        }
    }
}

/// What stage 2 gives an IPA, as far as its checks (Arm
/// `AArch64_S2Translate` before them): the physical address, and the leaf
/// the checks read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stage2Output {
    pa: PhysAddr,
    leaf: Stage2Leaf,
}

/// The permissions of the regime's privileged level: EL1's in EL1&0, EL2's
/// in the EL2 regime, which has no other.
pub(super) const PERM_EL1_R: u8 = 1 << 0;
pub(super) const PERM_EL1_W: u8 = 1 << 1;
pub(super) const PERM_EL1_X: u8 = 1 << 2;
/// EL0's permissions, which only the EL1&0 regime has.
pub(super) const PERM_EL0_R: u8 = 1 << 3;
pub(super) const PERM_EL0_W: u8 = 1 << 4;
pub(super) const PERM_EL0_X: u8 = 1 << 5;
const PERMS: u8 = PERM_EL1_R | PERM_EL1_W | PERM_EL1_X | PERM_EL0_R | PERM_EL0_W | PERM_EL0_X;
/// Not a permission: the descriptor's GP bit (FEAT_BTI), a guarded page.
const GUARDED: u8 = 1 << 6;
/// EL0's permissions.
const PERMS_EL0: u8 = PERM_EL0_R | PERM_EL0_W | PERM_EL0_X;

/// `ESR_ELx.ISS.WnR`, `S1PTW` and `CM` of an abort (`S1PTW` an Instruction
/// Abort's too).
const ISS_WNR: u32 = 1 << 6;
const ISS_S1PTW: u32 = 1 << 7;
const ISS_CM: u32 = 1 << 8;

/// `TCR_EL1.E0PD0` (FEAT_E0PD); `E0PD1` is the bit above it.
const TCR_E0PD0: u32 = 55;

// The PAN rule's level, EL1 (FEAT_PAN without FEAT_VHE has none at EL2),
// is the one whose index is odd, so that `Cpu::pan_refuses` tests the
// level and `PSTATE.PAN` together.
const _: () =
    assert!(El::El0.index() & 1 == 0 && El::El1.index() & 1 == 1 && El::El2.index() & 1 == 0);

/// Whether `perms` allow an access of `kind` checked against `el`'s
/// permissions: EL0's at EL0, and the regime's privileged level's at EL1
/// and EL2. One table, so that with `kind` known where the access is made
/// it compiles to a choice between two constants.
#[inline]
const fn permitted(perms: u8, kind: AccessKind, el: El) -> bool {
    let needed = match (kind, el) {
        (AccessKind::Read, El::El1 | El::El2) => PERM_EL1_R,
        (AccessKind::Write, El::El1 | El::El2) => PERM_EL1_W,
        (AccessKind::Fetch, El::El1 | El::El2) => PERM_EL1_X,
        (AccessKind::Read, El::El0) => PERM_EL0_R,
        (AccessKind::Write, El::El0) => PERM_EL0_W,
        (AccessKind::Fetch, El::El0) => PERM_EL0_X,
    };
    perms & needed != 0
}

/// Whether `tcr`'s FEAT_E0PD field for the half of the address space bit 55
/// of `va` names refuses EL0 every access there (`TCR_EL1.E0PD0` or
/// `E0PD1`, RES0 without the feature).
#[inline]
const fn e0pd_set(features: Features, tcr: u64, va: u64) -> bool {
    features.has(Feature::E0pd) && (tcr >> (TCR_E0PD0 + ((va >> 55) & 1) as u32)) & 1 == 1
}

/// The outcome of a translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Translated {
    pub(crate) pa: u64,
}

/// A translation that permits its access, with what an access reads of its
/// page past the permission checks: whether stage 1 made the page guarded
/// (FEAT_BTI's `InGuardedPage`, which an instruction fetch records; never
/// with stage 1 disabled, `AArch64_S1DisabledOutput`), and whether either
/// stage maps Device memory, which a misaligned data access checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mapping {
    pa: u64,
    guarded: bool,
    device: bool,
}

/// The physical address of `va` in the page at `pa_page`.
#[inline(always)]
const fn in_page(pa_page: u64, va: u64) -> u64 {
    pa_page | (va & 0xFFF)
}

/// The two adjacent elements of a pair access: `first` at the lower
/// address, `second` just above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ElementPair {
    pub(crate) first: u64,
    pub(crate) second: u64,
}

/// The two Q registers of a SIMD&FP pair access: `first` at the lower
/// address, `second` 16 bytes above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QuadwordPair {
    pub(crate) first: u128,
    pub(crate) second: u128,
}

/// A completed stage 1 walk, before permissions are checked against an
/// access. `ipa` is the output address, which stage 2 translates where it
/// translates the regime ([`Ipa::untranslated`] where it does not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Walked {
    ipa: Ipa,
    perms: u8,
    level: u8,
    /// The regime's `MAIR_ELx` attribute the descriptor's `AttrIndx`
    /// selects, and the descriptor's `SH`, as they are: an access reads
    /// only whether they make Device memory, and only address translation
    /// decodes them ([`Walked::attrs`]).
    attr: u8,
    sh: u8,
}

impl Walked {
    /// Whether stage 1 maps Device memory on a processor implementing
    /// `features` ([`memattr::stage1_device`]).
    #[inline]
    const fn device(&self, features: Features) -> bool {
        memattr::stage1_device(self.attr, features)
    }

    /// The attributes stage 1 gives (`S1DecodeMemAttrs`).
    const fn attrs(&self, features: Features) -> MemAttrs {
        MemAttrs::stage1(self.attr, self.sh, features)
    }
}

/// The address space a table walk outputs into, which its tables lie in
/// too: [`Ipa`] for stage 1's walk (the physical address itself where no
/// stage 2 translates it), [`PhysAddr`] for stage 2's (R4).
pub(super) trait WalkOutput: Copy {
    fn new(address: u64) -> Self;
    fn get(self) -> u64;
}

impl WalkOutput for Ipa {
    #[inline(always)]
    fn new(address: u64) -> Self {
        Ipa::new(address)
    }

    #[inline(always)]
    fn get(self) -> u64 {
        Ipa::get(self)
    }
}

impl WalkOutput for PhysAddr {
    #[inline(always)]
    fn new(address: u64) -> Self {
        PhysAddr::new(address)
    }

    #[inline(always)]
    fn get(self) -> u64 {
        PhysAddr::get(self)
    }
}

/// Where a table walk starts ([`Cpu::walk_tables`]): the start level, the
/// bits of the input address its table indexes (more than 9 for stage 2's
/// concatenated tables), the table's address in the walk's output address
/// space `A`, and the output size every table and output address must fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WalkStart<A> {
    pub(super) level: u32,
    pub(super) index_bits: u32,
    pub(super) table: A,
    pub(super) oa_bits: u32,
}

/// A translation table base register's `BADDR` with the 4 KiB granule and
/// a 48-bit output address, bits 47:1: its bit 0 is `CnP` (FEAT_TTCNP),
/// which has no effect on one processor.
pub(crate) const TTB_BADDR: u64 = 0x0000_FFFF_FFFF_FFFE;

impl<A: WalkOutput> WalkStart<A> {
    /// Where a walk of an `ia_bits`-bit input address starting at `level`
    /// from the base register `ttbr` starts, its tables' addresses fitting
    /// `oa_bits` (Arm `AArch64_S1TTBaseAddress` and
    /// `AArch64_S2TTBaseAddress` with the 4 KiB granule, without FEAT_LPA2
    /// or FEAT_D128): `BADDR` aligned to the start table, whose index is
    /// the input address's bits above the start level's, more than nine
    /// for stage 2's concatenated tables. A base outside the output size
    /// is a level 0 Address size fault: the start level's descriptor
    /// address is the base with an index below its alignment, so it fits
    /// exactly when the base does.
    pub(super) fn of_base(
        ttbr: u64,
        level: u32,
        ia_bits: u32,
        oa_bits: u32,
    ) -> Result<Self, TranslationFault> {
        let index_bits = ia_bits - (12 + 9 * (3 - level));
        let table = (ttbr & TTB_BADDR) & !ones(index_bits + 3);
        if table >> oa_bits != 0 {
            return Err(TranslationFault::AddressSize { level: 0 });
        }
        Ok(Self {
            level,
            index_bits,
            table: A::new(table),
            oa_bits,
        })
    }
}

/// The leaf a table walk ends at: its descriptor and level, the output
/// address of the input address, and the OR of every table descriptor's
/// bits 63:59.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WalkLeaf<A> {
    pub(super) desc: u64,
    pub(super) level: u32,
    pub(super) out: A,
    pub(super) tables: u64,
}

/// Why a table walk ended without a leaf: a fault of its own, or the
/// failure of a descriptor's address to translate — a stage 1 descriptor's
/// under stage 2; `Infallible` for stage 2's walk, whose descriptors'
/// addresses are physical.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WalkStop<E> {
    Fault(TranslationFault),
    Descriptor(E),
}

/// What stage 1 gave a translation, as its checks read it
/// ([`Cpu::s1_refusal`]): its permissions, and whether its page is guarded;
/// whether it maps Device memory for a data access; its walk's level; and
/// whether its walk made it. With stage 1 disabled, every permission, no
/// guarded page, Device memory unless `HCR_EL2.DC` makes a data access
/// Normal, and level 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stage1Leaf {
    perms: u8,
    device: bool,
    level: u8,
    stage1: Stage1,
}

/// What a TLB entry holds of stage 2: its permissions, as the stage 1
/// permission bits they leave ([`S2Walked::perms`]), whether it maps Device
/// memory, and its walk's level. Without stage 2, every permission, Normal
/// memory and level 0, which refuse nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stage2Leaf {
    perms: u8,
    device: bool,
    level: u8,
}

impl Stage2Leaf {
    /// The entry of a translation stage 2 takes no part in.
    const NONE: Self = Self {
        perms: PERMS,
        device: false,
        level: 0,
    };

    const fn of(walked: &S2Walked) -> Self {
        Self {
            perms: walked.perms,
            device: walked.device(),
            level: walked.level,
        }
    }

    /// Whether `perms` and `level` are a stage 2 descriptor's
    /// (`AArch64_S2DirectBasePermissions` without FEAT_XNX): read, write and
    /// execute alike at EL1 and EL0, at level 1 to 3.
    fn walked(perms: u8, level: u8) -> bool {
        let same_at_both = |el1: u8, el0: u8| (perms & el1 == 0) == (perms & el0 == 0);
        (1..=3).contains(&level)
            && perms & !PERMS == 0
            && same_at_both(PERM_EL1_R, PERM_EL0_R)
            && same_at_both(PERM_EL1_W, PERM_EL0_W)
            && same_at_both(PERM_EL1_X, PERM_EL0_X)
    }
}

/// A stage 2 translation of one IPA page the TLB keeps: the IPA page
/// number (or `INVALID_KEY`), the output page, and the leaf.
#[derive(Debug, Clone, Copy)]
struct IpaEntry {
    key: u64,
    pa_page: PhysAddr,
    leaf: Stage2Leaf,
}

const EMPTY_IPA_ENTRY: IpaEntry = IpaEntry {
    key: INVALID_KEY,
    pa_page: PhysAddr::new(0),
    leaf: Stage2Leaf::NONE,
};

/// How many stage 2 translations of IPA pages the TLB keeps: one for each
/// page of a guest's translation tables that its stage 1 walks read, across
/// the processes whose translations the TLB holds, and for each page those
/// walks lead to.
const IPA_ENTRIES: usize = 256;

// `Tlb::ipa_slot` folds an IPA page number, at most `PA_BITS - 12` bits
// (`AArch64_S2MinTxSZ`), eight bits at a time into a slot's eight.
const _: () = assert!(IPA_ENTRIES == 1 << 8 && id::PA_BITS - 12 <= 40);

/// The registers the EL1&0 regime's translations depend on: stage 1's
/// (`SCTLR_EL1`, `TCR_EL1`, `TTBR0_EL1`, `TTBR1_EL1`, `MAIR_EL1`), and with
/// EL2 stage 2's (`VTTBR_EL2`, `VTCR_EL2`) and the `HCR_EL2` bits that
/// decide which walks are made (`VM`, `PTW`, `DC`, `TGE`); 0 for each EL2
/// register without EL2. Every entry of the regime the TLB holds was made
/// under one such set, its context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct El10Context {
    pub(crate) sctlr_el1: u64,
    pub(crate) tcr_el1: u64,
    pub(crate) ttbr0_el1: u64,
    pub(crate) ttbr1_el1: u64,
    pub(crate) mair_el1: u64,
    pub(crate) hcr_el2: u64,
    pub(crate) vttbr_el2: u64,
    pub(crate) vtcr_el2: u64,
}

/// The `HCR_EL2` bits an [`El10Context`] holds.
const HCR_TRANSLATION: u64 = HCR_VM | HCR_PTW | HCR_DC | HCR_TGE;

impl El10Context {
    /// The context the EL1&0 regime's registers in `sys` and, with EL2,
    /// `el2` make ([`El10Context::normalized`]): the current one of a
    /// processor holding them.
    pub(crate) fn of(sys: &SysRegs, el2: Option<&El2Regs>) -> Self {
        Self {
            sctlr_el1: sys.sctlr_el1,
            tcr_el1: sys.tcr_el1,
            ttbr0_el1: sys.ttbr0_el1,
            ttbr1_el1: sys.ttbr1_el1,
            mair_el1: sys.mair_el1,
            hcr_el2: el2.map_or(0, |regs| regs.hcr),
            vttbr_el2: el2.map_or(0, |regs| regs.vttbr),
            vtcr_el2: el2.map_or(0, |regs| regs.vtcr),
        }
        .normalized()
    }

    /// The context `self`'s registers make, as far as a translation reads
    /// them: of `SCTLR_EL1` its `M` and, with stage 1 enabled, `WXN` (every
    /// other bit is read by the access, not cached); with stage 1 disabled
    /// no table base and no `MAIR_EL1` (`TCR_EL1` stays, whose `TBI` bits
    /// judge the addresses stage 1 then leaves); and with stage 2 disabled
    /// of `VTTBR_EL2` only its VMID, which tags the regime's translations
    /// whenever EL2 is enabled (Arm `AArch64_TLBContextEL10`, `VMID()`), and
    /// no `VTCR_EL2` or `HCR_EL2.PTW`. Register values that translate alike
    /// and carry one VMID make one context.
    pub(crate) const fn normalized(self) -> Self {
        let hcr_el2 = self.hcr_el2 & HCR_TRANSLATION;
        let stage1 = el10_stage1_enabled(self.sctlr_el1, hcr_el2);
        let mut context = Self {
            sctlr_el1: walk_sctlr(self.sctlr_el1, stage1),
            ttbr0_el1: walk_only(self.ttbr0_el1, stage1),
            ttbr1_el1: walk_only(self.ttbr1_el1, stage1),
            mair_el1: walk_only(self.mair_el1, stage1),
            hcr_el2,
            ..self
        };
        if !context.stage2() {
            context.hcr_el2 &= !HCR_PTW;
            context.vttbr_el2 &= VTTBR_VMID8;
            context.vtcr_el2 = 0;
        }
        context
    }

    /// `AArch64_S1Enabled` of the EL1&0 regime.
    const fn stage1(&self) -> bool {
        el10_stage1_enabled(self.sctlr_el1, self.hcr_el2)
    }

    /// Whether stage 2 translates the EL1&0 regime.
    const fn stage2(&self) -> bool {
        el10_stage2_enabled(self.hcr_el2)
    }

    /// Stage 2's context of this one, where stage 2 translates the regime.
    const fn stage2_context(&self) -> Option<Stage2Context> {
        if self.stage2() {
            Some(Stage2Context::of(self.vttbr_el2, self.vtcr_el2))
        } else {
            None
        }
    }
}

impl TaggedContext for El10Context {
    fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        for value in [
            self.sctlr_el1,
            self.tcr_el1,
            self.ttbr0_el1,
            self.ttbr1_el1,
            self.mair_el1,
            self.hcr_el2,
            self.vttbr_el2,
            self.vtcr_el2,
        ] {
            w.write_u64(value)?;
        }
        Ok(())
    }

    fn restore<R: SnapRead>(r: &mut R) -> Result<Self, SnapError> {
        Ok(Self {
            sctlr_el1: r.read_u64()?,
            tcr_el1: r.read_u64()?,
            ttbr0_el1: r.read_u64()?,
            ttbr1_el1: r.read_u64()?,
            mair_el1: r.read_u64()?,
            hcr_el2: r.read_u64()?,
            vttbr_el2: r.read_u64()?,
            vtcr_el2: r.read_u64()?,
        })
    }
}

/// What the EL2 regime's translations read of its registers: of
/// `SCTLR_EL2` its `M` and, with stage 1 enabled, `WXN`; `TCR_EL2`; and with
/// stage 1 enabled `TTBR0_EL2` and `MAIR_EL2`. Its entries carry no context:
/// a write that changes this empties the TLB, and one that leaves it as it
/// was changes nothing, as for [`El10Context`] without EL2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct El2RegimeContext {
    sctlr_el2: u64,
    tcr_el2: u64,
    ttbr0_el2: u64,
    mair_el2: u64,
}

impl El2RegimeContext {
    /// The context the EL2 registers `regs` make.
    pub(crate) const fn of(regs: &El2Regs) -> Self {
        let stage1 = regs.sctlr & sctlr::M != 0;
        Self {
            sctlr_el2: walk_sctlr(regs.sctlr, stage1),
            tcr_el2: regs.tcr,
            ttbr0_el2: walk_only(regs.ttbr0, stage1),
            mair_el2: walk_only(regs.mair, stage1),
        }
    }
}

/// What a regime's translations read of its `SCTLR_ELx`, `sctlr`: `M`, and
/// with stage 1 enabled (`stage1`; `HCR_EL2.DC` and `TGE` can disable the
/// EL1&0 regime's under `M`) `WXN`, which its walk folds into the
/// permissions it gives. Every other bit is read by the access, not cached.
const fn walk_sctlr(sctlr: u64, stage1: bool) -> u64 {
    sctlr & (sctlr::M | if stage1 { sctlr::WXN } else { 0 })
}

/// A register only stage 1's walk reads — a table base, `MAIR_ELx` — as a
/// regime's translations read it: its value with stage 1 enabled
/// (`stage1`), nothing without.
const fn walk_only(value: u64, stage1: bool) -> u64 {
    if stage1 {
        value
    } else {
        0
    }
}

/// The registers stage 2's translations of IPAs depend on: `VTTBR_EL2`, its
/// table base and VMID, and `VTCR_EL2`. Every stage 2 translation the TLB
/// keeps was made under one such pair, its context, whatever stage 1's
/// registers were, as Arm tags a stage 2 translation with its VMID alone:
/// a guest's switch between its processes keeps them. `HCR_EL2.VM` and
/// `DC`, which enable stage 2, leave its translations as they are; the
/// checks `HCR_EL2.PTW` adds are made on each use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stage2Context {
    pub(crate) vttbr_el2: u64,
    pub(crate) vtcr_el2: u64,
}

impl Stage2Context {
    /// The context `VTTBR_EL2` = `vttbr_el2` and `VTCR_EL2` = `vtcr_el2`
    /// make: the one way a stage 2 context is made from registers.
    pub(crate) const fn of(vttbr_el2: u64, vtcr_el2: u64) -> Self {
        Self {
            vttbr_el2,
            vtcr_el2,
        }
    }

    /// Whether `VTTBR_EL2` and `VTCR_EL2` can hold this context's values on
    /// a processor implementing `features`: each what an `MSR` keeps of
    /// itself ([`El2Reg::kept`], which reads its SPSR mask for `SPSR_EL2`
    /// alone).
    const fn held_by(&self, features: Features) -> bool {
        El2Reg::Vttbr.kept(features, 0, self.vttbr_el2) == self.vttbr_el2
            && El2Reg::Vtcr.kept(features, 0, self.vtcr_el2) == self.vtcr_el2
    }
}

impl TaggedContext for Stage2Context {
    fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u64(self.vttbr_el2)?;
        w.write_u64(self.vtcr_el2)
    }

    fn restore<R: SnapRead>(r: &mut R) -> Result<Self, SnapError> {
        Ok(Self {
            vttbr_el2: r.read_u64()?,
            vtcr_el2: r.read_u64()?,
        })
    }
}

/// A context a TLB's entries are tagged with ([`Contexts`]): the values of
/// the registers its translations depend on, which a snapshot saves.
pub(crate) trait TaggedContext: Copy + PartialEq {
    fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult;
    fn restore<R: SnapRead>(r: &mut R) -> Result<Self, SnapError>;
}

/// What decides which walks made a TLB's entries, as a snapshot restores
/// the processor: its features, the EL1&0 regime's registers now, and
/// `SCTLR_EL2`, which enables the EL2 regime's stage 1 (0 without EL2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TlbContext {
    pub(crate) features: Features,
    pub(crate) el10: El10Context,
    pub(crate) sctlr_el2: u64,
}

impl TlbContext {
    /// `AArch64_S1Enabled` of the EL2 regime.
    const fn el2_stage1(&self) -> bool {
        self.features.has(Feature::El2) && self.sctlr_el2 & sctlr::M != 0
    }
}

const TLB_ENTRIES: usize = 1024;
/// The key of an empty entry, which no translation's key equals.
const INVALID_KEY: u64 = u64::MAX;
/// A virtual page number's bits: `va >> 12` of a 64-bit address.
const PAGE_BITS: u32 = 52;
/// The bit of a TLB key that names the EL2 regime, above every page number.
const KEY_EL2: u64 = 1 << 63;
/// How many contexts of each kind a processor with EL2 keeps entries of at
/// once: EL1&0 contexts ([`El10Context`]) and stage 2 contexts
/// ([`Stage2Context`]).
pub(crate) const CONTEXTS: usize = 15;
/// Where a key holds its entry's context tag: bits 62:52, between the page
/// number and the regime. The tag is 0 for every entry without EL2 and for
/// every entry of the EL2 regime, whose registers no hypervisor switches;
/// with EL2 an EL1&0 entry's names its context's place in the TLB's table
/// of EL1&0 contexts, and a kept stage 2 translation's its context's in
/// the table of stage 2 contexts: the place plus one in bits 55:52, and the
/// generation of the place when the entry was made in bits 62:56, so that
/// the entries a context leaves in a place taken from it never match
/// again.
const TAG_SHIFT: u32 = PAGE_BITS;
const TAG_MASK: u64 = 0x7FF << TAG_SHIFT;
/// A place's tag bits, below its generation's.
const PLACE_BITS: u32 = 4;
/// How many generations a place counts before it starts again from 0,
/// when the entries of the generation it starts again at are emptied.
const GENERATIONS: u8 = 1 << 7;
const _: () = assert!(CONTEXTS < 1 << PLACE_BITS);
const _: () = assert!(TAG_SHIFT + PLACE_BITS + 7 == 63);

/// The key a translation of `va` in `regime` is cached under, without its
/// context's tag ([`Cpu::tlb_key`] adds it): its virtual page number, with
/// what it belongs to beyond it (Arm `TLBContext`,
/// `AArch64_GetS1TLBContext`), the regime, in bit 63. An access hits only
/// an entry of its own regime, so a page EL1 and EL2 both map at one
/// address is two entries, never one taken for the other.
#[inline(always)]
const fn page_key(va: u64, regime: Regime) -> u64 {
    (va >> 12) | ((regime as u64) << 63)
}

const _: () = assert!((Regime::El10 as u64) << 63 == 0 && (Regime::El2 as u64) << 63 == KEY_EL2);

/// The tag in a key of the context at `index` in one of the TLB's tables,
/// in the place's generation `generation`.
const fn context_tag(index: usize, generation: u8) -> u64 {
    (((generation as u64) << PLACE_BITS) | ((index as u64) + 1)) << TAG_SHIFT
}

/// A key's tag, if it carries one: the place in its table of the context
/// it names, and the place's generation then.
const fn tag_place(key: u64) -> Option<TagPlace> {
    let tag = (key & TAG_MASK) >> TAG_SHIFT;
    match tag & ((1 << PLACE_BITS) - 1) {
        0 => None,
        place => Some(TagPlace {
            index: place as usize - 1,
            generation: (tag >> PLACE_BITS) as u8,
        }),
    }
}

/// Whether `key` is an entry's made in the place `index` of its table, of
/// any generation.
fn made_in_place(key: u64, index: usize) -> bool {
    key != INVALID_KEY && tag_place(key).is_some_and(|place| place.index == index)
}

/// Where a tag puts its entry ([`tag_place`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TagPlace {
    index: usize,
    generation: u8,
}

/// The regime an entry's key names.
const fn key_regime(key: u64) -> Regime {
    if key & KEY_EL2 != 0 {
        Regime::El2
    } else {
        Regime::El10
    }
}

/// The regime whose discriminant a snapshot's byte is, if it is one.
const fn byte_regime(byte: u8) -> Option<Regime> {
    match byte {
        b if b == Regime::El10 as u8 => Some(Regime::El10),
        b if b == Regime::El2 as u8 => Some(Regime::El2),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct TlbEntry {
    /// [`Cpu::tlb_key`] of the translation, or `INVALID_KEY`.
    key: u64,
    /// The output page of both stages.
    pa_page: u64,
    /// Whether either stage maps Device memory for a data access: the
    /// memory an access through the entry is to.
    device: bool,
    /// What stage 1 gave.
    s1: Stage1Leaf,
    /// What stage 2 gave: [`Stage2Leaf::NONE`] where it takes no part.
    s2: Stage2Leaf,
}

impl TlbEntry {
    /// Whether this entry, the one in `key`'s slot ([`Tlb::entry_for`]),
    /// is the translation `key` names.
    #[inline(always)]
    const fn translates(&self, key: u64) -> bool {
        self.key == key
    }

    /// The translation of `va`, which this entry translates. An entry made
    /// with stage 1 disabled has every permission but `GUARDED`.
    #[inline(always)]
    const fn mapping(&self, va: u64) -> Mapping {
        Mapping {
            pa: in_page(self.pa_page, va),
            guarded: self.s1.perms & GUARDED != 0,
            device: self.device,
        }
    }

    /// What stage 1 gave, on a processor of the levels `L`: without EL2
    /// every entry is stage 1's walk's, only stage 2 keeping a translation
    /// stage 1 left alone.
    #[inline(always)]
    const fn stage1_leaf<L: Levels>(&self) -> Stage1Leaf {
        Stage1Leaf {
            stage1: if L::EL2 {
                self.s1.stage1
            } else {
                Stage1::Walked
            },
            ..self.s1
        }
    }
}

const EMPTY_ENTRY: TlbEntry = TlbEntry {
    key: INVALID_KEY,
    pa_page: 0,
    device: false,
    s1: Stage1Leaf {
        perms: 0,
        device: false,
        level: 0,
        stage1: Stage1::Walked,
    },
    s2: Stage2Leaf {
        perms: 0,
        device: false,
        level: 0,
    },
};

const _: () = assert!(core::mem::size_of::<TlbEntry>() == 24);

/// The translation cache: direct-mapped, 4 KiB pages, translations of
/// virtual addresses by every stage, and beside them stage 2 translations
/// of IPAs, which the stage 1 walks under stage 2 read their tables
/// through.
///
/// With EL2 the EL1&0 regime's entries are tagged with the context they
/// were made under — the values of the registers that regime's
/// translations depend on — and the kept stage 2 translations with stage
/// 2's, up to fifteen contexts of each at once, in tables the TLB keeps
/// beside them (`TlbContexts`), so that a hypervisor's switch between a
/// guest's registers and its own returns to the translations each had, as
/// Arm's VMID and ASID tags let a TLB keep them. A write of one of the
/// registers leaves the contexts to be found again; the first translation
/// that misses finds its own, or gives it the place of the context found
/// least recently, whose entries never match again. Without EL2 there are
/// no tables (`NoContexts`) and no tags.
pub struct Tlb<L: Levels> {
    entries: [TlbEntry; TLB_ENTRIES],
    ipa: [IpaEntry; IPA_ENTRIES],
    contexts: L::Contexts,
}

/// Where a TLB keeps its tables of contexts: [`TlbContexts`] on a processor
/// with EL2, nothing ([`NoContexts`]) on one without. Public as
/// [`Levels`] is.
pub trait ContextStorage:
    Clone + core::fmt::Debug + PartialEq + Eq + Send + Sync + 'static
{
    /// No context, as out of reset.
    const NONE: Self;
    /// The storage of `contexts`: kept with EL2; without it, where a TLB
    /// holds none, nothing.
    fn keep(contexts: TlbContexts) -> Self;
    /// The tables, if the processor has EL2: a constant `None` without.
    fn get(&self) -> Option<&TlbContexts>;
    fn get_mut(&mut self) -> Option<&mut TlbContexts>;
}

/// The tables of contexts of a processor without EL2: none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoContexts;

impl ContextStorage for NoContexts {
    const NONE: Self = Self;

    #[inline(always)]
    fn keep(_contexts: TlbContexts) -> Self {
        Self
    }

    #[inline(always)]
    fn get(&self) -> Option<&TlbContexts> {
        None
    }

    #[inline(always)]
    fn get_mut(&mut self) -> Option<&mut TlbContexts> {
        None
    }
}

/// The contexts whose tags a processor with EL2 gives what its TLB holds
/// ([`Tlb`]): the EL1&0 regime's, which its entries of that regime carry,
/// and stage 2's, which its kept stage 2 translations carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlbContexts {
    el10: Contexts<El10Context>,
    stage2: Contexts<Stage2Context>,
}

impl TlbContexts {
    /// Leave both current contexts to be found again.
    fn unfound(&mut self) {
        self.el10.current = 0;
        self.stage2.current = 0;
    }
}

impl ContextStorage for TlbContexts {
    /// No context of either kind.
    const NONE: Self = Self {
        el10: Contexts::NONE,
        stage2: Contexts::NONE,
    };

    #[inline(always)]
    fn keep(contexts: TlbContexts) -> Self {
        contexts
    }

    #[inline(always)]
    fn get(&self) -> Option<&TlbContexts> {
        Some(self)
    }

    #[inline(always)]
    fn get_mut(&mut self) -> Option<&mut TlbContexts> {
        Some(self)
    }
}

/// A table of the contexts of one kind the TLB tags what it holds with
/// ([`TlbContexts`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Contexts<C> {
    table: [Option<C>; CONTEXTS],
    /// Each place's generation: how many times, below [`GENERATIONS`], a
    /// context has taken it from another, whose entries carry an older one.
    generations: [u8; CONTEXTS],
    /// The places of `table` from the one found most recently to the one
    /// found least recently, whose place a new context takes: each place
    /// once.
    recency: [u8; CONTEXTS],
    /// The tag of the current context; 0 while it is to be found, which no
    /// key carries with EL2, so that every lookup misses until it is.
    current: u64,
}

impl<C: TaggedContext> Contexts<C> {
    /// No context, as out of reset, the first place the first a new
    /// context takes; the current one to be found.
    const NONE: Self = Self {
        table: [None; CONTEXTS],
        generations: [0; CONTEXTS],
        recency: {
            let mut recency = [0; CONTEXTS];
            let mut i = 0;
            while i < CONTEXTS {
                recency[i] = (CONTEXTS - 1 - i) as u8;
                i += 1;
            }
            recency
        },
        current: 0,
    };

    /// The tag of the context at `index`, in its place's generation now.
    const fn tag(&self, index: usize) -> u64 {
        context_tag(index, self.generations[index])
    }

    /// Whether `key`'s tag names a context the table holds, in its place's
    /// generation now: an entry that can match.
    const fn current_entry(&self, key: u64) -> bool {
        match tag_place(key) {
            Some(place) => {
                self.table[place.index].is_some()
                    && place.generation == self.generations[place.index]
            }
            None => false,
        }
    }

    /// Make `index` the place found most recently, the others keeping
    /// their order behind it.
    fn found(&mut self, index: usize) {
        let mut recency = [index as u8; CONTEXTS];
        let others = self
            .recency
            .iter()
            .filter(|&&place| usize::from(place) != index);
        for (slot, &place) in recency[1..].iter_mut().zip(others) {
            *slot = place;
        }
        self.recency = recency;
    }

    /// The place a new context takes: the one found least recently.
    const fn least_recent(&self) -> usize {
        self.recency[CONTEXTS - 1] as usize
    }

    /// Whether the current context, where one is found, is `context`.
    fn current_is(&self, context: C) -> bool {
        match tag_place(self.current) {
            Some(place) => self.table[place.index] == Some(context),
            None => true,
        }
    }

    /// Find `context`, the current one: its tag, the table's place of it or
    /// the place of the one found least recently, in the place's next
    /// generation, so that what was made in the context it held never
    /// matches again. When the generation starts again from 0, `restart`
    /// empties what was made in the place, of every generation, so that
    /// nothing made [`GENERATIONS`] generations ago matches either.
    fn find<F: FnOnce(usize)>(&mut self, context: C, restart: F) -> u64 {
        let index = match self.table.iter().position(|c| *c == Some(context)) {
            Some(index) => index,
            None => {
                let index = self.least_recent();
                if self.table[index].is_some() {
                    let next = (self.generations[index] + 1) % GENERATIONS;
                    self.generations[index] = next;
                    if next == 0 {
                        restart(index);
                    }
                }
                self.table[index] = Some(context);
                index
            }
        };
        self.found(index);
        self.current = self.tag(index);
        self.current
    }

    /// The table, for a snapshot: each place, a byte saying whether it
    /// holds a context and then its registers; each place's generation;
    /// the places from the one found most recently to the least; and the
    /// current context's place plus one (0 for none).
    fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        for context in &self.table {
            w.write_bool(context.is_some())?;
            if let Some(context) = context {
                context.save(w)?;
            }
        }
        w.write_bytes(&self.generations)?;
        w.write_bytes(&self.recency)?;
        let current = tag_place(self.current).map_or(0, |place| place.index + 1);
        w.write_u8(current as u8)
    }

    /// The inverse of [`Contexts::save`], for a processor whose registers
    /// make `now`, the current context of the kind (`None` where they make
    /// none, as stage 2's where it translates nothing). Without EL2 (`el2`)
    /// the table is empty. With EL2 it holds distinct contexts, each one
    /// `made` accepts; every generation is below [`GENERATIONS`]; the order
    /// names each place once; and the current tag is 0 or names `now`.
    fn restore<R: SnapRead, F: Fn(&C) -> bool>(
        r: &mut R,
        el2: bool,
        now: Option<C>,
        made: F,
    ) -> Result<Self, SnapError> {
        const UNMADE: SnapError =
            SnapError::Invalid("TLB holds a context no processor of its features makes");
        let mut contexts = Self::NONE;
        for index in 0..CONTEXTS {
            if !r.read_bool()? {
                continue;
            }
            let held = C::restore(r)?;
            if !el2 || !made(&held) || contexts.table.contains(&Some(held)) {
                return Err(UNMADE);
            }
            contexts.table[index] = Some(held);
        }
        r.read_bytes(&mut contexts.generations)?;
        if contexts
            .generations
            .iter()
            .any(|&generation| generation >= GENERATIONS)
            || (!el2 && contexts.generations != Self::NONE.generations)
        {
            return Err(UNMADE);
        }
        r.read_bytes(&mut contexts.recency)?;
        let current = usize::from(r.read_u8()?);
        let mut seen = [false; CONTEXTS];
        for &place in &contexts.recency {
            match seen.get_mut(usize::from(place)) {
                Some(seen) if !*seen => *seen = true,
                Some(_) | None => {
                    return Err(SnapError::Invalid(
                        "TLB's order of contexts names a place other than once",
                    ))
                }
            }
        }
        if current > CONTEXTS || (!el2 && contexts.recency != Self::NONE.recency) {
            return Err(SnapError::Invalid("TLB names a context past its table"));
        }
        if current != 0 {
            if now.is_none() || contexts.table[current - 1] != now {
                return Err(SnapError::Invalid(
                    "TLB's current context is not the registers'",
                ));
            }
            contexts.current = contexts.tag(current - 1);
        }
        Ok(contexts)
    }
}

/// Empty the entries in `entries` made in the place `index` of the table of
/// EL1&0 contexts, whatever their generation.
fn empty_entries(entries: &mut [TlbEntry; TLB_ENTRIES], index: usize) {
    for entry in entries {
        if made_in_place(entry.key, index) {
            *entry = EMPTY_ENTRY;
        }
    }
}

/// Empty the stage 2 translations in `kept` made in the place `index` of the
/// table of stage 2 contexts, whatever their generation.
fn empty_kept(kept: &mut [IpaEntry; IPA_ENTRIES], index: usize) {
    for entry in kept {
        if made_in_place(entry.key, index) {
            *entry = EMPTY_IPA_ENTRY;
        }
    }
}

impl<L: Levels> Tlb<L> {
    /// The TLB out of reset: no entry, no kept translation, no context.
    pub(crate) const fn new() -> Self {
        Self {
            entries: [EMPTY_ENTRY; TLB_ENTRIES],
            ipa: [EMPTY_IPA_ENTRY; IPA_ENTRIES],
            contexts: <L::Contexts as ContextStorage>::NONE,
        }
    }

    /// Empty every context's entries and kept stage 2 translations, the
    /// current contexts left to be found again by the next translation,
    /// from the registers as they are then. Only stage 2 keeps translations
    /// of IPAs, so without EL2 there are none to empty.
    pub(crate) fn flush(&mut self) {
        self.entries = [EMPTY_ENTRY; TLB_ENTRIES];
        debug_assert!(
            L::EL2 || self.ipa.iter().all(|kept| kept.key == INVALID_KEY),
            "a kept stage 2 translation without EL2"
        );
        if L::EL2 {
            self.ipa = [EMPTY_IPA_ENTRY; IPA_ENTRIES];
        }
        if let Some(contexts) = self.contexts.get_mut() {
            contexts.unfound();
        }
    }

    /// The TLB as out of reset ([`Tlb::new`]).
    pub(crate) fn reset(&mut self) {
        self.flush();
        self.contexts = <L::Contexts as ContextStorage>::NONE;
    }

    /// The tag of the EL1&0 regime's current context: 0 without EL2, and
    /// with it while a register write has left the context to be found.
    #[inline(always)]
    fn el10_current(&self) -> u64 {
        self.contexts
            .get()
            .map_or(0, |contexts| contexts.el10.current)
    }

    /// Leave the current contexts to be found again.
    fn unfound(&mut self) {
        if let Some(contexts) = self.contexts.get_mut() {
            contexts.unfound();
        }
    }

    /// The slot of `key`'s translation: the low bits of its page number,
    /// those above the first six turned by its context's tag, so that a
    /// page two contexts both map, a hypervisor's and a guest's kernel at
    /// one address, takes two slots. Without a tag, the page number's.
    #[inline(always)]
    const fn slot(key: u64) -> usize {
        ((key ^ ((key & TAG_MASK) >> (TAG_SHIFT - 6))) as usize) & (TLB_ENTRIES - 1)
    }

    /// The slot of the kept stage 2 translation `key` names: its IPA page
    /// number folded eight bits at a time, so that the first pages of the
    /// blocks a stage 2 maps, whose low bits are all zero, spread over the
    /// slots, and turned by its context's tag, as [`Tlb::slot`] turns a
    /// translation's.
    #[inline]
    const fn ipa_slot(key: u64) -> usize {
        let page = key & !TAG_MASK;
        let folded = page ^ (page >> 8) ^ (page >> 16) ^ (page >> 24) ^ (page >> 32);
        ((folded ^ ((key & TAG_MASK) >> (TAG_SHIFT - 2))) as usize) & (IPA_ENTRIES - 1)
    }

    /// The stage 2 translation `key` names ([`Stage2Made::key`]), if the
    /// TLB keeps it.
    fn ipa_hit(&self, key: u64) -> Option<IpaEntry> {
        let entry = self.ipa[Self::ipa_slot(key)];
        (entry.key == key).then_some(entry)
    }

    /// Keep the stage 2 translations a translation made.
    fn keep(&mut self, made: &Stage2Made) {
        for entry in made.made.iter().flatten() {
            self.ipa[Self::ipa_slot(entry.key)] = *entry;
        }
    }

    /// The entry in `key`'s slot, the one that translates `key`'s page if
    /// any does ([`TlbEntry::translates`]): the lookup every translation
    /// from the TLB makes.
    #[inline(always)]
    fn entry_for(&self, key: u64) -> TlbEntry {
        self.entries[Self::slot(key)]
    }

    /// The entry `key` names, if the TLB holds it.
    #[inline(always)]
    fn hit(&self, key: u64) -> Option<TlbEntry> {
        let entry = self.entry_for(key);
        entry.translates(key).then_some(entry)
    }

    /// The tag of stage 2's current context: 0 without EL2, and with it
    /// while a register write has left the context to be found.
    fn stage2_current(&self) -> u64 {
        self.contexts
            .get()
            .map_or(0, |contexts| contexts.stage2.current)
    }

    /// Whether the current EL1&0 context, where one is found, is `context`
    /// and stage 2's is `stage2` (where stage 2 translates the regime):
    /// what the registers make, as every write that changes them leaves
    /// the contexts to be found again.
    fn current_contexts_are(&self, context: El10Context, stage2: Option<Stage2Context>) -> bool {
        self.contexts.get().is_none_or(|contexts| {
            contexts.el10.current_is(context)
                && match stage2 {
                    Some(stage2) => contexts.stage2.current_is(stage2),
                    None => true,
                }
        })
    }

    /// Find `context` in the table of EL1&0 contexts ([`Contexts::find`]):
    /// its tag, now the current one. 0 without EL2.
    fn find_el10(&mut self, context: El10Context) -> u64 {
        let Self {
            entries, contexts, ..
        } = self;
        match contexts.get_mut() {
            Some(contexts) => contexts
                .el10
                .find(context, |index| empty_entries(entries, index)),
            None => 0,
        }
    }

    /// Find `context` in the table of stage 2 contexts, as
    /// [`Tlb::find_el10`] finds an EL1&0 one.
    fn find_stage2(&mut self, context: Stage2Context) -> u64 {
        let Self { ipa, contexts, .. } = self;
        match contexts.get_mut() {
            Some(contexts) => contexts
                .stage2
                .find(context, |index| empty_kept(ipa, index)),
            None => 0,
        }
    }

    /// The TLB, with its tables of contexts (empty without EL2), for a
    /// snapshot: the table of EL1&0 contexts and then the table of stage 2
    /// contexts ([`Contexts::save`]); then the entries that can match, each
    /// one's virtual page number with its context's tag and its output
    /// page, then its stage 1 flags, whether either stage maps Device
    /// memory, stage 1's level, its regime's byte, whether stage 1 maps
    /// Device memory, stage 2's permissions and level, and whether stage 1
    /// was disabled; then the kept stage 2 translations that can match,
    /// each one's IPA page number with its stage 2 context's tag and output
    /// page, then its permissions, Device-ness and level. An entry of an
    /// older generation than its place's, which nothing matches, is left
    /// out, as an empty entry would be.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        let contexts = self
            .contexts
            .get()
            .unwrap_or(&<TlbContexts as ContextStorage>::NONE);
        contexts.el10.save(w)?;
        contexts.stage2.save(w)?;
        let can_match = |key: u64| {
            key != INVALID_KEY && (tag_place(key).is_none() || contexts.el10.current_entry(key))
        };
        let valid = self.entries.iter().filter(|e| can_match(e.key)).count();
        w.write_u32(valid as u32)?;
        for e in self.entries.iter().filter(|e| can_match(e.key)) {
            w.write_u64(e.key & !KEY_EL2)?;
            w.write_u64(e.pa_page)?;
            w.write_bytes(&[
                e.s1.perms,
                u8::from(e.s1.device),
                e.s1.level,
                key_regime(e.key) as u8,
                e.s1.stage1 as u8,
                e.s2.perms,
                u8::from(e.s2.device),
                e.s2.level,
            ])?;
        }
        let can_match = |key: u64| key != INVALID_KEY && contexts.stage2.current_entry(key);
        let kept = self.ipa.iter().filter(|e| can_match(e.key)).count();
        w.write_u32(kept as u32)?;
        for e in self.ipa.iter().filter(|e| can_match(e.key)) {
            w.write_u64(e.key)?;
            w.write_u64(e.pa_page.get())?;
            w.write_bytes(&[e.leaf.perms, u8::from(e.leaf.device), e.leaf.level])?;
        }
        Ok(())
    }

    /// The inverse of [`Tlb::save`], for a processor restored to `context`.
    /// Without EL2 there are no contexts and no tags. With EL2 each table
    /// holds distinct contexts, and its current tag is 0 or names the one
    /// the registers make now ([`Contexts::restore`]); every EL1&0 entry
    /// names a context in the table of EL1&0 contexts, and every kept stage
    /// 2 translation one in the table of stage 2 contexts. An entry must be
    /// one the walks can produce in its context and the TLB can hold
    /// ([`Cpu::cache`]) — of a regime whose stage 1 makes it (or, disabled
    /// under stage 2, leaves the address to stage 2), with a stage 2 leaf
    /// exactly where stage 2 translates the regime. A snapshot refused
    /// leaves the TLB as out of reset.
    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R, context: &TlbContext) -> SnapResult {
        let restored = self.restore_parts(r, context);
        if restored.is_err() {
            self.reset();
        }
        restored
    }

    /// [`Tlb::restore`], leaving what it read so far where it refuses the
    /// snapshot.
    fn restore_parts<R: SnapRead>(&mut self, r: &mut R, context: &TlbContext) -> SnapResult {
        const NOT_WALKED: SnapError = SnapError::Invalid("TLB entry is not one a walk can produce");
        const NO_CONTEXT: SnapError =
            SnapError::Invalid("TLB entry's context is not one the TLB holds");
        let features = context.features;
        let el2 = L::EL2;
        debug_assert_eq!(el2, features.has(Feature::El2), "the level set's features");
        let guarded = if features.has(Feature::Bti) {
            GUARDED
        } else {
            0
        };
        self.reset();
        // A held context is one registers this processor holds make: an
        // EL1&0 one normalized, its stage 2 registers' values ones they can
        // hold where it has stage 2 (without, it keeps only the VMID, which
        // `VTTBR_EL2` holds whole).
        let contexts = TlbContexts {
            el10: Contexts::restore(r, el2, Some(context.el10), |held: &El10Context| {
                *held == held.normalized()
                    && held
                        .stage2_context()
                        .is_none_or(|stage2| stage2.held_by(features))
            })?,
            stage2: Contexts::restore(
                r,
                el2,
                context.el10.stage2_context(),
                |held: &Stage2Context| held.held_by(features),
            )?,
        };
        // The context an EL1&0 entry was made in: with EL2 the one its tag
        // names, in its place's generation now (a snapshot holds no entry
        // of an older one), without it the registers'.
        let context_of = |key: u64| -> Result<El10Context, SnapError> {
            match (el2, tag_place(key)) {
                (true, Some(place)) if contexts.el10.current_entry(key) => {
                    contexts.el10.table[place.index].ok_or(NO_CONTEXT)
                }
                (false, None) => Ok(context.el10),
                (true, Some(_) | None) | (false, Some(_)) => Err(NO_CONTEXT),
            }
        };
        let valid = r.read_count(TLB_ENTRIES)?;
        for _ in 0..valid {
            let page = r.read_u64()?;
            let pa_page = r.read_u64()?;
            let mut rest = [0u8; 8];
            r.read_bytes(&mut rest)?;
            let [perms, s1_device, level, regime, stage1, s2_perms, s2_device, s2_level] = rest;
            if page & !(TAG_MASK | ((1 << PAGE_BITS) - 1)) != 0
                || pa_page & 0xFFF != 0
                || pa_page >> id::PA_BITS != 0
                || s1_device > 1
                || s2_device > 1
            {
                return Err(NOT_WALKED);
            }
            let s1_device = s1_device == 1;
            let s2 = Stage2Leaf {
                perms: s2_perms,
                device: s2_device == 1,
                level: s2_level,
            };
            let regime = match byte_regime(regime) {
                Some(Regime::El10) => Regime::El10,
                Some(Regime::El2) if !el2 => {
                    return Err(SnapError::Invalid(
                        "TLB entry is of a regime this processor lacks",
                    ));
                }
                Some(Regime::El2) if page & TAG_MASK != 0 => return Err(NO_CONTEXT),
                Some(Regime::El2) => Regime::El2,
                None => return Err(SnapError::Invalid("TLB entry names no translation regime")),
            };
            let stage1 = match stage1 {
                0 => Stage1::Walked,
                1 => Stage1::Disabled,
                _ => return Err(NOT_WALKED),
            };
            let made_in = match regime {
                Regime::El10 => Some(context_of(page)?),
                Regime::El2 => None,
            };
            // Stage 1's walk gives a leaf at level 1 to 3 the permissions of
            // its regime (the EL2 regime's none of EL0's); stage 1 disabled
            // gives every permission, and a data access Device memory unless
            // `HCR_EL2.DC` makes it Normal.
            let (stage1_valid, stage2) = match (made_in, stage1) {
                (Some(made_in), Stage1::Walked) => (
                    made_in.stage1() && (1..=3).contains(&level) && perms & !(PERMS | guarded) == 0,
                    made_in.stage2(),
                ),
                (Some(made_in), Stage1::Disabled) => (
                    !made_in.stage1()
                        && made_in.stage2()
                        && level == 0
                        && perms == PERMS
                        && s1_device == !el10_s1_disabled_data_normal(made_in.hcr_el2),
                    true,
                ),
                (None, Stage1::Walked) => (
                    context.el2_stage1()
                        && (1..=3).contains(&level)
                        && perms & !(PERMS | guarded) == 0
                        && perms & PERMS_EL0 == 0,
                    false,
                ),
                (None, Stage1::Disabled) => (false, false),
            };
            // An entry stage 2 takes no part in holds `Stage2Leaf::NONE`; one
            // it does, a stage 2 leaf.
            let stage2_valid = if stage2 {
                Stage2Leaf::walked(s2.perms, s2.level)
            } else {
                s2 == Stage2Leaf::NONE
            };
            if !stage1_valid || !stage2_valid {
                return Err(NOT_WALKED);
            }
            let va = (page & !TAG_MASK) << 12;
            let e0pd_refused =
                made_in.is_some_and(|made_in| e0pd_set(features, made_in.tcr_el1, va));
            if stage1 == Stage1::Walked && perms & PERMS_EL0 != 0 && e0pd_refused {
                return Err(SnapError::Invalid(
                    "TLB entry gives EL0 a page FEAT_E0PD refuses it",
                ));
            }
            let key = page | ((regime as u64) << 63);
            let slot = &mut self.entries[Self::slot(key)];
            if slot.key != INVALID_KEY {
                return Err(SnapError::Invalid("two TLB entries claim one slot"));
            }
            *slot = TlbEntry {
                key,
                pa_page,
                device: s1_device || s2.device,
                s1: Stage1Leaf {
                    perms,
                    device: s1_device,
                    level,
                    stage1,
                },
                s2,
            };
        }
        // Stage 2 translations of IPAs, which only stage 2 keeps: an IPA as
        // wide as a physical address at most (`AArch64_S2MinTxSZ`), in a
        // stage 2 context the table holds.
        let kept = r.read_count(IPA_ENTRIES)?;
        for _ in 0..kept {
            let key = r.read_u64()?;
            let pa_page = r.read_u64()?;
            let mut rest = [0u8; 3];
            r.read_bytes(&mut rest)?;
            let [perms, device, level] = rest;
            if !contexts.stage2.current_entry(key) {
                return Err(NO_CONTEXT);
            }
            if (key & !TAG_MASK) >> (id::PA_BITS - 12) != 0
                || pa_page & 0xFFF != 0
                || pa_page >> id::PA_BITS != 0
                || device > 1
                || !Stage2Leaf::walked(perms, level)
            {
                return Err(NOT_WALKED);
            }
            let slot = &mut self.ipa[Self::ipa_slot(key)];
            if slot.key != INVALID_KEY {
                return Err(SnapError::Invalid("two TLB entries claim one slot"));
            }
            *slot = IpaEntry {
                key,
                pa_page: PhysAddr::new(pa_page),
                leaf: Stage2Leaf {
                    perms,
                    device: device == 1,
                    level,
                },
            };
        }
        self.contexts = L::Contexts::keep(contexts);
        Ok(())
    }
}

/// Arm `AlignmentEnforced` in a regime whose `SCTLR_ELx` holds `control`:
/// its `A` bit, under which every misaligned data access faults.
#[inline]
const fn alignment_enforced(control: u64) -> bool {
    control & sctlr::A != 0
}

impl<L: Levels> Cpu<L> {
    /// Empty the TLB. The running trace ends, whatever the caller: its
    /// fetches translate again. (The guest's TLBI and translation-register
    /// MSRs end their trace by `Op::ends_trace` as well;
    /// this does not rely on it.)
    /// The current contexts then are found again by the next translation,
    /// from the registers as they are then: a register written without a
    /// write's effect, as a host may, is seen ([`Tlb::flush`]).
    pub(crate) fn flush_tlb(&mut self) {
        self.tlb.flush();
        self.trace_break = true;
    }

    /// The tag of the EL1&0 regime's current context: 0 without EL2, and
    /// with it while a register write has left the context to be found.
    #[inline(always)]
    fn current_tag(&self) -> u64 {
        self.tlb.el10_current()
    }

    /// The key `va`'s translation in `regime` has in the TLB now: its page
    /// and regime ([`page_key`]), and with EL2, for the EL1&0 regime, the
    /// current context's tag — 0 while a register write has left it to be
    /// found, which no entry carries, so that the lookup misses and the
    /// translation that follows finds it ([`Cpu::found_el10_tag`]).
    #[inline(always)]
    fn tlb_key(&self, va: u64, regime: Regime) -> u64 {
        let tag = if L::EL2 && regime == Regime::El10 {
            self.current_tag()
        } else {
            0
        };
        page_key(va, regime) | tag
    }

    /// [`Cpu::tlb_key`], with the EL1&0 regime's context found first: the
    /// key a lookup that may be followed by a walk and a fill uses.
    fn found_tlb_key(&mut self, va: u64, regime: Regime) -> u64 {
        let tag = if L::EL2 && regime == Regime::El10 {
            self.found_el10_tag()
        } else {
            0
        };
        page_key(va, regime) | tag
    }

    /// The EL1&0 regime's context now ([`El10Context`]).
    pub(crate) fn el10_context(&self) -> El10Context {
        El10Context::of(&self.sys, self.el2_state.get().map(|state| &state.regs))
    }

    /// The tag of the EL1&0 regime's current context, with EL2: found in the
    /// TLB's table of EL1&0 contexts, or given the place of the context
    /// found least recently ([`Tlb::find_el10`]), if a register write left
    /// it to be found. The running trace ends when it is found, as a fill
    /// ends it.
    fn found_el10_tag(&mut self) -> u64 {
        let current = self.current_tag();
        if current != 0 {
            debug_assert!(self.current_contexts_hold(), "a stale current context");
            return current;
        }
        let tag = self.tlb.find_el10(self.el10_context());
        self.trace_break = true;
        tag
    }

    /// The tag of stage 2's current context ([`Stage2Context`]), which the
    /// stage 2 translations the EL1&0 regime's translations make under
    /// stage 2 are kept under: found in the TLB's table of stage 2
    /// contexts, or given the place of the one found least recently, if a
    /// register write left it to be found ([`Tlb::find_stage2`]). What it
    /// finds is no fetch's, so the running trace goes on.
    fn found_stage2_tag(&mut self) -> u64 {
        let current = self.tlb.stage2_current();
        if current != 0 {
            debug_assert!(self.current_contexts_hold(), "a stale current context");
            return current;
        }
        let el2 = self.el2();
        self.tlb.find_stage2(Stage2Context::of(el2.vttbr, el2.vtcr))
    }

    /// Whether the TLB's current contexts are the ones the registers make
    /// now: the invariant every write of a translation register keeps by
    /// leaving them to be found again ([`Cpu::el10_context_changed`],
    /// [`Cpu::flush_tlb`]), checked where a current tag is used.
    fn current_contexts_hold(&self) -> bool {
        let context = self.el10_context();
        self.tlb
            .current_contexts_are(context, context.stage2_context())
    }

    /// A write changed the EL1&0 regime's context ([`El10Context`]). With
    /// EL2 the regime's context, and stage 2's, are found again by the
    /// next translation that misses, and the entries of the ones they leave
    /// stay, for when the registers return to them — as a hypervisor
    /// returns them at each entry to a guest; without EL2 the TLB is
    /// emptied. Either way the running trace ends: its fetches translate
    /// again.
    pub(crate) fn el10_context_changed(&mut self) {
        if L::EL2 {
            self.tlb.unfound();
            self.trace_break = true;
        } else {
            self.flush_tlb();
        }
    }

    /// Arm `AlignmentEnforced` in the current regime.
    #[inline]
    fn strict_alignment(&self) -> bool {
        alignment_enforced(self.regime_sctlr(self.regime()))
    }

    /// Arm `AArch64_UnalignedAccessFaults` for a misaligned access of
    /// `bytes` at `va`: whether it faults before translation, by `SCTLR_ELx`
    /// of the current regime.
    fn misaligned_faults(&self, va: u64, bytes: u64, acc: DataAccess) -> bool {
        let control = self.regime_sctlr(self.regime());
        if alignment_enforced(control) {
            return true;
        }
        let lse2 = self.implements(Feature::Lse2);
        let in_granule = (va & 15) + bytes <= 16;
        match acc.alignment {
            AlignmentRule::Any => false,
            AlignmentRule::Atomic => !(lse2 && in_granule),
            AlignmentRule::Ordered => !lse2 || (control & sctlr::NAA == 0 && !in_granule),
        }
    }

    /// Translate `va` for an access of `kind` checked against `el`'s
    /// permissions (Arm `AArch64_FullTranslate`): at each stage the walk's
    /// faults, then Device memory's alignment check, then the permissions,
    /// stage 1's before stage 2's.
    #[inline(always)]
    pub(crate) fn translate<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        kind: AccessKind,
        el: El,
        on_device: OnDevice,
    ) -> Result<Translated, Abort> {
        self.translate_needs(bus, va, Needs::Access(kind), el, on_device)
            .map(|m| Translated { pa: m.pa })
    }

    /// Whether translations of `regime` are looked for in the TLB: with its
    /// stage 1 enabled, and with it disabled under stage 2. With stage 1
    /// disabled and no stage 2 an address translates to itself, which no
    /// entry holds.
    #[inline(always)]
    fn tlb_used(&self, regime: Regime) -> bool {
        self.s1_enabled(regime) || self.stage2_enabled(regime)
    }

    /// The translation of `va` by the TLB entry that translates it, if the
    /// TLB holds one that permits an access of `kind` checked against
    /// `el`'s permissions at each stage ([`Cpu::entry_refusal`]): the one
    /// case that raises no fault of a walk or of the page's permissions.
    /// [`Cpu::translate_data`] and [`Cpu::fetch_translate`] try it inline,
    /// and make their checks in their order out of line only when it is
    /// `None`; [`Cpu::fetch_probe`] answers from it alone where the TLB is
    /// used.
    #[inline(always)]
    fn permitted_hit(&self, va: u64, kind: AccessKind, el: El) -> Option<Mapping> {
        let regime = self.regime();
        if !self.tlb_used(regime) {
            return None;
        }
        let key = self.tlb_key(va, regime);
        let entry = self.tlb.entry_for(key);
        if !entry.translates(key)
            || self
                .entry_refusal(&entry, va, Needs::Access(kind), el, OnDevice::Permitted)
                .is_some()
        {
            return None;
        }
        Some(entry.mapping(va))
    }

    /// The fault the TLB entry `entry`, which translates `va`, raises for
    /// an access needing `needs` checked against `el`'s permissions, if it
    /// raises one: Arm `AArch64_FullTranslate`'s checks in its order —
    /// stage 1's ([`Cpu::s1_refusal`]), then stage 2's
    /// ([`Cpu::s2_refusal`]), which only a processor with EL2 makes, every
    /// entry giving them all without — the checks a walk's leaf meets on a
    /// miss ([`Cpu::translate_miss`]), made by the same two functions (R5).
    #[inline(always)]
    fn entry_refusal(
        &self,
        entry: &TlbEntry,
        va: u64,
        needs: Needs,
        el: El,
        on_device: OnDevice,
    ) -> Option<Abort> {
        if let Some(abort) = self.s1_refusal(&entry.stage1_leaf::<L>(), va, needs, el, on_device) {
            return Some(abort);
        }
        // An entry whose stage 2 gives every permission and Normal memory —
        // every entry stage 2 takes no part in, `Stage2Leaf::NONE`'s —
        // passes stage 2's checks.
        if L::EL2 && (entry.s2.perms != PERMS || entry.s2.device) {
            return self.s2_refusal(entry.s2, needs, el, on_device, needs.writes());
        }
        None
    }

    /// Stage 1's checks of an access needing `needs` checked against `el`'s
    /// permissions on what stage 1 gave it, `s1` — what a TLB entry holds
    /// on a hit ([`TlbEntry::stage1_leaf`]), a walk's leaf on a miss
    /// (`AArch64_S1Translate` after its walk, or
    /// `AArch64_S1DisabledOutput`): Device memory's alignment check
    /// (`AArch64_S1HasAlignmentFaultDueToMemType`, as `on_device` asks;
    /// an instruction fetch never does), then the permissions
    /// (`AArch64_S1CheckPermissions`), none where stage 1 is disabled.
    /// FEAT_E0PD is stage 1's walk's, ahead of it: a miss meets it in
    /// [`Cpu::s1_walk`]; the TLB holds no page stage 1's walk made that
    /// E0PD refuses EL0 and EL0 has a permission for ([`Cpu::cache`]), so
    /// every EL0 access E0PD refuses that hits such an entry is refused here,
    /// and [`Cpu::refusal`] names the fault. A leaf made with stage 1
    /// disabled meets no E0PD. Each of `s1`'s fields is read where a check
    /// reads it, so a hit reads only what its checks need.
    #[inline(always)]
    fn s1_refusal(
        &self,
        s1: &Stage1Leaf,
        va: u64,
        needs: Needs,
        el: El,
        on_device: OnDevice,
    ) -> Option<Abort> {
        let walked = s1.stage1 == Stage1::Walked;
        if on_device == OnDevice::Refused && s1.device {
            let fault = if walked {
                self.refusal(va, el, Fsc::Alignment)
            } else {
                Fsc::Alignment
            };
            return Some(Abort::first(fault, needs.writes()));
        }
        if walked {
            if let Some(refused) = self.first_refused(s1.perms, needs, el, Pan::Checked) {
                let fault = self.refusal(va, el, Fsc::Permission(s1.level));
                return Some(Abort::first(fault, refused == AccessKind::Write));
            }
        }
        None
    }

    /// Stage 2's checks of an access needing `needs` checked against `el`'s
    /// permissions on the leaf `leaf` (`AArch64_S2Translate` after its
    /// walk): Device memory's alignment check
    /// (`AArch64_S2HasAlignmentFaultDueToMemType`), then its permissions
    /// (`AArch64_S2CheckPermissions`), which have no PAN. Neither fault
    /// reports the IPA (`IPAValid`).
    #[inline(always)]
    fn s2_refusal(
        &self,
        leaf: Stage2Leaf,
        needs: Needs,
        el: El,
        on_device: OnDevice,
        write: bool,
    ) -> Option<Abort> {
        if on_device == OnDevice::Refused && leaf.device {
            return Some(Abort::second_unreported(Fsc::Alignment, write));
        }
        let refused = self.first_refused(leaf.perms, needs, el, Pan::Absent)?;
        let fault = Fsc::Permission(leaf.level);
        Some(Abort::second_unreported(
            fault,
            refused == AccessKind::Write,
        ))
    }

    /// The first permission `needs` lacks among `perms` at `el`, in the
    /// order the permission checks test them: read, then write, for a
    /// read-modify-write; a cache maintenance instruction's as
    /// [`maintenance_needs`] gives them. Stage 1's checks take PSTATE.PAN
    /// into a data access's (`pan`); no cache maintenance's does.
    #[inline(always)]
    fn first_refused(&self, perms: u8, needs: Needs, el: El, pan: Pan) -> Option<AccessKind> {
        let has = |kind: AccessKind| match pan {
            Pan::Checked => self.permits(perms, kind, el),
            Pan::Absent => permitted(perms, kind, el),
        };
        match needs {
            Needs::Access(kind) => (!has(kind)).then_some(kind),
            Needs::ReadWrite => [AccessKind::Read, AccessKind::Write]
                .into_iter()
                .find(|&kind| !has(kind)),
            Needs::Maintenance(op) => {
                maintenance_needs(op, el).filter(|&kind| !permitted(perms, kind, el))
            }
        }
    }

    /// The translation of `va` for an access needing `needs` checked
    /// against `el`'s permissions, the one way every translation but the
    /// inline hit ([`Cpu::permitted_hit`]) goes: where the TLB is used
    /// ([`Cpu::tlb_used`]), the checks of the entry it holds
    /// ([`Cpu::entry_refusal`]); else, or on a miss, the walks
    /// ([`Cpu::translate_miss`]).
    fn translate_needs<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        needs: Needs,
        el: El,
        on_device: OnDevice,
    ) -> Result<Mapping, Abort> {
        let regime = self.regime();
        if !self.tlb_used(regime) {
            return self.untranslated(va, needs, el, on_device);
        }
        let key = self.found_tlb_key(va, regime);
        if let Some(entry) = self.tlb.hit(key) {
            return match self.entry_refusal(&entry, va, needs, el, on_device) {
                Some(abort) => Err(abort),
                None => Ok(entry.mapping(va)),
            };
        }
        self.translate_miss(bus, va, key, needs, el, on_device)
    }

    /// The translation of `va` where no stage translates it — stage 1
    /// disabled and no stage 2 — which the TLB does not hold: stage 1's
    /// output ([`Cpu::s1_disabled`]) and its checks ([`Cpu::s1_refusal`]),
    /// inline, as every access made with the MMU off makes it.
    #[inline(always)]
    fn untranslated(
        &self,
        va: u64,
        needs: Needs,
        el: El,
        on_device: OnDevice,
    ) -> Result<Mapping, Abort> {
        let regime = self.regime();
        let ipa = self
            .s1_disabled(regime, va, needs.kind())
            .map_err(|f| f.abort(needs.writes()))?;
        let s1 = self.stage1_leaf(regime, &Stage1Output::Disabled(ipa));
        if let Some(abort) = self.s1_refusal(&s1, va, needs, el, on_device) {
            return Err(abort);
        }
        Ok(Mapping {
            pa: ipa.untranslated().get(),
            guarded: false,
            device: s1.device,
        })
    }

    /// A TLB miss: Arm `AArch64_FullTranslate` for an access needing `needs`
    /// checked against `el`'s permissions, cached under `key`
    /// ([`Cpu::found_tlb_key`]). Stage 1 ([`Cpu::stage1`]) and its checks
    /// ([`Cpu::s1_refusal`]); then under stage 2 the translation of the
    /// output address ([`Cpu::s2_leaf`]) and its checks
    /// ([`Cpu::s2_refusal`]). What stage 1's walk found is cached as soon
    /// as it is known without stage 2, and with stage 2 once both stages
    /// have translated, so an access stage 1 refuses leaves no entry there;
    /// the stage 2 translations made, in stage 2's current context
    /// ([`Cpu::found_stage2_tag`]), are kept whatever the outcome.
    #[inline(never)]
    fn translate_miss<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        key: u64,
        needs: Needs,
        el: El,
        on_device: OnDevice,
    ) -> Result<Mapping, Abort> {
        let mut made = if self.stage2_enabled(self.regime()) {
            Some(Stage2Made::new(self.found_stage2_tag()))
        } else {
            None
        };
        let translated = self.translate_walks(bus, va, key, needs, el, on_device, made.as_mut());
        if let Some(made) = &made {
            self.tlb.keep(made);
        }
        translated
    }

    /// [`Cpu::translate_miss`]'s walks and checks, the stage 2 translations
    /// they make recorded in `stage2` where stage 2 translates the regime
    /// (`None` where it does not).
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn translate_walks<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        key: u64,
        needs: Needs,
        el: El,
        on_device: OnDevice,
        mut stage2: Option<&mut Stage2Made>,
    ) -> Result<Mapping, Abort> {
        let regime = self.regime();
        let kind = needs.kind();
        let write = needs.writes();
        let mut source = match stage2.as_deref_mut() {
            Some(made) => Stage2Source::Cached(made),
            // No descriptor address for stage 2 to translate.
            None => Stage2Source::Walked,
        };
        let out = self
            .stage1(bus, regime, va, kind, el, &mut source)
            .map_err(|f| f.abort(write))?;
        let s1 = self.stage1_leaf(regime, &out);
        let ipa = out.ipa();
        if let (Stage1Output::Walked(_), None) = (&out, &stage2) {
            self.cache(
                key,
                va,
                kind,
                s1,
                Stage2Leaf::NONE,
                ipa.untranslated().get(),
            );
        }
        if let Some(abort) = self.s1_refusal(&s1, va, needs, el, on_device) {
            return Err(abort);
        }
        let guarded = s1.perms & GUARDED != 0;
        let Some(made) = stage2 else {
            return Ok(Mapping {
                pa: ipa.untranslated().get(),
                guarded,
                device: s1.device,
            });
        };
        let s2 = self
            .s2_leaf(bus, ipa, &mut Stage2Source::Cached(made))
            .map_err(|fault| {
                let stage = TranslationStage::Two { ipa };
                NoTranslation { stage, fault }.abort(write)
            })?;
        self.cache(key, va, kind, s1, s2.leaf, s2.pa.get());
        if let Some(abort) = self.s2_refusal(s2.leaf, needs, el, on_device, write) {
            return Err(abort);
        }
        Ok(Mapping {
            pa: s2.pa.get(),
            guarded,
            device: s1.device || s2.leaf.device,
        })
    }

    /// Cache in the TLB, under `key` ([`Cpu::found_tlb_key`]), the
    /// translation of `va` in the current regime to `pa` that stage 1
    /// (`s1`) and stage 2 (`leaf`, [`Stage2Leaf::NONE`] without stage 2)
    /// gave an access of `kind`.
    ///
    /// A tagged address keeps its tag in the entry's tag, so it hits only
    /// itself; no entry made before a TCR write, which can change what the
    /// walk ignores, hits after it (with EL2 the EL1&0 regime's were made in
    /// another context; every other write empties the TLB). A data access
    /// through a tag
    /// instruction fetches do not ignore (`TBID`) leaves no entry: a fetch
    /// from the same address would
    /// hit it, where its own walk faults. Nor does a page stage 1's walk
    /// made that FEAT_E0PD refuses EL0 and EL0 has a permission for: its
    /// accesses walk each time, so an EL0 access never passes a hit's
    /// permission check in a half E0PD refuses EL0, where the pages are
    /// normally all EL1's and cached. A translation with stage 1 disabled
    /// meets no E0PD, which only stage 1's walk applies, and is cached
    /// whatever `TCR_EL1` says.
    fn cache(
        &mut self,
        key: u64,
        va: u64,
        kind: AccessKind,
        s1: Stage1Leaf,
        leaf: Stage2Leaf,
        pa: u64,
    ) {
        let regime = key_regime(key);
        debug_assert!(
            !L::EL2 || regime == Regime::El2 || key & TAG_MASK != 0,
            "an EL1&0 entry is made in a found context"
        );
        let fetch_faults = kind != AccessKind::Fetch
            && regime.canonical(va) != va
            && !self.regime_tbi(regime, regime.upper(va), true);
        let e0pd_refuses_permitted = s1.stage1 == Stage1::Walked
            && s1.perms & PERMS_EL0 != 0
            && self.e0pd_refuses(va, El::El0);
        if fetch_faults || e0pd_refuses_permitted {
            return;
        }
        // The fill may evict the entry the running trace's fetches hit; the
        // next instruction's fetch translates again, as it would have.
        self.trace_break = true;
        self.tlb.entries[Tlb::<L>::slot(key)] = TlbEntry {
            key,
            pa_page: pa & !0xFFF,
            device: s1.device || leaf.device,
            s1,
            s2: leaf,
        };
    }

    /// The E0PD rule of Arm `AArch64_S1Translate` (FEAT_E0PD): with
    /// `TCR_EL1.E0PD0` or `E0PD1` set, an access at `va` checked against
    /// EL0's permissions (`accdesc.el`, EL0's for LDTR and STTR at EL1
    /// without PSTATE.UAO), which only the EL1&0 regime has, takes a level 0
    /// translation fault, ahead of the walk. Every such access meets it: on
    /// a TLB miss in [`Cpu::s1_walk`]; on a hit in [`Cpu::refusal`], where
    /// its permission check fails, as the TLB holds no page E0PD refuses EL0
    /// that EL0 has a permission for ([`Cpu::cache`]); and in
    /// [`Cpu::cache_maintenance`], whose accesses can need no permission.
    fn e0pd_refuses(&self, va: u64, el: El) -> bool {
        el == El::El0 && e0pd_set(self.imp.features, self.sys.tcr_el1, va)
    }

    /// Stage 1 of `regime` for an access of `kind` at `va` checked against
    /// `el`'s permissions, as far as its output: with stage 1 enabled its
    /// walk ([`Cpu::s1_walk`]), its descriptors' stage 2 translations from
    /// `source`; disabled, its output address ([`Cpu::s1_disabled`]). The
    /// one stage 1 an access's translation, an address translation
    /// instruction's and a debugger's make (R5).
    fn stage1<B: PhysBus>(
        &self,
        bus: &mut B,
        regime: Regime,
        va: u64,
        kind: AccessKind,
        el: El,
        source: &mut Stage2Source<'_>,
    ) -> Result<Stage1Output, NoTranslation> {
        if self.s1_enabled(regime) {
            self.s1_walk(bus, regime, va, kind, el, source)
                .map(Stage1Output::Walked)
        } else {
            self.s1_disabled(regime, va, kind)
                .map(Stage1Output::Disabled)
        }
    }

    /// Arm `AArch64_S1DisabledOutput`: the output address of an access of
    /// `kind` at `va` with `regime`'s stage 1 disabled
    /// ([`Cpu::s1_disabled_output`]), or the level 0 Address size fault of
    /// one wider than the physical address size.
    #[inline(always)]
    fn s1_disabled(&self, regime: Regime, va: u64, kind: AccessKind) -> Result<Ipa, NoTranslation> {
        self.s1_disabled_output(regime, va, kind)
            .ok_or(NoTranslation::stage1(TranslationFault::AddressSize {
                level: 0,
            }))
    }

    /// What an access's checks read of stage 1's output `out` in `regime`:
    /// a walk's permissions, memory type and level; with stage 1 disabled
    /// every permission, no guarded page, and Device-nGnRnE memory for a
    /// data access unless `HCR_EL2.DC` makes it Normal.
    #[inline(always)]
    fn stage1_leaf(&self, regime: Regime, out: &Stage1Output) -> Stage1Leaf {
        match out {
            Stage1Output::Walked(walked) => Stage1Leaf {
                perms: walked.perms,
                device: walked.device(self.imp.features),
                level: walked.level,
                stage1: Stage1::Walked,
            },
            Stage1Output::Disabled(_) => Stage1Leaf {
                perms: PERMS,
                device: !self.s1_disabled_data_normal(regime),
                level: 0,
                stage1: Stage1::Disabled,
            },
        }
    }

    /// Arm `AArch64_S1Translate` with stage 1 enabled, as far as the walk:
    /// FEAT_E0PD's refusal of an access checked against `el`'s permissions,
    /// then the walk of `regime` for an access of `kind` at `va`, its
    /// descriptors' stage 2 translations from `source`.
    fn s1_walk<B: PhysBus>(
        &self,
        bus: &mut B,
        regime: Regime,
        va: u64,
        kind: AccessKind,
        el: El,
        source: &mut Stage2Source<'_>,
    ) -> Result<Walked, NoTranslation> {
        if self.e0pd_refuses(va, el) {
            return Err(NoTranslation::stage1(TranslationFault::Translation {
                level: 0,
            }));
        }
        self.walk(bus, self.s1_params(regime, va), va, kind, source)
    }

    /// The fault an access at `va` checked against `el`'s permissions takes
    /// when its TLB entry refuses it `fault`: a level 0 translation fault
    /// where FEAT_E0PD refuses EL0 the page, which Arm's
    /// `AArch64_S1Translate` checks ahead of the walk and so of every other
    /// fault, and `fault` otherwise. Inline, so that where only whether an
    /// entry refuses is asked ([`Cpu::permitted_hit`]) nothing of it is
    /// made.
    #[inline(always)]
    fn refusal(&self, va: u64, el: El, fault: Fsc) -> Fsc {
        if self.e0pd_refuses(va, el) {
            Fsc::Translation(0)
        } else {
            fault
        }
    }

    /// Whether an access of `kind` checked against `el`'s permissions may
    /// use a page the walk gave `perms`: it has the permission it needs,
    /// and a data access is not one PSTATE.PAN refuses (every data access
    /// is subject to PAN, Arm `AccessDescriptor.pan`; an instruction fetch
    /// is not).
    #[inline(always)]
    fn permits(&self, perms: u8, kind: AccessKind, el: El) -> bool {
        permitted(perms, kind, el) && (kind == AccessKind::Fetch || !self.pan_refuses(perms, el))
    }

    /// The PAN rule of Arm `AArch64_S1DirectBasePermissions` (FEAT_PAN):
    /// while PSTATE.PAN is set, an EL1 access subject to it has no access
    /// to memory EL0 can read or write, nor, with FEAT_PAN3 and
    /// `SCTLR_EL1.EPAN` set, to memory EL0 can execute. EL0's execute
    /// permission in `perms` is after WXN, which differs from the rule's
    /// `ux` only where EL0 can write, which the rule refuses anyway.
    #[inline(always)]
    fn pan_refuses(&self, perms: u8, el: El) -> bool {
        // `PSTATE.PAN` set and `el` EL1, as one test of every data access:
        // PAN is 0 or 1 and EL1's is the one odd index (asserted beside
        // `permitted`).
        if el.index() & u64::from(self.pstate.pan) == 0 {
            return false;
        }
        let epan = self.implements(Feature::Pan3) && self.sys.sctlr_el1 & sctlr::EPAN != 0;
        let reachable = PERM_EL0_R | PERM_EL0_W | if epan { PERM_EL0_X } else { 0 };
        perms & reachable != 0
    }

    /// Arm `EffectiveTBI(address, instruction, PSTATE.EL)`: whether
    /// top-byte-ignore covers `address`, an instruction address or a data
    /// one, in the current regime — in EL1&0 the range its bit 55 selects,
    /// in EL2 the one range whatever the bit.
    #[inline]
    pub(crate) fn effective_tbi(&self, address: u64, instruction: bool) -> bool {
        let regime = self.regime();
        self.regime_tbi(regime, regime.upper(address), instruction)
    }

    /// The output address of an access of `kind` at `va` with `regime`'s
    /// stage 1 disabled (Arm `AArch64_S1DisabledOutput`): `va` below its
    /// `AddrTop`, the tag cleared where top-byte-ignore covers it, or `None`
    /// where that is wider than the physical address size, an address size
    /// fault at level 0.
    #[inline(always)]
    fn s1_disabled_output(&self, regime: Regime, va: u64, kind: AccessKind) -> Option<Ipa> {
        let tbi = self.regime_tbi(regime, regime.upper(va), kind == AccessKind::Fetch);
        let address = if tbi { va & 0x00FF_FFFF_FFFF_FFFF } else { va };
        (address >> id::PA_BITS == 0).then_some(Ipa::new(address))
    }

    /// The translation table walk of one regime (Arm `AArch64_S1Walk`, with
    /// `AArch64_VAIsOutOfRange` ahead of it) for the 4 KiB granule, with no
    /// hardware access-flag management, under the parameters `params` the
    /// regime's registers give the range `va` is in.
    ///
    /// The start level's descriptor address, every next-level table address
    /// and the output address must fit the output size the regime's
    /// `TCR_ELx.IPS` or `PS` gives (`AArch64_S1OAOutOfRange`), else an
    /// address size fault: the first at level 0, the others at the level of
    /// the descriptor that holds them. The table base leaves out
    /// `TTBRn_ELx.CnP` (FEAT_TTCNP), bit 0, which has no effect on one
    /// processor (`AArch64_S1TTBaseAddress`).
    ///
    /// Table descriptors' hierarchical permissions apply unless FEAT_HPDS's
    /// `HPD` turns them off for the range (`AArch64_S1NextWalkStateTable`,
    /// `AArch64_S1ApplyTablePerms`), and the leaf's permissions are those of
    /// `AArch64_S1ApplyOutputPerms` and `AArch64_S1DirectBasePermissions`:
    /// in EL1&0 `AP[2:1]`, `UXN`, `PXN`, `APTable`, `UXNTable` and
    /// `PXNTable`; in EL2, which has one privilege level, `AP[2]`, `XN` (bit
    /// 54), `APTable[1]` and `XNTable` (bit 60), the descriptor's other
    /// permission bits ignored.
    ///
    /// Under stage 2 every descriptor's address is an IPA, which stage 2
    /// translates before the descriptor is read ([`Cpu::s2_descriptor`]),
    /// from `source`.
    fn walk<B: PhysBus>(
        &self,
        bus: &mut B,
        params: S1Params,
        va: u64,
        kind: AccessKind,
        source: &mut Stage2Source<'_>,
    ) -> Result<Walked, NoTranslation> {
        let s1 = NoTranslation::stage1;
        // A TxSZ outside 16..=39 (`AArch64_S1MinTxSZ` and `AArch64_MaxTxSZ`
        // for the 4 KiB granule without FEAT_LPA2's `DS` or FEAT_TTST):
        // whether such a value is treated as the nearest one in range or
        // takes a level 0 Translation fault is IMPLEMENTATION DEFINED (Arm
        // `AArch64_GetS1TTWParams`, the ImpDef booleans "Fault on TxSZ value
        // above maximum" and "... below minimum"; `AArch64_S1InvalidTxSZ`).
        // This takes the fault in either regime, as QEMU does
        // (target/arm/ptw.c `get_phys_addr_lpae`, `param.tsz_oob`).
        if !(16..=39).contains(&params.txsz) {
            return Err(s1(TranslationFault::Translation { level: 0 }));
        }
        let ia_bits = 64 - params.txsz;
        let top = if self.regime_tbi(params.regime, params.upper, kind == AccessKind::Fetch) {
            56
        } else {
            64
        };
        let check = ones(top - ia_bits) << ia_bits;
        if va & check != if params.upper { check } else { 0 } {
            return Err(s1(TranslationFault::Translation { level: 0 }));
        }
        if params.epd {
            return Err(s1(TranslationFault::Translation { level: 0 }));
        }

        let start_level: u32 = if ia_bits > 39 {
            0
        } else if ia_bits > 30 {
            1
        } else {
            2
        };
        let start = WalkStart::<Ipa>::of_base(params.ttbr, start_level, ia_bits, params.oa_bits)
            .map_err(s1)?;
        let stage2 = self.stage2_enabled(params.regime);
        // The descriptor's address is an IPA, which stage 2 translates where
        // it translates the regime and is the physical address otherwise.
        let mut descriptor_address = |bus: &mut B, ipa: Ipa| {
            if stage2 {
                self.s2_descriptor(bus, ipa, source)
            } else {
                Ok(ipa.untranslated())
            }
        };
        let leaf = match self.walk_tables(bus, va, start, &mut descriptor_address) {
            Ok(leaf) => leaf,
            Err(WalkStop::Fault(fault)) => return Err(s1(fault)),
            Err(WalkStop::Descriptor(failure)) => return Err(failure),
        };
        let desc = leaf.desc;
        // `APTable`, `UXNTable` (`XNTable`, the same bit, in EL2) and
        // `PXNTable` of every table descriptor, bits 62:61, 60 and 59.
        let (ap_table, uxn_table, pxn_table) = if params.hpd {
            (0, false, false)
        } else {
            (
                (leaf.tables >> 2) & 3,
                (leaf.tables >> 1) & 1 == 1,
                leaf.tables & 1 == 1,
            )
        };

        let ap = (desc >> 6) & 3;
        let mut perms = if params.regime.has_unprivileged() {
            let mut el1_w = ap & 2 == 0;
            let mut el0_r = ap & 1 == 1;
            let mut el0_w = el0_r && el1_w;
            if ap_table & 1 != 0 {
                el0_r = false;
                el0_w = false;
            }
            if ap_table & 2 != 0 {
                el1_w = false;
                el0_w = false;
            }
            let uxn = (desc >> 54) & 1 == 1 || uxn_table;
            let pxn = (desc >> 53) & 1 == 1 || pxn_table;
            // EL1 never executes memory EL0 can write, and WXN makes every
            // writable region execute-never at its own EL.
            let el1_x = !pxn && !el0_w && !(params.wxn && el1_w);
            let el0_x = !uxn && !(params.wxn && el0_w);
            let mut perms = PERM_EL1_R;
            if el1_w {
                perms |= PERM_EL1_W;
            }
            if el1_x {
                perms |= PERM_EL1_X;
            }
            if el0_r {
                perms |= PERM_EL0_R;
            }
            if el0_w {
                perms |= PERM_EL0_W;
            }
            if el0_x {
                perms |= PERM_EL0_X;
            }
            perms
        } else {
            let w = ap & 2 == 0 && ap_table & 2 == 0;
            let xn = (desc >> 54) & 1 == 1 || uxn_table;
            let x = !xn && !(params.wxn && w);
            let mut perms = PERM_EL1_R;
            if w {
                perms |= PERM_EL1_W;
            }
            if x {
                perms |= PERM_EL1_X;
            }
            perms
        };
        // GP, bit 50 of a block or page descriptor, RES0 without FEAT_BTI.
        if (desc >> 50) & 1 == 1 && self.implements(Feature::Bti) {
            perms |= GUARDED;
        }
        Ok(Walked {
            ipa: leaf.out,
            perms,
            level: leaf.level as u8,
            attr: ((params.mair >> (((desc >> 2) & 7) * 8)) & 0xFF) as u8,
            sh: ((desc >> 8) & 3) as u8,
        })
    }

    /// The walk of translation tables from `start` for input address
    /// `input`, the part Arm's `AArch64_S1Walk` and `AArch64_S2Walk` share
    /// for the 4 KiB granule with no FEAT_LPA2, FEAT_D128 or hardware
    /// update: each level's descriptor, read at the physical address
    /// `descriptor_address` gives the address of its table entry, in the
    /// walk's output address space `A` (an IPA for stage 1's walk), must be
    /// valid, a read that reaches nothing being a synchronous External
    /// abort on the walk; a table descriptor gives the next level's table,
    /// whose address must fit the output size (else an Address size fault at
    /// the level of the descriptor that holds it); a level 3 descriptor with
    /// bit 1 clear is reserved and level 0 has no block
    /// (`AArch64_BlockDescSupported`); and the leaf's output address must fit
    /// the output size and its access flag be set. The leaf's `tables` is
    /// the OR of every table descriptor's bits 63:59, stage 1's hierarchical
    /// attributes.
    pub(super) fn walk_tables<B: PhysBus, A: WalkOutput, E, F>(
        &self,
        bus: &mut B,
        input: u64,
        start: WalkStart<A>,
        descriptor_address: &mut F,
    ) -> Result<WalkLeaf<A>, WalkStop<E>>
    where
        F: FnMut(&mut B, A) -> Result<PhysAddr, E>,
    {
        let mut table = start.table.get();
        let mut level = start.level;
        let mut tables = 0;
        let clock = self.clock();
        loop {
            let shift = 12 + 9 * (3 - level);
            let index_bits = if level == start.level {
                start.index_bits
            } else {
                9
            };
            let index = (input >> shift) & ones(index_bits);
            let at_level = level as u8;
            let fault = |fault| Err(WalkStop::Fault(fault));
            let address =
                descriptor_address(bus, A::new(table + index * 8)).map_err(WalkStop::Descriptor)?;
            let desc = match bus.read_descriptor(address, clock) {
                Ok(d) => d,
                Err(BusFault::Unmapped) => {
                    return fault(TranslationFault::ExternalAbortOnWalk { level: at_level })
                }
            };
            if desc & 1 == 0 {
                return fault(TranslationFault::Translation { level: at_level });
            }
            if level < 3 && desc & 2 != 0 {
                let next = desc & 0x0000_FFFF_FFFF_F000;
                if next >> start.oa_bits != 0 {
                    return fault(TranslationFault::AddressSize { level: at_level });
                }
                tables |= desc >> 59;
                table = next;
                level += 1;
                continue;
            }
            if (level == 3 && desc & 2 == 0) || level == 0 {
                return fault(TranslationFault::Translation { level: at_level });
            }
            let out = desc & 0x0000_FFFF_FFFF_F000 & !ones(shift);
            if out >> start.oa_bits != 0 {
                return fault(TranslationFault::AddressSize { level: at_level });
            }
            if (desc >> 10) & 1 == 0 {
                return fault(TranslationFault::AccessFlag { level: at_level });
            }
            return Ok(WalkLeaf {
                desc,
                level,
                out: A::new(out | (input & ones(shift))),
                tables,
            });
        }
    }

    /// Stage 2's translation of `ipa` (`AArch64_S2Translate`'s walk): the
    /// output address and the leaf, where `source` lets it from the stage 2
    /// translations the same translation made or the TLB keeps, else from
    /// the walk, which `source` records.
    fn s2_leaf<B: PhysBus>(
        &self,
        bus: &mut B,
        ipa: Ipa,
        source: &mut Stage2Source<'_>,
    ) -> Result<Stage2Output, TranslationFault> {
        if let Stage2Source::Cached(made) = source {
            let key = made.key(ipa);
            if let Some(kept) = made.find(key).or_else(|| self.tlb.ipa_hit(key)) {
                return Ok(Stage2Output {
                    pa: PhysAddr::new(in_page(kept.pa_page.get(), ipa.get())),
                    leaf: kept.leaf,
                });
            }
        }
        let walked = self.s2_walk(bus, ipa)?;
        let leaf = Stage2Leaf::of(&walked);
        if let Stage2Source::Cached(made) = source {
            made.record(IpaEntry {
                key: made.key(ipa),
                pa_page: PhysAddr::new(walked.pa.get() & !0xFFF),
                leaf,
            });
        }
        Ok(Stage2Output {
            pa: walked.pa,
            leaf,
        })
    }

    /// The physical address of the stage 1 descriptor at `ipa` (Arm
    /// `AArch64_S2Translate` for `AccessType_TTW`): stage 2's translation
    /// (from `source`), then its permission checks for a walk
    /// (`AArch64_S2CheckPermissions`): `HCR_EL2.PTW` refuses Device memory,
    /// and the descriptor needs read permission. A fault is stage 2's on
    /// stage 1's walk (`S1PTW`), and reports `ipa`.
    fn s2_descriptor<B: PhysBus>(
        &self,
        bus: &mut B,
        ipa: Ipa,
        source: &mut Stage2Source<'_>,
    ) -> Result<PhysAddr, NoTranslation> {
        let on_walk = |fault| NoTranslation {
            stage: TranslationStage::TwoOnWalk { ipa },
            fault,
        };
        let Stage2Output { pa, leaf } = self.s2_leaf(bus, ipa, source).map_err(on_walk)?;
        if (self.s2_ptw() && leaf.device) || leaf.perms & PERM_EL1_R == 0 {
            return Err(on_walk(TranslationFault::Permission { level: leaf.level }));
        }
        Ok(pa)
    }

    /// Where data address `va` translates to now in the current regime, for
    /// a debugger: the walks a data access would make, stage 2's included,
    /// with no check of the access's permissions, no TLB lookup or fill,
    /// and descriptors read through `bus` — which a caller makes a view that
    /// cannot disturb a device. Nothing in the processor changes.
    pub(crate) fn debug_translate<B: PhysBus>(
        &self,
        bus: &mut B,
        va: u64,
    ) -> Result<u64, NoTranslation> {
        let regime = self.regime();
        // Checked against EL1's permissions, a privileged level's, which
        // FEAT_E0PD leaves alone.
        let ipa = self
            .stage1(
                bus,
                regime,
                va,
                AccessKind::Read,
                El::El1,
                &mut Stage2Source::Walked,
            )?
            .ipa();
        if !self.stage2_enabled(regime) {
            return Ok(ipa.untranslated().get());
        }
        self.s2_leaf(bus, ipa, &mut Stage2Source::Walked)
            .map(|s2| s2.pa.get())
            .map_err(|fault| NoTranslation {
                stage: TranslationStage::Two { ipa },
                fault,
            })
    }

    /// The address translation instructions (Arm `AArch64_AT`): translate
    /// `va` in the regime `check` names, checking permissions as it says,
    /// by stage 1 alone or, for `AT S12E*` (`stages`), by stage 2 as well
    /// where it is enabled, and report in `PAR_EL1` (`AArch64_EncodePAR`)
    /// the output address and the memory attributes the stages give it, or
    /// the fault, with `PAR_EL1.S` for stage 2's and `PTW` for one on stage
    /// 1's walk.
    ///
    /// Two faults are taken rather than reported: a synchronous external
    /// abort on a walk, and from EL1 a stage 2 fault on stage 1's walk, each
    /// a Data Abort with `CM` and `WnR` set (`AArch64_FaultSyndrome`,
    /// `AccessType_AT`), leaving `PAR_EL1` as it was, which is one of the
    /// values the ARBITRARY one Arm gives it can be; QEMU takes them the same
    /// way.
    ///
    /// Stage 1's attributes are Normal Non-cacheable for Normal memory with
    /// the regime's `SCTLR_ELx.C` clear (`AArch64_S1TreatAsNormalNC`), and
    /// stage 2's with `HCR_EL2.CD` set (`AArch64_S2TreatAsNormalNC`). Stage 1
    /// reports its effective shareability under stage 2 as well as without
    /// (the IMPLEMENTATION DEFINED "Apply effective shareability at stage
    /// 1"), which no `PAR_EL1` tells apart: the shareability the two choices
    /// differ in is that of Device and Non-cacheable memory, which
    /// `PAREncodeShareability` and `S2CombineS1MemAttrs` make Outer
    /// Shareable either way.
    pub(crate) fn address_translate<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        write: bool,
        check: AtCheck,
        stages: AtStages,
    ) -> Result<(), Fault> {
        const RES1: u64 = 1 << 11;
        // This core runs in Non-secure state, where `PAR_EL1.NS` of a
        // successful translation is ARBITRARY: it reports 1.
        const NS: u64 = 1 << 9;
        // `PAR_EL1.PTW` and `S` of a fault.
        const PTW: u64 = 1 << 8;
        const S: u64 = 1 << 9;
        let kind = if write {
            AccessKind::Write
        } else {
            AccessKind::Read
        };
        let (regime, el) = match check {
            AtCheck::El0 => (Regime::El10, El::El0),
            AtCheck::El1 | AtCheck::El1Pan => (Regime::El10, El::El1),
            AtCheck::El2 => (Regime::El2, El::El2),
        };
        let permits = |cpu: &Self, perms: u8| match check {
            AtCheck::El0 | AtCheck::El1 | AtCheck::El2 => permitted(perms, kind, el),
            AtCheck::El1Pan => cpu.permits(perms, kind, el),
        };
        let stage1 = match self.stage1(bus, regime, va, kind, el, &mut Stage2Source::Walked) {
            Ok(Stage1Output::Walked(w)) if permits(self, w.perms) => {
                let non_cacheable = self.regime_sctlr(regime) & sctlr::C == 0;
                let attrs = w
                    .attrs(self.imp.features)
                    .normal_as_non_cacheable(non_cacheable);
                Ok((w.ipa, attrs.effective_shareability()))
            }
            Ok(Stage1Output::Walked(w)) => Err(Abort::first(Fsc::Permission(w.level), write)),
            Ok(Stage1Output::Disabled(ipa)) => {
                let attrs = if self.s1_disabled_data_normal(regime) {
                    MemAttrs::NORMAL_WRITE_BACK
                } else {
                    MemAttrs::DEVICE_NGNRNE
                };
                Ok((ipa, attrs))
            }
            Err(fault) => Err(fault.abort(write)),
        };
        let result = match stage1 {
            Ok((ipa, s1_attrs)) if stages == AtStages::OneAndTwo && self.stage2_enabled(regime) => {
                match self.s2_walk(bus, ipa) {
                    Ok(s2) => {
                        let leaf = Stage2Leaf::of(&s2);
                        match self.s2_refusal(
                            leaf,
                            Needs::Access(kind),
                            el,
                            OnDevice::Permitted,
                            write,
                        ) {
                            Some(abort) => Err(abort),
                            None => {
                                let cd = self.hcr_any(HCR_CD);
                                let s2_attrs = s2.attrs().normal_as_non_cacheable(cd);
                                Ok((s2.pa.get(), MemAttrs::combined(s1_attrs, s2_attrs)))
                            }
                        }
                    }
                    Err(fault) => Err(NoTranslation {
                        stage: TranslationStage::Two { ipa },
                        fault,
                    }
                    .abort(write)),
                }
            }
            // Stage 1's output address: for `AT S1E*` an IPA where the
            // regime has stage 2, and for `AT S12E*` with stage 2 disabled
            // the physical address that IPA is, which has the same value.
            Ok((ipa, attrs)) => Ok((ipa.get(), attrs)),
            Err(abort) => Err(abort),
        };
        if let Err(abort) = result {
            let on_walk = matches!(abort.stage, AbortStage::SecondOnWalk { .. });
            if matches!(abort.fsc, Fsc::SyncExternalOnWalk(_))
                || (on_walk && self.pstate.el == El::El1)
            {
                return Err(Self::maintenance_abort(va, abort));
            }
        }
        self.sys.par_el1 = match result {
            Ok((pa, attrs)) => {
                (u64::from(attrs.par_attr(self.imp.features)) << 56)
                    | (pa & 0x0000_FFFF_FFFF_F000)
                    | NS
                    | (u64::from(attrs.par_shareability()) << 7)
                    | RES1
            }
            // F = 1, FST, S and PTW, bit 11 RES1.
            Err(abort) => {
                let stage = match abort.stage {
                    AbortStage::First => 0,
                    AbortStage::Second { .. } => S,
                    AbortStage::SecondOnWalk { .. } => S | PTW,
                };
                RES1 | stage | (u64::from(abort.fsc.code()) << 1) | 1
            }
        };
        Ok(())
    }

    // ----- access paths -----

    /// The exception an abort of `class` with syndrome `iss` at `va` raises,
    /// as its stage takes it: a stage 1 abort where its class's function
    /// does, and a stage 2 abort to EL2 (`IsSecondStage`), reporting the IPA
    /// in `HPFAR_EL2` where `IPAValid`, with `S1PTW` on stage 1's walk. A
    /// Data Abort of stage 2 on the access itself leaves open the
    /// [`SyndromeSlot`] its access may (`slot`, [`DataAccess::syndrome_slot`])
    /// for the load or store that made it (`AArch64_InstructionSyndromeValid`),
    /// unless it is a synchronous External abort on stage 2's walk: the
    /// IMPLEMENTATION DEFINED "ISV on second stage translation table walk"
    /// is FALSE here, the value FEAT_RAS gives it.
    fn abort_exception(
        class: AddressedClass,
        iss: u32,
        va: u64,
        abort: Abort,
        slot: SyndromeSlot,
    ) -> Fault {
        match abort.stage {
            AbortStage::First => Exception::at_address(class, iss, va).into(),
            AbortStage::Second { hpfar } => {
                let slot = match abort.fsc {
                    Fsc::SyncExternalOnWalk(_) => SyndromeSlot::Closed,
                    Fsc::AddressSize(_)
                    | Fsc::Translation(_)
                    | Fsc::AccessFlag(_)
                    | Fsc::Permission(_)
                    | Fsc::SyncExternal
                    | Fsc::Alignment => slot,
                };
                Exception::second_stage_abort(class, iss, va, hpfar, slot).into()
            }
            AbortStage::SecondOnWalk { hpfar } => {
                let iss = iss | ISS_S1PTW;
                Exception::second_stage_abort(class, iss, va, hpfar, SyndromeSlot::Closed).into()
            }
        }
    }

    /// The Data Abort of a data access at `va`, `WnR` its
    /// `FaultRecord.write`, whose syndrome slot may be `slot`.
    fn data_abort(va: u64, abort: Abort, slot: SyndromeSlot) -> Fault {
        let iss = abort.fsc.code() | if abort.write { ISS_WNR } else { 0 };
        Self::abort_exception(AddressedClass::DataAbort, iss, va, abort, slot)
    }

    /// The Data Abort of an alignment check made before translation.
    fn alignment_abort(va: u64, write: bool) -> Fault {
        Self::data_abort(
            va,
            Abort::first(Fsc::Alignment, write),
            SyndromeSlot::Closed,
        )
    }

    /// The Data Abort of a cache maintenance instruction by address or an
    /// address translation instruction, with `CM` and `WnR` set
    /// (`AArch64_FaultSyndrome`, `AccessType_DC`, `AccessType_IC` and
    /// `AccessType_AT`). Its instruction has no syndrome to report.
    fn maintenance_abort(va: u64, abort: Abort) -> Fault {
        let iss = abort.fsc.code() | ISS_CM | ISS_WNR;
        Self::abort_exception(
            AddressedClass::DataAbort,
            iss,
            va,
            abort,
            SyndromeSlot::Closed,
        )
    }

    /// The Instruction Abort of a fetch at `va`, which reports no
    /// instruction syndrome.
    fn instruction_abort(va: u64, abort: Abort) -> Fault {
        let iss = abort.fsc.code();
        let class = AddressedClass::InstructionAbort;
        Self::abort_exception(class, iss, va, abort, SyndromeSlot::Closed)
    }

    /// A synchronous external abort: the bus refused the access. With
    /// FEAT_RAS its syndrome reports the PE's error state in `ISS.SET`
    /// (bits 12:11, `AArch64_EncodeSyncErrorSyndrome`), which `PEErrorState`
    /// leaves IMPLEMENTATION DEFINED; here it is Recoverable (UER, 0b00), the
    /// access refused before it took effect, as QEMU reports it. The
    /// syndrome is then the same with the feature as without.
    #[cold]
    #[inline(never)]
    fn external_abort(&self, va: u64, pa: u64, kind: AccessKind) -> Fault {
        if self.debug_stops.on_bus_fault {
            return Fault::Stop(StopCause::BusFault {
                pc: self.pc,
                addr: pa,
            });
        }
        let abort = Abort::first(Fsc::SyncExternal, kind == AccessKind::Write);
        match kind {
            AccessKind::Fetch => Self::instruction_abort(va, abort),
            AccessKind::Read | AccessKind::Write => {
                Self::data_abort(va, abort, SyndromeSlot::Closed)
            }
        }
    }

    /// The physical address of the instruction at `pc` (already known to be
    /// aligned), or the Instruction Abort its translation raises. Records
    /// whether the page is guarded (Arm `InGuardedPage`, FEAT_BTI) in
    /// [`Cpu::in_guarded_page`], for the instructions fetched from it.
    #[inline(always)]
    pub(crate) fn fetch_translate<B: PhysBus>(
        &mut self,
        bus: &mut B,
        pc: u64,
    ) -> Result<u64, Fault> {
        match self.permitted_hit(pc, AccessKind::Fetch, self.pstate.el) {
            Some(hit) => {
                self.in_guarded_page = hit.guarded;
                Ok(hit.pa)
            }
            None => self.fetch_translate_checked(bus, pc),
        }
    }

    /// The physical address a fetch at `pc` reads now, when neither the
    /// fetch nor the checks on the word fetched raise an exception and the
    /// fetch needs no walk: `None` on `PSTATE.IL` (the word raises the
    /// Illegal Execution state exception), a misaligned `pc`, or anything
    /// but a TLB hit that permits a fetch at the current EL. With stage 1
    /// disabled and no stage 2, which leave the TLB unused, it is
    /// [`Cpu::translate`]'s `AArch64_S1DisabledOutput` address. It never
    /// fills, flushes or marks anything.
    pub(crate) fn fetch_probe(&self, pc: u64) -> Option<u64> {
        if self.pstate.il || pc & 3 != 0 {
            return None;
        }
        let regime = self.regime();
        if !self.tlb_used(regime) {
            return self
                .s1_disabled_output(regime, pc, AccessKind::Fetch)
                .map(|ipa| ipa.untranslated().get());
        }
        self.permitted_hit(pc, AccessKind::Fetch, self.pstate.el)
            .map(|hit| hit.pa)
    }

    /// [`Cpu::fetch_translate`] with every check made: stage 1 disabled, a
    /// TLB miss, or a page that does not permit the fetch.
    #[cold]
    #[inline(never)]
    fn fetch_translate_checked<B: PhysBus>(&mut self, bus: &mut B, pc: u64) -> Result<u64, Fault> {
        let el = self.pstate.el;
        let needs = Needs::Access(AccessKind::Fetch);
        let m = self
            .translate_needs(bus, pc, needs, el, OnDevice::Permitted)
            .map_err(|abort| Self::instruction_abort(pc, abort))?;
        self.in_guarded_page = m.guarded;
        Ok(m.pa)
    }

    /// Read the instruction word at `pa`, which `pc` translated to.
    #[inline]
    pub(crate) fn fetch_word<B: PhysBus>(
        &mut self,
        bus: &mut B,
        pc: u64,
        pa: u64,
    ) -> Result<u32, Fault> {
        match bus.fetch(PhysAddr::new(pa), self.clock()) {
            Ok(word) => Ok(word),
            Err(BusFault::Unmapped) => Err(self.external_abort(pc, pa, AccessKind::Fetch)),
        }
    }

    /// The EL whose permissions a data access is checked against: EL0's for
    /// LDTR and STTR at EL1 unless PSTATE.UAO is set, the current EL's for
    /// every other access (Arm `AArch64_IsUnprivAccessPriv`: with no FEAT_NV
    /// and no FEAT_VHE they are privileged at EL2, whose regime has no EL0).
    /// At EL0 they are EL0's either way, so only a processor with EL2 tests
    /// the level.
    #[inline]
    fn access_el(&self, acc: DataAccess) -> El {
        let below_el2 = !L::EL2 || self.pstate.el != El::El2;
        if acc.unprivileged && below_el2 && !self.pstate.uao {
            El::El0
        } else {
            self.pstate.el
        }
    }

    /// Alignment checks that precede translation, then translation, with
    /// Device memory's alignment check ahead of the permissions, for an
    /// access of `bytes` (a power of two) at `va` that stays in one page.
    /// A read-modify-write checks read permission, then write permission.
    /// WnR is `!read && write` for every fault but a refused write.
    ///
    /// A read or a write whose TLB entry permits it, and that no alignment
    /// check can refuse — aligned and whole, or misaligned on Normal memory
    /// with no check that faults it before translation — raises none of
    /// these faults, and is answered inline; everything else makes the
    /// checks in order out of line.
    #[inline(always)]
    fn translate_data<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        direction: DataDirection,
        acc: DataAccess,
    ) -> Result<Translated, Fault> {
        let hit = match direction {
            DataDirection::Read => self.permitted_hit(va, AccessKind::Read, self.access_el(acc)),
            DataDirection::Write => self.permitted_hit(va, AccessKind::Write, self.access_el(acc)),
            DataDirection::ReadWrite => None,
        };
        if let Some(hit) = hit {
            // An ordinary access misaligned on Normal memory faults only
            // under SCTLR_ELx.A ([`Cpu::misaligned_faults`]).
            let aligned = va & (bytes - 1) == 0 && acc.part == Part::Whole;
            if aligned
                || (!hit.device && acc.alignment == AlignmentRule::Any && !self.strict_alignment())
            {
                return Ok(Translated { pa: hit.pa });
            }
        }
        self.translate_data_checked(bus, va, bytes, direction, acc)
    }

    /// [`Cpu::translate_data`] with every check made, in order: stage 1
    /// disabled, a TLB miss (which walks and fills), a page that does not
    /// permit the access, a read-modify-write, or an access an alignment
    /// check may refuse.
    #[cold]
    #[inline(never)]
    fn translate_data_checked<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        direction: DataDirection,
        acc: DataAccess,
    ) -> Result<Translated, Fault> {
        let misaligned = va & (bytes - 1) != 0;
        if misaligned && self.misaligned_faults(va, bytes, acc) {
            return Err(Self::alignment_abort(va, direction == DataDirection::Write));
        }
        let on_device = if misaligned || acc.part == Part::OfMisaligned {
            OnDevice::Refused
        } else {
            OnDevice::Permitted
        };
        let el = self.access_el(acc);
        let slot = acc.syndrome_slot(AddressInAccess::Lowest);
        match direction {
            DataDirection::Read => self
                .translate(bus, va, AccessKind::Read, el, on_device)
                .map_err(|abort| Self::data_abort(va, abort, slot)),
            DataDirection::Write => self
                .translate(bus, va, AccessKind::Write, el, on_device)
                .map_err(|abort| Self::data_abort(va, abort, slot)),
            DataDirection::ReadWrite => self
                .translate_rmw(bus, va, el, on_device)
                .map_err(|abort| Self::data_abort(va, abort, slot)),
        }
    }

    /// The translation of a read-modify-write at `va`: read permission,
    /// then write permission, at stage 1 and then at stage 2
    /// ([`Cpu::translate_needs`]).
    #[inline(never)]
    fn translate_rmw<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        el: El,
        on_device: OnDevice,
    ) -> Result<Translated, Abort> {
        self.translate_needs(bus, va, Needs::ReadWrite, el, on_device)
            .map(|m| Translated { pa: m.pa })
    }

    /// The physical address an access of `bytes` at `va` reaches, with every
    /// check of a real access applied: for the exclusive monitor, and for
    /// an atomic access, which then reads and writes it (an access that
    /// stays in one page, as both are).
    pub(crate) fn translate_for<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        direction: DataDirection,
        acc: DataAccess,
    ) -> Result<PhysAddr, Fault> {
        self.translate_data(bus, va, bytes, direction, acc)
            .map(|t| PhysAddr::new(t.pa))
    }

    /// The bus half of every data read: `1 << log2` bytes (`log2` in 0..=3)
    /// at `pa`, which `va` translated to, in one bus access, zero-extended.
    /// A bus fault is a synchronous external abort reported at `va`.
    #[inline(always)]
    fn bus_read<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: PhysAddr,
        log2: u32,
    ) -> Result<u64, Fault> {
        let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
        match bus.read(pa, width, self.clock()) {
            Ok(v) => Ok(v & width.value_mask()),
            Err(BusFault::Unmapped) => Err(self.external_abort(va, pa.get(), AccessKind::Read)),
        }
    }

    /// The bus half of every data write: the low `1 << log2` bytes of
    /// `value` at `pa`, which `va` translated to, in one bus access, after
    /// [`Cpu::note_store`]. A bus fault is a synchronous external abort
    /// reported at `va`.
    #[inline(always)]
    fn bus_write<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: PhysAddr,
        log2: u32,
        value: u64,
    ) -> Result<(), Fault> {
        let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
        self.note_store(pa.get(), 1 << log2);
        match bus.write(pa, width, value & width.value_mask(), self.clock()) {
            Ok(()) => Ok(()),
            Err(BusFault::Unmapped) => Err(self.external_abort(va, pa.get(), AccessKind::Write)),
        }
    }

    /// Read `1 << log2` bytes (`log2` in 0..=3) at `va`, zero-extended. An
    /// access within one page is inlined into the instruction that makes
    /// it; one that crosses a page boundary is out of line.
    #[inline(always)]
    pub(crate) fn read_mem<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        acc: DataAccess,
    ) -> Result<u64, Fault> {
        let bytes = 1u64 << log2;
        if (va & 0xFFF) + bytes <= 0x1000 {
            let t = self.translate_data(bus, va, bytes, DataDirection::Read, acc)?;
            return self.bus_read(bus, va, PhysAddr::new(t.pa), log2);
        }
        self.read_mem_across_pages(bus, va, bytes, acc)
    }

    /// [`Cpu::read_mem`] by a load of one general-purpose register, which a
    /// stage 2 Data Abort on its access reports the syndrome of (`ISV`):
    /// `syndrome` gives it (`None` for a form that has none, one with
    /// writeback), and is made only when the access faults, so a load that
    /// completes pays nothing for it.
    #[inline(always)]
    pub(crate) fn read_gpr<B: PhysBus, S: FnOnce() -> Option<AccessSyndrome>>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        acc: DataAccess,
        syndrome: S,
    ) -> Result<u64, Fault> {
        self.read_mem(bus, va, log2, acc.of_one_register())
            .map_err(|fault| fault.reporting(syndrome()))
    }

    /// A read of `bytes` at `va` that crosses a page boundary, and so is
    /// misaligned. Each byte is an access of its own (Arm ARM `Mem[]` on an
    /// unaligned address, `AArch64_MemSingle` of each byte, each with the
    /// whole access's alignment), so a translation or external fault
    /// reports the first byte that faulted — an address in the page that
    /// needs mapping, which is what an OS's fault handler goes by — and a
    /// Device page faults on alignment at the first byte that reaches it,
    /// before its permissions are checked (the `Constraint_FAULT` choice of
    /// `Unpredictable_DEVPAGE2` for a byte after the first). An alignment
    /// fault before translation is the whole access's, and reports its
    /// address. Only the first byte's access is at the access's lowest
    /// address, so only a stage 2 fault on it can report the instruction's
    /// syndrome ([`AddressInAccess`]).
    #[inline(never)]
    fn read_mem_across_pages<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        acc: DataAccess,
    ) -> Result<u64, Fault> {
        if self.misaligned_faults(va, bytes, acc) {
            return Err(Self::alignment_abort(va, false));
        }
        let el = self.access_el(acc);
        let mut value = 0u64;
        for i in 0..bytes {
            let byte_va = va.wrapping_add(i);
            let slot = acc.syndrome_slot(AddressInAccess::of_byte(i));
            let t = self
                .translate(bus, byte_va, AccessKind::Read, el, OnDevice::Refused)
                .map_err(|abort| Self::data_abort(byte_va, abort, slot))?;
            value |= self.bus_read(bus, byte_va, PhysAddr::new(t.pa), 0)? << (8 * i);
        }
        Ok(value)
    }

    /// Write the low `1 << log2` bytes of `value` at `va`. As for a read, an
    /// access within one page is inlined and one across pages is not.
    #[inline(always)]
    pub(crate) fn write_mem<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        value: u64,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        let bytes = 1u64 << log2;
        if (va & 0xFFF) + bytes <= 0x1000 {
            let t = self.translate_data(bus, va, bytes, DataDirection::Write, acc)?;
            return self.bus_write(bus, va, PhysAddr::new(t.pa), log2, value);
        }
        self.write_mem_across_pages(bus, va, bytes, value, acc)
    }

    /// [`Cpu::write_mem`] by a store of one general-purpose register, its
    /// syndrome reported as [`Cpu::read_gpr`] reports a load's.
    #[inline(always)]
    pub(crate) fn write_gpr<B: PhysBus, S: FnOnce() -> Option<AccessSyndrome>>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        value: u64,
        acc: DataAccess,
        syndrome: S,
    ) -> Result<(), Fault> {
        self.write_mem(bus, va, log2, value, acc.of_one_register())
            .map_err(|fault| fault.reporting(syndrome()))
    }

    /// A write of the low `bytes` bytes of `value` at `va` that crosses a
    /// page boundary, its faults reported as [`Cpu::read_mem_across_pages`]
    /// reports a read's. Both pages are translated before anything is
    /// written, so a fault on the second page leaves memory untouched: the
    /// access's own address in the first page, the start of the second page
    /// in the second.
    #[inline(never)]
    fn write_mem_across_pages<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        value: u64,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        if self.misaligned_faults(va, bytes, acc) {
            return Err(Self::alignment_abort(va, true));
        }
        let el = self.access_el(acc);
        self.translate_write_pages(bus, va, bytes, acc)?;
        for i in 0..bytes {
            let byte_va = va.wrapping_add(i);
            let slot = acc.syndrome_slot(AddressInAccess::of_byte(i));
            let t = self
                .translate(bus, byte_va, AccessKind::Write, el, OnDevice::Refused)
                .map_err(|abort| Self::data_abort(byte_va, abort, slot))?;
            self.bus_write(bus, byte_va, PhysAddr::new(t.pa), 0, value >> (8 * i))?;
        }
        Ok(())
    }

    /// Translate, for a write by `acc`, the first byte of a misaligned
    /// access of `bytes` at `va` that crosses a page boundary and the first
    /// byte on its second page, so that a fault on either is taken before
    /// anything is written, reported at the byte that faulted. Only the
    /// first is at the access's lowest address, and so can report the
    /// instruction's syndrome.
    fn translate_write_pages<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        let el = self.access_el(acc);
        let second_page = va.wrapping_add(bytes - 1) & !0xFFF;
        let probes = [
            (va, AddressInAccess::Lowest),
            (second_page, AddressInAccess::Later),
        ];
        for (probe, at) in probes {
            let slot = acc.syndrome_slot(at);
            self.translate(bus, probe, AccessKind::Write, el, OnDevice::Refused)
                .map_err(|abort| Self::data_abort(probe, abort, slot))?;
        }
        Ok(())
    }

    /// Whether a pair of `1 << log2`-byte elements at `va` is one
    /// single-copy atomic access, of 8 bytes: with FEAT_LSE2, a pair of
    /// words that lies in one 16-byte granule (Arm `Mem`, `AllInSCAGranule`,
    /// whose IMPLEMENTATION DEFINED quantity is 16 here).
    #[inline(always)]
    fn pair_is_one_access(&self, va: u64, log2: u32, acc: DataAccess) -> bool {
        log2 == 2
            && (va & 15) + 8 <= 16
            && acc.alignment == AlignmentRule::Any
            && self.implements(Feature::Lse2)
    }

    /// Read two adjacent elements of `1 << log2` bytes (`log2` in 0..=3),
    /// at `va` and `va + (1 << log2)`, each zero-extended and each an access
    /// of its own with `acc`'s constraints, as Arm ARM `LDP` makes them
    /// (`Mem[address, dbytes]` then `Mem[address + dbytes, dbytes]`), and as
    /// [`Cpu::read_mem_u128`] makes its halves.
    ///
    /// The result is that of two reads of the elements made in order: the
    /// same values, and the same fault with the same address. A pair of
    /// ordinary accesses within one page translates once, as the second
    /// translation would find what the first did: the second element is on
    /// the first's page, aligned exactly when the first is, with the same
    /// constraints, so it meets the same alignment checks, and the first
    /// translation's result may be used for it as a TLB holds it (a walk's
    /// result stays usable until TLB maintenance, Arm ARM D8.13). Each
    /// element still takes its own external abort.
    ///
    /// With FEAT_LSE2, a pair of words that lies in one 16-byte granule is
    /// one single-copy atomic access of 8 bytes instead (Arm `Mem`:
    /// `AllInSCAGranule`), whose alignment, as every pair's, is judged by
    /// its elements. A pair of doublewords there would be one 16-byte
    /// access, which the bus has no width for: it stays two.
    #[inline(always)]
    pub(crate) fn read_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        acc: DataAccess,
    ) -> Result<ElementPair, Fault> {
        let bytes = 1u64 << log2;
        if self.pair_is_one_access(va, log2, acc) {
            let t = self.translate_data(bus, va, bytes, DataDirection::Read, acc)?;
            let value = self.bus_read(bus, va, PhysAddr::new(t.pa), 3)?;
            return Ok(ElementPair {
                first: value & 0xFFFF_FFFF,
                second: value >> 32,
            });
        }
        if (va & 0xFFF) + 2 * bytes <= 0x1000 && acc.alignment == AlignmentRule::Any {
            let t = self.translate_data(bus, va, bytes, DataDirection::Read, acc)?;
            let first = self.bus_read(bus, va, PhysAddr::new(t.pa), log2)?;
            let second = self.bus_read(bus, va + bytes, PhysAddr::new(t.pa + bytes), log2)?;
            return Ok(ElementPair { first, second });
        }
        self.read_pair_apart(bus, va, log2, acc)
    }

    /// A [`Cpu::read_pair`] whose elements are read on their own: across a
    /// page boundary, or with an alignment rule that judges each element by
    /// where it lies.
    #[inline(never)]
    fn read_pair_apart<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        acc: DataAccess,
    ) -> Result<ElementPair, Fault> {
        let first = self.read_mem(bus, va, log2, acc)?;
        let second = self.read_mem(bus, va.wrapping_add(1 << log2), log2, acc)?;
        Ok(ElementPair { first, second })
    }

    /// Write the low `1 << log2` bytes (`log2` in 0..=3) of each element of
    /// `value`, `first` at `va` and `second` at `va + (1 << log2)`, in Arm
    /// ARM `STP`'s order, each an access of its own as for
    /// [`Cpu::read_pair`].
    ///
    /// The result is that of two writes of the elements made in order: the
    /// same fault with the same address, and the same stores noted
    /// ([`Cpu::note_store`]) in the same order, each just before its
    /// element is written. A pair within one page translates once, for the
    /// reasons [`Cpu::read_pair`] gives, and with FEAT_LSE2 a pair of words
    /// within one 16-byte granule is one access, as it reads.
    #[inline(always)]
    pub(crate) fn write_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        value: ElementPair,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        let bytes = 1u64 << log2;
        if self.pair_is_one_access(va, log2, acc) {
            let t = self.translate_data(bus, va, bytes, DataDirection::Write, acc)?;
            let both = (value.second << 32) | (value.first & 0xFFFF_FFFF);
            return self.bus_write(bus, va, PhysAddr::new(t.pa), 3, both);
        }
        if (va & 0xFFF) + 2 * bytes <= 0x1000 && acc.alignment == AlignmentRule::Any {
            let t = self.translate_data(bus, va, bytes, DataDirection::Write, acc)?;
            self.bus_write(bus, va, PhysAddr::new(t.pa), log2, value.first)?;
            return self.bus_write(
                bus,
                va + bytes,
                PhysAddr::new(t.pa + bytes),
                log2,
                value.second,
            );
        }
        self.write_pair_apart(bus, va, log2, value, acc)
    }

    /// A [`Cpu::write_pair`] whose elements are written on their own, as
    /// for [`Cpu::read_pair_apart`].
    #[inline(never)]
    fn write_pair_apart<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        value: ElementPair,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        self.write_mem(bus, va, log2, value.first, acc)?;
        self.write_mem(bus, va.wrapping_add(1 << log2), log2, value.second, acc)
    }

    /// The read half of an atomic access: `1 << log2` bytes (`log2` in
    /// 0..=4) at `pa`, which [`Cpu::translate_for`] translated `va` to with
    /// every check, as [`Cpu::read_mem`] reads them; 16 bytes, which are
    /// aligned (no 16-byte atomic may be misaligned), are two 8-byte halves.
    pub(crate) fn read_translated<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: PhysAddr,
        log2: u32,
    ) -> Result<u128, Fault> {
        if log2 == 4 {
            let lo = self.bus_read(bus, va, pa, 3)?;
            let hi = self.bus_read(bus, va.wrapping_add(8), PhysAddr::new(pa.get() + 8), 3)?;
            return Ok(u128::from(lo) | (u128::from(hi) << 64));
        }
        self.bus_read(bus, va, pa, log2).map(u128::from)
    }

    /// The write half of an atomic access: the low `1 << log2` bytes of
    /// `value` at `pa`, as [`Cpu::read_translated`] reads them.
    pub(crate) fn write_translated<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: PhysAddr,
        log2: u32,
        value: u128,
    ) -> Result<(), Fault> {
        if log2 == 4 {
            self.bus_write(bus, va, pa, 3, value as u64)?;
            return self.bus_write(
                bus,
                va.wrapping_add(8),
                PhysAddr::new(pa.get() + 8),
                3,
                (value >> 64) as u64,
            );
        }
        self.bus_write(bus, va, pa, log2, value as u64)
    }

    /// Read 16 bytes (a Q register) at `va`, as two 8-byte halves, each
    /// translated with the whole access's alignment (Arm `Mem[]`: a 16-byte
    /// SIMD&FP access is two single-copy atomic 8-byte accesses, each passed
    /// the access's own `aligned`).
    pub(crate) fn read_mem_u128<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        acc: DataAccess,
    ) -> Result<u128, Fault> {
        if va & 15 != 0 && self.misaligned_faults(va, 16, acc) {
            return Err(Self::alignment_abort(va, false));
        }
        let halves = self.read_pair(bus, va, 3, Self::halves_of(va, acc))?;
        Ok(u128::from(halves.first) | (u128::from(halves.second) << 64))
    }

    /// The access each 8-byte half of a 16-byte access at `va` makes.
    fn halves_of(va: u64, acc: DataAccess) -> DataAccess {
        if va & 15 != 0 {
            DataAccess {
                part: Part::OfMisaligned,
                ..acc
            }
        } else {
            acc
        }
    }

    /// Write 16 bytes (a Q register) at `va`, as [`Cpu::read_mem_u128`]
    /// reads them: the low half, then the high half. An access across a
    /// page boundary, misaligned and so refused by Device memory, translates
    /// both pages before either half is written, as a scalar store across
    /// pages does ([`Cpu::translate_write_pages`]): a store that takes a
    /// synchronous Data Abort leaves memory as it was.
    pub(crate) fn write_mem_u128<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        value: u128,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        if va & 15 != 0 && self.misaligned_faults(va, 16, acc) {
            return Err(Self::alignment_abort(va, true));
        }
        if (va & 0xFFF) + 16 > 0x1000 {
            self.translate_write_pages(bus, va, 16, acc)?;
        }
        let halves = ElementPair {
            first: value as u64,
            second: (value >> 64) as u64,
        };
        self.write_pair(bus, va, 3, halves, Self::halves_of(va, acc))
    }

    /// Read the 32 bytes of an LDP or LDNP of two Q registers at `va` (Arm
    /// `Mem{256}`, `AccessType_ASIMD` with `ispair`). `SCTLR_ELx.A` judges
    /// the access by its 16-byte elements. At an address 8-byte aligned the
    /// access is four 8-byte single-copy atomic accesses, each passed
    /// `aligned = TRUE`, so Device memory refuses none of them; at any other
    /// each Q register is a misaligned access, as [`Cpu::read_mem_u128`]
    /// makes it.
    pub(crate) fn read_mem_q_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        acc: DataAccess,
    ) -> Result<QuadwordPair, Fault> {
        if va & 7 != 0 {
            return Ok(QuadwordPair {
                first: self.read_mem_u128(bus, va, acc)?,
                second: self.read_mem_u128(bus, va.wrapping_add(16), acc)?,
            });
        }
        if va & 15 != 0 && self.misaligned_faults(va, 32, acc) {
            return Err(Self::alignment_abort(va, false));
        }
        let low = self.read_pair(bus, va, 3, acc)?;
        let high = self.read_pair(bus, va.wrapping_add(16), 3, acc)?;
        Ok(QuadwordPair {
            first: u128::from(low.first) | (u128::from(low.second) << 64),
            second: u128::from(high.first) | (u128::from(high.second) << 64),
        })
    }

    /// Write the 32 bytes of an STP or STNP of two Q registers at `va`, as
    /// [`Cpu::read_mem_q_pair`] reads them: at an address 8-byte aligned,
    /// four 8-byte accesses in ascending order, each written before the
    /// next is translated, as Arm's `Mem` setter makes them.
    pub(crate) fn write_mem_q_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        value: QuadwordPair,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        if va & 7 != 0 {
            self.write_mem_u128(bus, va, value.first, acc)?;
            return self.write_mem_u128(bus, va.wrapping_add(16), value.second, acc);
        }
        if va & 15 != 0 && self.misaligned_faults(va, 32, acc) {
            return Err(Self::alignment_abort(va, true));
        }
        let halves = |q: u128| ElementPair {
            first: q as u64,
            second: (q >> 64) as u64,
        };
        self.write_pair(bus, va, 3, halves(value.first), acc)?;
        self.write_pair(bus, va.wrapping_add(16), 3, halves(value.second), acc)
    }

    /// `DC IVAC`, `DC CVAC`, `DC CVAU`, `DC CVAP`, `DC CVADP`, `DC CIVAC`
    /// and `IC IVAU` at `va`: the translation Arm `AArch64_DC` and
    /// `AArch64_IC` make, as an aligned access of no size, checked as `op`
    /// says ([`maintenance_needs`]) at stage 1 and, under stage 2, at stage 2
    /// too. With no cache to clean or invalidate, the translation is all
    /// they do. A fault is a Data Abort at `va` with `CM` and `WnR` set
    /// (`AArch64_FaultSyndrome`); Device memory refuses none of them.
    pub(crate) fn cache_maintenance<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        op: CacheMaintenance,
    ) -> Result<(), Fault> {
        let regime = self.regime();
        let el = self.pstate.el;
        let needs = Needs::Maintenance(op);
        // Ahead of the TLB as well as the walk: an operation that needs no
        // permission would pass a hit E0PD refuses.
        if self.s1_enabled(regime) && self.e0pd_refuses(va, el) {
            let refused = Abort::first(Fsc::Translation(0), true);
            return Err(Self::maintenance_abort(va, refused));
        }
        self.translate_needs(bus, va, needs, el, OnDevice::Permitted)
            .map(|_| ())
            .map_err(|abort| Self::maintenance_abort(va, abort))
    }

    /// `DC ZVA`: zero the naturally aligned block containing `va`. Device
    /// memory refuses it whatever its alignment, at either stage
    /// (`AArch64_S1HasAlignmentFaultDueToMemType` and
    /// `AArch64_S2HasAlignmentFaultDueToMemType`, `AccessType_DCZero`).
    pub(crate) fn dc_zva<B: PhysBus>(&mut self, bus: &mut B, va: u64) -> Result<(), Fault> {
        let base = va & !(id::DC_ZVA_BYTES - 1);
        let el = self.pstate.el;
        let t = self
            .translate(bus, base, AccessKind::Write, el, OnDevice::Refused)
            .map_err(|abort| Self::data_abort(va, abort, SyndromeSlot::Closed))?;
        self.note_store(t.pa, id::DC_ZVA_BYTES);
        let clock = self.clock();
        for i in 0..id::DC_ZVA_BYTES / 8 {
            if bus
                .write(PhysAddr::new(t.pa + i * 8), AccessWidth::Dword, 0, clock)
                .is_err()
            {
                return Err(self.external_abort(va, t.pa + i * 8, AccessKind::Write));
            }
        }
        Ok(())
    }

    /// What a store of `bytes` at `pa`, within one page, does besides
    /// writing: it clears the local exclusive monitor if it overlaps the
    /// monitored granule (the IMPLEMENTATION DEFINED choice of Arm ARM
    /// B2.9.5 that every store by this processor clears it, which is what
    /// makes an interrupted `LDXR`/`STXR` sequence retry), and it forgets
    /// every trace holding an instruction it may overwrite. Every store the
    /// processor makes comes through here (R5: the one choke point for
    /// self-modifying code), before the bus sees it.
    #[inline]
    fn note_store(&mut self, pa: u64, bytes: u64) {
        let m = self.monitor;
        if m.armed && pa < m.addr + m.bytes && m.addr < pa + bytes {
            self.monitor.armed = false;
        }
        if self.traces.holds_code(pa) {
            if let Some(span) = PhysSpan::new(PhysAddr::new(pa), bytes) {
                self.forget_code(span);
            }
        }
    }

    /// The alignment check that comes before the translation of an access of
    /// `bytes` at `va` (Arm `AArch64_UnalignedAccessFaults`), on its own: a
    /// store-exclusive makes it before it compares its address with the
    /// monitor's (`AArch64.ExclusiveMonitorsPass`).
    pub(crate) fn check_alignment(
        &self,
        va: u64,
        bytes: u64,
        acc: DataAccess,
        write: bool,
    ) -> Result<(), Fault> {
        if va & (bytes - 1) != 0 && self.misaligned_faults(va, bytes, acc) {
            return Err(Self::alignment_abort(va, write));
        }
        Ok(())
    }

    /// The alignment check SVE `LDR` and `STR` make before their byte
    /// accesses: `if !aligned && AlignmentEnforced() then AArch64_Abort(
    /// AlignmentFault(accdesc, addr))`, `aligned` being the base register's
    /// alignment to the transfer's natural size, and the fault reported at
    /// `va`, the first byte's address.
    pub(crate) fn check_transfer_alignment(
        &self,
        aligned: bool,
        va: u64,
        write: bool,
    ) -> Result<(), Fault> {
        if !aligned && self.strict_alignment() {
            return Err(Self::alignment_abort(va, write));
        }
        Ok(())
    }

    /// The SP alignment check (Arm `CheckSPAlignment`: `SCTLR_EL1.SA0` at
    /// EL0, `SCTLR_ELx.SA` of the current level otherwise) for an access
    /// whose base register is SP.
    #[inline]
    pub(crate) fn check_sp_alignment(&self, base_is_sp: bool, base: u64) -> Result<(), Fault> {
        if !base_is_sp || base & 0xF == 0 {
            return Ok(());
        }
        let enabled = match self.pstate.el {
            El::El2 => self.el2().sctlr & sctlr::SA != 0,
            El::El1 => self.sys.sctlr_el1 & sctlr::SA != 0,
            El::El0 => self.sys.sctlr_el1 & sctlr::SA0 != 0,
        };
        if enabled {
            Err(Exception::new(SyndromeClass::SpAlignment, 0).into())
        } else {
            Ok(())
        }
    }
}

/// The layout [`Tlb::save`] writes, for the tests that read a saved TLB.
#[cfg(test)]
pub(crate) mod saved_layout {
    /// The places of each table of contexts.
    pub(crate) const CONTEXTS: usize = super::CONTEXTS;
    /// A held EL1&0 context: its eight registers in `El10Context::save`'s
    /// order, `SCTLR_EL1`, `TCR_EL1`, `TTBR0_EL1`, `TTBR1_EL1`, `MAIR_EL1`,
    /// `HCR_EL2`'s translation bits, `VTTBR_EL2` and `VTCR_EL2`.
    pub(crate) const CONTEXT_BYTES: usize = 8 * 8;
    /// A held stage 2 context: `VTTBR_EL2` and `VTCR_EL2`.
    pub(crate) const STAGE2_CONTEXT_BYTES: usize = 2 * 8;
    pub(crate) const CONTEXT_SCTLR: usize = 0;
    pub(crate) const CONTEXT_HCR: usize = 5 * 8;
    pub(crate) const CONTEXT_VTTBR: usize = 6 * 8;
    pub(crate) const CONTEXT_VTCR: usize = 7 * 8;
    /// An entry: its page with its tag, its output page, then its eight
    /// bytes — stage 1's permissions, Device memory and level, the regime,
    /// whether stage 1's walk made it, stage 2's permissions, Device memory
    /// and level.
    pub(crate) const ENTRY_BYTES: usize = 8 + 8 + 8;
    /// The byte of an entry that says whether stage 1's walk made it.
    pub(crate) const ENTRY_STAGE1: usize = 8 + 8 + 4;
    /// The bytes of an entry that hold stage 2's permissions, Device memory
    /// and level.
    pub(crate) const ENTRY_S2_PERMS: usize = 8 + 8 + 5;
    pub(crate) const ENTRY_S2_DEVICE: usize = 8 + 8 + 6;
    pub(crate) const ENTRY_S2_LEVEL: usize = 8 + 8 + 7;
    /// A kept stage 2 translation: its IPA page with its tag, its output
    /// page, then its three bytes.
    pub(crate) const KEPT_BYTES: usize = 8 + 8 + 3;
}
