//! The registered memory-mapped I/O regions, and nothing about the devices
//! behind them.
//!
//! Bochs stores a `(read_handler, write_handler, param)` triple per region and
//! calls it from inside `BX_MEM_C::writePhysicalPage`, so its memory subsystem
//! reaches devices directly. This port inherited that shape as three typed raw
//! device pointers on `BxMemC`, which is the coupling this module removes: a
//! region here carries only an [`MmioToken`], an opaque number the platform
//! mints and the memory subsystem never interprets. An access that lands on a
//! region reports the token to its caller, and the caller — which owns the
//! devices — performs the dispatch.
//!
//! Registration semantics are Bochs's, exactly. `registerMemoryHandlers`
//! decides overlap on a 16-bit bitmap of 64 KB sub-ranges within each 1 MB
//! page, not on the address intervals themselves, so two regions inside one
//! 64 KB sub-range collide even when their intervals are disjoint. That rule is
//! preserved below; replacing it with interval overlap would accept mappings
//! Bochs rejects.
//!
//! What is *not* Bochs's is the storage. Bochs allocates one pointer per
//! megabyte of the physical address space (`BX_MEM_HANDLERS`, 1M entries at a
//! 40-bit width) plus a heap-allocated chain node per region. Here a fixed
//! region array holds the mappings and a bitmap of occupied megabytes gives the
//! same one-compare rejection on the access path, which is where the cost
//! actually matters.

use super::error::MemoryError;
use crate::config::BxPhyAddress;

/// Identifies the owner of an MMIO region.
///
/// Opaque to the memory subsystem: it is minted by the platform layer, stored
/// verbatim, and handed back on an access so the platform can route it. Keeping
/// it uninterpreted here is the point — it is what lets memory stop naming
/// devices.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(transparent)]
pub struct MmioToken(pub u16);

/// Where a physical access landed: whose region, and how far into it.
///
/// The offset is what the region already knows — memory found the region by
/// its base, so subtracting that base costs nothing and saves the device
/// re-deriving it from a base of its own. Kept together in one value so no
/// dispatch site can pass the owner without the position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MmioHit {
    pub token: MmioToken,
    /// Bytes from the start of the region the token names.
    pub offset: u64,
}

/// Maximum regions mapped at once.
///
/// The PC machine maps five (the VGA text/graphics window, the VGA LFB and MMIO
/// apertures, the I/O APIC, and the HPET); the rest is headroom. A fixed bound
/// is deliberate: region bases come from PCI BARs, which the guest programs, and
/// the previous table was grown to `(end >> 20) + 1` entries on registration —
/// so a guest that parked a BAR high could size a host allocation.
const MAX_DEVICE_MMIO_REGIONS: usize = 16;

/// Megabytes covered by the occupancy bitmap — the low 4 GiB, which is where
/// every region this machine maps lives.
const MAPPED_MEGABYTES: usize = 4096;
const OCCUPANCY_WORDS: usize = MAPPED_MEGABYTES / 64;

/// One mapped region. `end` is inclusive, matching Bochs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MmioRegion {
    begin: BxPhyAddress,
    end: BxPhyAddress,
    token: MmioToken,
}

impl MmioRegion {
    #[inline]
    fn contains(&self, addr: BxPhyAddress) -> bool {
        self.begin <= addr && addr <= self.end
    }

    /// Whether this region occupies any part of the given 1 MB page.
    #[inline]
    fn touches_page(&self, page: usize) -> bool {
        let base = (page as BxPhyAddress) << 20;
        self.begin <= base + 0xF_FFFF && self.end >= base
    }
}

/// The mapped MMIO regions.
#[derive(Debug)]
pub(crate) struct MmioMap {
    regions: [Option<MmioRegion>; MAX_DEVICE_MMIO_REGIONS],
    /// One bit per 1 MB page of the low 4 GiB, set when any region covers it.
    /// The access path's fast rejection; `MAPPED_MEGABYTES` is its whole reach,
    /// so `above_bitmap` carries what it cannot.
    occupancy: [u64; OCCUPANCY_WORDS],
    /// Whether any region sits at or above 4 GiB, where the bitmap stops.
    above_bitmap: bool,
}

impl Default for MmioMap {
    fn default() -> Self {
        Self::new()
    }
}

impl MmioMap {
    pub(crate) const fn new() -> Self {
        Self {
            regions: [None; MAX_DEVICE_MMIO_REGIONS],
            occupancy: [0; OCCUPANCY_WORDS],
            above_bitmap: false,
        }
    }

