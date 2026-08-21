//! Which guest block lives where.
//!
//! Guest RAM is divided into fixed-size blocks. When the host grants at least
//! as much RAM as the guest asks for, every block sits at its own address and
//! nothing ever moves. When it grants less, only some blocks are resident at
//! any moment: each occupies a slot of the host backing, and the rest live in
//! an overflow file until something touches them.
//!
//! This type is the map from guest block to slot, together with everything
//! needed to keep that map true: the dimensions it is indexed by, the cursor
//! that picks the next victim, the file the evicted blocks go to, and the
//! epoch that tells cached offsets they have gone stale. They belong together
//! because none of them means anything alone — `used_blocks` is a count of
//! what, without `block_size`; the block table names slots that only the
//! overflow file can fill.
//!
//! It deliberately does NOT own the bytes. A residency change moves bytes — a
//! swap-in reads a block from the file into a slot, and the victim it
//! displaces is written out of one — so every mutating method takes the
//! resident region as a `&mut [u8]` parameter. Owning the bytes here would put
//! the RAM slice and the map behind a single borrow, and the paths that need
//! both at once would have to evade it rather than satisfy it.
//!
//! Offsets are relative to the start of the resident region, not to the
//! allocation: the caller adds its own `vector_offset`. That keeps the ROM and
//! scratch pages that share the allocation out of this type entirely.

#[cfg(feature = "std")]
use super::{MemorySnapshotGeometry, MemorySnapshotResidency};
use super::{Block, MemoryError, Result};
use crate::config::MAX_MEM_BLOCKS;
use core::ops::Range;

#[cfg(feature = "std")]
use std::fs::File;
#[cfg(feature = "std")]
use std::io::{Read, Seek, SeekFrom, Write};

#[cfg(feature = "std")]
const SNAPSHOT_IO_CHUNK: usize = 64 * 1024;

#[cfg(feature = "std")]
#[inline]
pub(super) fn snapshot_invalid(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
#[inline]
fn snapshot_other(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, message)
}

#[inline]
fn is_power_of_2(x: usize) -> bool {
    x != 0 && (x & (x - 1)) == 0
}

/// The dimensions a block table is indexed by, validated once.
///
/// A separate value because both construction paths — the heap one and the
/// caller-supplied-buffer one — need these figures *before* they have anywhere
/// to put a `Residency`: the resident length decides where the ROM image sits,
/// and the block count decides whether the request is servable at all. Making
/// it a type means the two paths share one validator instead of two copies
/// that can drift.
#[derive(Clone, Copy, Debug)]
pub(super) struct BlockGeometry {
    guest_len: usize,
    host_len: usize,
    resident_len: usize,
    block_size: usize,
    num_blocks: usize,
}

impl BlockGeometry {
    /// Validate a guest/host RAM pairing and derive the block layout from it.
    ///
    /// `host_len` below `guest_len` selects the swapping regime, which needs
    /// whole blocks to exchange, so the resident region rounds *up* to a block
    /// multiple — it may therefore exceed the host figure by less than one
    /// block. Full residency keeps the host figure exactly, because a partial
    /// final block never moves and so never needs a whole slot.
    pub(super) fn new(guest_len: usize, host_len: usize, block_size: usize) -> Result<Self> {
        if !is_power_of_2(block_size) {
            return Err(MemoryError::BlockSizeIsNotAPowerOfTwo(block_size).into());
        }
        #[cfg(not(feature = "std"))]
        if host_len < guest_len {
            // Without a filesystem there is nowhere for a swapped block to go.
            return Err(MemoryError::InsufficientRam.into());
        }

        let resident_len = if host_len < guest_len {
            host_len
                .checked_add(block_size - 1)
                .map(|bytes| bytes & !(block_size - 1))
                .ok_or(MemoryError::UnableToAllocateGuestMemory(host_len))?
        } else {
            host_len
        };
        if guest_len != 0 && resident_len == 0 {
            return Err(MemoryError::InsufficientRam.into());
        }

        let num_blocks = guest_len
            .checked_add(block_size - 1)
            .ok_or(MemoryError::UnableToAllocateGuestMemory(guest_len))?
            / block_size;
        if num_blocks > MAX_MEM_BLOCKS {
            return Err(MemoryError::UnableToAllocateGuestMemory(guest_len).into());
        }

        Ok(Self {
            guest_len,
            host_len,
            resident_len,
            block_size,
            num_blocks,
        })
    }

