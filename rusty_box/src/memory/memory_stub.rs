#[cfg(feature = "alloc")]
use alloc::vec::Vec;
#[cfg(feature = "std")]
use tempfile::tempfile;

#[cfg(feature = "std")]
use super::residency::snapshot_invalid;
use super::residency::{BlockGeometry, Residency};
use super::{BxMemoryStubC, MemoryError, Result};
#[cfg(feature = "std")]
use super::{MemorySnapshotGeometry, MemorySnapshotResidency};
use crate::config::BxPhyAddress;
use crate::config::BxPhyAddress as A20Mask;
use crate::memory::memory_rusty_box::{
    bx_guest_ram_span, bx_is_pci_hole_addr, BIOSROMSZ, EXROMSIZE,
};

#[cfg(feature = "std")]
use std::io::{Read, Write};

const BX_MEM_VECTOR_ALIGN: usize = 4096;

/// A snapshot names guest blocks in 32 bits; the block table indexes them in
/// host words. One conversion, so every snapshot entry point rejects an
/// unrepresentable index the same way.
#[cfg(feature = "std")]
#[inline]
fn guest_block_index(guest_block: u32) -> std::io::Result<usize> {
    usize::try_from(guest_block)
        .map_err(|_| snapshot_invalid("snapshot guest block conversion failed"))
}

/// One page of guest RAM, carrying the allocation's alignment in its type.
///
/// The backing must be `BX_MEM_VECTOR_ALIGN`-aligned. Expressing that as an
/// alignment on the element type rather than on a hand-built `Layout` is what
/// lets the owned case be an ordinary `Box<[GuestPage]>`: it is `Send`, it
/// frees itself, and it cannot be deallocated with the wrong alignment — the
/// hazard that forced the previous hand-rolled buffer and its manual `Drop`.
///
/// It also keeps `vector_offset` at zero. Were alignment instead absorbed as
/// leading padding, `rom()` (which indexes by `rom_offset` alone) and the
/// construction path (which computes `vector_offset + rom_offset`) would stop
/// agreeing; they coincide today only because the offset is zero.
#[cfg(feature = "alloc")]
#[repr(C, align(4096))]
#[derive(Clone, Copy)]
pub(super) struct GuestPage([u8; BX_MEM_VECTOR_ALIGN]);

#[cfg(feature = "alloc")]
const _: () = assert!(core::mem::size_of::<GuestPage>() == BX_MEM_VECTOR_ALIGN);

/// Where the guest's memory bytes live.
///
/// Both variants are plain owned or borrowed data, so `BxMemoryStubC` derives
/// `Send` instead of promising it.
pub(super) enum RamBacking {
    /// Allocated and freed by this struct.
    #[cfg(feature = "alloc")]
    Owned(alloc::boxed::Box<[GuestPage]>),
    /// Caller-provided storage for no-alloc targets, handed over for the life
    /// of the machine by `create_from_raw`.
    Borrowed(&'static mut [u8]),
}

impl core::fmt::Debug for RamBacking {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The bytes themselves are guest RAM; only the shape is useful here.
        let (kind, len) = match self {
            #[cfg(feature = "alloc")]
            Self::Owned(pages) => ("Owned", pages.len() * BX_MEM_VECTOR_ALIGN),
            Self::Borrowed(bytes) => ("Borrowed", bytes.len()),
        };
        write!(f, "RamBacking::{kind}({len} bytes)")
    }
}

impl RamBacking {
    #[inline(always)]
    pub(super) fn as_slice(&self) -> &[u8] {
        match self {
            #[cfg(feature = "alloc")]
            // SAFETY: `GuestPage` is `repr(C)` over `[u8; BX_MEM_VECTOR_ALIGN]`,
            // so a run of them is exactly that many contiguous initialised
            // bytes. This is a reinterpreting cast, not an owning pointer.
            Self::Owned(pages) => unsafe {
                core::slice::from_raw_parts(
                    pages.as_ptr() as *const u8,
                    pages.len() * BX_MEM_VECTOR_ALIGN,
                )
            },
            Self::Borrowed(bytes) => bytes,
        }
    }