    /// The region covering `addr`, if any.
    ///
    /// The hot half of this module: every physical access that misses the
    /// direct-RAM path asks. The occupancy bitmap answers the overwhelmingly
    /// common "no region here" in one load and one test, which is what the
    /// per-megabyte pointer table bought and why it is kept in this form.
    #[inline]
    fn find(&self, addr: BxPhyAddress) -> Option<&MmioRegion> {
        let page = (addr >> 20) as usize;
        if page < MAPPED_MEGABYTES {
            if self.occupancy[page / 64] & (1u64 << (page % 64)) == 0 {
                return None;
            }
        } else if !self.above_bitmap {
            return None;
        }
        self.regions
            .iter()
            .flatten()
            .find(|region| region.contains(addr))
    }

    /// Where an access to `addr` landed: whose region, and how far into it.
    #[inline]
    pub(crate) fn lookup(&self, addr: BxPhyAddress) -> Option<MmioHit> {
        self.find(addr).map(|region| MmioHit {
            token: region.token,
            offset: addr - region.begin,
        })
    }

    /// Whether any region covers `addr` — the direct-access path's question,
    /// which needs the decision but neither the owner nor the position, and so
    /// does not compute them.
    #[inline]
    pub(crate) fn covers(&self, addr: BxPhyAddress) -> bool {
        self.find(addr).is_some()
    }

    /// The 64 KB sub-range bitmap a region occupies within one 1 MB page.
    ///
    /// Bochs misc_mem.cc `registerMemoryHandlers` — the overlap currency.
    #[inline]
    fn page_bitmap(page: usize, begin: BxPhyAddress, end: BxPhyAddress) -> u16 {
        let mut bitmap = 0xFFFFu16;
        let base = (page as BxPhyAddress) << 20;
        if begin > base {
            bitmap &= 0xFFFFu16 << ((begin >> 16) & 0xF);
        }
        if end < base + 0x10_0000 {
            bitmap &= 0xFFFFu16 >> (0x0F - ((end >> 16) & 0xF));
        }
        bitmap
    }

    /// Bochs's overlap test for a candidate range, against everything mapped
    /// except the regions named by `ignoring`.
    ///
    /// `ignoring` exists for relocation, which must judge the new range against
    /// the state the old range has already left.
    fn would_overlap(
        &self,
        begin: BxPhyAddress,
        end: BxPhyAddress,
        ignoring: Option<(BxPhyAddress, BxPhyAddress, MmioToken)>,
    ) -> bool {
        let first_page = (begin >> 20) as usize;
        let last_page = (end >> 20) as usize;
        for page in first_page..=last_page {
            let candidate = Self::page_bitmap(page, begin, end);
            for region in self.regions.iter().flatten() {
                if ignoring == Some((region.begin, region.end, region.token)) {
                    continue;
                }
                if !region.touches_page(page) {
                    continue;
                }
                if candidate & Self::page_bitmap(page, region.begin, region.end) != 0 {
                    return true;
                }
            }
        }
        false
    }

    /// Map `[begin, end]` to `token`.
    ///
    /// Bochs `registerMemoryHandlers`. Bochs additionally permits a read-only
    /// handler to overlap an existing one; this port has no read-only regions —
    /// every registered device implements both directions — so that allowance
    /// has no expressible case here.
    pub(crate) fn map(
        &mut self,
        token: MmioToken,
        begin: BxPhyAddress,
        end: BxPhyAddress,
    ) -> Result<(), MemoryError> {
        if end < begin {
            return Err(MemoryError::InvalidAddressRange);
        }
        if self.would_overlap(begin, end, None) {
            tracing::error!(
                "register failed: {begin:#x}-{end:#x} overlaps a mapped MMIO region"
            );
            return Err(MemoryError::OverlappingHandlers);
        }
        let slot = self
            .regions
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(MemoryError::Internal("MMIO region table exhausted"))?;
        *slot = Some(MmioRegion { begin, end, token });
        self.reindex();
        tracing::debug!("MMIO region mapped: {begin:#x}-{end:#x} -> {token:?}");
        Ok(())
    }

    /// Unmap the region matching `token` and exactly `[begin, end]`.
    ///
    /// Bochs `unregisterMemoryHandlers` matches on the triple, not on the
    /// address alone, so a device cannot drop another device's mapping by
    /// naming its range. An unmatched range is not an error there and is not
    /// one here.
    pub(crate) fn unmap(
        &mut self,
        token: MmioToken,
        begin: BxPhyAddress,
        end: BxPhyAddress,
    ) -> Result<(), MemoryError> {
        if end < begin {
            return Err(MemoryError::InvalidAddressRange);
        }
        for slot in self.regions.iter_mut() {
            if slot.is_some_and(|r| r.token == token && r.begin == begin && r.end == end) {
                *slot = None;
                self.reindex();
                tracing::debug!("MMIO region unmapped: {begin:#x}-{end:#x} -> {token:?}");
                return Ok(());
            }
        }
        Ok(())
    }