    /// Bytes of the allocation reserved for resident guest RAM. What follows
    /// it in the allocation — the ROM image, the bogus page — is the caller's
    /// business.
    #[inline]
    pub(super) fn resident_len(self) -> usize {
        self.resident_len
    }

    /// Only the heap construction path reports this; the caller-supplied-buffer
    /// path reads it back off the finished map.
    #[cfg(feature = "alloc")]
    #[inline]
    pub(super) fn num_blocks(self) -> usize {
        self.num_blocks
    }

    /// Whether every guest block fits in the host backing at once. Under this
    /// regime no block is ever evicted, so nothing here ever changes again.
    #[inline]
    fn full_residency(self) -> bool {
        self.host_len >= self.guest_len
    }
}

/// The resident region of an allocation and the map that indexes it.
///
/// A named struct rather than a pair, so neither half can be passed in the
/// other's position (doctrine R0). This is the shape the memory crate's
/// eventual `RamStore::parts` hands out.
pub(super) struct ResidentParts<'a> {
    pub(super) map: &'a mut Residency,
    /// Exactly `map.resident_len()` bytes, starting at the region's base.
    pub(super) ram: &'a mut [u8],
}

/// The guest-block-to-slot map.
pub(super) struct Residency {
    geometry: BlockGeometry,
    /// Slot of each guest block, or `SwappedOut`. Only `[..num_blocks]` is
    /// ever read; the tail is kept `SwappedOut` so both construction paths
    /// leave identical state.
    blocks: [Block; MAX_MEM_BLOCKS],
    /// Slots handed out so far. Under partial residency the used slots are a
    /// dense prefix of the resident region, which is what lets a fresh block
    /// take `used_blocks * block_size` without searching.
    used_blocks: usize,
    /// Round-robin victim cursor, in guest blocks.
    next_swapout: usize,
    /// Cached "the block table is an identity map" verdict, consumed by
    /// `identity_guest_base` on every cpu-loop entry (per SMP slice — the
    /// O(num_blocks) table walk it replaces dominated the SMP hot path).
    /// Maintained at every block-table mutation.
    identity_map: bool,
    /// Monotonic count of guest-block relocations — the residency epoch.
    ///
    /// Bumped whenever a guest block's slot assignment changes, which is
    /// exactly when a cached RAM offset may have started pointing at different
    /// guest bytes. Consumers that cache offsets — the instruction TLB, the
    /// bounded fetch window and the VMCB backing — record the epoch they were
    /// filled at and discard the cache when it moves
    /// (`BxCpuC::revalidate_allocation_caches`, at fetch-window refill).
    ///
    /// Announcing staleness is what makes eviction unconditional: it costs one
    /// compare per fetch-window refill and always makes progress, where asking
    /// each CPU whether it still holds a reference into a candidate block would
    /// cost a scan per eviction and could refuse to evict at all.
    ///
    /// Cost is nil where it matters: under full residency — the default, and
    /// the only regime `no_alloc` has — no block is ever swapped out, so
    /// `allocate_block` is never entered and this never advances.
    swap_epoch: u64,
    /// Where non-resident blocks live, indexed by guest block offset. A plain
    /// field, not an `UnsafeCell`: every path that touches it holds `&mut self`
    /// and reaches it disjointly from the block table, so the borrow is one the
    /// compiler can check (doctrine R1).
    ///
    /// The one field of this type the memory module reaches directly, and only
    /// to *place* it: a heap-born stub is built into zeroed storage, where
    /// every other field is already a valid value of its type but this one is
    /// a live OS handle's worth of nothing. It must be written rather than
    /// assigned — assigning would drop those zeros as if they named an open
    /// file — and writing it is what makes the surrounding `Residency` a value
    /// `init_layout` may then be called on.
    #[cfg(feature = "std")]
    pub(super) overflow_file: File,
}

