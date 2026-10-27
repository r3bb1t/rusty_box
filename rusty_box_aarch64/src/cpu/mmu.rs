//! Stage-1 translation for the EL1&0 regime (VMSAv8-64, Arm ARM D8) with the
//! 4 KiB granule, the one `ID_AA64MMFR0_EL1` advertises; the TLB that caches
//! it; and the data and instruction access paths every load, store and fetch
//! goes through — alignment checks, translation, permission checks, and the
//! external abort when the bus has nothing at an address.
//!
//! The TLB is direct-mapped and holds one 4 KiB page per entry, whatever the
//! size of the block that mapped it. It is flushed whole by every TLB
//! maintenance instruction and by any write to a register a walk reads
//! (`SCTLR_EL1`, `TCR_EL1`, `TTBR0/1_EL1`, `MAIR_EL1`) — invalidation is never
//! narrower than the architecture requires, so ASIDs need no tagging.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::SnapError;

use super::dp::ones;
use super::exception::{Exception, ExceptionClass};
use super::exec::{Fault, StopCause};
use super::{id, Cpu, El};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus, PhysSpan};

/// What an access does, for permission checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessKind {
    Read,
    Write,
    Fetch,
}

/// The constraints a data access carries beyond its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DataAccess {
    /// `LDTR`/`STTR`: checked against EL0 permissions when executed at EL1.
    pub(crate) unprivileged: bool,
    /// Exclusive and acquire/release accesses fault when misaligned whatever
    /// the memory type (Arm ARM B2.5.2).
    pub(crate) strict_align: bool,
}

impl DataAccess {
    pub(crate) const NORMAL: Self = Self {
        unprivileged: false,
        strict_align: false,
    };
    pub(crate) const ORDERED: Self = Self {
        unprivileged: false,
        strict_align: true,
    };
    pub(crate) const UNPRIVILEGED: Self = Self {
        unprivileged: true,
        strict_align: false,
    };
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

const PERM_EL1_R: u8 = 1 << 0;
const PERM_EL1_W: u8 = 1 << 1;
const PERM_EL1_X: u8 = 1 << 2;
const PERM_EL0_R: u8 = 1 << 3;
const PERM_EL0_W: u8 = 1 << 4;
const PERM_EL0_X: u8 = 1 << 5;

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

/// The physical address size each `ID_AA64MMFR0_EL1.PARange` (and
/// `TCR_EL1.IPS`) encoding names (Arm ARM `AArch64.PAMax`; QEMU
/// target/arm/ptw.c `pamax_map`).
const PA_RANGE_BITS: [u32; 7] = [32, 36, 40, 42, 44, 48, 52];

/// `ID_AA64MMFR0_EL1.PARange`.
const PA_RANGE: usize = (id::ID_AA64MMFR0_EL1 & 0xF) as usize;
const _: () = assert!(PA_RANGE_BITS[PA_RANGE] == id::PA_BITS);

/// The output address size of a stage 1 walk under `tcr`: `TCR_EL1.IPS`,
/// bounded by the implemented PARange (QEMU target/arm/ptw.c
/// `get_phys_addr_lpae`).
fn output_address_bits(tcr: u64) -> u32 {
    let ips = ((tcr >> 32) & 7) as usize;
    PA_RANGE_BITS[ips.min(PA_RANGE)]
}

/// Every access permitted, as with stage 1 disabled.
const PERM_ALL: u8 = PERM_EL1_R | PERM_EL1_W | PERM_EL1_X | PERM_EL0_R | PERM_EL0_W | PERM_EL0_X;

/// The outcome of a translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Translated {
    pub(crate) pa: u64,
    /// Device memory: unaligned accesses fault, `DC ZVA` faults.
    pub(crate) device: bool,
}

/// What an address translates to before its permissions are checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Resolved {
    pa: u64,
    perms: u8,
    device: bool,
    /// The level of the descriptor, which a permission fault reports.
    level: u8,
}

impl Resolved {
    /// The translation, if the page permits an access of `kind` at `el`.
    #[inline(always)]
    fn permit(self, kind: AccessKind, el: El) -> Result<Translated, Fsc> {
        if !permitted(self.perms, kind, el) {
            return Err(Fsc::Permission(self.level));
        }
        Ok(Translated {
            pa: self.pa,
            device: self.device,
        })
    }
}

/// Whether a data access is aligned as Arm ARM `AArch64.CheckAlignment`
/// requires: a whole access to its own size, and every part of an access
/// made in parts to what its caller requires of each part. For an element
/// of an `LDP`/`STP` pair, general-purpose or SIMD&FP, that is the
/// element's own size, each element being an access of its own; for each
/// half of a Q register access, the whole 16 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fit {
    Aligned,
    Misaligned,
}

