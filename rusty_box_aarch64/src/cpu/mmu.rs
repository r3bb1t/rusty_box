//! Stage-1 translation (VMSAv8-64, Arm ARM D8) with the 4 KiB granule, the
//! one `ID_AA64MMFR0_EL1` advertises, in the EL1&0 regime and with FEAT_EL2
//! the EL2 regime ([`Regime`]; their walk parameters are `regime.rs`'s); the
//! TLB that caches it; and the data and instruction access paths every
//! load, store, fetch and cache maintenance by address goes through —
//! alignment checks, translation, permission checks, and the external abort
//! when the bus has nothing at an address.
//!
//! The TLB is direct-mapped and holds one 4 KiB page per entry, whatever the
//! size of the block that mapped it, tagged with the regime that translated
//! it (Arm `TLBContext`), so an access hits only an entry of its own regime.
//! It is flushed whole by every TLB maintenance instruction and by any write
//! to a register a walk reads (`SCTLR_EL1`, `TCR_EL1`, `TTBR0/1_EL1`,
//! `MAIR_EL1` and their EL2 counterparts) — invalidation is never narrower
//! than the architecture requires, so ASIDs need no tagging. `HCR_EL2.DC`
//! and `TGE` turn the EL1&0 regime's stage 1 off, which every access checks
//! before it looks in the TLB, so a write of `HCR_EL2` flushes nothing.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::dp::ones;
use super::exception::{Exception, ExceptionClass};
use super::exec::{Fault, StopCause};
use super::features::{Feature, Features};
use super::regime::{Regime, S1Params};
use super::{id, sctlr, Cpu, El, Levels};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};

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

/// The constraints a data access carries beyond its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DataAccess {
    /// `LDTR`/`STTR`: checked against EL0 permissions when executed at EL1
    /// with PSTATE.UAO clear (Arm `AArch64_IsUnprivAccessPriv`).
    pub(crate) unprivileged: bool,
    pub(crate) alignment: AlignmentRule,
    pub(crate) part: Part,
}

impl DataAccess {
    pub(crate) const NORMAL: Self = Self {
        unprivileged: false,
        alignment: AlignmentRule::Any,
        part: Part::Whole,
    };
    /// A load-acquire or store-release.
    pub(crate) const ORDERED: Self = Self {
        unprivileged: false,
        alignment: AlignmentRule::Ordered,
        part: Part::Whole,
    };
    /// An exclusive or atomic access.
    pub(crate) const ATOMIC: Self = Self {
        unprivileged: false,
        alignment: AlignmentRule::Atomic,
        part: Part::Whole,
    };
    pub(crate) const UNPRIVILEGED: Self = Self {
        unprivileged: true,
        alignment: AlignmentRule::Any,
        part: Part::Whole,
    };
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
/// `SCTLR_EL1.A` faults every misaligned access whatever the rule, and
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
    /// ARMv8.0 core faults on rather than sets.
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
            Self::ExternalAbortOnWalk { level } => write!(f, "external abort on the walk at level {level}"),
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

/// The permissions of the regime's privileged level: EL1's in EL1&0, EL2's
/// in the EL2 regime, which has no other.
const PERM_EL1_R: u8 = 1 << 0;
const PERM_EL1_W: u8 = 1 << 1;
const PERM_EL1_X: u8 = 1 << 2;
/// EL0's permissions, which only the EL1&0 regime has.
const PERM_EL0_R: u8 = 1 << 3;
const PERM_EL0_W: u8 = 1 << 4;
const PERM_EL0_X: u8 = 1 << 5;
const PERMS: u8 = PERM_EL1_R | PERM_EL1_W | PERM_EL1_X | PERM_EL0_R | PERM_EL0_W | PERM_EL0_X;
/// Not a permission: the descriptor's GP bit (FEAT_BTI), a guarded page.
const GUARDED: u8 = 1 << 6;
/// EL0's permissions.
const PERMS_EL0: u8 = PERM_EL0_R | PERM_EL0_W | PERM_EL0_X;

/// `ESR_ELx.ISS.WnR` and `CM` of a Data Abort.
const ISS_WNR: u32 = 1 << 6;
const ISS_CM: u32 = 1 << 8;

/// `TCR_EL1.E0PD0` (FEAT_E0PD); `E0PD1` is the bit above it.
const TCR_E0PD0: u32 = 55;

/// The distance from each of the privileged level's permissions to EL0's.
const PERM_EL0_SHIFT: u32 = 3;
const _: () = assert!(
    PERM_EL1_R << PERM_EL0_SHIFT == PERM_EL0_R
        && PERM_EL1_W << PERM_EL0_SHIFT == PERM_EL0_W
        && PERM_EL1_X << PERM_EL0_SHIFT == PERM_EL0_X
);

/// Whether `perms` allow an access of `kind` checked against `el`'s
/// permissions: EL0's at EL0, and the regime's privileged level's at EL1
/// and EL2.
#[inline]
const fn permitted(perms: u8, kind: AccessKind, el: El) -> bool {
    let privileged = match kind {
        AccessKind::Read => PERM_EL1_R,
        AccessKind::Write => PERM_EL1_W,
        AccessKind::Fetch => PERM_EL1_X,
    };
    let needed = if matches!(el, El::El0) { privileged << PERM_EL0_SHIFT } else { privileged };
    perms & needed != 0
}

/// Whether `tcr`'s FEAT_E0PD field for the half of the address space bit 55
/// of `va` names refuses EL0 every access there (`TCR_EL1.E0PD0` or
/// `E0PD1`, RES0 without the feature).
#[inline]
const fn e0pd_set(features: Features, tcr: u64, va: u64) -> bool {
    features.has(Feature::E0pd) && (tcr >> (TCR_E0PD0 + ((va >> 55) & 1) as u32)) & 1 == 1
}

/// An instruction word and the physical address it was fetched from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fetched {
    pub(crate) word: u32,
    pub(crate) pa: u64,
    /// Arm `InGuardedPage` (FEAT_BTI), as the fetch's translation sets it.
    pub(crate) guarded: bool,
}

