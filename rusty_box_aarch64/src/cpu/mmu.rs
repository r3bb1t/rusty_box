//! Stage-1 translation for the EL1&0 regime (VMSAv8-64, Arm ARM D8) with the
//! 4 KiB granule, the one `ID_AA64MMFR0_EL1` advertises; the TLB that caches
//! it; and the data and instruction access paths every load, store, fetch
//! and cache maintenance by address goes through — alignment checks,
//! translation, permission checks, and the external abort when the bus has
//! nothing at an address.
//!
//! The TLB is direct-mapped and holds one 4 KiB page per entry, whatever the
//! size of the block that mapped it. It is flushed whole by every TLB
//! maintenance instruction and by any write that changes a register a walk
//! reads (`SCTLR_EL1`, `TCR_EL1`, `TTBR0/1_EL1`, `MAIR_EL1`) — invalidation is
//! never narrower than the architecture requires, so ASIDs need no tagging.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::dp::ones;
use super::exception::{Exception, ExceptionClass};
use super::exec::{sext, Fault, StopCause};
use super::debug::WatchedAccess;
use super::features::{Feature, Features};
use super::{id, sctlr, Cpu, El};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus, PhysSpan};

/// What an access does, for permission checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessKind {
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

/// Whose permissions an `AT` instruction checks a translation against.
///
/// Closed (R5): the address translation instructions of the EL1&0 regime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtCheck {
    /// `AT S1E0R`, `AT S1E0W`: EL0's.
    El0,
    /// `AT S1E1R`, `AT S1E1W`: EL1's, which PSTATE.PAN takes no part in
    /// (Arm `CreateAccDescAT`, `ATAccess_Read`/`ATAccess_Write`).
    El1,
    /// `AT S1E1RP`, `AT S1E1WP` (FEAT_PAN2): EL1's, with PSTATE.PAN as a
    /// data access has it (`ATAccess_ReadPAN`/`ATAccess_WritePAN`).
    El1Pan,
}

/// The size of a data access at its address, as the checks made on it read
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AccessSpan {
    /// What the alignment checks judge: one element of a pair.
    bytes: u64,
    /// What the watchpoints watch: both elements of a pair.
    watched: u64,
}

impl AccessSpan {
    /// One access of `bytes`.
    #[inline(always)]
    const fn single(bytes: u64) -> Self {
        Self {
            bytes,
            watched: bytes,
        }
    }

    /// A pair of elements of `bytes` on one page.
    #[inline(always)]
    const fn pair(bytes: u64) -> Self {
        Self {
            bytes,
            watched: 2 * bytes,
        }
    }
}

/// The constraints a data access carries beyond its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DataAccess {
    /// `LDTR`/`STTR`: checked against EL0 permissions when executed at EL1
    /// with PSTATE.UAO clear (Arm `AArch64_IsUnprivAccessPriv`).
    pub(crate) unprivileged: bool,
    pub(crate) alignment: AlignmentRule,
}

impl DataAccess {
    pub(crate) const NORMAL: Self = Self {
        unprivileged: false,
        alignment: AlignmentRule::Any,
    };
    /// A load-acquire or store-release.
    pub(crate) const ORDERED: Self = Self {
        unprivileged: false,
        alignment: AlignmentRule::Ordered,
    };
    /// An exclusive or atomic access.
    pub(crate) const ATOMIC: Self = Self {
        unprivileged: false,
        alignment: AlignmentRule::Atomic,
    };
    pub(crate) const UNPRIVILEGED: Self = Self {
        unprivileged: true,
        alignment: AlignmentRule::Any,
    };
}

/// Which misaligned data accesses fault before translation, the part of Arm
/// `AArch64_UnalignedAccessFaults` an access's kind decides.
/// `SCTLR_EL1.A` faults every misaligned access whatever the rule, and
/// Device memory every one, after the walk and before the permissions
/// ([`OnDevice`]). On Normal memory that is not Write-Back Shareable a
/// misaligned access these rules let through is performed, the outcome
/// Arm lists for `Unpredictable_LSE2_ALIGNMENT_FAULT`.
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
    /// is 0. Without it, every one (Arm ARM B2.8.2.1.3 and C3.2.11; the
    /// pseudocode's rule reads only `nAA`, which FEAT_LSE2 adds).
    Ordered,
}

/// What a cache maintenance instruction by virtual address does, which
/// decides the permission its translation needs (Arm
/// `AArch64_S1CheckPermissions`, `AccessType_DC` and `AccessType_IC`).
/// PSTATE.PAN takes no part (`CreateAccDescDC` and `CreateAccDescIC` leave
/// `pan` clear). Whatever it does, the fault of a walk is taken, an Access
/// flag fault included, the IMPLEMENTATION DEFINED choice of rule RTRTWX
/// ("Generate access flag fault on IC/DC operations" being TRUE here), and
/// by `IC IVAU` a Translation fault, that of Arm ARM D7.5.9.1.
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
/// before the permission check: an access whose `aligned` is FALSE, and
/// `DC ZVA`, are refused, no Device location supporting unaligned accesses
/// here. `aligned` is the whole instruction's, so a part of a misaligned
/// transfer is refused however it is itself aligned: an 8-byte half of a
/// 16-byte SIMD&FP access, or a byte of an SVE `LDR` or `STR` whose base is
/// misaligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnDevice {
    Refused,
    Permitted,
}

impl OnDevice {
    /// As Arm's `aligned` of an access of `bytes` (a power of two) at `va`
    /// says.
    #[inline(always)]
    const fn of_alignment(va: u64, bytes: u64) -> Self {
        if va & (bytes - 1) == 0 {
            Self::Permitted
        } else {
            Self::Refused
        }
    }
}

/// A fault status code (`ESR_ELx.ISS.DFSC`/`IFSC`) with its lookup level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fsc {
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

/// Why a virtual address has no translation.
///
/// Closed (R5): the faults a stage-1 walk can end in before permissions are
/// considered. `level` is the lookup level the walk had reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranslationFault {
    /// The address, or a table or output address, is wider than the
    /// configured address size.
    AddressSize { level: u8 },
    /// No valid descriptor maps it.
    Translation { level: u8 },
    /// The descriptor that maps it has its access flag clear, which this
    /// core, with no FEAT_HAFDBS, faults on rather than sets.
    AccessFlag { level: u8 },
    /// A descriptor read reached nothing on the bus.
    ExternalAbortOnWalk { level: u8 },
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
        }
    }
}

const PERM_EL1_R: u8 = 1 << 0;
const PERM_EL1_W: u8 = 1 << 1;
const PERM_EL1_X: u8 = 1 << 2;
const PERM_EL0_R: u8 = 1 << 3;
const PERM_EL0_W: u8 = 1 << 4;
const PERM_EL0_X: u8 = 1 << 5;
const PERMS: u8 = PERM_EL1_R | PERM_EL1_W | PERM_EL1_X | PERM_EL0_R | PERM_EL0_W | PERM_EL0_X;
/// Not a permission: the descriptor's GP bit (FEAT_BTI), a guarded page.
const GUARDED: u8 = 1 << 6;
/// Not a permission: a TLB entry's tag holds a top byte instruction fetches
/// do not ignore ([`fetch_keeps_tag`]), so a fetch from the address, whose
/// own walk would fault, may not use it.
const TAG_KEPT_BY_FETCH: u8 = 1 << 7;
/// EL0's permissions.
const PERMS_EL0: u8 = PERM_EL0_R | PERM_EL0_W | PERM_EL0_X;
/// What a data access needs of an entry, which a TLB entry filled while a
/// watchpoint is armed does not hold ([`Cpu::walk_and_cache`]).
const PERMS_DATA: u8 = PERM_EL1_R | PERM_EL1_W | PERM_EL0_R | PERM_EL0_W;

/// `ESR_ELx.ISS.WnR` and `CM` of a Data Abort.
const ISS_WNR: u32 = 1 << 6;
const ISS_CM: u32 = 1 << 8;

/// `TCR_EL1.HPD0` (FEAT_HPDS); `HPD1` is the bit above it.
const TCR_HPD0: u32 = 41;
/// `TCR_EL1.E0PD0` (FEAT_E0PD); `E0PD1` is the bit above it.
const TCR_E0PD0: u32 = 55;

/// `PAR_EL1.SH`: Outer Shareable, which `PAREncodeShareability` reports for
/// Device and Normal Non-cacheable memory, and `DecodeShareability` makes of
/// a descriptor's reserved 0b01 (Arm's listed outcome of
/// `Unpredictable_Shareability`).
const OUTER_SHAREABLE: u8 = 0b10;
/// A descriptor's SH field: Non-shareable.
const NON_SHAREABLE: u8 = 0b00;
/// `PAR_EL1.ATTR` of Normal Inner and Outer Non-cacheable memory, and that
/// of such memory whose XS attribute is 0 on a processor with FEAT_XS
/// (`EncodePARAttrs`).
const ATTR_NON_CACHEABLE: u8 = 0x44;
const ATTR_NON_CACHEABLE_XS0: u8 = 0x40;

