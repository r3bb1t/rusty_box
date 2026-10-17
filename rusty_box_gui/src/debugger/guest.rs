//! The one way the debugger reads a paused guest: a port over the three
//! things a walk needs — translating a linear address through some page
//! tables, reading guest-physical memory, and listing the mappings of an
//! address space. `DebugCpu` answers it for a running machine through
//! Bochs's debugger primitives (`dbg_xlate_linear2phy`, `dbg_fetch_mem`);
//! tests answer it from a page map.

use rusty_box::cpu::{Fetched, Instrumentation, LinearRange, MappingWalk, PageMapping, WalkStep};
use rusty_box::emulator::{DebugCpu, SliceEngine};

/// Whose page tables a read walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Root {
    /// The shown processor's own CR3.
    Live,
    /// A process's `DirectoryTableBase`.
    Process(u64),
}

/// Whether a read came back whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestRead {
    Read,
    /// A page that does not map, or physical memory outside RAM, in an
    /// unbacked device window, or behind a failed device read.
    Unreadable,
}

const PAGE_BYTES: u64 = 0x1000;

/// A paused guest's memory, as the debugger reads it.
pub(crate) trait GuestMemory {
    /// The physical address `linear` maps to under `root`'s page tables.
    fn translate(&mut self, root: Root, linear: u64) -> Option<u64>;
    /// Fills `buf` from guest-physical `physical`.
    fn read_physical(&mut self, physical: u64, buf: &mut [u8]) -> GuestRead;
    /// Visits each mapping of `range` under the page tables at `cr3`.
    fn page_mappings<F>(&mut self, cr3: u64, range: LinearRange, visit: F) -> MappingWalk
    where
        F: FnMut(PageMapping) -> WalkStep;

    /// Fills `buf` from linear `linear` under `root`, translating a page at a
    /// time, since consecutive linear pages need not be consecutive in
    /// physical memory. Linear addresses wrap at 2^64, as the processor's do.
    fn read_linear(&mut self, root: Root, linear: u64, buf: &mut [u8]) -> GuestRead {
        let mut at = linear;
        let mut rest: &mut [u8] = buf;
        while !rest.is_empty() {
            // The bytes left in `at`'s page: 1 to `PAGE_BYTES`.
            let in_page = PAGE_BYTES.wrapping_sub(at & (PAGE_BYTES - 1));
            let take = usize::try_from(in_page).map_or(rest.len(), |in_page| in_page.min(rest.len()));
            let Some(physical) = self.translate(root, at) else {
                return GuestRead::Unreadable;
            };
            let Some((chunk, tail)) = std::mem::take(&mut rest).split_at_mut_checked(take) else {
                return GuestRead::Unreadable;
            };
            match self.read_physical(physical, chunk) {
                GuestRead::Read => {}
                GuestRead::Unreadable => return GuestRead::Unreadable,
            }
            rest = tail;
            at = at.wrapping_add(in_page);
        }
        GuestRead::Read
    }
}

impl<T: Instrumentation, E: SliceEngine<T>> GuestMemory for DebugCpu<'_, T, E> {
    fn translate(&mut self, root: Root, linear: u64) -> Option<u64> {
        match root {
            Root::Live => self.dbg_xlate_linear2phy(linear),
            Root::Process(cr3) => self.dbg_xlate_linear2phy_in(cr3, linear),
        }
    }

    fn read_physical(&mut self, physical: u64, buf: &mut [u8]) -> GuestRead {
        match self.dbg_fetch_mem(physical, buf) {
            Ok(Fetched::Valid) => GuestRead::Read,
            Ok(Fetched::OutsideMemory) => GuestRead::Unreadable,
            Err(error) => {
                tracing::warn!("debugger read at {physical:#x} failed: {error}");
                GuestRead::Unreadable
            }
        }
    }

    fn page_mappings<F>(&mut self, cr3: u64, range: LinearRange, visit: F) -> MappingWalk
    where
        F: FnMut(PageMapping) -> WalkStep,
    {
        self.dbg_page_mappings_in(cr3, range, visit)
    }
}

