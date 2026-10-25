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

use super::dp::ones;
use super::exception::{Exception, ExceptionClass};
use super::exec::{Fault, StopCause};
use super::{id, Cpu, El};
use crate::bus::{AccessWidth, BusFault, PhysAddr, PhysBus};

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

/// An instruction word and the physical address it was fetched from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fetched {
    pub(crate) word: u32,
    pub(crate) pa: u64,
}

/// The outcome of a translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Translated {
    pub(crate) pa: u64,
    /// Device memory: unaligned accesses fault, `DC ZVA` faults.
    pub(crate) device: bool,
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
}

impl Cpu {
    #[inline]
    fn mmu_enabled(&self) -> bool {
        self.sys.sctlr_el1 & 1 != 0
    }

    /// `SCTLR_EL1.A`: every misaligned data access faults.
    #[inline]
    fn strict_alignment(&self) -> bool {
        self.sys.sctlr_el1 & 2 != 0
    }

    /// Translate `va` for an access of `kind` checked against `el`'s
    /// permissions. `data` selects whether top-byte-ignore applies.
    pub(crate) fn translate<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        kind: AccessKind,
        el: El,
    ) -> Result<Translated, Fsc> {
        if !self.mmu_enabled() {
            // Arm ARM D8.2.11: with stage 1 disabled the output address is
            // the input address, data accesses are Device-nGnRnE and
            // instruction fetches are Normal.
            if va >> id::PA_BITS != 0 {
                return Err(Fsc::AddressSize(0));
            }
            return Ok(Translated {
                pa: va,
                device: kind != AccessKind::Fetch,
            });
        }

        let slot = Tlb::slot(va);
        let entry = self.tlb.entries[slot];
        if entry.tag == va >> 12 {
            if !permitted(entry.perms, kind, el) {
                return Err(Fsc::Permission(entry.level));
            }
            return Ok(Translated {
                pa: entry.pa_page | (va & 0xFFF),
                device: entry.device,
            });
        }

        let walked = self.walk(bus, va, kind != AccessKind::Fetch)?;
        // Only addresses with a canonical top byte are cached: a tagged data
        // address (TBI) and the untagged fetch of the same page must not
        // share an entry, because top-byte-ignore applies to data only.
        let canonical = {
            let top = va >> 55;
            top == 0 || top == 0x1FF
        };
        if canonical {
            self.tlb.entries[slot] = TlbEntry {
                tag: va >> 12,
                pa_page: walked.pa & !0xFFF,
                perms: walked.perms,
                device: walked.device,
                level: walked.level,
            };
        }
        if !permitted(walked.perms, kind, el) {
            return Err(Fsc::Permission(walked.level));
        }
        Ok(Translated {
            pa: walked.pa,
            device: walked.device,
        })
    }

    /// The translation table walk (Arm ARM `AArch64.TranslationTableWalk`)
    /// for the 4 KiB granule, with hierarchical permissions and no hardware
    /// access-flag management (ARMv8.0).
    fn walk<B: PhysBus>(&mut self, bus: &mut B, va: u64, data: bool) -> Result<Walked, Fsc> {
        let tcr = self.sys.tcr_el1;
        let upper = (va >> 55) & 1 == 1;
        let (txsz, tbi, epd, ttbr) = if upper {
            ((tcr >> 16) & 0x3F, (tcr >> 38) & 1 == 1, (tcr >> 23) & 1 == 1, self.sys.ttbr1_el1)
        } else {
            (tcr & 0x3F, (tcr >> 37) & 1 == 1, (tcr >> 7) & 1 == 1, self.sys.ttbr0_el1)
        };
        // T0SZ/T1SZ outside 16..=39 behave as the nearest supported value
        // (Arm ARM D8.1.6, an IMPLEMENTATION DEFINED choice made here).
        let txsz = txsz.clamp(16, 39) as u32;
        let ia_bits = 64 - txsz;
        let top = if tbi && data { 56 } else { 64 };
        let check = ones(top - ia_bits) << ia_bits;
        if va & check != if upper { check } else { 0 } {
            return Err(Fsc::Translation(0));
        }
        if epd {
            return Err(Fsc::Translation(0));
        }

        let start_level: u32 = if ia_bits > 39 {
            0
        } else if ia_bits > 30 {
            1
        } else {
            2
        };
        let start_bits = ia_bits - (12 + 9 * (3 - start_level));
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
            let desc = match bus.read(PhysAddr::new(table + index * 8), AccessWidth::Dword, clock) {
                Ok(d) => d,
                Err(BusFault::Unmapped) => return Err(Fsc::SyncExternalOnWalk(level as u8)),
            };
            if desc & 1 == 0 {
                return Err(Fsc::Translation(level as u8));
            }
            if level < 3 && desc & 2 != 0 {
                let next = desc & 0x0000_FFFF_FFFF_F000;
                if next >> id::PA_BITS != 0 {
                    return Err(Fsc::AddressSize(level as u8));
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
                return Err(Fsc::Translation(level as u8));
            }
            let out = desc & 0x0000_FFFF_FFFF_F000 & !ones(shift);
            if out >> id::PA_BITS != 0 {
                return Err(Fsc::AddressSize(level as u8));
            }
            if (desc >> 10) & 1 == 0 {
                return Err(Fsc::AccessFlag(level as u8));
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
            });
        }
    }

    /// `AT S1E1R/S1E1W/S1E0R/S1E0W`: translate and report in `PAR_EL1`.
    pub(crate) fn address_translate<B: PhysBus>(&mut self, bus: &mut B, va: u64, write: bool, el: El) {
        let kind = if write { AccessKind::Write } else { AccessKind::Read };
        let result = if self.mmu_enabled() {
            match self.walk(bus, va, true) {
                Ok(w) if permitted(w.perms, kind, el) => Ok((w.pa, u64::from(w.attr))),
                Ok(w) => Err(Fsc::Permission(w.level)),
                Err(f) => Err(f),
            }
        } else if va >> id::PA_BITS != 0 {
            Err(Fsc::AddressSize(0))
        } else {
            // Device-nGnRnE (attribute 0x00) with stage 1 disabled.
            Ok((va, 0))
        };
        self.sys.par_el1 = match result {
            Ok((pa, attr)) => (attr << 56) | (pa & 0x0000_FFFF_FFFF_F000),
            // F = 1, FST, bit 11 RES1. PTW and S are 0: stage 1, not on a walk
            // of a stage-2 table.
            Err(f) => (1 << 11) | (u64::from(f.code()) << 1) | 1,
        };
    }

    // ----- access paths -----

    fn data_abort(va: u64, fsc: Fsc, write: bool) -> Fault {
        let iss = fsc.code() | (u32::from(write) << 6);
        Exception::new(ExceptionClass::DataAbort, iss).with_far(va).into()
    }

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
        let t = self
            .translate(bus, pc, AccessKind::Fetch, el)
            .map_err(|f| Fault::from(Exception::new(ExceptionClass::InstructionAbort, f.code()).with_far(pc)))?;
        match bus.read(PhysAddr::new(t.pa), AccessWidth::Word, self.clock()) {
            Ok(word) => Ok(Fetched {
                word: word as u32,
                pa: t.pa,
            }),
            Err(BusFault::Unmapped) => Err(self.external_abort(pc, t.pa, AccessKind::Fetch)),
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

    /// Alignment checks that precede translation, then translation and the
    /// Device-memory alignment check, for an access that stays in one page.
    fn translate_data<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        bytes: u64,
        kind: AccessKind,
        acc: DataAccess,
    ) -> Result<Translated, Fault> {
        let write = kind == AccessKind::Write;
        let misaligned = va & (bytes - 1) != 0;
        if misaligned && (acc.strict_align || self.strict_alignment()) {
            return Err(Self::data_abort(va, Fsc::Alignment, write));
        }
        let el = self.access_el(acc);
        let t = self.translate(bus, va, kind, el).map_err(|f| Self::data_abort(va, f, write))?;
        if misaligned && t.device {
            return Err(Self::data_abort(va, Fsc::Alignment, write));
        }
        Ok(t)
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
        self.translate_data(bus, va, bytes, kind, acc).map(|t| t.pa)
    }

    /// Read `1 << log2` bytes (`log2` in 0..=3) at `va`, zero-extended.
    pub(crate) fn read_mem<B: PhysBus>(&mut self, bus: &mut B, va: u64, log2: u32, acc: DataAccess) -> Result<u64, Fault> {
        let bytes = 1u64 << log2;
        if (va & 0xFFF) + bytes <= 0x1000 {
            let t = self.translate_data(bus, va, bytes, AccessKind::Read, acc)?;
            let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
            return match bus.read(PhysAddr::new(t.pa), width, self.clock()) {
                Ok(v) => Ok(v & width.value_mask()),
                Err(BusFault::Unmapped) => Err(self.external_abort(va, t.pa, AccessKind::Read)),
            };
        }
        // Crosses a page boundary, so it is misaligned: Normal memory only.
        // Each byte is translated on its own; a fault reports the access's
        // own address.
        if acc.strict_align || self.strict_alignment() {
            return Err(Self::data_abort(va, Fsc::Alignment, false));
        }
        let el = self.access_el(acc);
        let mut value = 0u64;
        for i in 0..bytes {
            let byte_va = va.wrapping_add(i);
            let t = self
                .translate(bus, byte_va, AccessKind::Read, el)
                .map_err(|f| Self::data_abort(va, f, false))?;
            if t.device {
                return Err(Self::data_abort(va, Fsc::Alignment, false));
            }
            let b = match bus.read(PhysAddr::new(t.pa), AccessWidth::Byte, self.clock()) {
                Ok(b) => b & 0xFF,
                Err(BusFault::Unmapped) => return Err(self.external_abort(va, t.pa, AccessKind::Read)),
            };
            value |= b << (8 * i);
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
            let t = self.translate_data(bus, va, bytes, AccessKind::Write, acc)?;
            self.note_store(t.pa, bytes);
            let width = AccessWidth::from_log2(log2).unwrap_or(AccessWidth::Dword);
            return match bus.write(PhysAddr::new(t.pa), width, value & width.value_mask(), self.clock()) {
                Ok(()) => Ok(()),
                Err(BusFault::Unmapped) => Err(self.external_abort(va, t.pa, AccessKind::Write)),
            };
        }
        if acc.strict_align || self.strict_alignment() {
            return Err(Self::data_abort(va, Fsc::Alignment, true));
        }
        // Translate both pages before writing anything, so a fault on the
        // second page leaves memory untouched.
        let el = self.access_el(acc);
        let last = va.wrapping_add(bytes - 1);
        for probe in [va, last] {
            let t = self
                .translate(bus, probe, AccessKind::Write, el)
                .map_err(|f| Self::data_abort(va, f, true))?;
            if t.device {
                return Err(Self::data_abort(va, Fsc::Alignment, true));
            }
        }
        for i in 0..bytes {
            let byte_va = va.wrapping_add(i);
            let t = self
                .translate(bus, byte_va, AccessKind::Write, el)
                .map_err(|f| Self::data_abort(va, f, true))?;
            self.note_store(t.pa, 1);
            if bus
                .write(PhysAddr::new(t.pa), AccessWidth::Byte, (value >> (8 * i)) & 0xFF, self.clock())
                .is_err()
            {
                return Err(self.external_abort(va, t.pa, AccessKind::Write));
            }
        }
        Ok(())
    }

    /// Read 16 bytes (a Q register) at `va`, as two 8-byte halves. Alignment
    /// is judged on the whole access.
    pub(crate) fn read_mem_u128<B: PhysBus>(&mut self, bus: &mut B, va: u64, acc: DataAccess) -> Result<u128, Fault> {
        if va & 15 != 0 && (acc.strict_align || self.strict_alignment()) {
            return Err(Self::data_abort(va, Fsc::Alignment, false));
        }
        let lo = self.read_mem(bus, va, 3, acc)?;
        let hi = self.read_mem(bus, va.wrapping_add(8), 3, acc)?;
        Ok(u128::from(lo) | (u128::from(hi) << 64))
    }

    pub(crate) fn write_mem_u128<B: PhysBus>(
        &mut self,
        bus: &mut B,
        va: u64,
        value: u128,
        acc: DataAccess,
    ) -> Result<(), Fault> {
        if va & 15 != 0 && (acc.strict_align || self.strict_alignment()) {
            return Err(Self::data_abort(va, Fsc::Alignment, true));
        }
        self.write_mem(bus, va, 3, value as u64, acc)?;
        self.write_mem(bus, va.wrapping_add(8), 3, (value >> 64) as u64, acc)
    }

    /// `DC ZVA`: zero the naturally aligned block containing `va`.
    pub(crate) fn dc_zva<B: PhysBus>(&mut self, bus: &mut B, va: u64) -> Result<(), Fault> {
        let base = va & !(id::DC_ZVA_BYTES - 1);
        let el = self.pstate.el;
        let t = self
            .translate(bus, base, AccessKind::Write, el)
            .map_err(|f| Self::data_abort(va, f, true))?;
        if t.device {
            return Err(Self::data_abort(va, Fsc::Alignment, true));
        }
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