#[inline]
const fn permitted(perms: u8, kind: AccessKind, el: El) -> bool {
    let needed = match (kind, el) {
        (AccessKind::Read, El::El1) => PERM_EL1_R,
        (AccessKind::Write, El::El1) => PERM_EL1_W,
        (AccessKind::Fetch, El::El1) => PERM_EL1_X,
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

/// Arm `EffectiveTBI` for the EL1&0 regime under `tcr`: whether
/// top-byte-ignore applies to the instruction or data addresses of the half
/// of the address space `upper` names, TBI1 or TBI0, and with FEAT_PAuth
/// not for an instruction address TBID1 or TBID0 takes out of it.
#[inline]
const fn tbi_under(features: Features, tcr: u64, upper: bool, instruction: bool) -> bool {
    let tbi = (tcr >> if upper { 38 } else { 37 }) & 1 == 1;
    let tbid = (tcr >> if upper { 52 } else { 51 }) & 1 == 1;
    tbi && !(instruction && tbid && features.has(Feature::PAuth))
}

/// Whether `va` holds a tag that instruction fetches do not ignore under
/// `tcr` (`TCR_EL1.TBIDx`), where data accesses may: a fetch from it faults.
const fn fetch_keeps_tag(features: Features, tcr: u64, va: u64) -> bool {
    sext(va, 56) != va && !tbi_under(features, tcr, (va >> 55) & 1 == 1, true)
}

/// TCR_EL1.T1SZ or T0SZ, of the half of the address space `upper` names.
#[inline]
const fn txsz_under(tcr: u64, upper: bool) -> u32 {
    (if upper {
        (tcr >> 16) & 0x3F
    } else {
        tcr & 0x3F
    }) as u32
}

/// Whether a walk under `tcr` of the data or instruction address `va` reads
/// a descriptor at all: Arm `AArch64_S1Translate`'s and `AArch64_S1Walk`'s
/// first checks, all of them a level 0 Translation fault.
///
/// T0SZ/T1SZ outside 16..=39 (`AArch64_S1MinTxSZ` and `AArch64_MaxTxSZ` for
/// the 4 KiB granule without FEAT_LPA2's `TCR_EL1.DS` or FEAT_TTST): whether
/// such a value is treated as the nearest one in range or takes the fault
/// is IMPLEMENTATION DEFINED (`AArch64_S1TxSZFaults`, the ImpDef booleans
/// "Fault on TxSZ value above maximum" and "... below minimum"). This takes
/// the fault, as QEMU does (target/arm/ptw.c `get_phys_addr_lpae`,
/// `param.tsz_oob`).
fn input_address_valid(features: Features, tcr: u64, va: u64, instruction: bool) -> bool {
    let upper = (va >> 55) & 1 == 1;
    let txsz = txsz_under(tcr, upper);
    if !(16..=39).contains(&txsz) {
        return false;
    }
    let ia_bits = 64 - txsz;
    let top = if tbi_under(features, tcr, upper, instruction) {
        56
    } else {
        64
    };
    let check = ones(top - ia_bits) << ia_bits;
    let epd = (tcr >> if upper { 23 } else { 7 }) & 1 == 1;
    va & check == (if upper { check } else { 0 }) && !epd
}

/// The physical address size each `ID_AA64MMFR0_EL1.PARange` (and
/// `TCR_EL1.IPS`) encoding names (Arm `AArch64_PhysicalAddressSize`).
const PA_RANGE_BITS: [u32; 7] = [32, 36, 40, 42, 44, 48, 52];

/// `ID_AA64MMFR0_EL1.PARange`, which no feature changes.
const PA_RANGE: usize = (id::id_aa64mmfr0(Features::of(&[])) & 0xF) as usize;
const _: () = assert!(PA_RANGE_BITS[PA_RANGE] == id::PA_BITS);

/// The output address size of a stage 1 walk under `tcr`: `TCR_EL1.IPS`,
/// bounded by the implemented PARange (Arm `AArch64_S1OAOutOfRange`, with
/// `AArch64_PhysicalAddressSize`).
fn output_address_bits(tcr: u64) -> u32 {
    let ips = ((tcr >> 32) & 7) as usize;
    PA_RANGE_BITS[ips.min(PA_RANGE)]
}

/// Whether `MAIR_EL1` attribute byte `attr` is Inner and Outer Write-Back
/// (Arm `DecodeLDFAttr` of each nibble).
const fn attr_is_write_back(attr: u8) -> bool {
    const fn write_back(nibble: u8) -> bool {
        nibble & 0b0100 != 0 && nibble != 0b0100
    }
    write_back(attr >> 4) && write_back(attr & 0xF)
}

/// What the memory a page maps is, as far as an access to it cares: Arm's
/// `MemoryAttributes`, reduced to `memtype == MemType_Device` and
/// `IsWBShareable`.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Memory {
    /// Normal memory other than [`Memory::WriteBackShareable`].
    Normal,
    Device,
    /// Normal Inner and Outer Write-Back Cacheable Shareable memory, on
    /// which an access can be single-copy atomic above the byte.
    WriteBackShareable,
}

impl Memory {
    /// The memory `MAIR_EL1` attribute byte `attr` and the descriptor's SH
    /// field `shareability` describe (Arm `S1DecodeMemAttrs`), with
    /// `SCTLR_EL1.C` as `cached` says: clear, Normal memory is Non-cacheable
    /// (`AArch64_S1TreatAsNormalNC`). A reserved attribute byte, which Arm
    /// maps to an UNKNOWN allocated value (`Unpredictable_RESMAIR`), is read
    /// by its fields like any other, as QEMU reads it: its top nibble zero
    /// is Device memory, and anything else Normal.
    const fn decode(attr: u8, shareability: u8, cached: bool) -> Self {
        if attr & 0xF0 == 0 {
            Self::Device
        } else if cached && attr_is_write_back(attr) && shareability != NON_SHAREABLE {
            Self::WriteBackShareable
        } else {
            Self::Normal
        }
    }

    /// The value a snapshot holds for it.
    const fn code(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Device => 1,
            Self::WriteBackShareable => 2,
        }
    }

    /// The memory a snapshot's `code` names.
    const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Normal),
            1 => Some(Self::Device),
            2 => Some(Self::WriteBackShareable),
            _ => None,
        }
    }
}

/// A translation: where an address goes, the page's flags, what memory it
/// is and the lookup level of the descriptor that mapped it. With stage 1
/// disabled no page is guarded (`AArch64_S1DisabledOutput`), no access is
/// refused its permissions, and data accesses are to Device memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mapping {
    pa: PhysAddr,
    perms: u8,
    memory: Memory,
    level: u8,
}

/// The two adjacent elements of a pair access: `first` at the lower
/// address, `second` just above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ElementPair {
    pub(crate) first: u64,
    pub(crate) second: u64,
}

impl From<u128> for ElementPair {
    /// The two doublewords of a quadword, the low one first.
    #[inline]
    fn from(value: u128) -> Self {
        Self {
            first: value as u64,
            second: (value >> 64) as u64,
        }
    }
}

impl From<ElementPair> for u128 {
    /// The quadword two doublewords make, `first` the low one.
    #[inline]
    fn from(pair: ElementPair) -> Self {
        u128::from(pair.first) | (u128::from(pair.second) << 64)
    }
}

/// The two Q registers of a SIMD&FP pair access: `first` at the lower
/// address, `second` 16 bytes above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QuadwordPair {
    pub(crate) first: u128,
    pub(crate) second: u128,
}

/// A completed walk, before permissions are checked against an access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Walked {
    mapping: Mapping,
    /// The `MAIR_EL1` attribute byte.
    attr: u8,
    /// The descriptor's SH[1:0] field (bits [9:8]) as Arm decodes it.
    shareability: u8,
}

/// The two pages of a store that crosses a page boundary, both translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WritePages {
    /// Where the store's first byte goes.
    first: PhysAddr,
    /// Where its first byte on the second page goes.
    second: PhysAddr,
    /// How many of its bytes lie on the first page.
    first_bytes: u64,
}

impl WritePages {
    /// Where byte `offset` of the store goes.
    #[inline]
    const fn byte(self, offset: u64) -> PhysAddr {
        if offset < self.first_bytes {
            PhysAddr::new(self.first.get() + offset)
        } else {
            PhysAddr::new(self.second.get() + offset - self.first_bytes)
        }
    }

    /// Whether the `bytes` bytes from `offset` lie on one page.
    #[inline]
    const fn holds(self, offset: u64, bytes: u64) -> bool {
        offset + bytes <= self.first_bytes || offset >= self.first_bytes
    }
}

const TLB_ENTRIES: usize = 1024;
const INVALID_TAG: u64 = u64::MAX;
/// How many fills [`Tlb`] remembers since a flush, which then empties only
/// those entries.
const FILL_LOG: usize = 256;

#[derive(Debug, Clone, Copy)]
struct TlbEntry {
    /// `va >> 12`, or `INVALID_TAG`.
    tag: u64,
    /// The mapping of the page, `pa` its first byte.
    page: Mapping,
}

impl TlbEntry {
    /// Whether this entry, the one in `va`'s slot ([`Tlb::entry_for`]),
    /// translates `va`'s page.
    #[inline(always)]
    const fn translates(&self, va: u64) -> bool {
        self.tag == va >> 12
    }

    /// The translation of `va`, which this entry translates.
    #[inline(always)]
    const fn mapping(&self, va: u64) -> Mapping {
        Mapping {
            pa: PhysAddr::new(self.page.pa.get() | (va & 0xFFF)),
            ..self.page
        }
    }
}

const EMPTY_ENTRY: TlbEntry = TlbEntry {
    tag: INVALID_TAG,
    page: Mapping {
        pa: PhysAddr::new(0),
        perms: 0,
        memory: Memory::Normal,
        level: 0,
    },
};

/// What a snapshot's TLB entries are judged against: the implementation and
/// the registers of the processor the snapshot restores.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TlbContext {
    pub(crate) features: Features,
    pub(crate) tcr: u64,
    pub(crate) sctlr: u64,
    /// [`Cpu::watch_armed`].
    pub(crate) watch_armed: bool,
}

impl TlbContext {
    /// Whether a walk under this context can have filled an entry for `va`
    /// with `page` ([`Cpu::walk`], [`Cpu::walk_and_cache`]).
    fn walk_can_produce(&self, va: u64, page: Mapping) -> bool {
        let perms = page.perms;
        let walked = PERMS
            | TAG_KEPT_BY_FETCH
            | if self.features.has(Feature::Bti) {
                GUARDED
            } else {
                0
            };
        let wxn = self.sctlr & sctlr::WXN != 0;
        let has = |perm: u8| perms & perm != 0;
        let tag_flagged = has(TAG_KEPT_BY_FETCH);
        let data_perms_as_filled = if self.watch_armed {
            perms & PERMS_DATA == 0
        } else {
            has(PERM_EL1_R)
        };
        perms & !walked == 0
            && data_perms_as_filled
            && (1..=3).contains(&page.level)
            && page.pa.get() >> output_address_bits(self.tcr) == 0
            && input_address_valid(self.features, self.tcr, va, false)
            && tag_flagged == fetch_keeps_tag(self.features, self.tcr, va)
            && !(has(PERM_EL0_W) && !(has(PERM_EL0_R) && has(PERM_EL1_W)))
            && !(has(PERM_EL1_X) && has(PERM_EL0_W))
            && !(wxn && has(PERM_EL1_X) && has(PERM_EL1_W))
            && !(wxn && has(PERM_EL0_X) && has(PERM_EL0_W))
    }
}