/// The outcome of a translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Translated {
    pub(crate) pa: u64,
}

/// The flags and lookup level of the stage 1 translation of a page, which
/// only an instruction fetch and a read-modify-write read past the
/// permission check: whether the page is guarded (FEAT_BTI), and the
/// permissions a read-modify-write's write needs as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct S1Page {
    flags: u8,
    level: u8,
}

/// A translation, with what stage 1 gave its page: none with stage 1
/// disabled, when no page is guarded (`AArch64_S1DisabledOutput`) and no
/// access is refused its permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mapping {
    pa: u64,
    page: Option<S1Page>,
}

/// A completed walk, before permissions are checked against an access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Walked {
    pa: u64,
    perms: u8,
    device: bool,
    level: u8,
    /// The regime's `MAIR_ELx` attribute byte, as `PAR_EL1` reports it.
    attr: u8,
    /// The descriptor's SH[1:0] field (bits [9:8]).
    shareability: u8,
}

const TLB_ENTRIES: usize = 1024;
/// The key of an empty entry, which no translation's key equals.
const INVALID_KEY: u64 = u64::MAX;
/// A virtual page number's bits: `va >> 12` of a 64-bit address.
const PAGE_BITS: u32 = 52;
/// The bit of a TLB key that names the EL2 regime, above every page number.
const KEY_EL2: u64 = 1 << 63;

/// The key a translation of `va` in `regime` is cached under: its virtual
/// page number, with what it belongs to beyond it (Arm `TLBContext`,
/// `AArch64_GetS1TLBContext`), the regime, in bit 63. An access hits only
/// an entry of its own regime, so a page EL1 and EL2 both map at one
/// address is two entries, never one taken for the other.
#[inline(always)]
const fn tlb_key(va: u64, regime: Regime) -> u64 {
    (va >> 12) | ((regime as u64) << 63)
}

const _: () = assert!((Regime::El10 as u64) << 63 == 0 && (Regime::El2 as u64) << 63 == KEY_EL2);

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
    /// [`tlb_key`] of the translation, or `INVALID_KEY`.
    key: u64,
    pa_page: u64,
    perms: u8,
    device: bool,
    level: u8,
}

const EMPTY_ENTRY: TlbEntry = TlbEntry {
    key: INVALID_KEY,
    pa_page: 0,
    perms: 0,
    device: false,
    level: 0,
};

const _: () = assert!(core::mem::size_of::<TlbEntry>() == 24);

/// The translation cache: direct-mapped, 4 KiB pages.
pub struct Tlb {
    entries: [TlbEntry; TLB_ENTRIES],
}

impl Tlb {
    pub(crate) const fn new() -> Self {
        Self {
            entries: [EMPTY_ENTRY; TLB_ENTRIES],
        }
    }

    pub(crate) fn flush(&mut self) {
        self.entries = [EMPTY_ENTRY; TLB_ENTRIES];
    }

    #[inline]
    fn slot(va: u64) -> usize {
        ((va >> 12) as usize) & (TLB_ENTRIES - 1)
    }

    /// The entry of `regime` that translates `va`, if the TLB holds one.
    #[inline(always)]
    fn hit(&self, va: u64, regime: Regime) -> Option<&TlbEntry> {
        let entry = &self.entries[Self::slot(va)];
        (entry.key == tlb_key(va, regime)).then_some(entry)
    }