impl Fit {
    /// The fit of an access of `bytes` (a power of two) at `va`.
    #[inline(always)]
    pub(crate) const fn of(va: u64, bytes: u64) -> Self {
        if va & (bytes - 1) == 0 { Self::Aligned } else { Self::Misaligned }
    }
}

/// The two adjacent elements of a pair access: `first` at the lower
/// address, `second` just above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ElementPair {
    pub(crate) first: u64,
    pub(crate) second: u64,
}

/// A completed walk, before permissions are checked against an access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Walked {
    pa: u64,
    perms: u8,
    device: bool,
    level: u8,
    /// The `MAIR_EL1` attribute byte, as `PAR_EL1` reports it.
    attr: u8,
    /// The descriptor's SH[1:0] field (bits [9:8]).
    shareability: u8,
}

const TLB_ENTRIES: usize = 1024;
const INVALID_TAG: u64 = u64::MAX;

#[derive(Debug, Clone, Copy)]
struct TlbEntry {
    /// `va >> 12`, or `INVALID_TAG`.
    tag: u64,
    pa_page: u64,
    perms: u8,
    device: bool,
    level: u8,
}

const EMPTY_ENTRY: TlbEntry = TlbEntry {
    tag: INVALID_TAG,
    pa_page: 0,
    perms: 0,
    device: false,
    level: 0,
};

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

    /// The valid entries, for a snapshot.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        let valid = self.entries.iter().filter(|e| e.tag != INVALID_TAG).count();
        w.write_u32(valid as u32)?;
        for e in self.entries.iter().filter(|e| e.tag != INVALID_TAG) {
            w.write_u64(e.tag)?;
            w.write_u64(e.pa_page)?;
            w.write_bytes(&[e.perms, u8::from(e.device), e.level])?;
        }
        Ok(())
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let valid = r.read_count(TLB_ENTRIES)?;
        self.flush();
        for _ in 0..valid {
            let tag = r.read_u64()?;
            let pa_page = r.read_u64()?;
            let mut rest = [0u8; 3];
            r.read_bytes(&mut rest)?;
            let [perms, device, level] = rest;
            if tag == INVALID_TAG || pa_page & 0xFFF != 0 || perms >= 1 << 6 || device > 1 || level > 3 {
                return Err(SnapError::Invalid("TLB entry is not one a walk can produce"));
            }
            let slot = &mut self.entries[Self::slot(tag << 12)];
            if slot.tag != INVALID_TAG {
                return Err(SnapError::Invalid("two TLB entries claim one slot"));
            }
            *slot = TlbEntry {
                tag,
                pa_page,
                perms,
                device: device == 1,
                level,
            };
        }
        Ok(())
    }
}

impl Cpu {
    #[inline]
    fn mmu_enabled(&self) -> bool {
        self.sys.sctlr_el1 & 1 != 0
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
        self.sys.sctlr_el1 & 2 != 0
    }