impl core::fmt::Debug for Residency {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never the block table itself: it has a slot for every guest block a
        // machine may have, so a derived `Debug` would print `MAX_MEM_BLOCKS`
        // entries whatever the machine's actual size.
        f.debug_struct("Residency")
            .field("geometry", &self.geometry)
            .field("used_blocks", &self.used_blocks)
            .field("next_swapout", &self.next_swapout)
            .field("identity_map", &self.identity_map)
            .field("swap_epoch", &self.swap_epoch)
            .finish_non_exhaustive()
    }
}

impl Residency {
    /// A map of nothing, holding `overflow_file` (where there is one).
    ///
    /// Every figure is zero until `init_layout` runs, so this is only ever a
    /// step on the way to a real map. It exists for the construction path that
    /// builds a stub by value: the other places its residency into zeroed
    /// storage, where these same zeros are already present and the file has to
    /// be written rather than assigned.
    pub(super) fn empty(#[cfg(feature = "std")] overflow_file: File) -> Self {
        Self {
            geometry: BlockGeometry {
                guest_len: 0,
                host_len: 0,
                resident_len: 0,
                block_size: 0,
                num_blocks: 0,
            },
            blocks: [Block::SwappedOut; MAX_MEM_BLOCKS],
            used_blocks: 0,
            next_swapout: 0,
            identity_map: false,
            swap_epoch: 0,
            #[cfg(feature = "std")]
            overflow_file,
        }
    }

    /// Lay out `geometry`'s blocks. The single place a residency map's
    /// invariants are established (doctrine R5), so the heap path and the
    /// caller-supplied-buffer path cannot describe the same RAM differently.
    ///
    /// Full residency lays the blocks out as an identity map — guest block *n*
    /// in slot *n* — and marks every slot used, because nothing will ever move
    /// them. Partial residency starts with everything swapped out and faults
    /// blocks in on demand.
    pub(super) fn init_layout(&mut self, geometry: BlockGeometry) {
        self.geometry = geometry;
        self.blocks.fill(Block::SwappedOut);
        if geometry.full_residency() {
            for (guest_block, entry) in self
                .blocks
                .iter_mut()
                .take(geometry.num_blocks)
                .enumerate()
            {
                *entry = Block::Block {
                    offset: guest_block * geometry.block_size,
                };
            }
            self.used_blocks = geometry.num_blocks;
        } else {
            self.used_blocks = 0;
        }
        self.next_swapout = 0;
        self.swap_epoch = 0;
        self.identity_map = geometry.full_residency();
    }

    #[inline]
    pub(super) fn guest_len(&self) -> usize {
        self.geometry.guest_len
    }

    /// Bytes of the allocation the resident region occupies — the length of
    /// the `ram` slice every mutating method here expects.
    #[inline]
    pub(super) fn resident_len(&self) -> usize {
        self.geometry.resident_len
    }

    /// The live part of the block table.
    #[inline]
    pub(super) fn blocks(&self) -> &[Block] {
        &self.blocks[..self.geometry.num_blocks]
    }

    /// The current residency epoch. See the `swap_epoch` field.
    ///
    /// A consumer caching a RAM offset records this alongside it and discards
    /// the cache when the value changes. Reading is a plain load, and under
    /// full residency nothing swaps, so the only thing that ever moves it
    /// there is a snapshot restore rewriting the whole block table.
    #[inline(always)]
    pub(super) fn swap_epoch(&self) -> u64 {
        self.swap_epoch
    }

    /// Announce that guest blocks moved. The single writer of `swap_epoch`
    /// (doctrine R5) — every block-table mutation goes through here, so there
    /// is one place to audit for "did this invalidate cached offsets?".
    ///
    /// Saturating rather than wrapping: at one bump per swap, `u64` cannot
    /// realistically be exhausted, but a wrap would silently re-validate every
    /// stale cache, so the arithmetic should not be the thing that decides it.
    ///
    /// Only reachable where blocks can move, which needs a filesystem to move
    /// them to: without `std` every block is resident for the machine's whole
    /// life and the epoch is a constant zero.
    #[cfg(feature = "std")]
    #[inline]
    fn bump_swap_epoch(&mut self) {
        self.swap_epoch = self.swap_epoch.saturating_add(1);
    }

    /// The cached identity-map verdict.
    #[inline]
    pub(super) fn is_identity_map(&self) -> bool {
        self.identity_map
    }