    /// The valid entries, for a snapshot: each one's virtual page number,
    /// output page, flags and level, and its regime's byte.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        let valid = self.entries.iter().filter(|e| e.key != INVALID_KEY).count();
        w.write_u32(valid as u32)?;
        for e in self.entries.iter().filter(|e| e.key != INVALID_KEY) {
            w.write_u64(e.key & !KEY_EL2)?;
            w.write_u64(e.pa_page)?;
            w.write_bytes(&[e.perms, u8::from(e.device), e.level, key_regime(e.key) as u8])?;
        }
        Ok(())
    }

    /// The inverse of [`Tlb::save`], for a processor implementing
    /// `features` with `tcr` in TCR_EL1: an entry must be one its walk can
    /// produce and its TLB can hold (`Cpu::walk_and_cache`).
    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R, features: Features, tcr: u64) -> SnapResult {
        let walked = PERMS | if features.has(Feature::Bti) { GUARDED } else { 0 };
        let valid = r.read_count(TLB_ENTRIES)?;
        self.flush();
        for _ in 0..valid {
            let page = r.read_u64()?;
            let pa_page = r.read_u64()?;
            let mut rest = [0u8; 4];
            r.read_bytes(&mut rest)?;
            let [perms, device, level, regime] = rest;
            if page >> PAGE_BITS != 0 || pa_page & 0xFFF != 0 || perms & !walked != 0 || device > 1 || level > 3 {
                return Err(SnapError::Invalid("TLB entry is not one a walk can produce"));
            }
            let regime = match byte_regime(regime) {
                Some(Regime::El10) => Regime::El10,
                Some(Regime::El2) if !features.has(Feature::El2) => {
                    return Err(SnapError::Invalid("TLB entry is of a regime this processor lacks"));
                }
                // The EL2 regime has one privilege level: its walk gives EL0
                // no permission.
                Some(Regime::El2) if perms & PERMS_EL0 != 0 => {
                    return Err(SnapError::Invalid("TLB entry is not one a walk can produce"));
                }
                Some(Regime::El2) => Regime::El2,
                None => return Err(SnapError::Invalid("TLB entry names no translation regime")),
            };
            let va = page << 12;
            if perms & PERMS_EL0 != 0 && e0pd_set(features, tcr, va) {
                return Err(SnapError::Invalid("TLB entry gives EL0 a page FEAT_E0PD refuses it"));
            }
            let slot = &mut self.entries[Self::slot(va)];
            if slot.key != INVALID_KEY {
                return Err(SnapError::Invalid("two TLB entries claim one slot"));
            }
            *slot = TlbEntry {
                key: tlb_key(va, regime),
                pa_page,
                perms,
                device: device == 1,
                level,
            };
        }
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
    /// permissions, after Device memory's alignment check
    /// (`AArch64_S1Translate`'s order: the walk's faults, then the memory
    /// type's, then the permissions'). The TLB hit is inlined into every
    /// access path; a miss walks out of line.
    #[inline(always)]
    pub(crate) fn translate<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        kind: AccessKind,
        el: El,
        on_device: OnDevice,
    ) -> Result<Translated, Fsc> {
        self.map(bus, va, kind, el, on_device).map(|m| Translated { pa: m.pa })
    }

    /// [`Cpu::translate`], with the entry's flags.
    #[inline(always)]
    fn map<B: PhysBus>(&mut self, bus: &mut B, va: u64, kind: AccessKind, el: El, on_device: OnDevice) -> Result<Mapping, Fsc> {
        let regime = self.regime();
        if !self.s1_enabled(regime) {
            // Data accesses are Device-nGnRnE with stage 1 disabled, Normal
            // in EL1&0 under `HCR_EL2.DC`, and instruction fetches are
            // Normal.
            let address = self.s1_disabled_output(regime, va, kind).ok_or(Fsc::AddressSize(0))?;
            let device = kind != AccessKind::Fetch && !self.s1_disabled_data_normal(regime);
            if on_device == OnDevice::Refused && device {
                return Err(Fsc::Alignment);
            }
            return Ok(Mapping {
                pa: address,
                page: None,
            });
        }

        // The TLB holds no page FEAT_E0PD refuses EL0 that EL0 has a
        // permission for (`walk_and_cache`), so every EL0 access E0PD
        // refuses that hits is refused here, and `refusal` names the fault.
        if let Some(entry) = self.tlb.hit(va, regime) {
            if on_device == OnDevice::Refused && entry.device {
                return Err(self.refusal(va, el, Fsc::Alignment));
            }
            if !self.permits(entry.perms, kind, el) {
                return Err(self.refusal(va, el, Fsc::Permission(entry.level)));
            }
            return Ok(Mapping {
                pa: entry.pa_page | (va & 0xFFF),
                page: Some(S1Page {
                    flags: entry.perms,
                    level: entry.level,
                }),
            });
        }
        self.translate_miss(bus, va, kind, el, on_device)
    }

    /// A TLB miss: walk, cache what the walk found, check the memory type
    /// and the permissions.
    #[inline(never)]
    fn translate_miss<B: PhysBus>(&mut self, bus: &mut B, va: u64, kind: AccessKind, el: El, on_device: OnDevice) -> Result<Mapping, Fsc> {
        let walked = self.walk_and_cache(bus, va, kind, el)?;
        if on_device == OnDevice::Refused && walked.device {
            return Err(Fsc::Alignment);
        }
        if !self.permits(walked.perms, kind, el) {
            return Err(Fsc::Permission(walked.level));
        }
        Ok(Mapping {
            pa: walked.pa,
            page: Some(S1Page {
                flags: walked.perms,
                level: walked.level,
            }),
        })
    }

    /// The walk of a TLB miss for an access of `kind` at `va` checked
    /// against `el`'s permissions ([`Cpu::s1_walk`]) in the current regime,
    /// cached in the TLB.
    fn walk_and_cache<B: PhysBus>(&mut self, bus: &mut B, va: u64, kind: AccessKind, el: El) -> Result<Walked, TranslationFault> {
        let regime = self.regime();
        let walked = self.s1_walk(bus, regime, va, kind, el)?;
        // A tagged address keeps its tag in the entry's tag, so it hits only
        // itself; a TCR write, which can change what the walk ignores,
        // empties the TLB. A data access through a tag instruction fetches
        // do not ignore (`TBID`) leaves no entry: a fetch from the same
        // address would hit it, where its own walk faults. Nor does a page
        // FEAT_E0PD refuses EL0 that EL0 has a permission for: its accesses
        // walk each time, so an EL0 access never passes a hit's permission
        // check in a half E0PD refuses EL0, where the pages are normally all
        // EL1's and cached.
        let fetch_faults =
            kind != AccessKind::Fetch && regime.canonical(va) != va && !self.regime_tbi(regime, regime.upper(va), true);
        let e0pd_refuses_permitted = walked.perms & PERMS_EL0 != 0 && self.e0pd_refuses(va, El::El0);
        if !fetch_faults && !e0pd_refuses_permitted {
            self.tlb.entries[Tlb::slot(va)] = TlbEntry {
                key: tlb_key(va, regime),
                pa_page: walked.pa & !0xFFF,
                perms: walked.perms,
                device: walked.device,
                level: walked.level,
            };
        }
        Ok(walked)
    }

    /// The E0PD rule of Arm `AArch64_S1Translate` (FEAT_E0PD): with
    /// `TCR_EL1.E0PD0` or `E0PD1` set, an access at `va` checked against
    /// EL0's permissions (`accdesc.el`, EL0's for LDTR and STTR at EL1
    /// without PSTATE.UAO), which only the EL1&0 regime has, takes a level 0
    /// translation fault, ahead of the walk. Every such access meets it: on
    /// a TLB miss in [`Cpu::s1_walk`]; on a hit in [`Cpu::refusal`], where
    /// its permission check fails, as the TLB holds no page E0PD refuses EL0
    /// that EL0 has a permission for ([`Cpu::walk_and_cache`]); and in
    /// [`Cpu::cache_maintenance`], whose accesses can need no permission.
    fn e0pd_refuses(&self, va: u64, el: El) -> bool {
        el == El::El0 && e0pd_set(self.imp.features, self.sys.tcr_el1, va)
    }

    /// Arm `AArch64_S1Translate` with stage 1 enabled, as far as the walk:
    /// FEAT_E0PD's refusal of an access checked against `el`'s permissions,
    /// then the walk of `regime` for an access of `kind` at `va`.
    fn s1_walk<B: PhysBus>(&self, bus: &mut B, regime: Regime, va: u64, kind: AccessKind, el: El) -> Result<Walked, TranslationFault> {
        if self.e0pd_refuses(va, el) {
            return Err(TranslationFault::Translation { level: 0 });
        }
        self.walk(bus, self.s1_params(regime, va), va, kind)
    }

    /// The fault an access at `va` checked against `el`'s permissions takes
    /// when its TLB entry refuses it `fault`: a level 0 translation fault
    /// where FEAT_E0PD refuses EL0 the page, which Arm's
    /// `AArch64_S1Translate` checks ahead of the walk and so of every other
    /// fault, and `fault` otherwise.
    #[cold]
    #[inline(never)]
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
        if !self.pstate.pan || el != El::El1 {
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
    fn s1_disabled_output(&self, regime: Regime, va: u64, kind: AccessKind) -> Option<u64> {
        let tbi = self.regime_tbi(regime, regime.upper(va), kind == AccessKind::Fetch);
        let address = if tbi { va & 0x00FF_FFFF_FFFF_FFFF } else { va };
        (address >> id::PA_BITS == 0).then_some(address)
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
    fn walk<B: PhysBus>(&self, bus: &mut B, params: S1Params, va: u64, kind: AccessKind) -> Result<Walked, TranslationFault> {
        let ia_bits = 64 - params.txsz;
        let top = if self.regime_tbi(params.regime, params.upper, kind == AccessKind::Fetch) { 56 } else { 64 };
        let check = ones(top - ia_bits) << ia_bits;
        if va & check != if params.upper { check } else { 0 } {
            return Err(TranslationFault::Translation { level: 0 });
        }
        if params.epd {
            return Err(TranslationFault::Translation { level: 0 });
        }

        let start_level: u32 = if ia_bits > 39 {
            0
        } else if ia_bits > 30 {
            1
        } else {
            2
        };
        let start_bits = ia_bits - (12 + 9 * (3 - start_level));
        let mut table = (params.ttbr & 0x0000_FFFF_FFFF_FFFE) & !((8u64 << start_bits) - 1);
        // The start level's descriptor address is `table` with an index
        // below the table's alignment, so it fits the output size exactly
        // when `table` does.
        if table >> params.oa_bits != 0 {
            return Err(TranslationFault::AddressSize { level: 0 });
        }
        let mut level = start_level;
        let mut ap_table = 0u64;
        // `UXNTable` in EL1&0; `XNTable`, the same bit, in EL2.
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
                Err(BusFault::Unmapped) => return Err(TranslationFault::ExternalAbortOnWalk { level: at_level }),
            };
            if desc & 1 == 0 {
                return Err(TranslationFault::Translation { level: at_level });
            }
            if level < 3 && desc & 2 != 0 {
                let next = desc & 0x0000_FFFF_FFFF_F000;
                if next >> params.oa_bits != 0 {
                    return Err(TranslationFault::AddressSize { level: at_level });
                }
                if !params.hpd {
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
            if out >> params.oa_bits != 0 {
                return Err(TranslationFault::AddressSize { level: at_level });
            }
            if (desc >> 10) & 1 == 0 {
                return Err(TranslationFault::AccessFlag { level: at_level });
            }

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
                // EL1 never executes memory EL0 can write, and WXN makes
                // every writable region execute-never at its own EL.
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
            // GP, bit 50 of a block or page descriptor, RES0 without
            // FEAT_BTI.
            if (desc >> 50) & 1 == 1 && self.implements(Feature::Bti) {
                perms |= GUARDED;
            }
            let attr = ((params.mair >> (((desc >> 2) & 7) * 8)) & 0xFF) as u8;
            return Ok(Walked {
                pa: out | (va & ones(shift)),
                perms,
                device: attr & 0xF0 == 0,
                level: level as u8,
                attr,
                shareability: ((desc >> 8) & 3) as u8,
            });
        }
    }

    /// Where data address `va` translates to now in the current regime, for
    /// a debugger: the walk a data access would make, with no permission
    /// check, no TLB lookup or fill, and descriptors read through `bus` —
    /// which a caller makes a view that cannot disturb a device. Nothing in
    /// the processor changes.
    pub(crate) fn debug_translate<B: PhysBus>(&self, bus: &mut B, va: u64) -> Result<u64, TranslationFault> {
        let regime = self.regime();
        if !self.s1_enabled(regime) {
            return self
                .s1_disabled_output(regime, va, AccessKind::Read)
                .ok_or(TranslationFault::AddressSize { level: 0 });
        }
        self.walk(bus, self.s1_params(regime, va), va, AccessKind::Read).map(|w| w.pa)
    }

    /// `AT S1E1R/S1E1W/S1E0R/S1E0W/S1E1RP/S1E1WP/S1E2R/S1E2W` (Arm
    /// `AArch64_AT` with a stage 1 translation): translate `va` in the
    /// regime `check` names, checking permissions as it says, and report in
    /// `PAR_EL1` (`AArch64_EncodePAR`). A synchronous external abort on the
    /// walk is not reported there but taken, as a Data Abort with `CM` and
    /// `WnR` set (`AArch64_FaultSyndrome`, `AccessType_AT`), leaving
    /// `PAR_EL1` as it was, which is one of the values the ARBITRARY one Arm
    /// gives it can be.
    pub(crate) fn address_translate<B: PhysBus>(&mut self, bus: &mut B, va: u64, write: bool, check: AtCheck) -> Result<(), Fault> {
        const RES1: u64 = 1 << 11;
        // This core runs in Non-secure state, where `PAR_EL1.NS` of a
        // successful translation is ARBITRARY: it reports 1.
        const NS: u64 = 1 << 9;
        let kind = if write { AccessKind::Write } else { AccessKind::Read };
        let (regime, el) = match check {
            AtCheck::El0 => (Regime::El10, El::El0),
            AtCheck::El1 | AtCheck::El1Pan => (Regime::El10, El::El1),
            AtCheck::El2 => (Regime::El2, El::El2),
        };
        let permits = |cpu: &Self, perms: u8| match check {
            AtCheck::El0 | AtCheck::El1 | AtCheck::El2 => permitted(perms, kind, el),
            AtCheck::El1Pan => cpu.permits(perms, kind, el),
        };
        let result = if self.s1_enabled(regime) {
            match self.s1_walk(bus, regime, va, kind, el) {
                Ok(w) if permits(self, w.perms) => Ok((w.pa, w.attr, w.shareability)),
                Ok(w) => Err(Fsc::Permission(w.level)),
                Err(TranslationFault::ExternalAbortOnWalk { level }) => {
                    return Err(Self::maintenance_abort(va, Fsc::SyncExternalOnWalk(level)));
                }
                Err(f) => Err(Fsc::from(f)),
            }
        } else {
            // Normal Inner and Outer Write-Back, Read- and Write-Allocate,
            // Non-shareable, under `HCR_EL2.DC`; Device-nGnRnE (attribute
            // 0x00) otherwise.
            let attr = if self.s1_disabled_data_normal(regime) { 0xFF } else { 0 };
            self.s1_disabled_output(regime, va, kind)
                .map(|address| (address, attr, 0))
                .ok_or(Fsc::AddressSize(0))
        };
        self.sys.par_el1 = match result {
            Ok((pa, attr, shareability)) => {
                // Arm `PAREncodeShareability`: Device and Normal
                // Non-cacheable memory report Outer Shareable.
                let sh = if attr & 0xF0 == 0 || attr == 0x44 || attr == 0x40 { 2 } else { shareability };
                (u64::from(attr) << 56) | (pa & 0x0000_FFFF_FFFF_F000) | NS | (u64::from(sh) << 7) | RES1
            }
            // F = 1, FST, bit 11 RES1. PTW and S are 0: stage 1, not on a walk
            // of a stage-2 table.
            Err(f) => RES1 | (u64::from(f.code()) << 1) | 1,
        };
        Ok(())
    }

    // ----- access paths -----

    fn data_abort(va: u64, fsc: Fsc, write: bool) -> Fault {
        Self::data_abort_iss(va, fsc.code() | if write { ISS_WNR } else { 0 })
    }

    /// The Data Abort of a cache maintenance instruction by address or an
    /// address translation instruction, with `CM` and `WnR` set
    /// (`AArch64_FaultSyndrome`, `AccessType_DC`, `AccessType_IC` and
    /// `AccessType_AT`).
    fn maintenance_abort(va: u64, fsc: Fsc) -> Fault {
        Self::data_abort_iss(va, fsc.code() | ISS_CM | ISS_WNR)
    }

    fn data_abort_iss(va: u64, iss: u32) -> Fault {
        Exception::new(ExceptionClass::DataAbort, iss).with_far(va).into()
    }

    /// A synchronous external abort: the bus refused the access. With
    /// FEAT_RAS its syndrome reports the PE's error state in `ISS.SET`
    /// (bits 12:11, `AArch64_EncodeSyncErrorSyndrome`), which `PEErrorState`
    /// leaves IMPLEMENTATION DEFINED; here it is Recoverable (UER, 0b00), the
    /// access refused before it took effect, as QEMU reports it. The
    /// syndrome is then the same with the feature as without.
    fn external_abort(&self, va: u64, pa: u64, kind: AccessKind) -> Fault {
        if self.debug_stops.on_bus_fault {
            return Fault::Stop(StopCause::BusFault { pc: self.pc, addr: pa });
        }
        match kind {
            AccessKind::Fetch => Exception::new(ExceptionClass::InstructionAbort, Fsc::SyncExternal.code())
                .with_far(va)
                .into(),
            AccessKind::Read => Self::data_abort(va, Fsc::SyncExternal, false),
            AccessKind::Write => Self::data_abort(va, Fsc::SyncExternal, true),
        }
    }

    /// Fetch the instruction at `pc` (already known to be aligned).
    #[inline]
    pub(crate) fn fetch<B: PhysBus>(&mut self, bus: &mut B, pc: u64) -> Result<Fetched, Fault> {
        let el = self.pstate.el;
        let m = self
            .map(bus, pc, AccessKind::Fetch, el, OnDevice::Permitted)
            .map_err(|f| Fault::from(Exception::new(ExceptionClass::InstructionAbort, f.code()).with_far(pc)))?;
        match bus.fetch(PhysAddr::new(m.pa), self.clock()) {
            Ok(word) => Ok(Fetched {
                word,
                pa: m.pa,
                guarded: m.page.is_some_and(|page| page.flags & GUARDED != 0),
            }),
            Err(BusFault::Unmapped) => Err(self.external_abort(pc, m.pa, AccessKind::Fetch)),
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
    /// access that stays in one page. A read-modify-write checks read
    /// permission, then write permission. WnR is `!read && write` for every
    /// fault but a refused write.
    fn translate_data<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        direction: DataDirection,
        acc: DataAccess,
    ) -> Result<Translated, Fault> {
        let misaligned = va & (bytes - 1) != 0;
        if misaligned && self.misaligned_faults(va, bytes, acc) {
            return Err(Self::data_abort(va, Fsc::Alignment, direction == DataDirection::Write));
        }
        let on_device = if misaligned || acc.part == Part::OfMisaligned {
            OnDevice::Refused
        } else {
            OnDevice::Permitted
        };
        let el = self.access_el(acc);
        match direction {
            DataDirection::Read => {
                self.translate(bus, va, AccessKind::Read, el, on_device).map_err(|f| Self::data_abort(va, f, false))
            }
            DataDirection::Write => {
                self.translate(bus, va, AccessKind::Write, el, on_device).map_err(|f| Self::data_abort(va, f, true))
            }
            DataDirection::ReadWrite => self.translate_rmw(bus, va, el, on_device),
        }
    }

    /// The translation of a read-modify-write at `va`: read permission,
    /// then write permission, checked against the flags of the one lookup.
    #[inline(never)]
    fn translate_rmw<B: PhysBus>(&mut self, bus: &mut B, va: u64, el: El, on_device: OnDevice) -> Result<Translated, Fault> {
        let m = self.map(bus, va, AccessKind::Read, el, on_device).map_err(|f| Self::data_abort(va, f, false))?;
        match m.page {
            Some(page) if !self.permits(page.flags, AccessKind::Write, el) => {
                Err(Self::data_abort(va, Fsc::Permission(page.level), true))
            }
            Some(_) | None => Ok(Translated { pa: m.pa }),
        }
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
        self.translate_data(bus, va, bytes, direction, acc).map(|t| PhysAddr::new(t.pa))
    }

    /// The bus half of every data read: `1 << log2` bytes (`log2` in 0..=3)
    /// at `pa`, which `va` translated to, in one bus access, zero-extended.
    /// A bus fault is a synchronous external abort reported at `va`.
    #[inline(always)]
    fn bus_read<B: PhysBus>(&mut self, bus: &mut B, va: u64, pa: PhysAddr, log2: u32) -> Result<u64, Fault> {
        let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
        match bus.read(pa, width, self.clock()) {
            Ok(v) => Ok(v & width.value_mask()),
            Err(BusFault::Unmapped) => Err(self.external_abort(va, pa.get(), AccessKind::Read)),
        }
    }

    /// The bus half of every data write: the low `1 << log2` bytes of
    /// `value` at `pa`, which `va` translated to, in one bus access, after
    /// clearing an exclusive monitor the store overlaps. A bus fault is a
    /// synchronous external abort reported at `va`.
    #[inline(always)]
    fn bus_write<B: PhysBus>(&mut self, bus: &mut B, va: u64, pa: PhysAddr, log2: u32, value: u64) -> Result<(), Fault> {
        let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
        self.note_store(pa.get(), 1 << log2);
        match bus.write(pa, width, value & width.value_mask(), self.clock()) {
            Ok(()) => Ok(()),
            Err(BusFault::Unmapped) => Err(self.external_abort(va, pa.get(), AccessKind::Write)),
        }
    }

    /// Read `1 << log2` bytes (`log2` in 0..=3) at `va`, zero-extended.
    pub(crate) fn read_mem<B: PhysBus>(&mut self, bus: &mut B, va: u64, log2: u32, acc: DataAccess) -> Result<u64, Fault> {
        let bytes = 1u64 << log2;
        if (va & 0xFFF) + bytes <= 0x1000 {
            let t = self.translate_data(bus, va, bytes, DataDirection::Read, acc)?;
            return self.bus_read(bus, va, PhysAddr::new(t.pa), log2);
        }
        // Crosses a page boundary, so it is misaligned: Normal memory only.
        // Each byte is an access of its own (Arm ARM `AArch64.MemSingle` on
        // an unaligned address), so a translation or external fault reports
        // the first byte that faulted — an address in the page that needs
        // mapping, which is what an OS's fault handler goes by. An alignment
        // fault is the whole access's, and reports its address.
        if self.misaligned_faults(va, bytes, acc) {
            return Err(Self::data_abort(va, Fsc::Alignment, false));
        }
        let el = self.access_el(acc);
        let mut value = 0u64;
        for i in 0..bytes {
            let byte_va = va.wrapping_add(i);
            let t = self
                .translate(bus, byte_va, AccessKind::Read, el, OnDevice::Refused)
                .map_err(|f| Self::data_abort(if f == Fsc::Alignment { va } else { byte_va }, f, false))?;
            value |= self.bus_read(bus, byte_va, PhysAddr::new(t.pa), 0)? << (8 * i);
        }
        Ok(value)
    }

    /// Write the low `1 << log2` bytes of `value` at `va`.
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
        if self.misaligned_faults(va, bytes, acc) {
            return Err(Self::data_abort(va, Fsc::Alignment, true));
        }
        // Translate both pages before writing anything, so a fault on the
        // second page leaves memory untouched. As for a read, a translation
        // fault reports the first byte that faulted: the access's own address
        // in the first page, the start of the second page in the second.
        let el = self.access_el(acc);
        let second_page = va.wrapping_add(bytes - 1) & !0xFFF;
        for probe in [va, second_page] {
            self.translate(bus, probe, AccessKind::Write, el, OnDevice::Refused)
                .map_err(|f| Self::data_abort(if f == Fsc::Alignment { va } else { probe }, f, true))?;
        }
        for i in 0..bytes {
            let byte_va = va.wrapping_add(i);
            let t = self
                .translate(bus, byte_va, AccessKind::Write, el, OnDevice::Refused)
                .map_err(|f| Self::data_abort(byte_va, f, true))?;
            self.bus_write(bus, byte_va, PhysAddr::new(t.pa), 0, value >> (8 * i))?;
        }
        Ok(())
    }

    /// The read half of an atomic access: `1 << log2` bytes (`log2` in
    /// 0..=4) at `pa`, which [`Cpu::translate_for`] translated `va` to with
    /// every check, as [`Cpu::read_mem`] reads them; 16 bytes, which are
    /// aligned (no 16-byte atomic may be misaligned), are two 8-byte halves.
    pub(crate) fn read_translated<B: PhysBus>(&mut self, bus: &mut B, va: u64, pa: PhysAddr, log2: u32) -> Result<u128, Fault> {
        if log2 == 4 {
            let lo = self.bus_read(bus, va, pa, 3)?;
            let hi = self.bus_read(bus, va.wrapping_add(8), PhysAddr::new(pa.get() + 8), 3)?;
            return Ok(u128::from(lo) | (u128::from(hi) << 64));
        }
        self.bus_read(bus, va, pa, log2).map(u128::from)
    }

    /// The write half of an atomic access: the low `1 << log2` bytes of
    /// `value` at `pa`, as [`Cpu::read_translated`] reads them.
    pub(crate) fn write_translated<B: PhysBus>(&mut self, bus: &mut B, va: u64, pa: PhysAddr, log2: u32, value: u128) -> Result<(), Fault> {
        if log2 == 4 {
            self.bus_write(bus, va, pa, 3, value as u64)?;
            return self.bus_write(bus, va.wrapping_add(8), PhysAddr::new(pa.get() + 8), 3, (value >> 64) as u64);
        }
        self.bus_write(bus, va, pa, log2, value as u64)
    }

    /// Read 16 bytes (a Q register) at `va`, as two 8-byte halves, each
    /// translated with the whole access's alignment (Arm `Mem[]`: a 16-byte
    /// SIMD&FP access is two single-copy atomic 8-byte accesses, each passed
    /// the access's own `aligned`).
    pub(crate) fn read_mem_u128<B: PhysBus>(&mut self, bus: &mut B, va: u64, acc: DataAccess) -> Result<u128, Fault> {
        if va & 15 != 0 && self.misaligned_faults(va, 16, acc) {
            return Err(Self::data_abort(va, Fsc::Alignment, false));
        }
        let half = Self::halves_of(va, acc);
        let lo = self.read_mem(bus, va, 3, half)?;
        let hi = self.read_mem(bus, va.wrapping_add(8), 3, half)?;
        Ok(u128::from(lo) | (u128::from(hi) << 64))
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
    /// reads them.
    pub(crate) fn write_mem_u128<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        value: u128,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        if va & 15 != 0 && self.misaligned_faults(va, 16, acc) {
            return Err(Self::data_abort(va, Fsc::Alignment, true));
        }
        let half = Self::halves_of(va, acc);
        self.write_mem(bus, va, 3, value as u64, half)?;
        self.write_mem(bus, va.wrapping_add(8), 3, (value >> 64) as u64, half)
    }

    /// `DC IVAC`, `DC CVAC`, `DC CVAU`, `DC CVAP`, `DC CVADP`, `DC CIVAC`
    /// and `IC IVAU` at `va`: the translation Arm `AArch64_DC` and
    /// `AArch64_IC` make, as an aligned access of no size, checked as `op`
    /// says. With no cache to clean or invalidate, the translation is all
    /// they do. A fault is a Data Abort at `va` with `CM` and `WnR` set
    /// (`AArch64_FaultSyndrome`); Device memory refuses none of them.
    pub(crate) fn cache_maintenance<B: PhysBus>(&mut self, bus: &mut B, va: u64, op: CacheMaintenance) -> Result<(), Fault> {
        let abort = |fsc: Fsc| Self::maintenance_abort(va, fsc);
        let regime = self.regime();
        if !self.s1_enabled(regime) {
            return match self.s1_disabled_output(regime, va, AccessKind::Read) {
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
        let (perms, level) = match self.tlb.hit(va, regime) {
            Some(entry) => (entry.perms, entry.level),
            None => {
                let walked = self.walk_and_cache(bus, va, AccessKind::Read, el).map_err(|f| abort(Fsc::from(f)))?;
                (walked.perms, walked.level)
            }
        };
        let needed = match (op, el) {
            (CacheMaintenance::Invalidate, _) => Some(AccessKind::Write),
            (CacheMaintenance::Clean, El::El0) => Some(AccessKind::Read),
            (CacheMaintenance::Clean, El::El1 | El::El2) | (CacheMaintenance::InstructionInvalidate, _) => None,
        };
        match needed {
            Some(kind) if !permitted(perms, kind, el) => Err(abort(Fsc::Permission(level))),
            _ => Ok(()),
        }
    }

    /// `DC ZVA`: zero the naturally aligned block containing `va`. Device
    /// memory refuses it whatever its alignment
    /// (`AArch64_S1HasAlignmentFaultDueToMemType`, `AccessType_DCZero`).
    pub(crate) fn dc_zva<B: PhysBus>(&mut self, bus: &mut B, va: u64) -> Result<(), Fault> {
        let base = va & !(id::DC_ZVA_BYTES - 1);
        let el = self.pstate.el;
        let t = self
            .translate(bus, base, AccessKind::Write, el, OnDevice::Refused)
            .map_err(|f| Self::data_abort(va, f, true))?;
        self.note_store(t.pa, id::DC_ZVA_BYTES);
        let clock = self.clock();
        for i in 0..id::DC_ZVA_BYTES / 8 {
            if bus.write(PhysAddr::new(t.pa + i * 8), AccessWidth::Dword, 0, clock).is_err() {
                return Err(self.external_abort(va, t.pa + i * 8, AccessKind::Write));
            }
        }
        Ok(())
    }

    /// A store to `pa` clears the local exclusive monitor if it overlaps the
    /// monitored granule (the IMPLEMENTATION DEFINED choice of Arm ARM
    /// B2.9.5 that every store by this processor clears it, which is what
    /// makes an interrupted `LDXR`/`STXR` sequence retry).
    #[inline]
    fn note_store(&mut self, pa: u64, bytes: u64) {
        let m = self.monitor;
        if m.armed && pa < m.addr + m.bytes && m.addr < pa + bytes {
            self.monitor.armed = false;
        }
    }

    /// The alignment check SVE `LDR` and `STR` make before their byte
    /// accesses: `if !aligned && AlignmentEnforced() then AArch64_Abort(
    /// AlignmentFault(accdesc, addr))`, `aligned` being the base register's
    /// alignment to the transfer's natural size, and the fault reported at
    /// `va`, the first byte's address.
    pub(crate) fn check_transfer_alignment(&self, aligned: bool, va: u64, write: bool) -> Result<(), Fault> {
        if !aligned && self.strict_alignment() {
            return Err(Self::data_abort(va, Fsc::Alignment, write));
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
            Err(Exception::new(ExceptionClass::SpAlignment, 0).into())
        } else {
            Ok(())
        }
    }
}
