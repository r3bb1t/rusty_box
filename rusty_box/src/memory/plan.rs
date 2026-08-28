//! The guest-physical map stated as a whole, rather than decided one access at
//! a time.
//!
//! [`super::misc_mem::BxMemC::host_mem_range`] answers "where does THIS address
//! live", which is what an interpreter needs. An execution engine that runs the
//! guest on real hardware needs the other form: the complete set of windows it
//! may serve from host memory, installed once and re-installed when the chipset
//! moves something. This module derives the second from the same rules as the
//! first, so the two cannot disagree about the machine.
//!
//! ## What it means for a range to be absent
//!
//! A window that is not in the plan is not a hole in the guest's address space
//! — it is a range the *machine* services. Device MMIO, the local APIC page,
//! the VGA aperture and unbacked memory are all absent for that reason, and an
//! access to them leaves the engine so the machine can answer it.
//!
//! ## Two facts from `host_mem_range` that shape everything below
//!
//! 1. **No write between 0xC0000 and 0x100000 is ever direct.**
//!    `direct_host_write_allowed` excludes the whole range regardless of the
//!    PAM write bit, so a shadowed area is at most read-and-execute. The
//!    chipset's write bit selects what the machine does with the trapped
//!    write, not whether it traps.
//! 2. **Guest RAM is dense across the PCI hole.** A guest-physical address at
//!    or above 4 GiB lives 1 GiB lower in the allocation
//!    (`bx_translate_gpa_to_linear`), so RAM above the hole is a second window
//!    with its own host offset — never an alias of the first.

use rusty_box_core::{GpaPerms, GpaPlan, GpaPlanError, GpaWindow, HostOffset, GUEST_PAGE};

use super::memory_rusty_box::{
    bios_map_last128k, BIOSROMSZ, BX_PCI_HOLE_SIZE, BX_PCI_HOLE_START, EXROM_MASK,
};
use super::BxMemC;
use crate::config::BxPhyAddress;

/// The video aperture. Never direct: reads are vetoed so the VGA model can
/// answer them, and writes are excluded from the direct path outright.
const VIDEO_APERTURE: core::ops::Range<u64> = 0x000A_0000..0x000C_0000;
/// The shadowable region the chipset's PAM registers control.
const SHADOW_REGION: core::ops::Range<u64> = 0x000C_0000..0x0010_0000;
/// The local APIC's architectural page. Excluded from both the direct read and
/// the direct write paths, because a CPU answers it, not memory.
const LOCAL_APIC_REGION: core::ops::Range<u64> = 0xFEE0_0000..0xFEF0_0000;
/// Bytes per PAM area for the twelve 16 KiB areas below 0xF0000.
const PAM_AREA: u64 = 0x4000;
/// Where the last, larger PAM area begins. `(addr >> 14) & 0x0f` saturates at
/// twelve, so 0xF0000 through 0x100000 share one area rather than four.
const LAST_PAM_AREA_BASE: u64 = 0x000F_0000;

/// Windows a derived plan may hold.
///
/// Seventeen candidates before any splitting — low RAM, thirteen PAM areas,
/// RAM above 1 MiB, RAM above the PCI hole, and the BIOS — and a registered
/// MMIO region can split one of them in two. The bound is generous rather
/// than tight because exceeding it is reported, and a silently truncated map
/// would strand guest memory.
pub const MAX_PLAN_WINDOWS: usize = 64;

/// Why this machine's memory cannot be stated as a guest-physical map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryPlanError {
    /// Guest RAM is only partially resident. A plan says where memory *is*,
    /// and under partial residency there is no lasting answer: the residency
    /// map moves guest blocks between host slots as the guest touches them, so
    /// any window derived now describes a layout that changes underfoot.
    PartiallyResident,
    /// More windows than [`MAX_PLAN_WINDOWS`], with the count that was needed.
    TooManyWindows { needed: usize },
    /// The derived windows are not a valid plan. Always a defect in this
    /// module rather than a property of the machine — carried rather than
    /// panicked on so a caller sees which invariant broke.
    NotAPlan(GpaPlanError),
}