    /// Move `token`'s mapping from `old` to `new`, atomically.
    ///
    /// Either range may be absent, which covers first registration and final
    /// removal. The new range is judged against the map with the old range
    /// already discounted, so a device that relocates onto ground it currently
    /// occupies — a PCI BAR moving within its own aperture — is not rejected by
    /// its own presence. Nothing is mutated unless the whole move can succeed.
    pub(crate) fn relocate(
        &mut self,
        token: MmioToken,
        old: Option<(BxPhyAddress, BxPhyAddress)>,
        new: Option<(BxPhyAddress, BxPhyAddress)>,
    ) -> Result<(), MemoryError> {
        for (begin, end) in old.into_iter().chain(new) {
            if end < begin {
                return Err(MemoryError::InvalidAddressRange);
            }
        }
        if let Some((begin, end)) = new {
            let ignoring = old.map(|(b, e)| (b, e, token));
            if self.would_overlap(begin, end, ignoring) {
                return Err(MemoryError::OverlappingHandlers);
            }
            // Capacity is checked before anything is removed, so a failure
            // here leaves the old mapping in place.
            let free = self.regions.iter().filter(|slot| slot.is_none()).count()
                + usize::from(old.is_some());
            if free == 0 {
                return Err(MemoryError::Internal("MMIO region table exhausted"));
            }
        }
        if let Some((begin, end)) = old {
            self.unmap(token, begin, end)?;
        }
        if let Some((begin, end)) = new {
            self.map(token, begin, end)?;
        }
        Ok(())
    }

    /// Rebuild the occupancy bitmap from the regions.
    ///
    /// Called on every mapping change rather than maintained incrementally,
    /// because unmapping cannot clear a megabyte another region still shares.
    fn reindex(&mut self) {
        self.occupancy = [0; OCCUPANCY_WORDS];
        self.above_bitmap = false;
        for region in self.regions.iter().flatten() {
            let first = (region.begin >> 20) as usize;
            let last = (region.end >> 20) as usize;
            if last >= MAPPED_MEGABYTES {
                self.above_bitmap = true;
            }
            for page in first..=last.min(MAPPED_MEGABYTES - 1) {
                if page < MAPPED_MEGABYTES {
                    self.occupancy[page / 64] |= 1u64 << (page % 64);
                }
            }
        }
    }