    /// Full O(num_blocks) identity-map scan — the ground truth behind the
    /// cached verdict. Used to (re)compute the cache at block-table rewrites
    /// and as the debug oracle in `identity_guest_base`.
    pub(super) fn scan_identity_map(&self) -> bool {
        let block_size = self.geometry.block_size;
        self.geometry.full_residency()
            && self.blocks().iter().enumerate().all(|(guest_block, block)| {
                matches!(block, Block::Block { offset } if *offset == guest_block * block_size)
            })
    }

    /// Re-derive the cached identity verdict after a bulk block-table rewrite.
    /// Snapshot restore is the only such rewrite, and it needs `std`.
    #[cfg(feature = "std")]
    fn recompute_identity_map(&mut self) {
        self.identity_map = self.scan_identity_map();
    }

    /// How many bytes of guest RAM block `block` actually holds. The final
    /// block is short whenever guest RAM is not a whole number of blocks.
    ///
    /// Asked only when moving a block to or from the overflow file, which is
    /// why it follows `std`: without one, no block ever moves.
    #[cfg(feature = "std")]
    #[inline]
    fn logical_block_len(&self, block: usize) -> usize {
        self.geometry
            .guest_len
            .saturating_sub(block * self.geometry.block_size)
            .min(self.geometry.block_size)
    }

    /// Where an already-translated guest-RAM offset currently lives *within
    /// the resident region*, and how far the span runs. Never crosses a guest
    /// block, and makes the block resident first if it was swapped out.
    ///
    /// The offset, not a pointer, is the primitive: it is what the CPU caches
    /// and what the eviction check compares, and unlike an address it does not
    /// depend on where the allocation happens to sit.
    pub(super) fn slot_range(&mut self, ram: &mut [u8], addr: usize) -> Result<Range<usize>> {
        if addr >= self.geometry.guest_len {
            return Err(MemoryError::Internal("translated RAM offset out of range").into());
        }
        let block_size = self.geometry.block_size;
        let guest_block = addr / block_size;
        if matches!(self.blocks()[guest_block], Block::SwappedOut) {
            self.allocate_block(ram, guest_block)?;
        }
        let Block::Block { offset } = self.blocks()[guest_block] else {
            return Err(MemoryError::Internal("allocated block is not resident").into());
        };
        let within = addr & (block_size - 1);
        let start = offset
            .checked_add(within)
            .ok_or(MemoryError::Internal("resident block offset overflow"))?;
        let remaining = (block_size - within).min(self.geometry.guest_len - addr);
        Ok(start..start + remaining)
    }

    /// Load block `block` from the overflow file into the slot at
    /// `slot_offset`, zeroing any tail the block does not cover.
    #[cfg(feature = "std")]
    fn read_block_into(&mut self, ram: &mut [u8], block: usize, slot_offset: usize) -> Result<()> {
        let logical_len = self.logical_block_len(block);
        let slot_end = slot_offset
            .checked_add(self.geometry.block_size)
            .ok_or(MemoryError::Internal("resident slot overflow"))?;
        if slot_end > self.geometry.resident_len {
            return Err(MemoryError::Internal("resident slot outside host backing").into());
        }
        let offset = block
            .checked_mul(self.geometry.block_size)
            .ok_or(MemoryError::Internal("overflow file offset overflow"))?;
        let chosen = &mut ram[slot_offset..slot_end];
        chosen.fill(0);
        let file = &mut self.overflow_file;
        file.seek(SeekFrom::Start(u64::try_from(offset)?))
            .map_err(|e| MemoryError::CantSeekToAddressOverflowFile(offset, e))?;
        file.read_exact(&mut chosen[..logical_len])?;
        Ok(())
    }