/// A derived guest-physical map and the storage holding it.
#[derive(Clone, Copy, Debug)]
pub struct MemoryPlan {
    windows: [GpaWindow; MAX_PLAN_WINDOWS],
    len: usize,
}

/// A window still being assembled, before carve-outs are applied.
///
/// `delta` is what makes splitting sound: within one candidate the host offset
/// is a fixed distance from the guest-physical address, so a piece of it can be
/// emitted without re-deriving where that piece lives.
#[derive(Clone, Copy)]
struct Candidate {
    gpa: u64,
    end: u64,
    /// `host_offset = gpa.wrapping_add(delta)`.
    delta: u64,
    perms: GpaPerms,
}

impl MemoryPlan {
    /// Derive the map from a machine's memory as it stands.
    ///
    /// # Errors
    /// [`MemoryPlanError`] — see its variants; the common one by far is
    /// [`MemoryPlanError::PartiallyResident`].
    pub fn derive(memory: &BxMemC) -> Result<Self, MemoryPlanError> {
        let mut carve_outs = CarveOuts::new();
        // The 0xA0000-0xBFFFF window is video memory, EXCEPT while the chipset
        // has SMRAM open over it — then it is plain guest RAM, which is Bochs
        // `misc_mem.cc`'s own routing order (SMRAM is checked before the device
        // handlers) and the whole reason firmware can reach its SMM handler.
        // Leaving it carved out would trap the chipset's own handler install,
        // one exit per byte of it.
        if !memory.smram_is_open() {
            carve_outs.add(VIDEO_APERTURE.start, VIDEO_APERTURE.end);
        }
        carve_outs.add(LOCAL_APIC_REGION.start, LOCAL_APIC_REGION.end);
        for region in memory.mmio.regions() {
            // The map stores an inclusive end; a carve-out is half-open.
            carve_outs.add(region.begin, u64::from(region.end).saturating_add(1));
        }

        let mut plan = Self { windows: [EMPTY_WINDOW; MAX_PLAN_WINDOWS], len: 0 };
        let mut needed = 0usize;
        let mut emit = |candidate: Candidate| {
            carve_outs.split(candidate, &mut |piece| {
                needed += 1;
                plan.push(piece);
            });
        };

        for candidate in memory.plan_candidates()? {
            emit(candidate);
        }

        if needed > MAX_PLAN_WINDOWS {
            return Err(MemoryPlanError::TooManyWindows { needed });
        }
        plan.merge_adjacent();
        // The one validation choke point (R5): a caller reads the windows
        // through `GpaPlan`, so proving they form one here means no engine
        // ever sees a map this module got wrong.
        GpaPlan::new(plan.windows()).map_err(MemoryPlanError::NotAPlan)?;
        Ok(plan)
    }

    /// The windows, in ascending guest-physical order.
    ///
    /// Guaranteed by [`MemoryPlan::derive`] to be accepted by
    /// [`GpaPlan::new`], which is how a caller turns them into a plan.
    pub fn windows(&self) -> &[GpaWindow] {
        &self.windows[..self.len]
    }

    fn push(&mut self, window: GpaWindow) {
        if self.len < MAX_PLAN_WINDOWS {
            self.windows[self.len] = window;
            self.len += 1;
        }
    }

    /// Fuse windows that are contiguous in guest-physical space AND in the
    /// host allocation AND agree on permissions.
    ///
    /// Not cosmetic. The twelve 16 KiB PAM areas are almost always uniform, so
    /// this turns thirteen mappings into one — and mapping cost grows with the
    /// number of ranges already mapped, measured on this port's own reference
    /// host at 13 µs each for the first few hundred and 50 µs beyond.
    fn merge_adjacent(&mut self) {
        let mut write = 0usize;
        for read in 0..self.len {
            let window = self.windows[read];
            if write > 0 {
                let previous = &mut self.windows[write - 1];
                let joins_in_guest = previous.gpa.checked_add(previous.len) == Some(window.gpa);
                let joins_in_host = previous
                    .host
                    .get()
                    .checked_add(previous.len)
                    == Some(window.host.get());
                if joins_in_guest && joins_in_host && previous.perms == window.perms {
                    previous.len += window.len;
                    continue;
                }
            }
            self.windows[write] = window;
            write += 1;
        }
        self.len = write;
    }
}