/// The translation cache: direct-mapped, 4 KiB pages.
pub struct Tlb {
    entries: [TlbEntry; TLB_ENTRIES],
    /// The slots filled since the last flush, as far as [`FILL_LOG`] of
    /// them.
    filled: [u16; FILL_LOG],
    /// How many fills since the last flush, stopping past [`FILL_LOG`].
    fills: usize,
}

impl Tlb {
    pub(crate) const fn new() -> Self {
        Self {
            entries: [EMPTY_ENTRY; TLB_ENTRIES],
            filled: [0; FILL_LOG],
            fills: 0,
        }
    }

    /// Empty the TLB. Only an entry's tag says whether it is valid.
    pub(crate) fn flush(&mut self) {
        if self.fills <= FILL_LOG {
            for &slot in &self.filled[..self.fills] {
                self.entries[usize::from(slot)].tag = INVALID_TAG;
            }
        } else {
            for entry in self.entries.iter_mut() {
                entry.tag = INVALID_TAG;
            }
        }
        self.fills = 0;
    }

    #[inline]
    fn slot(va: u64) -> usize {
        ((va >> 12) as usize) & (TLB_ENTRIES - 1)
    }

    /// The entry in `va`'s slot, the one that translates `va` if any does
    /// ([`TlbEntry::translates`]): the lookup every translation from the
    /// TLB makes.
    #[inline(always)]
    fn entry_for(&self, va: u64) -> &TlbEntry {
        &self.entries[Self::slot(va)]
    }

    /// The entry that translates `va`, if the TLB holds one.
    #[inline(always)]
    fn hit(&self, va: u64) -> Option<&TlbEntry> {
        let entry = self.entry_for(va);
        entry.translates(va).then_some(entry)
    }

    /// Hold `page`, its `pa` the page's first byte, as the translation of
    /// `va`'s page.
    fn fill(&mut self, va: u64, page: Mapping) {
        let slot = Self::slot(va);
        if self.fills < FILL_LOG {
            self.filled[self.fills] = slot as u16;
        }
        self.fills = (self.fills + 1).min(FILL_LOG + 1);
        self.entries[slot] = TlbEntry {
            tag: va >> 12,
            page,
        };
    }

    /// The valid entries, for a snapshot.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        let valid = self.entries.iter().filter(|e| e.tag != INVALID_TAG).count();
        w.write_u32(valid as u32)?;
        for e in self.entries.iter().filter(|e| e.tag != INVALID_TAG) {
            w.write_u64(e.tag)?;
            w.write_u64(e.page.pa.get())?;
            w.write_bytes(&[e.page.perms, e.page.memory.code(), e.page.level])?;
        }
        Ok(())
    }

    /// The inverse of [`Tlb::save`], for a processor in `context`: an entry
    /// must be one its walk can produce and its TLB can hold
    /// ([`Cpu::walk_and_cache`]).
    pub(crate) fn restore<R: SnapRead>(
        &mut self,
        r: &mut R,
        context: TlbContext,
    ) -> SnapResult {
        let valid = r.read_count(TLB_ENTRIES)?;
        self.flush();
        self.fills = FILL_LOG + 1;
        for _ in 0..valid {
            let tag = r.read_u64()?;
            let pa_page = r.read_u64()?;
            let mut rest = [0u8; 3];
            r.read_bytes(&mut rest)?;
            let [perms, memory, level] = rest;
            let (Some(memory), true) = (
                Memory::from_code(memory),
                tag >> 52 == 0 && pa_page & 0xFFF == 0,
            ) else {
                return Err(SnapError::Invalid(
                    "TLB entry is not one a walk can produce",
                ));
            };
            let va = tag << 12;
            let page = Mapping {
                pa: PhysAddr::new(pa_page),
                perms,
                memory,
                level,
            };
            if !context.walk_can_produce(va, page) {
                return Err(SnapError::Invalid(
                    "TLB entry is not one a walk can produce",
                ));
            }
            if perms & PERMS_EL0 != 0 && e0pd_set(context.features, context.tcr, va) {
                return Err(SnapError::Invalid(
                    "TLB entry gives EL0 a page FEAT_E0PD refuses it",
                ));
            }
            let slot = &mut self.entries[Self::slot(va)];
            if slot.tag != INVALID_TAG {
                return Err(SnapError::Invalid("two TLB entries claim one slot"));
            }
            *slot = TlbEntry { tag, page };
        }
        Ok(())
    }
}

impl Cpu {
    #[inline]
    fn mmu_enabled(&self) -> bool {
        self.sys.sctlr_el1 & sctlr::M != 0
    }

    /// Empty the TLB. The running trace ends, whatever the caller: its
    /// fetches translate again. (The guest's TLBI and translation-register
    /// MSRs end their trace by `Op::ends_trace` as well;
    /// this does not rely on it.)
    pub(crate) fn flush_tlb(&mut self) {
        self.tlb.flush();
        self.trace_break = true;
    }

    /// `SCTLR_EL1.A`: every misaligned data access faults.
    #[inline]
    fn strict_alignment(&self) -> bool {
        self.sys.sctlr_el1 & sctlr::A != 0
    }

    /// Arm `AArch64_UnalignedAccessFaults` for a misaligned access of
    /// `bytes` at `va`: whether it faults before translation.
    fn misaligned_faults(&self, va: u64, bytes: u64, acc: DataAccess) -> bool {
        if self.strict_alignment() {
            return true;
        }
        let lse2 = self.implements(Feature::Lse2);
        let in_granule = (va & 15) + bytes <= 16;
        match acc.alignment {
            AlignmentRule::Any => false,
            AlignmentRule::Atomic => !(lse2 && in_granule),
            AlignmentRule::Ordered => {
                !lse2 || (self.sys.sctlr_el1 & sctlr::NAA == 0 && !in_granule)
            }
        }
    }

    /// The TLB entry that translates `va` when stage 1 is enabled, if the
    /// TLB holds one and it permits an access of `kind` checked against
    /// `el`'s permissions: the one case that raises no fault of the walk or
    /// of the page's permissions. FEAT_E0PD refuses none of these, as the
    /// TLB holds no page E0PD refuses EL0 that EL0 has a permission for
    /// ([`Cpu::walk_and_cache`]). Everything else walks
    /// ([`Cpu::translate_miss`]), which a TLB may always do, and the walk
    /// raises the fault.
    #[inline(always)]
    fn permitted_hit(&self, va: u64, kind: AccessKind, el: El) -> Option<&TlbEntry> {
        if !self.mmu_enabled() {
            return None;
        }
        let entry = self.tlb.entry_for(va);
        if !entry.translates(va) || !self.permits(entry.page.perms, kind, el) {
            return None;
        }
        Some(entry)
    }