    /// Make guest block `block` resident, evicting another block if every slot
    /// is already taken.
    // `ram` is the swap target, and a build with no filesystem has nothing to
    // swap from — it answers before ever reaching the bytes.
    #[cfg_attr(not(feature = "std"), allow(unused_variables))]
    pub(super) fn allocate_block(&mut self, ram: &mut [u8], block: usize) -> Result<()> {
        if block >= self.geometry.num_blocks {
            return Err(MemoryError::Internal("guest block out of range").into());
        }
        if !matches!(self.blocks()[block], Block::SwappedOut) {
            return Ok(());
        }
        #[cfg(not(feature = "std"))]
        {
            Err(MemoryError::InsufficientRam.into())
        }
        #[cfg(feature = "std")]
        {
            let block_size = self.geometry.block_size;
            let capacity = self.geometry.resident_len / block_size;
            if capacity == 0 {
                return Err(MemoryError::InsufficientRam.into());
            }
            let used_blocks = self.used_blocks;
            let (slot_offset, victim, uses_new_slot) = if used_blocks < capacity {
                (used_blocks * block_size, None, true)
            } else {
                // Round-robin victim selection, with no veto: any resident
                // block is a legal victim. The swap bumps the residency epoch
                // and the holder of a cached offset discards it at its next
                // instruction fetch, so no live reference can make a block
                // unevictable and stall the machine.
                let mut selected = None;
                for _ in 0..self.geometry.num_blocks {
                    let guest = self.next_swapout;
                    self.next_swapout = (guest + 1) % self.geometry.num_blocks;
                    let Block::Block { offset } = self.blocks()[guest] else {
                        continue;
                    };
                    selected = Some((guest, offset));
                    break;
                }
                let (guest, offset) = selected.ok_or(MemoryError::InsufficientRam)?;
                (offset, Some(guest), false)
            };
            if let Some(victim_guest) = victim {
                let logical_len = self.logical_block_len(victim_guest);
                let file_offset = victim_guest
                    .checked_mul(block_size)
                    .ok_or(MemoryError::Internal("overflow file offset overflow"))?;
                let victim_bytes = &ram[slot_offset..slot_offset + logical_len];
                let file = &mut self.overflow_file;
                file.seek(SeekFrom::Start(u64::try_from(file_offset)?))
                    .map_err(|e| MemoryError::CantSeekToAddressOverflowFile(file_offset, e))?;
                file.write_all(victim_bytes)
                    .map_err(|e| MemoryError::FailedToWriteToOverflowFIle(file_offset, e))?;
            }
            if let Err(error) = self.read_block_into(ram, block, slot_offset) {
                // The victim stays logically resident until the target reload
                // completes, so restore its slot from the bytes just persisted
                // above.
                if let Some(victim_guest) = victim {
                    if let Err(restore_error) = self.read_block_into(ram, victim_guest, slot_offset)
                    {
                        // The slot now holds neither block's bytes. The victim
                        // is still recoverable — its contents reached the
                        // overflow file — but only if it stops claiming to be
                        // resident here, so demote it rather than leave the
                        // table pointing at garbage. The slot is left owned by
                        // nothing: capacity drops by one block until the next
                        // reset, which is the price of not handing a caller
                        // memory that silently reads back wrong.
                        self.blocks[victim_guest] = Block::SwappedOut;
                        tracing::error!(
                            "guest block {victim_guest} could not be restored after a failed                              swap-in of block {block}; it is now swapped out and its slot is                              unusable: {restore_error:?}"
                        );
                        return Err(restore_error);
                    }
                }
                return Err(error);
            }
            if let Some(victim_guest) = victim {
                self.blocks[victim_guest] = Block::SwappedOut;
            }
            self.blocks[block] = Block::Block {
                offset: slot_offset,
            };
            if uses_new_slot {
                self.used_blocks = used_blocks + 1;
            }
            // A block changed slots, so every cached RAM offset is suspect.
            // Bumped unconditionally rather than only when a victim was
            // evicted: filling a fresh slot cannot strand a live offset (a
            // swapped-out block has none), but keeping one rule for "the block
            // table moved" leaves no case to get wrong later, and this path is
            // unreachable under full residency anyway.
            self.bump_swap_epoch();
            // Swapping regime: blocks land at arbitrary slots (and this path
            // is only reachable when residency is partial), so the identity
            // map is broken. Exact by construction — under full residency no
            // block is ever SwappedOut and this function is never entered.
            self.identity_map = false;
            Ok(())
        }
    }
}

// ── Snapshot support ───────────────────────────────────────────────────────
// Block-logical only: a snapshot records which guest block occupied which slot
// and streams the bytes in guest-physical order, so it restores identically on
// a machine whose swapping happened to run differently.