const EMPTY_WINDOW: GpaWindow =
    GpaWindow { gpa: 0, len: 0, host: HostOffset::ZERO, perms: GpaPerms::NONE };

/// The ranges no window may cover, kept sorted so a candidate is split in one
/// pass.
struct CarveOuts {
    ranges: [(u64, u64); MAX_PLAN_WINDOWS],
    len: usize,
}

impl CarveOuts {
    fn new() -> Self {
        Self { ranges: [(0, 0); MAX_PLAN_WINDOWS], len: 0 }
    }

    /// Record `[start, end)` as host-serviced.
    ///
    /// Rounded OUTWARD to page boundaries, deliberately: an engine maps whole
    /// pages, so a carve-out that covered part of one would otherwise leave
    /// the rest of that page mapped and the device behind it unreachable.
    /// Unmapping slightly more than asked costs an exit; unmapping slightly
    /// less loses a write.
    fn add(&mut self, start: u64, end: u64) {
        if end <= start || self.len == self.ranges.len() {
            return;
        }
        let start = start & !(GUEST_PAGE - 1);
        let end = end.saturating_add(GUEST_PAGE - 1) & !(GUEST_PAGE - 1);
        let mut at = self.len;
        while at > 0 && self.ranges[at - 1].0 > start {
            self.ranges[at] = self.ranges[at - 1];
            at -= 1;
        }
        self.ranges[at] = (start, end);
        self.len += 1;
    }

    /// Emit the parts of `candidate` no carve-out claims.
    fn split(&self, candidate: Candidate, emit: &mut impl FnMut(GpaWindow)) {
        let mut cursor = candidate.gpa;
        for &(start, end) in &self.ranges[..self.len] {
            if end <= cursor || start >= candidate.end {
                continue;
            }
            if start > cursor {
                emit(window_of(cursor, start, candidate));
            }
            cursor = cursor.max(end);
            if cursor >= candidate.end {
                return;
            }
        }
        if cursor < candidate.end {
            emit(window_of(cursor, candidate.end, candidate));
        }
    }
}

fn window_of(gpa: u64, end: u64, candidate: Candidate) -> GpaWindow {
    GpaWindow {
        gpa,
        len: end - gpa,
        host: HostOffset::new(gpa.wrapping_add(candidate.delta)),
        perms: candidate.perms,
    }
}