    #[inline(always)]
    pub(super) fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            #[cfg(feature = "alloc")]
            // SAFETY: see `as_slice`.
            Self::Owned(pages) => unsafe {
                core::slice::from_raw_parts_mut(
                    pages.as_mut_ptr() as *mut u8,
                    pages.len() * BX_MEM_VECTOR_ALIGN,
                )
            },
            Self::Borrowed(bytes) => bytes,
        }
    }
}

impl BxMemoryStubC {

    pub fn get_memory_len(&self) -> usize {
        self.guest_len()
    }

    #[cfg(feature = "alloc")]
    pub fn create_and_init(
        guest: usize,
        host: usize,
        block_size: usize,
    ) -> Result<alloc::boxed::Box<Self>> {
        const ONE_MEGABYTE: usize = 1 << 20;

        if !host.is_multiple_of(ONE_MEGABYTE) || !guest.is_multiple_of(ONE_MEGABYTE) {
            return Err(MemoryError::MemorySizeIsNotAMultiplyOf1Megabyte.into());
        }

        let geometry = BlockGeometry::new(guest, host, block_size)?;
        let resident_len = geometry.resident_len();

        let aux_len = BIOSROMSZ
            .checked_add(EXROMSIZE)
            .and_then(|n| n.checked_add(4096))
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;
        let total_len = resident_len
            .checked_add(aux_len)
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;
        // Rounded up to whole pages so the backing can be `[GuestPage]`, which
        // carries the required alignment in its type. `actual_vector_len` keeps
        // the exact figure, so the slack is never addressable.
        let total_pages = total_len.div_ceil(BX_MEM_VECTOR_ALIGN);
        let mut pages: alloc::vec::Vec<GuestPage> = alloc::vec::Vec::new();
        pages
            .try_reserve_exact(total_pages)
            .map_err(|_| MemoryError::UnableToAllocateGuestMemory(total_len))?;
        pages.resize(total_pages, GuestPage([0u8; BX_MEM_VECTOR_ALIGN]));
        let mut actual_vector = RamBacking::Owned(pages.into_boxed_slice());
        let vector_offset = 0;
        tracing::debug!(
            "allocated memory at {:p}, block_size = {}k",
            actual_vector.as_slice().as_ptr(),
            block_size / 1024
        );

        let rom_offset = resident_len;
        let bogus_offset = resident_len
            .checked_add(BIOSROMSZ)
            .and_then(|n| n.checked_add(EXROMSIZE))
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;

        let rom_start = vector_offset + rom_offset;
        actual_vector.as_mut_slice()[rom_start..total_len].fill(0xFF);

        tracing::debug!("{}MB", guest / (1024 * 1024));
        tracing::debug!(
            "mem block size = {:8X}, blocks={}",
            block_size,
            geometry.num_blocks()
        );

        let mut smc_stamps = Vec::new();
        smc_stamps
            .try_reserve_exact(crate::cpu::icache::SMC_STAMP_ENTRIES)
            .map_err(|_| {
                MemoryError::UnableToAllocateGuestMemory(
                    crate::cpu::icache::SMC_STAMP_ENTRIES * core::mem::size_of::<u32>(),
                )
            })?;
        smc_stamps.resize(crate::cpu::icache::SMC_STAMP_ENTRIES, 0u32);
        #[cfg(feature = "std")]
        let overflow_file = {
            let file = tempfile().map_err(MemoryError::UnableToCreateTempFile)?;
            if host < guest {
                file.set_len(u64::try_from(guest).map_err(|_| MemoryError::InsufficientRam)?)?;
            }
            file
        };
        // The residency block table alone is 256 KiB, so the stub is placed
        // into a zeroed allocation rather than constructed by value: returning
        // one would build it on the stack and then move it.
        let layout = alloc::alloc::Layout::new::<Self>();
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) } as *mut Self;
        if ptr.is_null() {
            return Err(MemoryError::UnableToAllocateGuestMemory(layout.size()).into());
        }

        unsafe {
            core::ptr::addr_of_mut!((*ptr).backing).write(actual_vector);
            core::ptr::addr_of_mut!((*ptr).actual_vector_len).write(total_len);
            core::ptr::addr_of_mut!((*ptr).vector_offset).write(vector_offset);
            core::ptr::addr_of_mut!((*ptr).rom_offset).write(rom_offset);
            core::ptr::addr_of_mut!((*ptr).bogus_offset).write(bogus_offset);
            // Zeroed storage is already a valid `Residency` in every field but
            // the overflow file, so that one is *written* — assigning would
            // drop the zeros as though they named an open file. With it in
            // place the residency is a value, and lays its own blocks out
            // through the one initializer both construction paths share,
            // in place and without a 256 KiB block table crossing the stack.
            #[cfg(feature = "std")]
            core::ptr::addr_of_mut!((*ptr).residency.overflow_file).write(overflow_file);
            (*ptr).residency.init_layout(geometry);
            // Machine-wide SMC write-stamp table (Bochs icache.h
            // bxPageWriteStampTable ctor allocates + resetWriteStamps).
            core::ptr::addr_of_mut!((*ptr).smc_stamps).write(smc_stamps);
            core::ptr::addr_of_mut!((*ptr).smc_pending).write(
                [crate::cpu::icache::PendingSmc::default(); crate::cpu::icache::SMC_PENDING_CAP],
            );
            core::ptr::addr_of_mut!((*ptr).smc_pending_len).write(0);
            core::ptr::addr_of_mut!((*ptr).smc_seq_next).write(0);
            core::ptr::addr_of_mut!((*ptr).smc_overflow_seq).write(0);
            Ok(alloc::boxed::Box::from_raw(ptr))
        }
    }

    /// Create a memory stub from an externally-provided buffer (no-alloc path).
    ///
    /// # Safety
    /// `ptr` must be a non-null, 4096-byte-aligned, valid, exclusively-owned
    /// buffer of `len` bytes that outlives the returned stub.
    pub unsafe fn create_from_raw(
        ptr: *mut u8,
        len: usize,
        guest: usize,
        host: usize,
        block_size: usize,
    ) -> Result<Self> {
        let geometry = BlockGeometry::new(guest, host, block_size)?;
        let resident_len = geometry.resident_len();
        if ptr.is_null() || (ptr as usize & (BX_MEM_VECTOR_ALIGN - 1)) != 0 {
            return Err(MemoryError::Internal("raw memory must be 4K aligned").into());
        }

        let aux_len = BIOSROMSZ
            .checked_add(EXROMSIZE)
            .and_then(|n| n.checked_add(4096))
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;
        if len
            < resident_len
                .checked_add(aux_len)
                .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?
        {
            return Err(MemoryError::UnableToAllocateGuestMemory(host).into());
        }
        let vector_offset = 0;
        let rom_offset = resident_len;
        let bogus_offset = resident_len
            .checked_add(BIOSROMSZ)
            .and_then(|n| n.checked_add(EXROMSIZE))
            .ok_or(MemoryError::UnableToAllocateGuestMemory(host))?;

        #[cfg(feature = "std")]
        let overflow_file = {
            let file = tempfile().map_err(MemoryError::UnableToCreateTempFile)?;
            if host < guest {
                file.set_len(u64::try_from(guest).map_err(|_| MemoryError::InsufficientRam)?)?;
            }
            file
        };
        #[cfg(feature = "alloc")]
        let smc_stamps = {
            let mut v = Vec::new();
            v.try_reserve_exact(crate::cpu::icache::SMC_STAMP_ENTRIES)
                .map_err(|_| {
                    MemoryError::UnableToAllocateGuestMemory(
                        crate::cpu::icache::SMC_STAMP_ENTRIES * core::mem::size_of::<u32>(),
                    )
                })?;
            v.resize(crate::cpu::icache::SMC_STAMP_ENTRIES, 0u32);
            v
        };
        #[cfg(not(feature = "alloc"))]
        let smc_stamps = [0u32; crate::cpu::icache::SMC_STAMP_ENTRIES];
        let mut residency = Residency::empty(
            #[cfg(feature = "std")]
            overflow_file,
        );
        residency.init_layout(geometry);
        Ok(Self {
            // SAFETY: the caller of this raw entry point guarantees `ptr` is
            // valid, 4 KiB-aligned (checked above), `len` bytes long, and
            // unaliased for the life of the machine. Wrapping it once here is
            // the only place that contract is taken on trust; everything below
            // works from the resulting slice.
            backing: RamBacking::Borrowed(unsafe {
                core::slice::from_raw_parts_mut(ptr, len)
            }),
            actual_vector_len: len,
            residency,
            vector_offset,
            rom_offset,
            bogus_offset,
            smc_stamps,
            smc_pending: [crate::cpu::icache::PendingSmc::default();
                crate::cpu::icache::SMC_PENDING_CAP],
            smc_pending_len: 0,
            smc_seq_next: 0,
            smc_overflow_seq: 0,
        })
    }

    // ── Machine-wide SMC write-stamp table ─────────────────────────────────
    // Bochs icache.h bxPageWriteStampTable: ONE shared instance per machine.
    // Trace creation by any cpu marks lines here; any write hitting marked
    // lines must invalidate EVERY cpu's icache (Bochs icache.cc handleSMC).

    /// Bochs icache.h `bxPageWriteStampTable::markICacheMask`.
    #[inline]
    pub(crate) fn smc_mark_icache_mask(&mut self, p_addr: BxPhyAddress, mask: u32) {
        self.smc_stamps[crate::cpu::icache::smc_page_index(p_addr)] |= mask;
    }

    /// Return whether any cached instruction line overlaps this single-page
    /// physical range. Bulk writers use this non-mutating probe to fall back
    /// to scalar ordering before consuming externally visible input.
    #[inline]
    pub(crate) fn smc_range_has_stamps(&self, p_addr: BxPhyAddress, len: u32) -> bool {
        let stamps = self.smc_stamps[crate::cpu::icache::smc_page_index(p_addr)];
        stamps != 0 && stamps & crate::cpu::icache::smc_cache_line_mask(p_addr, len) != 0
    }

    /// Bochs icache.h `bxPageWriteStampTable::decWriteStamp(pAddr, len)`:
    /// check a write against the stamp table; on a hit, clear the lines and
    /// queue the invalidation for every cpu (Bochs calls `handleSMC`
    /// synchronously; the emulator drains the queue at slice boundaries, and
    /// cpu-context writers apply it to themselves immediately via their
    /// `smc_seq_seen` watermark).
    #[inline]
    pub(crate) fn smc_dec_write_stamp(&mut self, p_addr: BxPhyAddress, len: u32) {
        let index = crate::cpu::icache::smc_page_index(p_addr);
        let stamps = self.smc_stamps[index];
        if stamps == 0 {
            return;
        }
        let mask = crate::cpu::icache::smc_cache_line_mask(p_addr, len);
        if stamps & mask == 0 {
            return;
        }
        self.smc_stamps[index] = stamps & !mask;
        self.smc_push_pending(p_addr, mask);
    }

    /// Bochs icache.h `bxPageWriteStampTable::decWriteStamp(pAddr)` — the
    /// whole-page variant used by handler-path and DMA writes (`handleSMC`
    /// with mask 0xffffffff).
    #[inline]
    pub(crate) fn smc_dec_write_stamp_page(&mut self, p_addr: BxPhyAddress) {
        let index = crate::cpu::icache::smc_page_index(p_addr);
        if self.smc_stamps[index] == 0 {
            return;
        }
        self.smc_stamps[index] = 0;
        self.smc_push_pending(p_addr, u32::MAX);
    }

    fn smc_push_pending(&mut self, p_addr: BxPhyAddress, mask: u32) {
        if self.smc_pending_len < crate::cpu::icache::SMC_PENDING_CAP {
            self.smc_pending[self.smc_pending_len] =
                crate::cpu::icache::PendingSmc { p_addr, mask };
            self.smc_pending_len += 1;
        } else {
            // Queue full: every cpu that has not caught up past this event
            // must do a full icache flush instead (conservative, correct).
            self.smc_overflow_seq = self.smc_seq_next + 1;
        }
        self.smc_seq_next += 1;
    }

    /// Sequence number the next SMC event will get. A cpu whose
    /// `smc_seq_seen` watermark is below this has invalidations to apply.
    #[inline]
    pub(crate) fn smc_seq_next(&self) -> u64 {
        self.smc_seq_next
    }

    /// Events a watermark of `since` has not seen yet.
    /// Returns `(needs_full_flush, new_events)`.
    #[inline]
    pub(crate) fn smc_pending_since(
        &self,
        since: u64,
    ) -> (bool, &[crate::cpu::icache::PendingSmc]) {
        let needs_full_flush = since < self.smc_overflow_seq;
        let base = self.smc_seq_next - self.smc_pending_len as u64;
        let start = since.saturating_sub(base) as usize;
        (
            needs_full_flush,
            &self.smc_pending[start.min(self.smc_pending_len)..self.smc_pending_len],
        )
    }

    /// Drop drained events. Called by the emulator once every cpu's
    /// watermark has caught up (sequence numbers stay monotonic).
    #[inline]
    pub(crate) fn smc_clear_pending(&mut self) {
        self.smc_pending_len = 0;
    }

    /// True when SMC events are queued. An empty queue means every cpu is
    /// caught up (the drain only clears it after catching every cpu up), so
    /// the per-slice drain can early-out on a single load.
    #[inline]
    pub(crate) fn smc_has_pending(&self) -> bool {
        self.smc_pending_len != 0
    }

    /// Bochs icache.h `bxPageWriteStampTable::resetWriteStamps` — hardware
    /// reset only (every cpu's icache is flushed there too).
    pub(crate) fn smc_reset_stamps(&mut self) {
        self.smc_stamps.fill(0);
        self.smc_pending_len = 0;
    }

    /// The bytes of one resident slot, as the block map places them in the
    /// allocation. The map bounds the slot within the resident region; this
    /// only shifts that offset to where the region begins.
    #[cfg(feature = "std")]
    fn snapshot_resident_block(&self, slot: usize, len: usize) -> std::io::Result<&[u8]> {
        let offset = self.residency.snapshot_slot_offset(slot, len)?;
        let start = self
            .vector_offset
            .checked_add(offset)
            .ok_or_else(|| snapshot_invalid("snapshot resident block offset overflow"))?;
        Ok(&self.backing.as_slice()[start..start + len])
    }

    #[cfg(feature = "std")]
    fn snapshot_resident_block_mut(
        &mut self,
        slot: usize,
        len: usize,
    ) -> std::io::Result<&mut [u8]> {
        let offset = self.residency.snapshot_slot_offset(slot, len)?;
        let start = self
            .vector_offset
            .checked_add(offset)
            .ok_or_else(|| snapshot_invalid("snapshot resident block offset overflow"))?;
        Ok(&mut self.backing.as_mut_slice()[start..start + len])
    }

    /// Return the configured block geometry without exposing host backing.
    #[cfg(feature = "std")]
    pub(super) fn snapshot_geometry(&self) -> MemorySnapshotGeometry {
        self.residency.snapshot_geometry()
    }

    /// Describe one guest block's current backing without making it resident.
    #[cfg(feature = "std")]
    pub(super) fn snapshot_residency(
        &self,
        guest_block: u32,
    ) -> std::io::Result<MemorySnapshotResidency> {
        self.residency.snapshot_residency(guest_block_index(guest_block)?)
    }

    /// Stream one logical guest block in GPA order without changing residency.
    #[cfg(feature = "std")]
    pub(super) fn write_snapshot_block<W: Write>(
        &mut self,
        guest_block: u32,
        out: &mut W,
    ) -> std::io::Result<()> {
        let guest_block = guest_block_index(guest_block)?;
        let logical_len = self.residency.snapshot_logical_block_len(guest_block)?;
        match self.residency.snapshot_residency(guest_block)? {
            MemorySnapshotResidency::Resident { slot } => {
                let slot = usize::try_from(slot)
                    .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?;
                out.write_all(self.snapshot_resident_block(slot, logical_len)?)
            }
            MemorySnapshotResidency::Swapped => {
                self.residency
                    .write_swapped_block(guest_block, logical_len, out)
            }
        }
    }

    /// Restore one logical guest block to its saved slot or overflow extent.
    ///
    /// The block map itself remains untouched until `finish_snapshot_restore`
    /// has validated all descriptors and the complete transfer succeeds.
    #[cfg(feature = "std")]
    pub(super) fn read_snapshot_block<R: Read>(
        &mut self,
        guest_block: u32,
        saved: MemorySnapshotResidency,
        input: &mut R,
    ) -> std::io::Result<()> {
        let guest_block = guest_block_index(guest_block)?;
        let logical_len = self.residency.snapshot_logical_block_len(guest_block)?;
        match saved {
            MemorySnapshotResidency::Resident { slot } => {
                let slot = usize::try_from(slot)
                    .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?;
                input.read_exact(self.snapshot_resident_block_mut(slot, logical_len)?)
            }
            MemorySnapshotResidency::Swapped => {
                self.residency
                    .read_swapped_block(guest_block, logical_len, input)
            }
        }
    }

    /// Validate and atomically install snapshot residency metadata.
    #[cfg(feature = "std")]
    pub(super) fn finish_snapshot_restore(
        &mut self,
        geometry: MemorySnapshotGeometry,
        saved_map: &[MemorySnapshotResidency],
    ) -> std::io::Result<()> {
        let parts = self.resident_parts();
        parts
            .map
            .finish_snapshot_restore(parts.ram, geometry, saved_map)?;
        self.smc_stamps.fill(0);
        self.smc_pending.fill(crate::cpu::icache::PendingSmc::default());
        self.smc_pending_len = 0;
        self.smc_seq_next = 0;
        self.smc_overflow_seq = 0;
        Ok(())
    }

    /// Where an already translated guest-RAM offset currently lives *within
    /// the allocation*, and how far the span runs. Never crosses a guest
    /// block, and makes the block resident first if it was swapped out.
    ///
    /// The offset, not a pointer, is the primitive: it is what the CPU caches
    /// and what the eviction check compares, and unlike an address it does not
    /// depend on where the allocation happens to sit.
    pub(super) fn resident_slot_range(
        &mut self,
        addr: usize,
    ) -> Result<core::ops::Range<usize>> {
        let vector_offset = self.vector_offset;
        let parts = self.resident_parts();
        let range = parts.map.slot_range(parts.ram, addr)?;
        // The map answers in region offsets; callers index the allocation.
        let start = vector_offset
            .checked_add(range.start)
            .ok_or(MemoryError::Internal("resident block offset overflow"))?;
        Ok(start..start + range.len())
    }

    /// The same resident span as `resident_slot_range`, as bytes.
    pub(super) fn get_vector_offset<'a>(
        &'a mut self,
        addr: usize,
    ) -> Result<&'a mut [u8]> {
        let range = self.resident_slot_range(addr)?;
        Ok(&mut self.actual_vector_mut()[range])
    }

    /// Where the ROM image sits in the allocation, from `offset` onwards.
    ///
    /// Runs to the END of the allocation, exactly as `rom()[offset..]` does
    /// rather than stopping at the ROM's nominal size — instruction fetch
    /// tests the returned length against a full page, so a tighter bound here
    /// would silently drop the ITLB entry for the top of the ROM.
    pub(super) fn rom_range(&self, offset: usize) -> core::ops::Range<usize> {
        (self.rom_offset + offset)..self.actual_vector_len
    }

    /// Where the bogus page sits in the allocation, from `offset` onwards.
    /// Open-ended for the same reason as `rom_range`.
    pub(super) fn bogus_range(&self, offset: usize) -> core::ops::Range<usize> {
        (self.bogus_offset + offset)..self.actual_vector_len
    }


    pub(crate) fn write_physical_page(
        &mut self,
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
        a20_mask: A20Mask,
    ) -> Result<()> {
        if data.len() < len {
            return Err(MemoryError::WritePhysicalPage { addr, len }.into());
        }
        if len == 0 {
            return Ok(());
        }
        let a20_addr = addr & a20_mask;

        // Note: accesses should always be contained within a single page
        if (addr >> 12) != ((addr + len as u64 - 1) >> 12) {
            return Err(MemoryError::WritePhysicalPage { addr, len }.into());
        }

        if bx_is_pci_hole_addr(a20_addr) {
            // PCI MMIO hole — writes are silently dropped
            return Ok(());
        }
        if bx_guest_ram_span(a20_addr, len, self.guest_len()).is_some() {
            // A typed physical access may straddle independently resident
            // guest blocks.  Do not hand a block-short slice to endian helpers.
            for (offset, byte) in data.iter().copied().take(len).enumerate() {
                let byte_addr = a20_addr + offset as u64;
                self.smc_dec_write_stamp(byte_addr, 1);
                let span = bx_guest_ram_span(byte_addr, 1, self.guest_len())
                    .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                self.get_vector_offset(span.start)?[0] = byte;
            }
            return Ok(());
        }
        Ok(())
    }

    pub(crate) fn read_physical_page(
        &mut self,
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
        a20_mask: A20Mask,
    ) -> Result<()> {
        let a20_addr = addr & a20_mask;

        if data.len() < len {
            return Err(MemoryError::ReadPhysicalPage { addr, len }.into());
        }
        if len == 0 {
            return Ok(());
        }
        // Note: accesses should always be contained within a single page
        if (addr >> 12) != ((addr + len as u64 - 1) >> 12) {
            return Err(MemoryError::ReadPhysicalPage { addr, len }.into());
        }

        if bx_is_pci_hole_addr(a20_addr) {
            // PCI MMIO hole — reads return 0xFF
            data[..len].fill(0xff);
            return Ok(());
        }
        if bx_guest_ram_span(a20_addr, len, self.guest_len()).is_some() {
            // The resident primitive is block-bounded; assemble typed accesses
            // bytewise when a guest-block boundary lies within the span.
            for (offset, byte) in data.iter_mut().take(len).enumerate() {
                let byte_addr = a20_addr + offset as u64;
                let span = bx_guest_ram_span(byte_addr, 1, self.guest_len())
                    .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                *byte = self.get_vector_offset(span.start)?[0];
            }
            Ok(())
        } else {
            // access outside limits of physical memory
            let bogus = self.bogus();
            let fill_len = len.min(bogus.len());
            data[..fill_len].copy_from_slice(&bogus[..fill_len]);
            if len > fill_len {
                data[fill_len..].fill(0xff);
            }
            Ok(())
        }
    }

}