#[cfg(feature = "std")]
impl Residency {
    /// The block count this geometry demands, recomputed from the guest length
    /// rather than read back from the field, so a restore validates the field
    /// instead of trusting it.
    fn expected_num_blocks(&self) -> std::io::Result<usize> {
        let last_byte = self
            .geometry
            .block_size
            .checked_sub(1)
            .ok_or_else(|| snapshot_invalid("snapshot block size is zero"))?;
        self.geometry
            .guest_len
            .checked_add(last_byte)
            .ok_or_else(|| snapshot_invalid("snapshot block count overflow"))
            .map(|bytes| bytes / self.geometry.block_size)
    }

    #[inline]
    fn resident_capacity(&self) -> usize {
        // Full backing has an identity resident entry for every logical guest
        // block, including a partial final block. Swapped backing may use only
        // complete host slots because eviction always exchanges a full block.
        if self.geometry.full_residency() {
            self.geometry.num_blocks
        } else {
            (self.geometry.resident_len / self.geometry.block_size).min(self.geometry.num_blocks)
        }
    }

    /// `logical_block_len`, but rejecting a block that is not part of guest RAM
    /// instead of answering zero — a snapshot descriptor naming one is corrupt.
    pub(super) fn snapshot_logical_block_len(&self, guest_block: usize) -> std::io::Result<usize> {
        if guest_block >= self.geometry.num_blocks {
            return Err(snapshot_invalid("snapshot guest block is out of range"));
        }
        let start = guest_block
            .checked_mul(self.geometry.block_size)
            .ok_or_else(|| snapshot_invalid("snapshot guest block offset overflow"))?;
        if start >= self.geometry.guest_len {
            return Err(snapshot_invalid("snapshot guest block starts beyond guest RAM"));
        }
        Ok((self.geometry.guest_len - start).min(self.geometry.block_size))
    }

    /// Where slot `slot` begins in the resident region, checked to hold
    /// `logical_len` bytes. The bound is the resident region alone: the caller
    /// sizes that region from `BlockGeometry::resident_len`, so a slot inside
    /// it is inside the allocation by construction.
    pub(super) fn snapshot_slot_offset(
        &self,
        slot: usize,
        logical_len: usize,
    ) -> std::io::Result<usize> {
        if slot >= self.resident_capacity() {
            return Err(snapshot_invalid("snapshot resident slot is out of range"));
        }
        if logical_len > self.geometry.block_size {
            return Err(snapshot_invalid("snapshot logical block exceeds slot"));
        }
        let offset = slot
            .checked_mul(self.geometry.block_size)
            .ok_or_else(|| snapshot_invalid("snapshot resident slot offset overflow"))?;
        let end = offset
            .checked_add(logical_len)
            .ok_or_else(|| snapshot_invalid("snapshot resident slot length overflow"))?;
        if end > self.geometry.resident_len {
            return Err(snapshot_invalid("snapshot resident slot exceeds host backing"));
        }
        Ok(offset)
    }

    /// Return the configured block geometry without exposing host backing.
    pub(super) fn snapshot_geometry(&self) -> MemorySnapshotGeometry {
        MemorySnapshotGeometry {
            guest_len: self.geometry.guest_len as u64,
            host_ram_len: self.geometry.host_len as u64,
            block_size: self.geometry.block_size as u64,
            num_blocks: self.geometry.num_blocks as u32,
            resident_capacity: self.resident_capacity() as u32,
            used_blocks: self.used_blocks as u32,
            next_swapout_guest_block: self.next_swapout as u32,
        }
    }

    /// Describe one guest block's current backing without making it resident.
    pub(super) fn snapshot_residency(
        &self,
        guest_block: usize,
    ) -> std::io::Result<MemorySnapshotResidency> {
        let logical_len = self.snapshot_logical_block_len(guest_block)?;
        match self.blocks()[guest_block] {
            Block::SwappedOut => Ok(MemorySnapshotResidency::Swapped),
            Block::Block { offset } => {
                if offset % self.geometry.block_size != 0 {
                    return Err(snapshot_invalid("snapshot resident block offset is unaligned"));
                }
                let slot = offset / self.geometry.block_size;
                self.snapshot_slot_offset(slot, logical_len)?;
                Ok(MemorySnapshotResidency::Resident {
                    slot: u32::try_from(slot)
                        .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?,
                })
            }
        }
    }