impl BxMemC {
    /// The windows this machine's memory would offer before device regions are
    /// carved out of them.
    ///
    /// Kept here rather than in `plan` because it reads the routing state, and
    /// the point of the module boundary is that the derivation cannot drift
    /// from the rules `host_mem_range` applies.
    fn plan_candidates(&self) -> Result<CandidateList, MemoryPlanError> {
        if !self.inherited_memory_stub.is_identity_map() {
            return Err(MemoryPlanError::PartiallyResident);
        }
        let guest_len = self.inherited_memory_stub.guest_len() as u64;
        // Read straight from the field: `plan` is a child of `memory`, and the
        // allocation's layout is exactly what this derivation is about. The
        // stub's own comment records that `vector_offset` is zero and that
        // `rom()` would disagree with construction if it ever were not, so the
        // assertion below is the place that notices.
        debug_assert_eq!(
            self.inherited_memory_stub.vector_offset, 0,
            "a non-zero vector offset would shift every host offset in the plan"
        );
        let rom_base = self.inherited_memory_stub.rom_offset as u64;
        let mut list = CandidateList::new();

        // RAM below the video aperture. Identity-mapped, so the host offset is
        // the guest-physical address itself.
        // RAM runs to the video aperture, or straight through it when the
        // chipset has SMRAM open there — see the carve-outs in `derive`.
        let low_ram_top = if self.smram_is_open() {
            SHADOW_REGION.start
        } else {
            VIDEO_APERTURE.start
        };
        list.push(ram_between(0, guest_len.min(low_ram_top), 0));

        // The shadowable region, one PAM area at a time, with BOTH PAM bits
        // honoured — the read bit chooses what backs the area, and the write
        // bit chooses whether a write lands or leaves the engine.
        //
        // The write bit is not a nicety. A chipset opens an area for writing
        // exactly so the firmware can copy itself into it, and Bochs's own
        // BIOS does that a byte at a time: `mov cl,[eax]; mov [eax+0xC0000],cl;
        // inc eax` over 256 KiB. An engine that trapped every one of those
        // writes would spend a quarter of a million exits — about half a
        // minute — on a copy that takes a millisecond when the window is
        // simply writable.
        let mut area_base = SHADOW_REGION.start;
        while area_base < SHADOW_REGION.end {
            let area_len = if area_base >= LAST_PAM_AREA_BASE {
                SHADOW_REGION.end - LAST_PAM_AREA_BASE
            } else {
                PAM_AREA
            };
            let reads_ram = self.pci_enabled && self.pam_area_reads_ram(area_base);
            if reads_ram {
                // Shadow RAM: the guest's own bytes. Writable when the chipset
                // says so; otherwise writes leave the engine, where the
                // machine drops them exactly as Bochs `misc_mem.cc` does for a
                // write-protected shadow area.
                let writes_ram = self.pam_area_writes_ram(area_base);
                if area_base + area_len <= guest_len {
                    list.push(Candidate {
                        gpa: area_base,
                        end: area_base + area_len,
                        delta: 0,
                        perms: if writes_ram { GpaPerms::RWX } else { GpaPerms::RX },
                    });
                }
            } else {
                // The ROM image behind the area. Its offset is affine within
                // each of the two sub-ranges, which is what lets one candidate
                // span an area.
                let rom_offset = rom_image_offset(area_base);
                list.push(Candidate {
                    gpa: area_base,
                    end: area_base + area_len,
                    delta: rom_base.wrapping_add(rom_offset).wrapping_sub(area_base),
                    perms: GpaPerms::RX,
                });
            }
            area_base += area_len;
        }

        // RAM from 1 MiB to wherever it stops, capped at the PCI hole.
        let low_ram_end = guest_len.min(BX_PCI_HOLE_START);
        if low_ram_end > SHADOW_REGION.end {
            list.push(ram_between(SHADOW_REGION.end, low_ram_end, 0));
        }

        // RAM above the hole, stored densely behind the RAM below it — a
        // second window, never an alias.
        if let Some(above_hole) = ram_above_the_hole(guest_len) {
            list.push(above_hole);
        }

        // The BIOS at the top of the 32-bit space. Read-and-execute: `is_bios`
        // excludes it from the direct write path, so a write reaches the flash
        // model through the machine.
        let bios_base = u64::from(self.bios_rom_addr);
        if bios_base < FOUR_GIB {
            list.push(Candidate {
                gpa: bios_base,
                end: FOUR_GIB,
                delta: rom_base
                    .wrapping_add(bios_map_last128k(bios_base as usize) as u64)
                    .wrapping_sub(bios_base),
                perms: GpaPerms::RX,
            });
        }

        Ok(list)
    }

    /// Whether the chipset has this PAM area reading from guest RAM rather
    /// than from the ROM image.
    fn pam_area_reads_ram(&self, addr: u64) -> bool {
        self.memory_type[pam_area_index(addr)][0]
    }

    /// Whether the chipset has SMRAM open over the video aperture for ordinary
    /// accesses.
    ///
    /// The engine's half of Bochs `misc_mem.cc`'s SMRAM test. That test also
    /// admits a processor already IN system-management mode, which cannot
    /// apply here: a guest on a hypervisor never enters SMM — its handler runs
    /// on the shadow processor, where the interpreter applies the full test.
    /// So a map only ever opens the window for the `D_OPEN` case, and the
    /// stricter one stays where it can be answered.
    fn smram_is_open(&self) -> bool {
        self.smram_available && self.smram_enable
    }

    /// Whether the chipset has this PAM area's writes landing in guest RAM
    /// rather than being dropped on the bus.
    ///
    /// The second half of `memory_type[area][rw]`, and the one that decides
    /// whether a shadow copy runs at memory speed or at one exit per byte.
    fn pam_area_writes_ram(&self, addr: u64) -> bool {
        self.memory_type[pam_area_index(addr)][1]
    }
}