/// A guest made of a page map, for tests: physical pages, a kernel half every
/// address space shares, per address space a user half, and per address space
/// session pages — kernel-half pages only that space maps, as Windows maps
/// `win32k` and its session data. An isolated address space (a KVA-shadow
/// user space) sees its session pages and none of the shared kernel half.
/// Every page is writable and executable; user-half pages are user pages. A
/// CR3 value names its space by its table address alone, as the processor's
/// walk reads it (the PCID bits and bit 63 are not part of it).
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use rusty_box::cpu::PageAccess;

    pub(crate) const PAGE: u64 = 0x1000;
    /// The first address of the kernel half.
    pub(crate) const KERNEL_HALF: u64 = 0xFFFF_8000_0000_0000;
    /// The bits of a CR3 value that address its top-level table.
    const CR3_TABLE: u64 = 0x000F_FFFF_FFFF_F000;

    pub(crate) struct FakeGuest {
        /// The live processor's address space.
        live_cr3: u64,
        physical: BTreeMap<u64, Box<[u8; PAGE as usize]>>,
        /// Linear page to physical page, for the shared kernel half.
        kernel: BTreeMap<u64, u64>,
        /// Linear page to physical page, per address space, for the user half.
        users: BTreeMap<u64, BTreeMap<u64, u64>>,
        /// Linear page to physical page, per address space, for kernel-half
        /// pages only that space maps.
        sessions: BTreeMap<u64, BTreeMap<u64, u64>>,
        /// Address spaces that do not map the shared kernel half.
        isolated: BTreeSet<u64>,
        next_free: u64,
    }

    /// Which map a write lands in.
    #[derive(Clone, Copy)]
    enum Space {
        /// The user half of `cr3`, or the shared kernel half.
        Ordinary,
        /// `cr3`'s own kernel-half pages.
        Session,
    }

    impl FakeGuest {
        pub(crate) fn new(live_cr3: u64) -> Self {
            FakeGuest {
                live_cr3,
                physical: BTreeMap::new(),
                kernel: BTreeMap::new(),
                users: BTreeMap::new(),
                sessions: BTreeMap::new(),
                isolated: BTreeSet::new(),
                next_free: 0x10_0000,
            }
        }

        /// Makes the space at `cr3` the one the live processor runs on.
        pub(crate) fn set_live_cr3(&mut self, cr3: u64) {
            self.live_cr3 = cr3;
        }

        /// Maps the user half of the space at `from` into the space at `to`
        /// too, onto the same physical pages, as a process's KVA-shadow
        /// tables map its user half.
        pub(crate) fn alias_user_half(&mut self, from: u64, to: u64) {
            let pages = self.users.get(&(from & CR3_TABLE)).cloned().unwrap_or_default();
            self.users.insert(to & CR3_TABLE, pages);
        }

        /// Leaves the shared kernel half out of the space at `cr3`: only its
        /// session pages map there, as a KVA-shadow user space maps only the
        /// kernel's entry pages.
        pub(crate) fn isolate(&mut self, cr3: u64) {
            self.isolated.insert(cr3 & CR3_TABLE);
        }

        fn pages_of(&mut self, cr3: u64, page: u64, space: Space) -> &mut BTreeMap<u64, u64> {
            let cr3 = cr3 & CR3_TABLE;
            match space {
                Space::Session => self.sessions.entry(cr3).or_default(),
                Space::Ordinary if page >= KERNEL_HALF => &mut self.kernel,
                Space::Ordinary => self.users.entry(cr3).or_default(),
            }
        }

        fn physical_page(&mut self, cr3: u64, page: u64, space: Space) -> u64 {
            let fresh = self.next_free;
            let physical = *self.pages_of(cr3, page, space).entry(page).or_insert(fresh);
            if physical == fresh {
                self.next_free += PAGE;
                self.physical.insert(physical, Box::new([0; PAGE as usize]));
            }
            physical
        }

        fn write_in(&mut self, cr3: u64, linear: u64, bytes: &[u8], space: Space) {
            for (offset, byte) in bytes.iter().enumerate() {
                let at = linear.wrapping_add(offset as u64);
                let physical = self.physical_page(cr3, at & !(PAGE - 1), space);
                if let Some(page) = self.physical.get_mut(&physical) {
                    page[(at & (PAGE - 1)) as usize] = *byte;
                }
            }
        }

        /// Writes `bytes` at `linear` in the space at `cr3` (any space, for a
        /// kernel-half address), mapping zeroed pages as needed. A write that
        /// runs past the top of the address space wraps, as the processor's
        /// addresses do.
        pub(crate) fn write(&mut self, cr3: u64, linear: u64, bytes: &[u8]) {
            self.write_in(cr3, linear, bytes, Space::Ordinary);
        }

        /// Writes `bytes` at kernel-half `linear` in session pages only the
        /// space at `cr3` maps.
        pub(crate) fn write_session(&mut self, cr3: u64, linear: u64, bytes: &[u8]) {
            self.write_in(cr3, linear, bytes, Space::Session);
        }

        pub(crate) fn write_u64(&mut self, cr3: u64, linear: u64, value: u64) {
            self.write(cr3, linear, &value.to_le_bytes());
        }

        pub(crate) fn write_u32(&mut self, cr3: u64, linear: u64, value: u32) {
            self.write(cr3, linear, &value.to_le_bytes());
        }

        pub(crate) fn write_u16(&mut self, cr3: u64, linear: u64, value: u16) {
            self.write(cr3, linear, &value.to_le_bytes());
        }

        /// Unmaps the page holding `linear`: a page the guest paged out.
        pub(crate) fn page_out(&mut self, cr3: u64, linear: u64) {
            let page = linear & !(PAGE - 1);
            self.pages_of(cr3, page, Space::Ordinary).remove(&page);
        }

        fn lookup(&self, cr3: u64, page: u64) -> Option<u64> {
            if page >= KERNEL_HALF {
                let session = self.sessions.get(&cr3).and_then(|pages| pages.get(&page));
                let shared = || if self.isolated.contains(&cr3) { None } else { self.kernel.get(&page) };
                session.or_else(shared).copied()
            } else {
                self.users.get(&cr3)?.get(&page).copied()
            }
        }
    }

    impl GuestMemory for FakeGuest {
        fn translate(&mut self, root: Root, linear: u64) -> Option<u64> {
            let cr3 = match root {
                Root::Live => self.live_cr3,
                Root::Process(cr3) => cr3,
            } & CR3_TABLE;
            let page = self.lookup(cr3, linear & !(PAGE - 1))?;
            Some(page + (linear & (PAGE - 1)))
        }

        fn read_physical(&mut self, physical: u64, buf: &mut [u8]) -> GuestRead {
            for (offset, byte) in buf.iter_mut().enumerate() {
                let at = physical + offset as u64;
                match self.physical.get(&(at & !(PAGE - 1))) {
                    Some(page) => *byte = page[(at & (PAGE - 1)) as usize],
                    None => return GuestRead::Unreadable,
                }
            }
            GuestRead::Read
        }

        fn page_mappings<F>(&mut self, cr3: u64, range: LinearRange, mut visit: F) -> MappingWalk
        where
            F: FnMut(PageMapping) -> WalkStep,
        {
            if range.start >= range.end {
                return MappingWalk::Complete;
            }
            let cr3 = cr3 & CR3_TABLE;
            let empty = BTreeMap::new();
            let user = self.users.get(&cr3).unwrap_or(&empty);
            let session = self.sessions.get(&cr3).unwrap_or(&empty);
            let shared = if self.isolated.contains(&cr3) { &empty } else { &self.kernel };
            let pages = user
                .range(range.start..range.end)
                .chain(session.range(range.start..range.end))
                .chain(shared.range(range.start..range.end));
            for (&linear, &physical) in pages {
                let mapping = PageMapping {
                    linear,
                    physical,
                    size: PAGE,
                    access: PageAccess {
                        user: linear < KERNEL_HALF,
                        writable: true,
                        executable: true,
                    },
                };
                match visit(mapping) {
                    WalkStep::Continue => {}
                    WalkStep::Stop => return MappingWalk::Stopped,
                }
            }
            MappingWalk::Complete
        }
    }
}