    /// Stream a non-resident block out of the overflow file, in guest-physical
    /// order and without making it resident.
    ///
    /// Chunked through a fixed scratch buffer so the transfer costs the same
    /// whatever the block size is, and so the file borrow is released around
    /// each caller-controlled write. Every chunk seeks by absolute guest-block
    /// offset, so releasing it cannot affect stream order.
    pub(super) fn write_swapped_block<W: Write>(
        &mut self,
        guest_block: usize,
        logical_len: usize,
        out: &mut W,
    ) -> std::io::Result<()> {
        let offset = guest_block
            .checked_mul(self.geometry.block_size)
            .ok_or_else(|| snapshot_invalid("snapshot overflow offset overflow"))?;
        let mut scratch = [0u8; SNAPSHOT_IO_CHUNK];
        let mut remaining = logical_len;
        while remaining != 0 {
            let chunk_len = remaining.min(scratch.len());
            let chunk_offset = offset
                .checked_add(logical_len - remaining)
                .ok_or_else(|| snapshot_invalid("snapshot overflow offset overflow"))?;
            {
                let file = &mut self.overflow_file;
                file.seek(SeekFrom::Start(u64::try_from(chunk_offset).map_err(
                    |_| snapshot_invalid("snapshot overflow offset conversion failed"),
                )?))?;
                let mut read = 0;
                while read != chunk_len {
                    let count = file.read(&mut scratch[read..chunk_len])?;
                    if count == 0 {
                        // The file is sparse past its written extent; a block
                        // that was never evicted reads back as the zeros the
                        // guest would have found there.
                        scratch[read..chunk_len].fill(0);
                        break;
                    }
                    read += count;
                }
            }
            out.write_all(&scratch[..chunk_len])?;
            remaining -= chunk_len;
        }
        Ok(())
    }

    /// Restore a non-resident block into the overflow file. The block map
    /// itself stays untouched until `finish_snapshot_restore` has validated
    /// every descriptor and the whole transfer has succeeded.
    pub(super) fn read_swapped_block<R: Read>(
        &mut self,
        guest_block: usize,
        logical_len: usize,
        input: &mut R,
    ) -> std::io::Result<()> {
        let offset = guest_block
            .checked_mul(self.geometry.block_size)
            .ok_or_else(|| snapshot_invalid("snapshot overflow offset overflow"))?;
        let mut scratch = [0u8; SNAPSHOT_IO_CHUNK];
        let mut remaining = logical_len;
        while remaining != 0 {
            let chunk_len = remaining.min(scratch.len());
            input.read_exact(&mut scratch[..chunk_len])?;
            let chunk_offset = offset
                .checked_add(logical_len - remaining)
                .ok_or_else(|| snapshot_invalid("snapshot overflow offset overflow"))?;
            {
                let file = &mut self.overflow_file;
                file.seek(SeekFrom::Start(u64::try_from(chunk_offset).map_err(
                    |_| snapshot_invalid("snapshot overflow offset conversion failed"),
                )?))?;
                file.write_all(&scratch[..chunk_len])?;
            }
            remaining -= chunk_len;
        }
        Ok(())
    }