    /// Number of mapped regions — for diagnostics and tests.
    pub(crate) fn len(&self) -> usize {
        self.regions.iter().flatten().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VGA: MmioToken = MmioToken(1);
    const IOAPIC: MmioToken = MmioToken(2);

    /// The owning token at `addr`. Most cases here are about *which* region
    /// answers, not where in it the access landed; the offset has its own test.
    fn owner(map: &MmioMap, addr: BxPhyAddress) -> Option<MmioToken> {
        map.lookup(addr).map(|hit| hit.token)
    }

    #[test]
    fn lookup_finds_the_owning_region_and_rejects_neighbours() {
        let mut map = MmioMap::new();
        map.map(VGA, 0xA_0000, 0xB_FFFF).unwrap();
        assert_eq!(owner(&map, 0xA_0000), Some(VGA));
        assert_eq!(owner(&map, 0xB_FFFF), Some(VGA));
        assert_eq!(owner(&map, 0x9_FFFF), None);
        assert_eq!(owner(&map, 0xC_0000), None);
    }

    /// The offset is measured from the region that answered, so a device never
    /// sees an absolute address and a relocated region needs no base of its own
    /// to subtract. A region moved by a BAR write reports the same offsets at
    /// its new base as it did at the old one.
    #[test]
    fn a_hit_reports_the_distance_from_the_region_base() {
        let mut map = MmioMap::new();
        map.map(VGA, 0xA_0000, 0xB_FFFF).unwrap();
        assert_eq!(map.lookup(0xA_0000), Some(MmioHit { token: VGA, offset: 0 }));
        assert_eq!(
            map.lookup(0xA_0500),
            Some(MmioHit {
                token: VGA,
                offset: 0x500
            })
        );
        assert_eq!(
            map.lookup(0xB_FFFF),
            Some(MmioHit {
                token: VGA,
                offset: 0x1_FFFF
            })
        );

        map.relocate(VGA, Some((0xA_0000, 0xB_FFFF)), Some((0xF00_0000, 0xF01_FFFF)))
            .unwrap();
        assert_eq!(map.lookup(0xA_0500), None);
        assert_eq!(
            map.lookup(0xF00_0500),
            Some(MmioHit {
                token: VGA,
                offset: 0x500
            })
        );
    }

    /// Bochs decides overlap on 64 KB sub-ranges of a 1 MB page, not on the
    /// intervals. Two disjoint intervals inside one 64 KB sub-range therefore
    /// collide, and an interval-based map would wrongly accept them.
    #[test]
    fn overlap_is_judged_on_bochs_64k_subranges_not_on_intervals() {
        let mut map = MmioMap::new();
        map.map(VGA, 0xA_0000, 0xA_0FFF).unwrap();
        assert!(
            map.lookup(0xA_5000).is_none(),
            "the region itself must not cover the rest of its 64 KB sub-range"
        );
        assert!(
            matches!(
                map.map(IOAPIC, 0xA_5000, 0xA_5FFF),
                Err(MemoryError::OverlappingHandlers)
            ),
            "disjoint intervals sharing a 64 KB sub-range collide in Bochs"
        );
        // The next 64 KB sub-range of the same megabyte is free.
        map.map(IOAPIC, 0xB_0000, 0xB_0FFF)
            .expect("an adjacent 64 KB sub-range must still be mappable");
    }

    #[test]
    fn unmap_requires_the_exact_range_and_token() {
        let mut map = MmioMap::new();
        map.map(VGA, 0xE000_0000, 0xE0FF_FFFF).unwrap();
        map.unmap(IOAPIC, 0xE000_0000, 0xE0FF_FFFF).unwrap();
        assert_eq!(owner(&map, 0xE000_0000), Some(VGA), "wrong token must not unmap");
        map.unmap(VGA, 0xE000_0000, 0xE0FF_0000).unwrap();
        assert_eq!(owner(&map, 0xE000_0000), Some(VGA), "wrong range must not unmap");
        map.unmap(VGA, 0xE000_0000, 0xE0FF_FFFF).unwrap();
        assert_eq!(owner(&map, 0xE000_0000), None);
        assert_eq!(map.len(), 0);
    }

    /// A BAR moving within its own aperture must not be rejected by the
    /// mapping it is itself about to vacate.
    #[test]
    fn relocation_discounts_the_range_it_vacates() {
        let mut map = MmioMap::new();
        map.map(VGA, 0xE000_0000, 0xE00F_FFFF).unwrap();
        map.relocate(
            VGA,
            Some((0xE000_0000, 0xE00F_FFFF)),
            Some((0xE000_0000, 0xE01F_FFFF)),
        )
        .expect("a region must be allowed to grow over its own ground");
        assert_eq!(owner(&map, 0xE01F_FFFF), Some(VGA));
        assert_eq!(map.len(), 1);
    }

    /// A rejected relocation must leave the original mapping intact, because
    /// the caller has already told the device its BAR moved.
    #[test]
    fn a_rejected_relocation_leaves_the_old_mapping_in_place() {
        let mut map = MmioMap::new();
        map.map(VGA, 0xE000_0000, 0xE00F_FFFF).unwrap();
        map.map(IOAPIC, 0xFEC0_0000, 0xFEC0_0FFF).unwrap();
        assert!(matches!(
            map.relocate(
                VGA,
                Some((0xE000_0000, 0xE00F_FFFF)),
                Some((0xFEC0_0000, 0xFEC0_0FFF)),
            ),
            Err(MemoryError::OverlappingHandlers)
        ));
        assert_eq!(owner(&map, 0xE000_0000), Some(VGA));
        assert_eq!(owner(&map, 0xFEC0_0000), Some(IOAPIC));
    }

    /// The occupancy bitmap is a rejection accelerator, never an answer: it
    /// must not report coverage a region does not actually provide, and
    /// unmapping must not clear a megabyte another region still shares.
    #[test]
    fn occupancy_tracks_shared_megabytes_across_unmap() {
        let mut map = MmioMap::new();
        map.map(VGA, 0x10_0000, 0x10_0FFF).unwrap();
        map.map(IOAPIC, 0x11_0000, 0x11_0FFF).unwrap();
        map.unmap(VGA, 0x10_0000, 0x10_0FFF).unwrap();
        assert_eq!(owner(&map, 0x11_0000), Some(IOAPIC), "shared megabyte lost");
        assert_eq!(owner(&map, 0x10_0000), None);
    }

    #[test]
    fn regions_above_the_bitmap_are_still_found() {
        let mut map = MmioMap::new();
        let high: BxPhyAddress = 1 << 32;
        map.map(VGA, high, high + 0xFFFF).unwrap();
        assert_eq!(owner(&map, high), Some(VGA));
        assert_eq!(owner(&map, high - 1), None);
    }

    #[test]
    fn exhausting_the_region_table_is_an_error_not_a_silent_drop() {
        let mut map = MmioMap::new();
        for index in 0..MAX_DEVICE_MMIO_REGIONS {
            let base = (index as BxPhyAddress) << 20;
            map.map(MmioToken(index as u16), base, base + 0xFFFF).unwrap();
        }
        let base = (MAX_DEVICE_MMIO_REGIONS as BxPhyAddress) << 20;
        assert!(matches!(
            map.map(MmioToken(0xFF), base, base + 0xFFFF),
            Err(MemoryError::Internal("MMIO region table exhausted"))
        ));
    }
}