    /// Translate `va` for an access of `kind` checked against `el`'s
    /// permissions, after Device memory's alignment check
    /// (`AArch64_S1Translate`'s order: the walk's faults, then the memory
    /// type's, then the permissions').
    #[inline(always)]
    fn translate<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        kind: AccessKind,
        el: El,
        on_device: OnDevice,
    ) -> Result<Mapping, Fsc> {
        if !self.mmu_enabled() {
            // Data accesses are Device-nGnRnE with stage 1 disabled, and
            // instruction fetches are Normal.
            let address = self
                .s1_disabled_output(va, kind)
                .ok_or(Fsc::AddressSize(0))?;
            let memory = if kind == AccessKind::Fetch {
                Memory::Normal
            } else {
                Memory::Device
            };
            if on_device == OnDevice::Refused && memory == Memory::Device {
                return Err(Fsc::Alignment);
            }
            return Ok(Mapping {
                pa: address,
                perms: 0,
                memory,
                level: 0,
            });
        }
        if let Some(entry) = self.permitted_hit(va, kind, el) {
            if on_device == OnDevice::Permitted || entry.page.memory != Memory::Device {
                return Ok(entry.mapping(va));
            }
        }
        self.translate_miss(bus, va, kind, el, on_device)
    }

    /// A translation the TLB did not answer: walk, cache what the walk
    /// found, check the memory type and the permissions.
    #[inline(never)]
    fn translate_miss<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        kind: AccessKind,
        el: El,
        on_device: OnDevice,
    ) -> Result<Mapping, Fsc> {
        let mapping = self.walk_and_cache(bus, va, kind, el)?.mapping;
        if on_device == OnDevice::Refused && mapping.memory == Memory::Device {
            return Err(Fsc::Alignment);
        }
        if !self.permits(mapping.perms, kind, el) {
            return Err(Fsc::Permission(mapping.level));
        }
        Ok(mapping)
    }

    /// The walk for an access of `kind` at `va` checked against `el`'s
    /// permissions ([`Cpu::s1_walk`]), cached in the TLB.
    fn walk_and_cache<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        kind: AccessKind,
        el: El,
    ) -> Result<Walked, TranslationFault> {
        let walked = self.s1_walk(bus, va, kind, el)?;
        // A tagged address keeps its tag in the entry's tag, so it hits only
        // itself; a TCR_EL1 write, which can change what the walk ignores,
        // empties the TLB. A page FEAT_E0PD refuses EL0 that EL0 has a
        // permission for leaves no entry: its accesses walk each time, so
        // an EL0 access never passes a hit's permission check in a half
        // E0PD refuses EL0, where the pages are normally all EL1's and
        // cached. While a watchpoint is armed an entry holds no data
        // permission, so that a data access never hits, and reaches
        // [`Cpu::check_watchpoints`]; a fetch, which no watchpoint watches,
        // still does.
        let mapping = walked.mapping;
        if mapping.perms & PERMS_EL0 == 0 || !self.e0pd_refuses(va, El::El0) {
            // The fill may evict the entry the running trace's fetches hit;
            // the next instruction's fetch translates again, as it would
            // have.
            self.trace_break = true;
            let tag_kept = fetch_keeps_tag(self.imp.features, self.sys.tcr_el1, va);
            let kept = if self.watch_armed { !PERMS_DATA } else { !0 };
            self.tlb.fill(
                va,
                Mapping {
                    pa: PhysAddr::new(mapping.pa.get() & !0xFFF),
                    perms: (mapping.perms & kept) | if tag_kept { TAG_KEPT_BY_FETCH } else { 0 },
                    ..mapping
                },
            );
        }
        Ok(walked)
    }

    /// The E0PD rule of Arm `AArch64_S1Translate` (FEAT_E0PD): with
    /// `TCR_EL1.E0PD0` or `E0PD1` set, an access at `va` checked against
    /// EL0's permissions (`accdesc.el`, EL0's for LDTR and STTR at EL1
    /// without PSTATE.UAO) takes a level 0 translation fault, ahead of the
    /// walk. Every such access walks ([`Cpu::s1_walk`]), as the TLB holds no
    /// page E0PD refuses EL0 that EL0 has a permission for
    /// ([`Cpu::walk_and_cache`]), or is [`Cpu::cache_maintenance`]'s, whose
    /// accesses can need no permission.
    fn e0pd_refuses(&self, va: u64, el: El) -> bool {
        el == El::El0 && e0pd_set(self.imp.features, self.sys.tcr_el1, va)
    }

    /// Arm `AArch64_S1Translate` with stage 1 enabled, as far as the walk:
    /// FEAT_E0PD's refusal of an access checked against `el`'s permissions,
    /// then the walk for an access of `kind` at `va`.
    fn s1_walk<B: PhysBus>(
        &self,
        bus: &mut B,
        va: u64,
        kind: AccessKind,
        el: El,
    ) -> Result<Walked, TranslationFault> {
        if self.e0pd_refuses(va, el) {
            return Err(TranslationFault::Translation { level: 0 });
        }
        self.walk(bus, va, kind)
    }

    /// Whether an access of `kind` checked against `el`'s permissions may
    /// use a page with flags `perms`: it has the permission it needs, a
    /// fetch does not use an entry for a tag it does not ignore, and a data
    /// access is not one PSTATE.PAN refuses (every data access is subject
    /// to PAN, Arm `AccessDescriptor.pan`; an instruction fetch is not).
    #[inline(always)]
    fn permits(&self, perms: u8, kind: AccessKind, el: El) -> bool {
        permitted(perms, kind, el)
            && match kind {
                AccessKind::Fetch => perms & TAG_KEPT_BY_FETCH == 0,
                AccessKind::Read | AccessKind::Write => !self.pan_refuses(perms, el),
            }
    }

    /// The PAN rule of Arm `AArch64_S1DirectBasePermissions` (FEAT_PAN):
    /// while PSTATE.PAN is set, an EL1 access subject to it has no access
    /// to memory EL0 can read or write, nor, with FEAT_PAN3 and
    /// `SCTLR_EL1.EPAN` set, to memory EL0 can execute. EL0's execute
    /// permission in `perms` is after WXN, which differs from the rule's
    /// `ux` only where EL0 can write, which the rule refuses anyway.
    #[inline(always)]
    fn pan_refuses(&self, perms: u8, el: El) -> bool {
        if !self.pstate.pan || el != El::El1 {
            return false;
        }
        let epan = self.implements(Feature::Pan3) && self.sys.sctlr_el1 & sctlr::EPAN != 0;
        let reachable = PERM_EL0_R | PERM_EL0_W | if epan { PERM_EL0_X } else { 0 };
        perms & reachable != 0
    }

    /// Whether top-byte-ignore covers `va` for an access of `kind` (Arm
    /// `AddrTop`).
    #[inline]
    fn tbi(&self, va: u64, kind: AccessKind) -> bool {
        self.effective_tbi(va, kind == AccessKind::Fetch)
    }

    /// Arm `EffectiveTBI` for the EL1&0 regime: whether top-byte-ignore
    /// covers `address`, an instruction address or a data one, in the half
    /// of the address space its bit 55 selects.
    #[inline]
    pub(crate) fn effective_tbi(&self, address: u64, instruction: bool) -> bool {
        self.tbi_applies((address >> 55) & 1 == 1, instruction)
    }

    /// Whether top-byte-ignore applies to the instruction or data addresses
    /// of the half of the address space `upper` names ([`tbi_under`]).
    #[inline]
    pub(crate) fn tbi_applies(&self, upper: bool, instruction: bool) -> bool {
        tbi_under(self.imp.features, self.sys.tcr_el1, upper, instruction)
    }

    /// TCR_EL1.T1SZ or T0SZ, of the half of the address space `upper`
    /// names, as it sets the bottom bit of a pointer authentication code:
    /// held within 16..=39, a value outside them counting as the nearest
    /// (Arm `AArch64_PACEffectiveTxSZ`, the `Constraint_FORCE` outcome of
    /// `Unpredictable_RESTnSZ`). The walk does not hold it so
    /// ([`input_address_valid`]).
    #[inline]
    pub(crate) fn effective_txsz(&self, upper: bool) -> u32 {
        txsz_under(self.sys.tcr_el1, upper).clamp(16, 39)
    }

    /// `va` with its tag cleared when top-byte-ignore covers it.
    #[inline]
    fn untagged(&self, va: u64, kind: AccessKind) -> u64 {
        if self.tbi(va, kind) {
            va & 0x00FF_FFFF_FFFF_FFFF
        } else {
            va
        }
    }

    /// The output address of an access of `kind` at `va` with stage 1
    /// disabled (Arm ARM D8.2.12, `AArch64_S1DisabledOutput`): `va` below
    /// `AddrTop`, or `None` where that is wider than the physical address
    /// size, an address size fault at level 0.
    #[inline(always)]
    fn s1_disabled_output(&self, va: u64, kind: AccessKind) -> Option<PhysAddr> {
        let address = self.untagged(va, kind);
        (address >> id::PA_BITS == 0).then_some(PhysAddr::new(address))
    }

    /// Arm `AArch64_BranchAddr`: the address a branch, an exception entry
    /// or an exception return writes to the PC. Under top-byte-ignore of
    /// instruction addresses the tag is dropped, bits [63:56] becoming
    /// copies of bit 55 (the sign extension EL0 and EL1 use, and this core
    /// has no other level); a tag TBID keeps from being ignored stays, for
    /// the fetch to fault on.
    #[inline]
    pub(crate) fn branch_addr(&self, target: u64) -> u64 {
        if self.tbi(target, AccessKind::Fetch) {
            sext(target, 56)
        } else {
            target
        }
    }

    /// The translation table walk (Arm `AArch64_S1Walk`) for the 4 KiB
    /// granule, with no hardware access-flag management. Table descriptors'
    /// hierarchical permissions apply unless FEAT_HPDS's `TCR_EL1.HPD0` or
    /// `HPD1` turns them off for the half of the address space being walked
    /// (`AArch64_S1NextWalkStateTable`). The base address leaves out
    /// `TTBRn_EL1.CnP` (FEAT_TTCNP), bit 0, which has no effect on one
    /// processor. A descriptor's Contiguous bit is ignored, as Arm's
    /// `AArch64_ContiguousBit` allows ("Treat Contiguous bit as 0"), so it
    /// never faults.
    fn walk<B: PhysBus>(
        &self,
        bus: &mut B,
        va: u64,
        kind: AccessKind,
    ) -> Result<Walked, TranslationFault> {
        let tcr = self.sys.tcr_el1;
        let features = self.imp.features;
        if !input_address_valid(features, tcr, va, kind == AccessKind::Fetch) {
            return Err(TranslationFault::Translation { level: 0 });
        }
        let upper = (va >> 55) & 1 == 1;
        let ttbr = if upper {
            self.sys.ttbr1_el1
        } else {
            self.sys.ttbr0_el1
        };
        let hpd = self.implements(Feature::Hpds) && (tcr >> (TCR_HPD0 + u32::from(upper))) & 1 == 1;
        let ia_bits = 64 - txsz_under(tcr, upper);
        let start_level: u32 = if ia_bits > 39 {
            0
        } else if ia_bits > 30 {
            1
        } else {
            2
        };
        let start_bits = ia_bits - (12 + 9 * (3 - start_level));
        // The walk's output addresses, the table base included, must fit
        // the output size TCR_EL1.IPS selects, bounded by PARange.
        let output_bits = output_address_bits(tcr);
        if (ttbr & 0x0000_FFFF_FFFF_FFFF) >> output_bits != 0 {
            return Err(TranslationFault::AddressSize { level: 0 });
        }
        let mut table = (ttbr & 0x0000_FFFF_FFFF_FFFE) & !((8u64 << start_bits) - 1);
        let mut level = start_level;
        let mut ap_table = 0u64;
        let mut uxn_table = false;
        let mut pxn_table = false;
        let clock = self.clock();
        loop {
            let shift = 12 + 9 * (3 - level);
            let index_bits = if level == start_level { start_bits } else { 9 };
            let index = (va >> shift) & ones(index_bits);
            let at_level = level as u8;
            let desc = match bus.read_descriptor(PhysAddr::new(table + index * 8), clock) {
                Ok(d) => d,
                Err(BusFault::Unmapped) => {
                    return Err(TranslationFault::ExternalAbortOnWalk { level: at_level })
                }
            };
            if desc & 1 == 0 {
                return Err(TranslationFault::Translation { level: at_level });
            }
            if level < 3 && desc & 2 != 0 {
                let next = desc & 0x0000_FFFF_FFFF_F000;
                if next >> output_bits != 0 {
                    return Err(TranslationFault::AddressSize { level: at_level });
                }
                if !hpd {
                    ap_table |= (desc >> 61) & 3;
                    uxn_table |= (desc >> 60) & 1 == 1;
                    pxn_table |= (desc >> 59) & 1 == 1;
                }
                table = next;
                level += 1;
                continue;
            }
            // A level-3 descriptor with bit 1 clear is reserved; level 0 has
            // no block descriptors with the 4 KiB granule.
            if (level == 3 && desc & 2 == 0) || level == 0 {
                return Err(TranslationFault::Translation { level: at_level });
            }
            let out = desc & 0x0000_FFFF_FFFF_F000 & !ones(shift);
            if out >> output_bits != 0 {
                return Err(TranslationFault::AddressSize { level: at_level });
            }
            if (desc >> 10) & 1 == 0 {
                return Err(TranslationFault::AccessFlag { level: at_level });
            }

            let ap = (desc >> 6) & 3;
            let el1_w = ap & 2 == 0 && ap_table & 2 == 0;
            let el0_r = ap & 1 == 1 && ap_table & 1 == 0;
            let el0_w = el0_r && el1_w;
            let uxn = (desc >> 54) & 1 == 1 || uxn_table;
            let pxn = (desc >> 53) & 1 == 1 || pxn_table;
            let wxn = self.sys.sctlr_el1 & sctlr::WXN != 0;
            // EL1 never executes memory EL0 can write (Arm ARM D8.4.1.2), and
            // WXN makes every writable region execute-never at its own EL.
            let el1_x = !pxn && !el0_w && !(wxn && el1_w);
            let el0_x = !uxn && !(wxn && el0_w);
            let flag = |on: bool, bit: u8| if on { bit } else { 0 };
            // GP, bit 50 of a block or page descriptor, RES0 without
            // FEAT_BTI.
            let guarded = (desc >> 50) & 1 == 1 && self.implements(Feature::Bti);
            let perms = PERM_EL1_R
                | flag(el1_w, PERM_EL1_W)
                | flag(el1_x, PERM_EL1_X)
                | flag(el0_r, PERM_EL0_R)
                | flag(el0_w, PERM_EL0_W)
                | flag(el0_x, PERM_EL0_X)
                | flag(guarded, GUARDED);
            let attr = ((self.sys.mair_el1 >> (((desc >> 2) & 7) * 8)) & 0xFF) as u8;
            let shareability = match ((desc >> 8) & 3) as u8 {
                0b01 => OUTER_SHAREABLE,
                sh => sh,
            };
            let cached = self.sys.sctlr_el1 & sctlr::C != 0;
            return Ok(Walked {
                mapping: Mapping {
                    pa: PhysAddr::new(out | (va & ones(shift))),
                    perms,
                    memory: Memory::decode(attr, shareability, cached),
                    level: level as u8,
                },
                attr,
                shareability,
            });
        }
    }

    /// Where data address `va` translates to now, for a debugger: the walk a
    /// data access would make, with no permission check, no TLB lookup or
    /// fill, and descriptors read through `bus` — which a caller makes a view
    /// that cannot disturb a device. Nothing in the processor changes.
    pub(crate) fn debug_translate<B: PhysBus>(
        &self,
        bus: &mut B,
        va: u64,
    ) -> Result<PhysAddr, TranslationFault> {
        if !self.mmu_enabled() {
            return self
                .s1_disabled_output(va, AccessKind::Read)
                .ok_or(TranslationFault::AddressSize { level: 0 });
        }
        self.walk(bus, va, AccessKind::Read).map(|w| w.mapping.pa)
    }

    /// `AT S1E1R/S1E1W/S1E0R/S1E0W/S1E1RP/S1E1WP`: translate, checking
    /// permissions as `check` says, and report in `PAR_EL1`, in the 64-bit
    /// format of Arm `AArch64_EncodePAR`. A synchronous external abort on
    /// the walk is not reported there but taken as a Data Abort
    /// (`AArch64_AT`: `IsExternalAbort` of the fault), with `CM` and `WnR`
    /// set, as for every address translation instruction
    /// (`AArch64_FaultSyndrome`).
    pub(crate) fn address_translate<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        write: bool,
        check: AtCheck,
    ) -> Result<(), Fault> {
        const RES1: u64 = 1 << 11;
        // This core runs in Non-secure state, where Arm leaves NS UNKNOWN;
        // it reports 1, as QEMU's `virt` does without `secure=on`.
        const NS: u64 = 1 << 9;
        let kind = if write {
            AccessKind::Write
        } else {
            AccessKind::Read
        };
        let el = match check {
            AtCheck::El0 => El::El0,
            AtCheck::El1 | AtCheck::El1Pan => El::El1,
        };
        let permits = |cpu: &Self, perms: u8| match check {
            AtCheck::El0 | AtCheck::El1 => permitted(perms, kind, el),
            AtCheck::El1Pan => cpu.permits(perms, kind, el),
        };
        let result = if self.mmu_enabled() {
            match self.s1_walk(bus, va, kind, el) {
                Ok(w) if permits(self, w.mapping.perms) => Ok(self.reported_attributes(w)),
                Ok(w) => Err(Fsc::Permission(w.mapping.level)),
                Err(f) => Err(Fsc::from(f)),
            }
        } else {
            // Device-nGnRnE (attribute 0x00) with stage 1 disabled.
            self.s1_disabled_output(va, kind)
                .map(|pa| Walked {
                    mapping: Mapping {
                        pa,
                        perms: 0,
                        memory: Memory::Device,
                        level: 0,
                    },
                    attr: 0,
                    shareability: OUTER_SHAREABLE,
                })
                .ok_or(Fsc::AddressSize(0))
        };
        if let Err(fsc @ Fsc::SyncExternalOnWalk(_)) = result {
            return Err(Self::maintenance_abort(va, fsc));
        }
        self.sys.par_el1 = match result {
            Ok(walked) => {
                (u64::from(walked.attr) << 56)
                    | (walked.mapping.pa.get() & 0x0000_FFFF_FFFF_F000)
                    | NS
                    | (u64::from(walked.shareability) << 7)
                    | RES1
            }
            // F = 1, FST, bit 11 RES1. PTW and S are 0: stage 1, not on a walk
            // of a stage-2 table.
            Err(f) => RES1 | (u64::from(f.code()) << 1) | 1,
        };
        Ok(())
    }

    /// `walked` with the attributes `PAR_EL1` reports: the effect of
    /// `SCTLR_EL1.C` shows there (Arm ARM D7.5.5, rule RYLPDG), Normal
    /// memory being Non-cacheable with it clear (`AArch64_S1TreatAsNormalNC`),
    /// which `EncodePARAttrs` encodes by the memory's XS attribute, 0 for
    /// Write-Back memory, where FEAT_XS has an encoding for it. Device and
    /// Normal Non-cacheable memory report Outer Shareable
    /// (`PAREncodeShareability`).
    fn reported_attributes(&self, walked: Walked) -> Walked {
        let attr = walked.attr;
        let device = attr & 0xF0 == 0;
        let uncached = !device && self.sys.sctlr_el1 & sctlr::C == 0;
        let attr = if !uncached {
            attr
        } else if self.implements(Feature::Xs) && attr_is_write_back(attr) {
            ATTR_NON_CACHEABLE_XS0
        } else {
            ATTR_NON_CACHEABLE
        };
        let non_cacheable = attr == ATTR_NON_CACHEABLE || attr == ATTR_NON_CACHEABLE_XS0;
        Walked {
            attr,
            shareability: if device || non_cacheable {
                OUTER_SHAREABLE
            } else {
                walked.shareability
            },
            ..walked
        }
    }

    // ----- access paths -----

    fn data_abort(va: u64, fsc: Fsc, write: bool) -> Fault {
        Self::data_abort_iss(va, fsc.code() | if write { ISS_WNR } else { 0 })
    }

    /// The Data Abort of an address translation instruction or of a cache
    /// maintenance instruction by address, with `CM` and `WnR` set
    /// (`AArch64_FaultSyndrome`, `AccessType_AT`, `AccessType_DC` and
    /// `AccessType_IC`).
    fn maintenance_abort(va: u64, fsc: Fsc) -> Fault {
        Self::data_abort_iss(va, fsc.code() | ISS_CM | ISS_WNR)
    }

    fn data_abort_iss(va: u64, iss: u32) -> Fault {
        Exception::new(ExceptionClass::DataAbort, iss)
            .with_far(va)
            .into()
    }

    fn instruction_abort(va: u64, fsc: Fsc) -> Fault {
        Exception::new(ExceptionClass::InstructionAbort, fsc.code())
            .with_far(va)
            .into()
    }

    /// A synchronous external abort: the bus refused the access. With
    /// FEAT_RAS its syndrome reports the PE's error state in `ISS.SET`
    /// (bits 12:11, `AArch64_EncodeSyncErrorSyndrome`), which `PEErrorState`
    /// leaves IMPLEMENTATION DEFINED; here it is Recoverable (UER, 0b00), the
    /// access refused before it took effect, as QEMU reports it. The
    /// syndrome is then the same with the feature as without.
    #[cold]
    #[inline(never)]
    fn external_abort(&self, va: u64, pa: PhysAddr, kind: AccessKind) -> Fault {
        if self.debug_stops.on_bus_fault {
            return Fault::Stop(StopCause::BusFault {
                pc: self.pc,
                addr: pa.get(),
            });
        }
        match kind {
            AccessKind::Fetch => Self::instruction_abort(va, Fsc::SyncExternal),
            AccessKind::Read | AccessKind::Write => {
                Self::data_abort(va, Fsc::SyncExternal, kind == AccessKind::Write)
            }
        }
    }

    /// The physical address of the instruction at `pc` (already known to be
    /// aligned), or the Instruction Abort its translation raises. Records
    /// whether the page is guarded (Arm `InGuardedPage`, FEAT_BTI) in
    /// [`Cpu::in_guarded_page`], for the instructions fetched from it. A
    /// fetch from Device memory that is not execute-never runs as Normal
    /// Non-cacheable, the outcome Arm lists for `Unpredictable_INSTRDEVICE`
    /// (Arm ARM B2.10.2 allows a Permission fault instead).
    #[inline(always)]
    pub(crate) fn fetch_translate<B: PhysBus>(
        &mut self,
        bus: &mut B,
        pc: u64,
    ) -> Result<PhysAddr, Fault> {
        match self.permitted_hit(pc, AccessKind::Fetch, self.pstate.el) {
            Some(entry) => {
                let mapping = entry.mapping(pc);
                self.in_guarded_page = mapping.perms & GUARDED != 0;
                Ok(mapping.pa)
            }
            None => self.fetch_translate_checked(bus, pc),
        }
    }

    /// The physical address a fetch at `pc` reads now, when neither the
    /// fetch nor the checks on the word fetched raise an exception and the
    /// fetch needs no walk: `None` on `PSTATE.IL` (the word raises the
    /// Illegal Execution state exception), a misaligned `pc`, or anything
    /// but a TLB hit that permits a fetch at the current EL. With stage 1
    /// disabled it is [`Cpu::translate`]'s `AArch64_S1DisabledOutput`
    /// address. It never fills, flushes or marks anything.
    pub(crate) fn fetch_probe(&self, pc: u64) -> Option<PhysAddr> {
        if self.pstate.il || pc & 3 != 0 {
            return None;
        }
        if !self.mmu_enabled() {
            return self.s1_disabled_output(pc, AccessKind::Fetch);
        }
        self.permitted_hit(pc, AccessKind::Fetch, self.pstate.el)
            .map(|entry| entry.mapping(pc).pa)
    }

    /// [`Cpu::fetch_translate`] with every check made: stage 1 disabled, a
    /// TLB miss, or a page that does not permit the fetch.
    #[cold]
    #[inline(never)]
    fn fetch_translate_checked<B: PhysBus>(
        &mut self,
        bus: &mut B,
        pc: u64,
    ) -> Result<PhysAddr, Fault> {
        let el = self.pstate.el;
        let m = self
            .translate(bus, pc, AccessKind::Fetch, el, OnDevice::Permitted)
            .map_err(|f| Self::instruction_abort(pc, f))?;
        self.in_guarded_page = m.perms & GUARDED != 0;
        Ok(m.pa)
    }

    /// Read the instruction word at `pa`, which `pc` translated to.
    #[inline]
    pub(crate) fn fetch_word<B: PhysBus>(
        &mut self,
        bus: &mut B,
        pc: u64,
        pa: PhysAddr,
    ) -> Result<u32, Fault> {
        match bus.fetch(pa, self.clock()) {
            Ok(word) => Ok(word),
            Err(BusFault::Unmapped) => Err(self.external_abort(pc, pa, AccessKind::Fetch)),
        }
    }

    /// The EL whose permissions a data access is checked against: EL0's for
    /// LDTR and STTR unless PSTATE.UAO is set (Arm
    /// `AArch64_IsUnprivAccessPriv`, at EL1 with no EL2), the current EL's
    /// for every other access.
    #[inline]
    fn access_el(&self, acc: DataAccess) -> El {
        if acc.unprivileged && !self.pstate.uao {
            El::El0
        } else {
            self.pstate.el
        }
    }

    /// Alignment checks that precede translation, then translation, with
    /// Device memory's alignment check as `on_device` says ahead of the
    /// permissions, for an access of `bytes` (a power of two) at `va` that
    /// stays in one page. A read-modify-write checks read permission, then
    /// write permission; write permission implies read permission, so a
    /// TLB entry that permits the write permits both. WnR is `!read &&
    /// write` for every fault but a refused write.
    ///
    /// An access whose TLB entry permits it, and that no alignment check
    /// can refuse — aligned and whole, or misaligned on Normal memory with
    /// no check that faults it before translation — raises none of these
    /// faults, and is answered inline; everything else makes the checks in
    /// order out of line.
    #[inline(always)]
    fn translate_data<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        span: AccessSpan,
        direction: DataDirection,
        acc: DataAccess,
        on_device: OnDevice,
    ) -> Result<Mapping, Fault> {
        let kind = match direction {
            DataDirection::Read => AccessKind::Read,
            DataDirection::Write | DataDirection::ReadWrite => AccessKind::Write,
        };
        if let Some(entry) = self.permitted_hit(va, kind, self.access_el(acc)) {
            // An ordinary access misaligned on Normal memory faults only
            // under SCTLR_EL1.A ([`Cpu::misaligned_faults`]).
            if on_device == OnDevice::Permitted
                || (entry.page.memory != Memory::Device
                    && acc.alignment == AlignmentRule::Any
                    && !self.strict_alignment())
            {
                return Ok(entry.mapping(va));
            }
        }
        self.translate_data_checked(bus, va, span, direction, acc, on_device)
    }

    /// [`Cpu::translate_data`] with every check made, in order: stage 1
    /// disabled, a TLB miss (which walks and fills), a page that does not
    /// permit the access, a watchpoint armed (which leaves the TLB no data
    /// permission to hit), or an access an alignment check may refuse. The
    /// watchpoints come last, after the access's translation and permission
    /// checks (Arm `AArch64_TranslateAddress`).
    #[cold]
    #[inline(never)]
    fn translate_data_checked<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        span: AccessSpan,
        direction: DataDirection,
        acc: DataAccess,
        on_device: OnDevice,
    ) -> Result<Mapping, Fault> {
        self.check_alignment(va, span.bytes, acc, direction == DataDirection::Write)?;
        let el = self.access_el(acc);
        let mapping = match direction {
            DataDirection::Read => self
                .translate(bus, va, AccessKind::Read, el, on_device)
                .map_err(|f| Self::data_abort(va, f, false)),
            DataDirection::Write => self
                .translate(bus, va, AccessKind::Write, el, on_device)
                .map_err(|f| Self::data_abort(va, f, true)),
            DataDirection::ReadWrite => self.translate_rmw(bus, va, el, on_device),
        }?;
        self.check_watchpoints(va, span.watched, direction.into(), el)?;
        Ok(mapping)
    }

    /// The translation of a read-modify-write at `va`: read permission,
    /// then write permission, checked against the flags of the one lookup.
    #[inline(never)]
    fn translate_rmw<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        el: El,
        on_device: OnDevice,
    ) -> Result<Mapping, Fault> {
        let m = self
            .translate(bus, va, AccessKind::Read, el, on_device)
            .map_err(|f| Self::data_abort(va, f, false))?;
        if self.mmu_enabled() && !self.permits(m.perms, AccessKind::Write, el) {
            return Err(Self::data_abort(va, Fsc::Permission(m.level), true));
        }
        Ok(m)
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
        let on_device = OnDevice::of_alignment(va, bytes);
        self.translate_data(bus, va, AccessSpan::single(bytes), direction, acc, on_device)
            .map(|m| m.pa)
    }

    /// The bus half of every data read: `width` at `pa`, which `va`
    /// translated to, in one bus access, zero-extended. A bus fault is a
    /// synchronous external abort reported at `va`.
    #[inline(always)]
    fn bus_read<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: PhysAddr,
        width: AccessWidth,
    ) -> Result<u64, Fault> {
        match bus.read(pa, width, self.clock()) {
            Ok(v) => Ok(v),
            Err(BusFault::Unmapped) => Err(self.external_abort(va, pa, AccessKind::Read)),
        }
    }

    /// Two adjacent elements of `width` at `pa`, which `va` translated to,
    /// each a bus access of its own.
    #[inline(always)]
    fn bus_read_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: PhysAddr,
        width: AccessWidth,
    ) -> Result<ElementPair, Fault> {
        let bytes = width.bytes();
        let first = self.bus_read(bus, va, pa, width)?;
        let second_pa = PhysAddr::new(pa.get() + bytes);
        let second = self.bus_read(bus, va.wrapping_add(bytes), second_pa, width)?;
        Ok(ElementPair { first, second })
    }

    /// The low `width` bytes of `value` at `pa`, which `va` translated to,
    /// in one bus access, with nothing else a store does. A bus fault is a
    /// synchronous external abort reported at `va`.
    #[inline(always)]
    fn bus_store<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: PhysAddr,
        width: AccessWidth,
        value: u64,
    ) -> Result<(), Fault> {
        match bus.write(pa, width, value, self.clock()) {
            Ok(()) => Ok(()),
            Err(BusFault::Unmapped) => Err(self.external_abort(va, pa, AccessKind::Write)),
        }
    }

    /// The bus half of every data write: [`Cpu::bus_store`], after
    /// [`Cpu::note_store`].
    #[inline(always)]
    fn bus_write<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: PhysAddr,
        width: AccessWidth,
        value: u64,
    ) -> Result<(), Fault> {
        self.note_store(pa, width.bytes());
        self.bus_store(bus, va, pa, width, value)
    }

    /// Two adjacent elements of `width` at `pa`, as
    /// [`Cpu::bus_read_pair`] reads them.
    #[inline(always)]
    fn bus_write_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: PhysAddr,
        width: AccessWidth,
        value: ElementPair,
    ) -> Result<(), Fault> {
        let bytes = width.bytes();
        self.bus_write(bus, va, pa, width, value.first)?;
        let second_pa = PhysAddr::new(pa.get() + bytes);
        self.bus_write(bus, va.wrapping_add(bytes), second_pa, width, value.second)
    }

    /// Read `width` at `va`, zero-extended. An access within one page is
    /// inlined into the instruction that makes it; one that crosses a page
    /// boundary is out of line.
    #[inline(always)]
    pub(crate) fn read_mem<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        acc: DataAccess,
    ) -> Result<u64, Fault> {
        let on_device = OnDevice::of_alignment(va, width.bytes());
        self.read_single(bus, va, width, acc, on_device)
    }

    /// [`Cpu::read_mem`] of one part of a transfer misaligned as a whole,
    /// which Device memory refuses however the part is aligned: Arm passes
    /// the transfer's `aligned` to `AArch64_MemSingle` for each part.
    pub(crate) fn read_part_of_misaligned<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        acc: DataAccess,
    ) -> Result<u64, Fault> {
        self.read_single(bus, va, width, acc, OnDevice::Refused)
    }

    #[inline(always)]
    fn read_single<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        acc: DataAccess,
        on_device: OnDevice,
    ) -> Result<u64, Fault> {
        let bytes = width.bytes();
        if (va & 0xFFF) + bytes <= 0x1000 {
            let span = AccessSpan::single(bytes);
            let m = self.translate_data(bus, va, span, DataDirection::Read, acc, on_device)?;
            return self.bus_read(bus, va, m.pa, width);
        }
        self.read_mem_across_pages(bus, va, bytes, acc)
    }

    /// A read of `bytes` at `va` that crosses a page boundary, and so is
    /// misaligned. Each byte is an access of its own (Arm `Mem` on an
    /// unaligned address, `AArch64_MemSingle` of each byte, each with the
    /// whole access's alignment), so a translation or external fault
    /// reports the first byte that faulted — an address in the page that
    /// needs mapping, which is what an OS's fault handler goes by — and a
    /// Device page faults on alignment at the first byte that reaches it,
    /// before its permissions are checked (the `Constraint_FAULT` choice of
    /// `Unpredictable_DEVPAGE2` for a byte after the first). An alignment
    /// fault before translation is the whole access's, and reports its
    /// address.
    #[inline(never)]
    fn read_mem_across_pages<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        acc: DataAccess,
    ) -> Result<u64, Fault> {
        self.check_alignment(va, bytes, acc, false)?;
        let el = self.access_el(acc);
        let mut value = 0u64;
        for i in 0..bytes {
            let byte_va = va.wrapping_add(i);
            let m = self
                .translate(bus, byte_va, AccessKind::Read, el, OnDevice::Refused)
                .map_err(|f| Self::data_abort(byte_va, f, false))?;
            self.check_watchpoints(byte_va, 1, WatchedAccess::Load, el)?;
            value |= self.bus_read(bus, byte_va, m.pa, AccessWidth::Byte)? << (8 * i);
        }
        Ok(value)
    }

    /// Write the low `width` bytes of `value` at `va`. As for a read, an
    /// access within one page is inlined and one across pages is not.
    #[inline(always)]
    pub(crate) fn write_mem<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        value: u64,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        let on_device = OnDevice::of_alignment(va, width.bytes());
        self.write_single(bus, va, width, value, acc, on_device)
    }

    /// [`Cpu::write_mem`] of one part of a transfer misaligned as a whole,
    /// as [`Cpu::read_part_of_misaligned`] reads one.
    pub(crate) fn write_part_of_misaligned<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        value: u64,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        self.write_single(bus, va, width, value, acc, OnDevice::Refused)
    }

    #[inline(always)]
    fn write_single<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        value: u64,
        acc: DataAccess,
        on_device: OnDevice,
    ) -> Result<(), Fault> {
        let bytes = width.bytes();
        if (va & 0xFFF) + bytes <= 0x1000 {
            let span = AccessSpan::single(bytes);
            let m = self.translate_data(bus, va, span, DataDirection::Write, acc, on_device)?;
            return self.bus_write(bus, va, m.pa, width, value);
        }
        self.write_mem_across_pages(bus, va, width, value, acc)
    }

    /// A write of the low `width` bytes of `value` at `va` that crosses a
    /// page boundary, its faults reported as
    /// [`Cpu::read_mem_across_pages`] reports a read's.
    #[inline(never)]
    fn write_mem_across_pages<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        value: u64,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        let bytes = width.bytes();
        self.check_alignment(va, bytes, acc, true)?;
        let el = self.access_el(acc);
        let pages = self.translate_write_pages(bus, va, bytes, el, OnDevice::Refused)?;
        self.write_element_on_pages(bus, va, 0, width, value, pages)
    }

    /// Translate, for a write checked against `el`'s permissions, the first
    /// byte of an access of `bytes` at `va` that crosses a page boundary
    /// and its first byte on the second page, Device memory refusing them
    /// as `on_device` says, so that a fault on either, or a watchpoint on
    /// any of its bytes, is taken before anything is written, reported at
    /// the byte that faulted. Arm's `Mem` setter writes byte by byte, and
    /// leaves what a faulting store has written unspecified; every store
    /// that crosses a page here leaves memory as it was on a translation
    /// fault, as QEMU's do (accel/tcg/cputlb.c `mmu_lookup`).
    fn translate_write_pages<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        el: El,
        on_device: OnDevice,
    ) -> Result<WritePages, Fault> {
        let second_page = va.wrapping_add(bytes - 1) & !0xFFF;
        let first = self
            .translate(bus, va, AccessKind::Write, el, on_device)
            .map_err(|f| Self::data_abort(va, f, true))?;
        let second = self
            .translate(bus, second_page, AccessKind::Write, el, on_device)
            .map_err(|f| Self::data_abort(second_page, f, true))?;
        self.check_watchpoints(va, bytes, WatchedAccess::Store, el)?;
        Ok(WritePages {
            first: first.pa,
            second: second.pa,
            first_bytes: second_page.wrapping_sub(va),
        })
    }

    /// Write the low `width` bytes of `value`, an element at byte `offset`
    /// of the store at `va` to `pages`: one access if it lies on one page,
    /// a byte at a time if it crosses.
    fn write_element_on_pages<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        offset: u64,
        width: AccessWidth,
        value: u64,
        pages: WritePages,
    ) -> Result<(), Fault> {
        let bytes = width.bytes();
        if pages.holds(offset, bytes) {
            return self.bus_write(bus, va.wrapping_add(offset), pages.byte(offset), width, value);
        }
        for i in 0..bytes {
            let at = offset + i;
            let byte = value >> (8 * i);
            self.bus_write(
                bus,
                va.wrapping_add(at),
                pages.byte(at),
                AccessWidth::Byte,
                byte,
            )?;
        }
        Ok(())
    }

    /// Whether a pair of `width` elements at `va` on the memory `m` is one
    /// single-copy atomic access of 8 bytes: with FEAT_LSE2, a pair of
    /// words that lies in one 16-byte granule of Normal Write-Back
    /// Shareable memory (Arm `Mem`, `AllInSCAGranule`, whose IMPLEMENTATION
    /// DEFINED quantity is 16 here, and `AArch64_MemSingleRead`,
    /// `IsWBShareable`). On any other memory `AArch64_MemSingleRead` makes
    /// the two accesses of the pair.
    #[inline(always)]
    fn pair_is_one_access(&self, va: u64, width: AccessWidth, m: Mapping) -> bool {
        width == AccessWidth::Word
            && (va & 15) + 8 <= 16
            && m.memory == Memory::WriteBackShareable
            && self.implements(Feature::Lse2)
    }

    /// Read two adjacent elements of `width`, at `va` and `va` plus `width`,
    /// each zero-extended, with `acc`'s constraints: Arm `LDP`'s one
    /// `Mem{2*datasize}` with `ispair` set, whose alignment is judged by the
    /// elements, as [`Cpu::read_mem_u128`] makes its halves.
    ///
    /// The result is that of two reads of the elements made in order: the
    /// same values, and the same fault with the same address. A pair of
    /// ordinary accesses within one page translates once, as the second
    /// translation would find what the first did: the second element is on
    /// the first's page, aligned exactly when the first is, with the same
    /// constraints, so it meets the same alignment checks, and the first
    /// translation's result may be used for it as a TLB holds it (a walk's
    /// result stays usable until TLB maintenance, Arm ARM D8.16). Each
    /// element still takes its own external abort.
    ///
    /// With FEAT_LSE2, a pair of words that lies in one 16-byte granule of
    /// Normal Write-Back Shareable memory is one single-copy atomic access
    /// of 8 bytes instead ([`Cpu::pair_is_one_access`]). A pair of
    /// doublewords there would be one 16-byte access, which the bus has no
    /// width for: it stays two.
    #[inline(always)]
    pub(crate) fn read_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        acc: DataAccess,
    ) -> Result<ElementPair, Fault> {
        let on_device = OnDevice::of_alignment(va, width.bytes());
        self.read_elements(bus, va, width, acc, on_device)
    }

    #[inline(always)]
    fn read_elements<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        acc: DataAccess,
        on_device: OnDevice,
    ) -> Result<ElementPair, Fault> {
        let bytes = width.bytes();
        if (va & 0xFFF) + 2 * bytes <= 0x1000 && acc.alignment == AlignmentRule::Any {
            let span = AccessSpan::pair(bytes);
            let m = self.translate_data(bus, va, span, DataDirection::Read, acc, on_device)?;
            if self.pair_is_one_access(va, width, m) {
                let value = self.bus_read(bus, va, m.pa, AccessWidth::Dword)?;
                return Ok(ElementPair {
                    first: value & 0xFFFF_FFFF,
                    second: value >> 32,
                });
            }
            return self.bus_read_pair(bus, va, m.pa, width);
        }
        self.read_pair_apart(bus, va, width, acc, on_device)
    }

    /// A [`Cpu::read_pair`] whose elements are read on their own: across a
    /// page boundary, or with an alignment rule that judges each element by
    /// where it lies.
    #[inline(never)]
    fn read_pair_apart<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        acc: DataAccess,
        on_device: OnDevice,
    ) -> Result<ElementPair, Fault> {
        let first = self.read_single(bus, va, width, acc, on_device)?;
        let second_va = va.wrapping_add(width.bytes());
        let second = self.read_single(bus, second_va, width, acc, on_device)?;
        Ok(ElementPair { first, second })
    }

    /// Write the low `width` bytes of each element of `value`, `first` at
    /// `va` and `second` at `va` plus `width`, in the order of Arm `STP`'s
    /// `Mem{2*datasize}`, each an access of its own as for
    /// [`Cpu::read_pair`].
    ///
    /// The result is that of two writes of the elements made in order: the
    /// same fault with the same address, and the same stores noted
    /// ([`Cpu::note_store`]) in the same order, each just before its
    /// element is written. A pair within one page translates once, for the
    /// reasons [`Cpu::read_pair`] gives, and with FEAT_LSE2 a pair of words
    /// within one 16-byte granule of Normal Write-Back Shareable memory is
    /// one access, as it reads.
    #[inline(always)]
    pub(crate) fn write_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        value: ElementPair,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        let on_device = OnDevice::of_alignment(va, width.bytes());
        self.write_elements(bus, va, width, value, acc, on_device)
    }

    #[inline(always)]
    fn write_elements<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        value: ElementPair,
        acc: DataAccess,
        on_device: OnDevice,
    ) -> Result<(), Fault> {
        let bytes = width.bytes();
        if (va & 0xFFF) + 2 * bytes <= 0x1000 && acc.alignment == AlignmentRule::Any {
            let span = AccessSpan::pair(bytes);
            let m = self.translate_data(bus, va, span, DataDirection::Write, acc, on_device)?;
            if self.pair_is_one_access(va, width, m) {
                let both = (value.second << 32) | (value.first & 0xFFFF_FFFF);
                return self.bus_write(bus, va, m.pa, AccessWidth::Dword, both);
            }
            return self.bus_write_pair(bus, va, m.pa, width, value);
        }
        self.write_pair_apart(bus, va, width, value, acc, on_device)
    }

    /// A [`Cpu::write_pair`] whose elements are written on their own, as
    /// for [`Cpu::read_pair_apart`]: across a page boundary, both pages
    /// translated first ([`Cpu::translate_write_pages`]).
    #[inline(never)]
    fn write_pair_apart<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        width: AccessWidth,
        value: ElementPair,
        acc: DataAccess,
        on_device: OnDevice,
    ) -> Result<(), Fault> {
        let bytes = width.bytes();
        if (va & 0xFFF) + 2 * bytes > 0x1000 {
            self.check_alignment(va, bytes, acc, true)?;
            let el = self.access_el(acc);
            let pages = self.translate_write_pages(bus, va, 2 * bytes, el, on_device)?;
            self.write_element_on_pages(bus, va, 0, width, value.first, pages)?;
            return self.write_element_on_pages(bus, va, bytes, width, value.second, pages);
        }
        self.write_single(bus, va, width, value.first, acc, on_device)?;
        let second_va = va.wrapping_add(bytes);
        self.write_single(bus, second_va, width, value.second, acc, on_device)
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
            return self
                .bus_read_pair(bus, va, pa, AccessWidth::Dword)
                .map(u128::from);
        }
        self.bus_read(bus, va, pa, AccessWidth::from_size_field(log2))
            .map(u128::from)
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
            return self.bus_write_pair(bus, va, pa, AccessWidth::Dword, value.into());
        }
        let width = AccessWidth::from_size_field(log2);
        self.bus_write(bus, va, pa, width, value as u64)
    }

    /// Read 16 bytes (a Q register) at `va`, as two 8-byte halves, each
    /// translated with the whole access's alignment (Arm `Mem`: a 16-byte
    /// SIMD&FP access is two single-copy atomic 8-byte accesses, each passed
    /// the access's own `aligned`).
    pub(crate) fn read_mem_u128<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        acc: DataAccess,
    ) -> Result<u128, Fault> {
        self.check_alignment(va, 16, acc, false)?;
        let on_device = OnDevice::of_alignment(va, 16);
        self.read_elements(bus, va, AccessWidth::Dword, acc, on_device)
            .map(u128::from)
    }

    /// Write 16 bytes (a Q register) at `va`, as [`Cpu::read_mem_u128`]
    /// reads them: the low half, then the high half. An access across a
    /// page boundary translates both pages before either half is written,
    /// as every store across pages does
    /// ([`Cpu::translate_write_pages`]).
    pub(crate) fn write_mem_u128<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        value: u128,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        self.check_alignment(va, 16, acc, true)?;
        let on_device = OnDevice::of_alignment(va, 16);
        self.write_elements(bus, va, AccessWidth::Dword, value.into(), acc, on_device)
    }

    /// Read the 32 bytes of an LDP or LDNP of two Q registers at `va` (Arm
    /// `Mem{256}`, `AccessType_ASIMD` with `ispair`). `SCTLR_EL1.A` judges
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
        self.check_alignment(va, 16, acc, false)?;
        let first = self.read_pair(bus, va, AccessWidth::Dword, acc)?;
        let second = self.read_pair(bus, va.wrapping_add(16), AccessWidth::Dword, acc)?;
        Ok(QuadwordPair {
            first: first.into(),
            second: second.into(),
        })
    }

    /// Write the 32 bytes of an STP or STNP of two Q registers at `va`, as
    /// [`Cpu::read_mem_q_pair`] reads them, in ascending order. An access
    /// across a page boundary translates both pages first, as every store
    /// across pages does ([`Cpu::translate_write_pages`]).
    pub(crate) fn write_mem_q_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        value: QuadwordPair,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        self.check_alignment(va, 16, acc, true)?;
        let eight_byte_aligned = va & 7 == 0;
        if (va & 0xFFF) + 32 > 0x1000 {
            let on_device = OnDevice::of_alignment(va, 8);
            let el = self.access_el(acc);
            let pages = self.translate_write_pages(bus, va, 32, el, on_device)?;
            let [first, second] = [ElementPair::from(value.first), value.second.into()];
            let doublewords = [first.first, first.second, second.first, second.second];
            for (offset, doubleword) in (0..).step_by(8).zip(doublewords) {
                let width = AccessWidth::Dword;
                self.write_element_on_pages(bus, va, offset, width, doubleword, pages)?;
            }
            return Ok(());
        }
        if !eight_byte_aligned {
            self.write_mem_u128(bus, va, value.first, acc)?;
            return self.write_mem_u128(bus, va.wrapping_add(16), value.second, acc);
        }
        self.write_pair(bus, va, AccessWidth::Dword, value.first.into(), acc)?;
        let second_va = va.wrapping_add(16);
        self.write_pair(bus, second_va, AccessWidth::Dword, value.second.into(), acc)
    }

    /// `DC IVAC`, `DC CVAC`, `DC CVAU`, `DC CVAP`, `DC CVADP`, `DC CIVAC`
    /// and `IC IVAU` at `va`: the translation Arm `AArch64_DC` and
    /// `AArch64_IC` make, as an aligned access of no size, checked as `op`
    /// says. With no cache to clean or invalidate, the translation is all
    /// they do. A fault is a Data Abort at `va` with `CM` and `WnR` set
    /// (`AArch64_FaultSyndrome`); Device memory refuses none of them. `DC
    /// IVAC`, which operates on a data cache line of [`id::DC_LINE_BYTES`],
    /// is watched as one aligned access of that size.
    pub(crate) fn cache_maintenance<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        op: CacheMaintenance,
    ) -> Result<(), Fault> {
        self.maintenance_translation(bus, va, op)?;
        if op == CacheMaintenance::Invalidate {
            let line = va & !(id::DC_LINE_BYTES - 1);
            let kind = WatchedAccess::CacheInvalidate;
            self.check_watchpoints(line, id::DC_LINE_BYTES, kind, self.pstate.el)?;
        }
        Ok(())
    }

    /// The translation and the permission check of [`Cpu::cache_maintenance`].
    fn maintenance_translation<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        op: CacheMaintenance,
    ) -> Result<(), Fault> {
        let abort = |fsc: Fsc| Self::maintenance_abort(va, fsc);
        if !self.mmu_enabled() {
            return match self.s1_disabled_output(va, AccessKind::Read) {
                Some(_) => Ok(()),
                None => Err(abort(Fsc::AddressSize(0))),
            };
        }
        let el = self.pstate.el;
        // Ahead of the TLB as well as the walk: an operation that needs no
        // permission would pass a hit E0PD refuses.
        if self.e0pd_refuses(va, el) {
            return Err(abort(Fsc::Translation(0)));
        }
        // While a watchpoint is armed an entry holds no data permission to
        // read here, so the walk answers.
        let page = match self.tlb.hit(va).filter(|_| !self.watch_armed) {
            Some(entry) => entry.page,
            None => {
                self.walk_and_cache(bus, va, AccessKind::Read, el)
                    .map_err(|f| abort(Fsc::from(f)))?
                    .mapping
            }
        };
        let needed = match (op, el) {
            (CacheMaintenance::Invalidate, _) => Some(AccessKind::Write),
            (CacheMaintenance::Clean, El::El0) => Some(AccessKind::Read),
            (CacheMaintenance::Clean, El::El1) | (CacheMaintenance::InstructionInvalidate, _) => {
                None
            }
        };
        match needed {
            Some(kind) if !permitted(page.perms, kind, el) => {
                Err(abort(Fsc::Permission(page.level)))
            }
            _ => Ok(()),
        }
    }

    /// `DC ZVA`: zero the naturally aligned block containing `va`. Device
    /// memory refuses it whatever its alignment
    /// (`AArch64_S1HasAlignmentFaultDueToMemType`, `AccessType_DCZero`).
    pub(crate) fn dc_zva<B: PhysBus>(&mut self, bus: &mut B, va: u64) -> Result<(), Fault> {
        let base = va & !(id::DC_ZVA_BYTES - 1);
        let el = self.pstate.el;
        let m = self
            .translate(bus, base, AccessKind::Write, el, OnDevice::Refused)
            .map_err(|f| Self::data_abort(va, f, true))?;
        self.check_watchpoints(base, id::DC_ZVA_BYTES, WatchedAccess::Store, el)?;
        self.note_store(m.pa, id::DC_ZVA_BYTES);
        for i in 0..id::DC_ZVA_BYTES / 8 {
            let at = PhysAddr::new(m.pa.get() + i * 8);
            self.bus_store(bus, va, at, AccessWidth::Dword, 0)?;
        }
        Ok(())
    }

    /// What a store of `bytes` at `pa`, within one page, does besides
    /// writing: it clears the local exclusive monitor if it overlaps the
    /// monitored granule (of the IMPLEMENTATION DEFINED cases of Arm ARM
    /// B2.12.1, a store by this processor to the marked address clears it,
    /// which is what makes an interrupted `LDXR`/`STXR` sequence retry; one
    /// elsewhere leaves it), and it forgets every trace holding an
    /// instruction it may overwrite. Every store the processor makes comes
    /// through here (R5: the one choke point for self-modifying code),
    /// before the bus sees it.
    #[inline]
    fn note_store(&mut self, pa: PhysAddr, bytes: u64) {
        let m = self.monitor;
        let at = pa.get();
        if m.armed && at < m.addr + m.bytes && m.addr < at + bytes {
            self.monitor.armed = false;
        }
        if self.traces.holds_code(at) {
            if let Some(span) = PhysSpan::new(pa, bytes) {
                self.forget_code(span);
            }
        }
    }

    /// The alignment check that comes before the translation of an access of
    /// `bytes` at `va` (Arm `AArch64_UnalignedAccessFaults`), on its own: a
    /// store-exclusive makes it before it compares its address with the
    /// monitor's (`AArch64_ExclusiveMonitorsPass`).
    pub(crate) fn check_alignment(
        &self,
        va: u64,
        bytes: u64,
        acc: DataAccess,
        write: bool,
    ) -> Result<(), Fault> {
        if va & (bytes - 1) != 0 && self.misaligned_faults(va, bytes, acc) {
            return Err(Self::data_abort(va, Fsc::Alignment, write));
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
            return Err(Self::data_abort(va, Fsc::Alignment, write));
        }
        Ok(())
    }

    /// The SP alignment check (`SCTLR_EL1.SA`/`SA0`) for an access whose
    /// base register is SP.
    #[inline]
    pub(crate) fn check_sp_alignment(&self, base_is_sp: bool, base: u64) -> Result<(), Fault> {
        if !base_is_sp || base & 0xF == 0 {
            return Ok(());
        }
        let enabled = match self.pstate.el {
            El::El1 => self.sys.sctlr_el1 & sctlr::SA != 0,
            El::El0 => self.sys.sctlr_el1 & sctlr::SA0 != 0,
        };
        if enabled {
            Err(Exception::new(ExceptionClass::SpAlignment, 0).into())
        } else {
            Ok(())
        }
    }
}