/// Which PAM entry governs a shadowable address.
///
/// Bochs misc_mem.cc `getHostMemAddr` computes `(addr >> 14) & 0x0f` and then
/// clamps anything past `MemoryAreaT::F0000` back to it, which is why the top
/// 64 KiB is one area and the twelve below it are 16 KiB each. Stated as its
/// own function because a plan that got this wrong would shadow the wrong
/// 16 KiB and there would be nothing to see until the guest read it.
fn pam_area_index(addr: u64) -> usize {
    usize::min((addr >> 14) as usize & 0x0F, LAST_PAM_AREA_INDEX)
}

const FOUR_GIB: u64 = 0x1_0000_0000;
/// `MemoryAreaT::F0000`, the index the area calculation saturates at.
const LAST_PAM_AREA_INDEX: usize = 12;

/// Where in the ROM image a shadowable address reads from, matching the two
/// arms of `host_mem_range`.
fn rom_image_offset(addr: u64) -> u64 {
    if addr & 0xFFFE_0000 == 0x000E_0000 {
        bios_map_last128k(addr as usize) as u64
    } else {
        (addr & EXROM_MASK as u64) + BIOSROMSZ as u64
    }
}

/// The window for guest RAM past the PCI hole, if the machine has any.
///
/// A separate function so it can be checked without building a machine that
/// owns more than three gigabytes — the only size at which this arm exists at
/// all, and therefore the one an integration test cannot reach.
///
/// The guest sees this memory at 4 GiB and upwards; the allocation holds it
/// immediately after the memory below the hole, so the host offset trails the
/// guest-physical address by the hole's own size
/// (`bx_translate_gpa_to_linear`).
fn ram_above_the_hole(guest_len: u64) -> Option<Candidate> {
    let above = guest_len.checked_sub(BX_PCI_HOLE_START)?;
    if above == 0 {
        return None;
    }
    let base = BX_PCI_HOLE_START + BX_PCI_HOLE_SIZE;
    Some(Candidate {
        gpa: base,
        end: base + above,
        delta: 0u64.wrapping_sub(BX_PCI_HOLE_SIZE),
        perms: GpaPerms::RWX,
    })
}

fn ram_between(gpa: u64, end: u64, delta: u64) -> Candidate {
    Candidate { gpa, end, delta, perms: GpaPerms::RWX }
}

/// The candidates, before carve-outs. Fixed storage for the same reason the
/// plan has fixed storage: no allocator is available on every build this runs
/// on.
struct CandidateList {
    items: [Candidate; MAX_PLAN_WINDOWS],
    len: usize,
}

impl CandidateList {
    fn new() -> Self {
        Self {
            items: [Candidate { gpa: 0, end: 0, delta: 0, perms: GpaPerms::NONE };
                MAX_PLAN_WINDOWS],
            len: 0,
        }
    }

    /// Empty candidates are dropped here rather than at every call site: a
    /// machine with less than 640 KiB of RAM, or with no BIOS, simply has
    /// fewer windows.
    fn push(&mut self, candidate: Candidate) {
        if candidate.end > candidate.gpa && self.len < self.items.len() {
            self.items[self.len] = candidate;
            self.len += 1;
        }
    }
}

impl IntoIterator for CandidateList {
    type Item = Candidate;
    type IntoIter = core::iter::Take<core::array::IntoIter<Candidate, MAX_PLAN_WINDOWS>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter().take(self.len)
    }
}