    /// Translate `va` for an access of `kind` checked against `el`'s
    /// permissions.
    #[inline]
    pub(crate) fn translate<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        kind: AccessKind,
        el: El,
    ) -> Result<Translated, Fsc> {
        let r = self.resolve(bus, va, kind)?;
        r.permit(kind, el)
    }

    /// The translation of `va` when stage 1 is enabled and the TLB holds an
    /// entry for it that permits an access of `kind` at `el`: the one case
    /// that raises no fault of the walk or of the page's permissions.
    /// [`Cpu::translate_data`] and [`Cpu::fetch_translate`] try it inline,
    /// and make their checks in their order out of line only when it is
    /// `None`; [`Cpu::fetch_probe`] answers from it alone with stage 1
    /// enabled. Every other translation goes through [`Cpu::resolve`].
    #[inline(always)]
    fn permitted_hit(&self, va: u64, kind: AccessKind, el: El) -> Option<Translated> {
        if !self.mmu_enabled() {
            return None;
        }
        let entry = self.tlb.entries[Tlb::slot(va)];
        if entry.tag != va >> 12 || !permitted(entry.perms, kind, el) {
            return None;
        }
        Some(Translated {
            pa: entry.pa_page | (va & 0xFFF),
            device: entry.device,
        })
    }

    /// What `va` translates to, before any permission check: the walk's
    /// faults, but not the faults an access raises against the page's
    /// permissions or memory type, which a data access checks in the order
    /// Arm ARM `AArch64.S1Translate` gives them.
    #[inline(always)]
    fn resolve<B: PhysBus>(&mut self, bus: &mut B, va: u64, kind: AccessKind) -> Result<Resolved, Fsc> {
        if !self.mmu_enabled() {
            // Arm ARM D8.2.11 (`AArch64.TranslateAddressS1Off`): with stage 1
            // disabled the output address is the input address below
            // `AddrTop`, data accesses are Device-nGnRnE and instruction
            // fetches are Normal.
            let address = self.untagged(va);
            if address >> id::PA_BITS != 0 {
                return Err(Fsc::AddressSize(0));
            }
            return Ok(Resolved {
                pa: address,
                perms: PERM_ALL,
                device: kind != AccessKind::Fetch,
                level: 0,
            });
        }

        let entry = self.tlb.entries[Tlb::slot(va)];
        if entry.tag == va >> 12 {
            return Ok(Resolved {
                pa: entry.pa_page | (va & 0xFFF),
                perms: entry.perms,
                device: entry.device,
                level: entry.level,
            });
        }
        self.resolve_miss(bus, va)
    }

    /// A TLB miss: walk, and cache what the walk found.
    #[inline(never)]
    fn resolve_miss<B: PhysBus>(&mut self, bus: &mut B, va: u64) -> Result<Resolved, Fsc> {
        let slot = Tlb::slot(va);
        let walked = self.walk(bus, va)?;
        // The fill may evict the entry the running trace's fetches hit; the
        // next instruction's fetch translates again, as it would have.
        self.trace_break = true;
        // A tagged address keeps its tag in the entry's tag, so it hits only
        // itself; a TCR_EL1 write, which can change what the walk ignores,
        // empties the TLB.
        self.tlb.entries[slot] = TlbEntry {
            tag: va >> 12,
            pa_page: walked.pa & !0xFFF,
            perms: walked.perms,
            device: walked.device,
            level: walked.level,
        };
        Ok(Resolved {
            pa: walked.pa,
            perms: walked.perms,
            device: walked.device,
            level: walked.level,
        })
    }

    /// Whether top-byte-ignore covers `va` (Arm ARM `AddrTop`): TCR_EL1.TBI1
    /// for the upper half of the address space, TBI0 for the lower. ARMv8.0
    /// has no TBID, so the bit covers instruction addresses as well as data.
    #[inline]
    fn tbi(&self, va: u64) -> bool {
        let bit = if (va >> 55) & 1 == 1 { 38 } else { 37 };
        (self.sys.tcr_el1 >> bit) & 1 == 1
    }

    /// `va` with its tag cleared when top-byte-ignore covers it.
    #[inline]
    fn untagged(&self, va: u64) -> u64 {
        if self.tbi(va) { va & 0x00FF_FFFF_FFFF_FFFF } else { va }
    }

    /// Arm ARM `AArch64.BranchAddr`: the address a branch, an exception
    /// entry or an exception return writes to the PC. Under top-byte-ignore
    /// the tag is dropped, bits [63:56] becoming copies of bit 55 (the sign
    /// extension EL0 and EL1 use, and this core has no other level). QEMU
    /// does the same: `gen_a64_set_pc` (translate-a64.c) for branches and
    /// `HELPER(exception_return)` (helper-a64.c) for ERET.
    #[inline]
    pub(crate) fn branch_addr(&self, target: u64) -> u64 {
        if self.tbi(target) { (((target << 8) as i64) >> 8) as u64 } else { target }
    }

    /// The translation table walk (Arm ARM `AArch64.TranslationTableWalk`)
    /// for the 4 KiB granule, with hierarchical permissions and no hardware
    /// access-flag management (ARMv8.0).
    fn walk<B: PhysBus>(&self, bus: &mut B, va: u64) -> Result<Walked, TranslationFault> {
        let tcr = self.sys.tcr_el1;
        let upper = (va >> 55) & 1 == 1;
        let (txsz, epd, ttbr) = if upper {
            ((tcr >> 16) & 0x3F, (tcr >> 23) & 1 == 1, self.sys.ttbr1_el1)
        } else {
            (tcr & 0x3F, (tcr >> 7) & 1 == 1, self.sys.ttbr0_el1)
        };
        // T0SZ/T1SZ outside 16..=39 (`AArch64.S1MinTxSZ` and
        // `AArch64.MaxTxSZ` for the 4 KiB granule without FEAT_LPA2's
        // `TCR_EL1.DS` or FEAT_TTST): whether such a value is treated as the
        // nearest one in range or takes a level 0 Translation fault is
        // IMPLEMENTATION DEFINED (Arm ARM `AArch64.GetS1TTWParams`, the
        // ImpDef booleans "Fault on TxSZ value above maximum" and "...
        // below minimum"; `AArch64.S1InvalidTxSZ`). This takes the fault, as
        // QEMU does (target/arm/ptw.c `get_phys_addr_lpae`,
        // `param.tsz_oob`).
        if !(16..=39).contains(&txsz) {
            return Err(TranslationFault::Translation { level: 0 });
        }
        let ia_bits = 64 - txsz as u32;
        let top = if self.tbi(va) { 56 } else { 64 };
        let check = ones(top - ia_bits) << ia_bits;
        if va & check != if upper { check } else { 0 } {
            return Err(TranslationFault::Translation { level: 0 });
        }
        if epd {
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
                Err(BusFault::Unmapped) => return Err(TranslationFault::ExternalAbortOnWalk { level: at_level }),
            };
            if desc & 1 == 0 {
                return Err(TranslationFault::Translation { level: at_level });
            }
            if level < 3 && desc & 2 != 0 {
                let next = desc & 0x0000_FFFF_FFFF_F000;
                if next >> output_bits != 0 {
                    return Err(TranslationFault::AddressSize { level: at_level });
                }
                ap_table |= (desc >> 61) & 3;
                uxn_table |= (desc >> 60) & 1 == 1;
                pxn_table |= (desc >> 59) & 1 == 1;
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
            let wxn = (self.sys.sctlr_el1 >> 19) & 1 == 1;
            // EL1 never executes memory EL0 can write (Arm ARM D8.4.1.2), and
            // WXN makes every writable region execute-never at its own EL.
            let el1_x = !pxn && !el0_w && !(wxn && el1_w);
            let el0_x = !uxn && !(wxn && el0_w);
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
            let attr = ((self.sys.mair_el1 >> (((desc >> 2) & 7) * 8)) & 0xFF) as u8;
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

    /// Where data address `va` translates to now, for a debugger: the walk a
    /// data access would make, with no permission check, no TLB lookup or
    /// fill, and descriptors read through `bus` — which a caller makes a view
    /// that cannot disturb a device. Nothing in the processor changes.
    pub(crate) fn debug_translate<B: PhysBus>(&self, bus: &mut B, va: u64) -> Result<u64, TranslationFault> {
        if !self.mmu_enabled() {
            let address = self.untagged(va);
            if address >> id::PA_BITS != 0 {
                return Err(TranslationFault::AddressSize { level: 0 });
            }
            return Ok(address);
        }
        self.walk(bus, va).map(|w| w.pa)
    }

    /// `AT S1E1R/S1E1W/S1E0R/S1E0W`: translate and report in `PAR_EL1`,
    /// in the 64-bit format QEMU's `do_ats_write` (target/arm/tcg/cpregs-at.c)
    /// builds. A synchronous external abort on the walk is not reported
    /// there but taken as a Data Abort (Arm ARM `AArch64.AT`), as QEMU takes
    /// it: CM set, for an address translation instruction, and WnR set.
    pub(crate) fn address_translate<B: PhysBus>(&mut self, bus: &mut B, va: u64, write: bool, el: El) -> Result<(), Fault> {
        const RES1: u64 = 1 << 11;
        // This core runs in Non-secure state, as QEMU's `virt` does without
        // `secure=on`, so a successful translation reports NS = 1.
        const NS: u64 = 1 << 9;
        let kind = if write { AccessKind::Write } else { AccessKind::Read };
        let result = if self.mmu_enabled() {
            match self.walk(bus, va) {
                Ok(w) if permitted(w.perms, kind, el) => Ok((w.pa, w.attr, w.shareability)),
                Ok(w) => Err(Fsc::Permission(w.level)),
                Err(f) => Err(Fsc::from(f)),
            }
        } else {
            let address = self.untagged(va);
            if address >> id::PA_BITS != 0 {
                Err(Fsc::AddressSize(0))
            } else {
                // Device-nGnRnE (attribute 0x00) with stage 1 disabled.
                Ok((address, 0, 0))
            }
        };
        if let Err(fsc @ Fsc::SyncExternalOnWalk(_)) = result {
            const CM: u32 = 1 << 8;
            const WNR: u32 = 1 << 6;
            return Err(Exception::new(ExceptionClass::DataAbort, fsc.code() | CM | WNR).with_far(va).into());
        }
        self.sys.par_el1 = match result {
            Ok((pa, attr, shareability)) => {
                // Arm ARM `PAREncodeShareability`, as QEMU's
                // `par_el1_shareability`: Device and Normal Non-cacheable
                // memory report Outer Shareable.
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
        let iss = fsc.code() | (u32::from(write) << 6);
        Exception::new(ExceptionClass::DataAbort, iss).with_far(va).into()
    }

    #[cold]
    #[inline(never)]
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

    /// The physical address of the instruction at `pc` (already known to be
    /// aligned), or the Instruction Abort its translation raises.
    #[inline(always)]
    pub(crate) fn fetch_translate<B: PhysBus>(&mut self, bus: &mut B, pc: u64) -> Result<u64, Fault> {
        match self.permitted_hit(pc, AccessKind::Fetch, self.pstate.el) {
            Some(t) => Ok(t.pa),
            None => self.fetch_translate_checked(bus, pc),
        }
    }

    /// The physical address a fetch at `pc` reads now, when the fetch raises
    /// no exception and needs no walk: `None` on `PSTATE.IL`, a misaligned
    /// `pc`, or anything but a TLB hit that permits a fetch at the current
    /// EL. With stage 1 disabled it is [`Cpu::translate`]'s
    /// `AArch64.TranslateAddressS1Off` address. It never fills, flushes or
    /// marks anything.
    pub(crate) fn fetch_probe(&self, pc: u64) -> Option<u64> {
        if self.pstate.il || pc & 3 != 0 {
            return None;
        }
        if !self.mmu_enabled() {
            let address = self.untagged(pc);
            return (address >> id::PA_BITS == 0).then_some(address);
        }
        self.permitted_hit(pc, AccessKind::Fetch, self.pstate.el).map(|t| t.pa)
    }

    /// [`Cpu::fetch_translate`] with every check made: stage 1 disabled, a
    /// TLB miss, or a page that does not permit the fetch.
    #[cold]
    #[inline(never)]
    fn fetch_translate_checked<B: PhysBus>(&mut self, bus: &mut B, pc: u64) -> Result<u64, Fault> {
        let el = self.pstate.el;
        self.translate(bus, pc, AccessKind::Fetch, el)
            .map(|t| t.pa)
            .map_err(|f| Fault::from(Exception::new(ExceptionClass::InstructionAbort, f.code()).with_far(pc)))
    }

    /// Read the instruction word at `pa`, which `pc` translated to.
    #[inline]
    pub(crate) fn fetch_word<B: PhysBus>(&mut self, bus: &mut B, pc: u64, pa: u64) -> Result<u32, Fault> {
        match bus.fetch(PhysAddr::new(pa), self.clock()) {
            Ok(word) => Ok(word),
            Err(BusFault::Unmapped) => Err(self.external_abort(pc, pa, AccessKind::Fetch)),
        }
    }

    #[inline]
    fn access_el(&self, acc: DataAccess) -> El {
        if acc.unprivileged {
            El::El0
        } else {
            self.pstate.el
        }
    }

    /// Alignment checks that precede translation, then translation, then
    /// the faults the page raises, in Arm ARM `AArch64.S1Translate`'s order:
    /// the Device-memory alignment fault (`AArch64.S1HasAlignmentFault`)
    /// before the permission fault. For an access, or a part of a larger
    /// one, that stays in one page, with the alignment `fit` its caller
    /// requires of it ([`Fit`]).
    ///
    /// A permitted TLB hit that is aligned, or is on Normal memory with no
    /// alignment check enabled, raises none of these faults, and is answered
    /// inline; everything else makes the checks in order out of line.
    #[inline(always)]
    fn translate_data<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        fit: Fit,
        kind: AccessKind,
        acc: DataAccess,
    ) -> Result<Translated, Fault> {
        if let Some(t) = self.permitted_hit(va, kind, self.access_el(acc)) {
            if fit == Fit::Aligned || !(t.device || acc.strict_align || self.strict_alignment()) {
                return Ok(t);
            }
        }
        self.translate_data_checked(bus, va, fit, kind, acc)
    }

    /// [`Cpu::translate_data`] with every check made, in order.
    #[cold]
    #[inline(never)]
    fn translate_data_checked<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        fit: Fit,
        kind: AccessKind,
        acc: DataAccess,
    ) -> Result<Translated, Fault> {
        let write = kind == AccessKind::Write;
        if fit == Fit::Misaligned && (acc.strict_align || self.strict_alignment()) {
            return Err(Self::data_abort(va, Fsc::Alignment, write));
        }
        let el = self.access_el(acc);
        let r = self.resolve(bus, va, kind).map_err(|f| Self::data_abort(va, f, write))?;
        if fit == Fit::Misaligned && r.device {
            return Err(Self::data_abort(va, Fsc::Alignment, write));
        }
        r.permit(kind, el).map_err(|f| Self::data_abort(va, f, write))
    }

    /// Translate one byte of an access that crosses a page boundary, and so
    /// is misaligned. Arm ARM `Mem[]` makes such an access one byte at a
    /// time, each with the whole access's alignment, so a Device page faults
    /// on alignment at the first byte that reaches it, before its
    /// permissions are checked.
    fn translate_byte<B: PhysBus>(&mut self, bus: &mut B, va: u64, kind: AccessKind, el: El) -> Result<Translated, Fault> {
        let write = kind == AccessKind::Write;
        let r = self.resolve(bus, va, kind).map_err(|f| Self::data_abort(va, f, write))?;
        if r.device {
            return Err(Self::data_abort(va, Fsc::Alignment, write));
        }
        r.permit(kind, el).map_err(|f| Self::data_abort(va, f, write))
    }

    /// The physical address an aligned access of `bytes` at `va` would reach,
    /// with every check of a real access applied. For the exclusive monitor.
    pub(crate) fn translate_for<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        write: bool,
        acc: DataAccess,
    ) -> Result<u64, Fault> {
        let kind = if write { AccessKind::Write } else { AccessKind::Read };
        self.translate_data(bus, va, Fit::of(va, bytes), kind, acc).map(|t| t.pa)
    }

    /// Read `1 << log2` bytes (`log2` in 0..=3) at `va`, zero-extended. An
    /// access within one page is inlined into the instruction that makes
    /// it; one that crosses a page boundary is out of line.
    #[inline(always)]
    pub(crate) fn read_mem<B: PhysBus>(&mut self, bus: &mut B, va: u64, log2: u32, acc: DataAccess) -> Result<u64, Fault> {
        self.read_part(bus, va, log2, Fit::of(va, 1 << log2), acc)
    }

    /// Read `1 << log2` bytes at `va`, with the alignment `fit` its caller
    /// requires of it: a whole access, or a part of a larger one.
    #[inline(always)]
    fn read_part<B: PhysBus>(&mut self, bus: &mut B, va: u64, log2: u32, fit: Fit, acc: DataAccess) -> Result<u64, Fault> {
        let bytes = 1u64 << log2;
        if (va & 0xFFF) + bytes <= 0x1000 {
            let t = self.translate_data(bus, va, fit, AccessKind::Read, acc)?;
            let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
            return self.read_translated(bus, va, t.pa, width);
        }
        self.read_mem_across_pages(bus, va, bytes, acc)
    }

    /// Read `width` at `pa`, which `va` translated to, zero-extended; an
    /// address nothing answers is the external abort of `va`.
    #[inline(always)]
    fn read_translated<B: PhysBus>(&mut self, bus: &mut B, va: u64, pa: u64, width: AccessWidth) -> Result<u64, Fault> {
        match bus.read(PhysAddr::new(pa), width, self.clock()) {
            Ok(v) => Ok(v & width.value_mask()),
            Err(BusFault::Unmapped) => Err(self.external_abort(va, pa, AccessKind::Read)),
        }
    }

    /// Read two adjacent elements of `1 << log2` bytes (`log2` in 0..=3),
    /// at `va` and `va + (1 << log2)`, each zero-extended. `fit` is the
    /// alignment the caller requires of each element: for Arm ARM `LDP`,
    /// the element's own, as `LDP` makes its elements' accesses
    /// `Mem[address, dbytes]` then `Mem[address + dbytes, dbytes]`, each
    /// aligned to `dbytes` exactly when `address` is.
    ///
    /// The result is that of two reads of the elements made in order: the
    /// same values, and the same fault with the same address. A pair within
    /// one page translates once, as the second translation would find what
    /// the first did: the second element is on the first's page, with the
    /// first's `fit`, and the first translation leaves the page in the TLB
    /// (or, with stage 1 disabled, maps the page to itself). Each element
    /// still takes its own external abort.
    #[inline(always)]
    pub(crate) fn read_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        fit: Fit,
        acc: DataAccess,
    ) -> Result<ElementPair, Fault> {
        let bytes = 1u64 << log2;
        if (va & 0xFFF) + 2 * bytes <= 0x1000 {
            let t = self.translate_data(bus, va, fit, AccessKind::Read, acc)?;
            let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
            let first = self.read_translated(bus, va, t.pa, width)?;
            let second = self.read_translated(bus, va + bytes, t.pa + bytes, width)?;
            return Ok(ElementPair { first, second });
        }
        self.read_pair_across_pages(bus, va, log2, fit, acc)
    }

    /// A [`Cpu::read_pair`] that does not fit in one page: each element is
    /// read on its own.
    #[inline(never)]
    fn read_pair_across_pages<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        fit: Fit,
        acc: DataAccess,
    ) -> Result<ElementPair, Fault> {
        let first = self.read_part(bus, va, log2, fit, acc)?;
        let second = self.read_part(bus, va.wrapping_add(1 << log2), log2, fit, acc)?;
        Ok(ElementPair { first, second })
    }

    /// A read of `bytes` at `va` that crosses a page boundary.
    #[inline(never)]
    fn read_mem_across_pages<B: PhysBus>(&mut self, bus: &mut B, va: u64, bytes: u64, acc: DataAccess) -> Result<u64, Fault> {
        // Crosses a page boundary, so it is misaligned: Normal memory only.
        // Each byte is an access of its own (Arm ARM `AArch64.MemSingle` on
        // an unaligned address), so a fault reports the first byte that
        // faulted — an address in the page that needs mapping, which is what
        // an OS's fault handler goes by. The SCTLR_EL1.A check is the whole
        // access's, and reports its address.
        if acc.strict_align || self.strict_alignment() {
            return Err(Self::data_abort(va, Fsc::Alignment, false));
        }
        let el = self.access_el(acc);
        let mut value = 0u64;
        for i in 0..bytes {
            let byte_va = va.wrapping_add(i);
            let t = self.translate_byte(bus, byte_va, AccessKind::Read, el)?;
            let b = match bus.read(PhysAddr::new(t.pa), AccessWidth::Byte, self.clock()) {
                Ok(b) => b & 0xFF,
                Err(BusFault::Unmapped) => return Err(self.external_abort(byte_va, t.pa, AccessKind::Read)),
            };
            value |= b << (8 * i);
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
        self.write_part(bus, va, log2, value, Fit::of(va, 1 << log2), acc)
    }

    /// Write the low `1 << log2` bytes of `value` at `va`, with the
    /// alignment `fit` its caller requires of it: a whole access, or a part
    /// of a larger one.
    #[inline(always)]
    fn write_part<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        value: u64,
        fit: Fit,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        let bytes = 1u64 << log2;
        if (va & 0xFFF) + bytes <= 0x1000 {
            let t = self.translate_data(bus, va, fit, AccessKind::Write, acc)?;
            let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
            return self.write_translated(bus, va, t.pa, width, value);
        }
        self.write_mem_across_pages(bus, va, bytes, value, acc)
    }

    /// Write the low `width` of `value` at `pa`, which `va` translated to,
    /// after [`Cpu::note_store`]; an address nothing answers is the external
    /// abort of `va`.
    #[inline(always)]
    fn write_translated<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        pa: u64,
        width: AccessWidth,
        value: u64,
    ) -> Result<(), Fault> {
        self.note_store(pa, width.bytes());
        match bus.write(PhysAddr::new(pa), width, value & width.value_mask(), self.clock()) {
            Ok(()) => Ok(()),
            Err(BusFault::Unmapped) => Err(self.external_abort(va, pa, AccessKind::Write)),
        }
    }

    /// Write the low `1 << log2` bytes (`log2` in 0..=3) of each element of
    /// `value`, `first` at `va` and `second` at `va + (1 << log2)`, in Arm
    /// ARM `STP`'s order. `fit` is the alignment the caller requires of
    /// each element: for `STP`, the element's own, as for [`Cpu::read_pair`].
    ///
    /// The result is that of two writes of the elements made in order: the
    /// same fault with the same address, and the same stores noted
    /// ([`Cpu::note_store`]) in the same order, each just before its
    /// element is written. A pair within one page translates once, for the
    /// reasons [`Cpu::read_pair`] gives.
    #[inline(always)]
    pub(crate) fn write_pair<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        value: ElementPair,
        fit: Fit,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        let bytes = 1u64 << log2;
        if (va & 0xFFF) + 2 * bytes <= 0x1000 {
            let t = self.translate_data(bus, va, fit, AccessKind::Write, acc)?;
            let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
            self.write_translated(bus, va, t.pa, width, value.first)?;
            return self.write_translated(bus, va + bytes, t.pa + bytes, width, value.second);
        }
        self.write_pair_across_pages(bus, va, log2, value, fit, acc)
    }

    /// A [`Cpu::write_pair`] that does not fit in one page: each element is
    /// written on its own.
    #[inline(never)]
    fn write_pair_across_pages<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        log2: u32,
        value: ElementPair,
        fit: Fit,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        self.write_part(bus, va, log2, value.first, fit, acc)?;
        self.write_part(bus, va.wrapping_add(1 << log2), log2, value.second, fit, acc)
    }

    /// A write of the low `bytes` bytes of `value` at `va` that crosses a
    /// page boundary.
    #[inline(never)]
    fn write_mem_across_pages<B: PhysBus>(&mut self, bus: &mut B, va: u64, bytes: u64, value: u64, acc: DataAccess) -> Result<(), Fault> {
        if acc.strict_align || self.strict_alignment() {
            return Err(Self::data_abort(va, Fsc::Alignment, true));
        }
        // Translate both pages before writing anything, so a fault on the
        // second page leaves memory untouched. As for a read, a fault reports
        // the first byte that faulted: the access's own address in the first
        // page, the start of the second page in the second.
        let el = self.access_el(acc);
        self.probe_pages(bus, va, bytes, el)?;
        for i in 0..bytes {
            let byte_va = va.wrapping_add(i);
            let t = self.translate_byte(bus, byte_va, AccessKind::Write, el)?;
            self.note_store(t.pa, 1);
            if bus
                .write(PhysAddr::new(t.pa), AccessWidth::Byte, (value >> (8 * i)) & 0xFF, self.clock())
                .is_err()
            {
                return Err(self.external_abort(byte_va, t.pa, AccessKind::Write));
            }
        }
        Ok(())
    }

    /// Translate, for a write, the first byte of an access of `bytes` at `va`
    /// that crosses a page boundary and the first byte on its second page,
    /// so that a fault on either is taken before anything is written.
    fn probe_pages<B: PhysBus>(&mut self, bus: &mut B, va: u64, bytes: u64, el: El) -> Result<(), Fault> {
        let second_page = va.wrapping_add(bytes - 1) & !0xFFF;
        for probe in [va, second_page] {
            self.translate_byte(bus, probe, AccessKind::Write, el)?;
        }
        Ok(())
    }

    /// Read 16 bytes (a Q register) at `va`, as two 8-byte halves. Alignment
    /// is judged on the whole access (Arm ARM `Mem[]` checks
    /// `AArch64.CheckAlignment(address, 16, ...)`), so on Device memory, and
    /// with stage 1 disabled, both halves fault unless `va` is 16-byte
    /// aligned; QEMU 11.1.2 checks the same (`finalize_memop_atom` gives
    /// every access `MO_ALIGN_TLB_ONLY`, which `target/arm/ptw.c` checks on
    /// Device pages at the access's whole size).
    pub(crate) fn read_mem_u128<B: PhysBus>(&mut self, bus: &mut B, va: u64, acc: DataAccess) -> Result<u128, Fault> {
        let fit = Fit::of(va, 16);
        if fit == Fit::Misaligned && (acc.strict_align || self.strict_alignment()) {
            return Err(Self::data_abort(va, Fsc::Alignment, false));
        }
        let halves = self.read_pair(bus, va, 3, fit, acc)?;
        Ok(u128::from(halves.first) | (u128::from(halves.second) << 64))
    }

    /// Write 16 bytes (a Q register) at `va`, as two 8-byte halves, with
    /// alignment judged as for a read. An access across a page boundary
    /// translates both pages before either half is written, as a scalar
    /// store across pages does.
    pub(crate) fn write_mem_u128<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        value: u128,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        let fit = Fit::of(va, 16);
        if fit == Fit::Misaligned && (acc.strict_align || self.strict_alignment()) {
            return Err(Self::data_abort(va, Fsc::Alignment, true));
        }
        if (va & 0xFFF) + 16 > 0x1000 {
            let el = self.access_el(acc);
            self.probe_pages(bus, va, 16, el)?;
        }
        let halves = ElementPair {
            first: value as u64,
            second: (value >> 64) as u64,
        };
        self.write_pair(bus, va, 3, halves, fit, acc)
    }

    /// `DC ZVA`: zero the naturally aligned block containing `va`. On
    /// Device memory it always faults on alignment, ahead of the page's
    /// permissions (Arm ARM `AArch64.S1HasAlignmentFault`, for
    /// `AccType_DCZVA`).
    pub(crate) fn dc_zva<B: PhysBus>(&mut self, bus: &mut B, va: u64) -> Result<(), Fault> {
        let base = va & !(id::DC_ZVA_BYTES - 1);
        let el = self.pstate.el;
        let r = self
            .resolve(bus, base, AccessKind::Write)
            .map_err(|f| Self::data_abort(va, f, true))?;
        if r.device {
            return Err(Self::data_abort(va, Fsc::Alignment, true));
        }
        let t = r.permit(AccessKind::Write, el).map_err(|f| Self::data_abort(va, f, true))?;
        self.note_store(t.pa, id::DC_ZVA_BYTES);
        let clock = self.clock();
        for i in 0..id::DC_ZVA_BYTES / 8 {
            if bus.write(PhysAddr::new(t.pa + i * 8), AccessWidth::Dword, 0, clock).is_err() {
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

    /// The SP alignment check (`SCTLR_EL1.SA`/`SA0`) for an access whose
    /// base register is SP.
    #[inline]
    pub(crate) fn check_sp_alignment(&self, base_is_sp: bool, base: u64) -> Result<(), Fault> {
        if !base_is_sp || base & 0xF == 0 {
            return Ok(());
        }
        let enabled = match self.pstate.el {
            El::El1 => self.sys.sctlr_el1 & (1 << 3) != 0,
            El::El0 => self.sys.sctlr_el1 & (1 << 4) != 0,
        };
        if enabled {
            Err(Exception::new(ExceptionClass::SpAlignment, 0).into())
        } else {
            Ok(())
        }
    }
}