    /// Validate and atomically install snapshot residency metadata.
    ///
    /// `ram` is the resident region, needed only to clear the physical tail of
    /// a short final block: a partial guest block must not make the bytes past
    /// its end architectural.
    pub(super) fn finish_snapshot_restore(
        &mut self,
        ram: &mut [u8],
        geometry: MemorySnapshotGeometry,
        saved_map: &[MemorySnapshotResidency],
    ) -> std::io::Result<()> {
        let expected_num_blocks = self.expected_num_blocks()?;
        let resident_capacity = self.resident_capacity();
        if geometry.guest_len != self.geometry.guest_len as u64
            || geometry.host_ram_len != self.geometry.host_len as u64
            || geometry.block_size != self.geometry.block_size as u64
            || usize::try_from(geometry.num_blocks)
                .map_err(|_| snapshot_invalid("snapshot block count conversion failed"))?
                != expected_num_blocks
            || self.geometry.num_blocks != expected_num_blocks
            || usize::try_from(geometry.resident_capacity)
                .map_err(|_| snapshot_invalid("snapshot resident capacity conversion failed"))?
                != resident_capacity
            || saved_map.len() != expected_num_blocks
        {
            return Err(snapshot_invalid("snapshot memory geometry does not match machine"));
        }

        let used_blocks = usize::try_from(geometry.used_blocks)
            .map_err(|_| snapshot_invalid("snapshot used block count conversion failed"))?;
        if used_blocks > resident_capacity || used_blocks > expected_num_blocks {
            return Err(snapshot_invalid("snapshot used block count is out of range"));
        }

        let next_swapout = usize::try_from(geometry.next_swapout_guest_block)
            .map_err(|_| snapshot_invalid("snapshot swap cursor conversion failed"))?;
        if (expected_num_blocks == 0 && next_swapout != 0)
            || (expected_num_blocks != 0 && next_swapout >= expected_num_blocks)
        {
            return Err(snapshot_invalid("snapshot swap cursor is out of range"));
        }

        // Descriptor storage is O(number of blocks), never O(guest RAM).
        // Allocate only while validating; byte streaming itself is fixed-size.
        let mut seen_slots = std::vec::Vec::new();
        seen_slots
            .try_reserve_exact(resident_capacity)
            .map_err(|_| snapshot_other("unable to validate snapshot resident slots"))?;
        seen_slots.resize(resident_capacity, false);

        let mut resident_count = 0usize;
        for (guest_block, &saved) in saved_map.iter().enumerate() {
            if let MemorySnapshotResidency::Resident { slot } = saved {
                let slot = usize::try_from(slot)
                    .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?;
                self.snapshot_slot_offset(slot, self.snapshot_logical_block_len(guest_block)?)?;
                let seen = seen_slots
                    .get_mut(slot)
                    .ok_or_else(|| snapshot_invalid("snapshot resident slot is out of range"))?;
                if *seen {
                    return Err(snapshot_invalid("snapshot resident slots are not unique"));
                }
                *seen = true;
                resident_count += 1;
            }
        }
        if resident_count != used_blocks || seen_slots[..used_blocks].iter().any(|seen| !seen) {
            return Err(snapshot_invalid(
                "snapshot resident slots do not form a dense used prefix",
            ));
        }

        // Flush all transferred swapped bytes before changing ownership.
        self.overflow_file.flush()?;

        // A partial final guest block never makes physical tail bytes
        // architectural. Fully backed RAM can have no tail at all, so clear
        // only the bytes that actually exist in its resident slot.
        if let Some(MemorySnapshotResidency::Resident { slot }) = saved_map.last().copied() {
            let final_len = self.snapshot_logical_block_len(expected_num_blocks - 1)?;
            if final_len < self.geometry.block_size {
                let slot = usize::try_from(slot)
                    .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?;
                let slot_offset = self.snapshot_slot_offset(slot, final_len)?;
                let tail_start = slot_offset
                    .checked_add(final_len)
                    .ok_or_else(|| snapshot_invalid("snapshot final slot tail overflow"))?;
                let tail_len = self
                    .geometry
                    .resident_len
                    .saturating_sub(tail_start)
                    .min(self.geometry.block_size - final_len);
                if tail_len != 0 {
                    ram[tail_start..tail_start + tail_len].fill(0);
                }
            }
        }

        // Commit only after every descriptor, byte transfer, and overflow flush
        // succeeded. ROM, bogus/APIC scratch, padding, and CPU TLB pointers are
        // intentionally outside this block-logical state.
        for (guest_block, saved) in saved_map.iter().copied().enumerate() {
            self.blocks[guest_block] = match saved {
                MemorySnapshotResidency::Swapped => Block::SwappedOut,
                MemorySnapshotResidency::Resident { slot } => Block::Block {
                    offset: usize::try_from(slot)
                        .map_err(|_| snapshot_invalid("snapshot resident slot conversion failed"))?
                        * self.geometry.block_size,
                },
            };
        }
        self.used_blocks = used_blocks;
        self.next_swapout = next_swapout;
        // The whole block table was just rewritten from the snapshot, so any
        // offset cached against the pre-restore layout is stale.
        self.bump_swap_epoch();
        self.recompute_identity_map();
        Ok(())
    }
}