/// `BxPhyAddress` is the address type the MMIO map stores; the plan speaks
/// `u64`. They are the same width, and this is where that is stated once.
const _: () = assert!(core::mem::size_of::<BxPhyAddress>() == core::mem::size_of::<u64>());

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(candidate: Candidate, carve_outs: &CarveOuts) -> alloc::vec::Vec<GpaWindow> {
        let mut out = alloc::vec::Vec::new();
        carve_outs.split(candidate, &mut |window| out.push(window));
        out
    }

    /// The whole reason a piece can be emitted without re-deriving where it
    /// lives: within a candidate the host offset trails the guest-physical
    /// address by a fixed distance, so a split keeps each piece pointing at
    /// its own bytes rather than at the start of the original.
    #[test]
    fn splitting_a_window_keeps_each_piece_pointing_at_its_own_host_bytes() {
        let mut carve_outs = CarveOuts::new();
        carve_outs.add(0x2000, 0x3000);
        let pieces = collect(ram_between(0x1000, 0x5000, 0), &carve_outs);
        assert_eq!(pieces.len(), 2);
        assert_eq!((pieces[0].gpa, pieces[0].len), (0x1000, 0x1000));
        assert_eq!(pieces[0].host, HostOffset::new(0x1000));
        assert_eq!((pieces[1].gpa, pieces[1].len), (0x3000, 0x2000));
        assert_eq!(
            pieces[1].host,
            HostOffset::new(0x3000),
            "the piece after the hole must not restart at the window's host base"
        );
    }

    /// RAM above the PCI hole is stored a gigabyte lower. A split there is
    /// where an off-by-one-gigabyte would hide, so pin the negative delta.
    #[test]
    fn a_window_whose_host_trails_its_address_splits_with_the_offset_intact() {
        let mut carve_outs = CarveOuts::new();
        carve_outs.add(0x1_0000_2000, 0x1_0000_3000);
        let above_hole = Candidate {
            gpa: 0x1_0000_0000,
            end: 0x1_0000_5000,
            delta: 0u64.wrapping_sub(BX_PCI_HOLE_SIZE),
            perms: GpaPerms::RWX,
        };
        let pieces = collect(above_hole, &carve_outs);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].host, HostOffset::new(0xC000_0000), "4 GiB maps to 3 GiB");
        assert_eq!(pieces[1].host, HostOffset::new(0xC000_3000));
    }

    /// An engine maps whole pages. A carve-out covering part of a page must
    /// take the whole page, or the device behind it stays unreachable through
    /// the mapped remainder.
    #[test]
    fn a_carve_out_rounds_outward_to_whole_pages() {
        let mut carve_outs = CarveOuts::new();
        carve_outs.add(0x2100, 0x2200);
        let pieces = collect(ram_between(0, 0x4000, 0), &carve_outs);
        assert_eq!(pieces.len(), 2);
        assert_eq!((pieces[0].gpa, pieces[0].len), (0, 0x2000));
        assert_eq!((pieces[1].gpa, pieces[1].len), (0x3000, 0x1000));
    }

    #[test]
    fn carve_outs_apply_in_address_order_however_they_were_added() {
        let mut carve_outs = CarveOuts::new();
        carve_outs.add(0x4000, 0x5000);
        carve_outs.add(0x1000, 0x2000);
        let pieces = collect(ram_between(0, 0x8000, 0), &carve_outs);
        let spans: alloc::vec::Vec<(u64, u64)> =
            pieces.iter().map(|w| (w.gpa, w.len)).collect();
        assert_eq!(spans, [(0, 0x1000), (0x2000, 0x2000), (0x5000, 0x3000)]);
    }

    #[test]
    fn a_carve_out_covering_the_whole_window_leaves_nothing() {
        let mut carve_outs = CarveOuts::new();
        carve_outs.add(0, 0x10000);
        assert!(collect(ram_between(0x1000, 0x2000, 0), &carve_outs).is_empty());
    }

    fn plan_of(windows: &[GpaWindow]) -> MemoryPlan {
        let mut plan = MemoryPlan { windows: [EMPTY_WINDOW; MAX_PLAN_WINDOWS], len: 0 };
        for window in windows {
            plan.push(*window);
        }
        plan.merge_adjacent();
        plan
    }

    fn window(gpa: u64, len: u64, host: u64, perms: GpaPerms) -> GpaWindow {
        GpaWindow { gpa, len, host: HostOffset::new(host), perms }
    }

    #[test]
    fn windows_contiguous_in_both_spaces_and_agreeing_on_permissions_fuse() {
        let plan = plan_of(&[
            window(0, 0x1000, 0, GpaPerms::RWX),
            window(0x1000, 0x1000, 0x1000, GpaPerms::RWX),
            window(0x2000, 0x1000, 0x2000, GpaPerms::RWX),
        ]);
        assert_eq!(plan.windows(), [window(0, 0x3000, 0, GpaPerms::RWX)]);
    }

    /// Fusing on guest-adjacency alone would join a shadowed area to the RAM
    /// beside it and make its writes land silently.
    #[test]
    fn windows_that_disagree_on_permissions_stay_separate() {
        let plan = plan_of(&[
            window(0, 0x1000, 0, GpaPerms::RWX),
            window(0x1000, 0x1000, 0x1000, GpaPerms::RX),
        ]);
        assert_eq!(plan.windows().len(), 2);
    }

    /// The dangerous case: adjacent in guest space but NOT in the host
    /// allocation. Fusing these would silently alias a gigabyte of RAM.
    #[test]
    fn windows_adjacent_in_guest_space_but_not_in_the_host_stay_separate() {
        let plan = plan_of(&[
            window(0, 0x1000, 0, GpaPerms::RWX),
            window(0x1000, 0x1000, 0x9000, GpaPerms::RWX),
        ]);
        assert_eq!(plan.windows().len(), 2);
    }

    /// Bochs's own area calculation: twelve 16 KiB entries, then one that the
    /// clamp makes 64 KiB wide.
    #[test]
    fn the_top_sixty_four_kilobytes_of_the_shadow_region_share_one_pam_entry() {
        assert_eq!(pam_area_index(0x000C_0000), 0);
        assert_eq!(pam_area_index(0x000C_4000), 1);
        assert_eq!(pam_area_index(0x000E_C000), 11);
        for addr in [0x000F_0000u64, 0x000F_4000, 0x000F_8000, 0x000F_C000] {
            assert_eq!(pam_area_index(addr), LAST_PAM_AREA_INDEX, "{addr:#x}");
        }
    }

    /// The two arms of `host_mem_range`'s ROM lookup, and that each is
    /// contiguous — a plan states one window per area and would map the wrong
    /// bytes if the offset jumped inside it.
    #[test]
    fn the_rom_image_offset_is_contiguous_within_each_of_its_two_arms() {
        assert_eq!(rom_image_offset(0x000C_0000), BIOSROMSZ as u64);
        assert_eq!(rom_image_offset(0x000C_4000), BIOSROMSZ as u64 + 0x4000);
        assert_eq!(rom_image_offset(0x000D_C000), BIOSROMSZ as u64 + 0x1_C000);
        // 0xE0000 upwards is the BIOS's own last 128 KiB, not the expansion
        // ROM area.
        assert_eq!(rom_image_offset(0x000E_0000), 0x3E_0000);
        assert_eq!(rom_image_offset(0x000E_4000), 0x3E_4000);
        assert_eq!(rom_image_offset(0x000F_C000), 0x3F_C000);
    }

    /// The arm no integration test can reach, because reaching it means
    /// owning more than three gigabytes of guest RAM.
    #[test]
    fn ram_past_the_hole_starts_at_four_gigabytes_and_trails_by_one() {
        assert!(ram_above_the_hole(0).is_none());
        assert!(
            ram_above_the_hole(BX_PCI_HOLE_START).is_none(),
            "exactly three gigabytes stops at the hole and needs no second window"
        );

        let sixty_four_mib = 64 * 1024 * 1024;
        let above = ram_above_the_hole(BX_PCI_HOLE_START + sixty_four_mib)
            .expect("memory past the hole needs its own window");
        assert_eq!(above.gpa, 0x1_0000_0000, "the guest sees it at four gigabytes");
        assert_eq!(above.end - above.gpa, sixty_four_mib);
        assert_eq!(
            above.gpa.wrapping_add(above.delta),
            BX_PCI_HOLE_START,
            "and it lives immediately after the memory below the hole"
        );
    }

    #[test]
    fn an_empty_candidate_is_dropped_rather_than_emitted() {
        let mut list = CandidateList::new();
        // A machine with too little RAM to reach a region produces a candidate
        // whose end equals its base; it must vanish rather than become a
        // zero-length window no engine would accept.
        list.push(ram_between(0x1000, 0x1000, 0));
        list.push(ram_between(0x2000, 0x5000, 0));
        assert_eq!(list.len, 1, "a zero-length candidate must not become a window");
    }
}
