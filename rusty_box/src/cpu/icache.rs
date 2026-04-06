#![allow(dead_code)]
use alloc::{vec, vec::Vec};

use crate::{
    config::BxPhyAddress,
    cpu::{
        cpu::BX_ASYNC_EVENT_STOP_TRACE,
        decoder::{decode32, decode64, Instruction, Opcode},
        tlb::{lpf_of, page_offset, ppf_of},
        BxCpuC, BxCpuIdTrait, Result,
    },
    memory::BxMemC,
};

// Slice-based BxPageWriteStampTable for use with memory system
#[derive(Debug)]
pub struct BxPageWriteStampTable<'a> {
    pub(crate) fine_granularity_mapping: &'a mut [u32],
}

impl<'a> BxPageWriteStampTable<'a> {
    pub fn new(fine_granularity_mapping: &'a mut [u32]) -> Self {
        Self {
            fine_granularity_mapping,
        }
    }

    /// Increment write stamp for a partial page write
    /// Assumption: write does not split 4K page
    pub fn inc_write_stamp_with_len(&mut self, p_addr: BxPhyAddress, len: u32) {
        if self.fine_granularity_mapping.is_empty() {
            return;
        }
        let index = Self::hash(p_addr);
        if index < self.fine_granularity_mapping.len()
            && self.fine_granularity_mapping[index] != 0 {
                // Calculate mask for affected cache lines (128-byte granularity)
                let page_offset = (p_addr as u32) & 0xfff;
                let shift1 = (page_offset >> 7).min(31);
                let shift2 = ((page_offset + len - 1) >> 7).min(31);
                let mut mask: u32 = 1 << shift1;
                mask |= 1 << shift2;

                if self.fine_granularity_mapping[index] & mask != 0 {
                    // TODO: Call handle_smc to invalidate instruction cache
                    // This requires access to CPUs which we don't have here
                    // For now, just clear the affected bits
                    self.fine_granularity_mapping[index] &= !mask;
                }
            }
    }

    /// Decrement write stamp for a whole page
    /// This invalidates instruction cache entries for the entire page
    pub fn dec_write_stamp(&mut self, p_addr: BxPhyAddress) {
        self.dec_write_stamp_with_len(p_addr, 4096)
    }

    /// Decrement write stamp for a partial page write
    /// Assumption: write does not split 4K page
    pub fn dec_write_stamp_with_len(&mut self, p_addr: BxPhyAddress, len: u32) {
        if self.fine_granularity_mapping.is_empty() {
            return;
        }
        let index = Self::hash(p_addr);
        if index < self.fine_granularity_mapping.len()
            && self.fine_granularity_mapping[index] != 0 {
                // Calculate mask for affected cache lines (128-byte granularity)
                let page_offset = (p_addr as u32) & 0xfff;
                let shift1 = (page_offset >> 7).min(31);
                let shift2 = ((page_offset + len - 1) >> 7).min(31);
                let mut mask: u32 = 1 << shift1;
                mask |= 1 << shift2;

                if self.fine_granularity_mapping[index] & mask != 0 {
                    // TODO: Call handle_smc to invalidate instruction cache
                    // This requires access to CPUs which we don't have here
                    // For now, just clear the affected bits
                    self.fine_granularity_mapping[index] &= !mask;
                }
            }
    }

    pub fn mark_icache_mask(&mut self, p_addr: BxPhyAddress, mask: u32) {
        if self.fine_granularity_mapping.is_empty() {
            return;
        }
        let index = Self::hash(p_addr);
        if index < self.fine_granularity_mapping.len() {
            self.fine_granularity_mapping[index] |= mask;
        }
    }

    fn hash(p_addr: BxPhyAddress) -> usize {
        lpf_of(p_addr) as usize
    }
}

// Internal array-based BxPageWriteStampTable for use within icache (not exported)
#[derive(Debug)]
struct BxPageWriteStampTableInternal {
    fine_granularity_mapping: [u32; 32768], // 128MB / 4KB = 32768 pages
}

fn handle_smc<I: BxCpuIdTrait>(cpus: &mut [BxCpuC<I>], p_addr: BxPhyAddress, mask: u32) {
    // INC_SMC_STAT(smc);
    for cpu in cpus {
        cpu.i_cache.handle_smc(p_addr, mask);
    }
}

const BX_ICACHE_INVALID_PHY_ADDRESS: BxPhyAddress = BxPhyAddress::MAX;
const BX_ICACHE_ENTRIES: usize = 8192;
pub(super) const BX_ICACHE_MEM_POOL: usize = 576 * 1024;
const BX_MAX_TRACE_LENGTH: usize = 32;

#[derive(Debug, PartialEq, Clone, Default, Copy)]
pub(crate) enum IcacheAddress {
    #[default]
    Invalid,
    Address(BxPhyAddress),
}

impl From<IcacheAddress> for BxPhyAddress {
    fn from(value: IcacheAddress) -> Self {
        match value {
            IcacheAddress::Invalid => BX_ICACHE_INVALID_PHY_ADDRESS,
            IcacheAddress::Address(addr) => addr,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BxICacheEntry {
    // p_addr: BxPhyAddress, // Physical address of the instruction
    pub(super) p_addr: IcacheAddress, // Physical address of the instruction
    pub(super) trace_mask: u32,
    // orignial replaced
    //tlen: u32, // Trace length in instructions
    pub(super) tlen: usize, // Trace length in instructions
    pub(super) i: Instruction,
    // mpool_start_idx: Index in mpool where this trace starts
    // In C++, entry->i is a pointer, so we can do pointer arithmetic
    // In Rust, we need to store the index explicitly
    pub(super) mpool_start_idx: usize,
    /// First 8 bytes of this trace's instruction stream (for SMC detection).
    /// Compared against current memory on icache lookup to detect stale entries
    /// when code memory has been overwritten (e.g., LILO loading kernel via REP INSW).
    /// Bochs uses per-page write stamps; this is a simpler but effective alternative.
    pub(super) first_bytes: [u8; 8],
}

/// Number of pages in 4GB physical address space (4GB / 4KB = 1M pages).
const PHY_MEM_PAGES: usize = 1024 * 1024;

pub struct BxICache {
    pub(crate) entry: [BxICacheEntry; BX_ICACHE_ENTRIES],
    /// Vec to avoid stack overflow - this is ~15 MB!
    /// Using Vec instead of array moves allocation to heap
    pub(crate) mpool: Vec<Instruction>,
    pub(crate) mpindex: usize,
    next_page_split_index: usize,
    page_split_index: [PageSplitEntry; BX_ICACHE_ENTRIES],
    /// Per-page fine-granularity bitmask for SMC (Self-Modifying Code) detection.
    /// Matching Bochs `bxPageWriteStampTable::fineGranularityMapping`.
    ///
    /// Each 4KB page gets one u32 entry. Each bit represents a 128-byte "cache line"
    /// within the page (4096 / 128 = 32 lines = 32 bits). If a bit is set, there
    /// exists an icache trace covering that cache line on that page.
    ///
    /// On memory write, if the written cache line's bit is set in the stamp,
    /// `handle_smc_scan()` is called to invalidate affected icache entries.
    /// This avoids the previous bug where `invalidate_page()` only checked one
    /// hash index and missed entries at other offsets within the page.
    page_write_stamps: Vec<u32>,
}

#[derive(Clone, Debug)]
struct PageSplitEntry {
    ppf: BxPhyAddress,
    e: BxICacheEntry,
}

impl Default for PageSplitEntry {
    fn default() -> Self {
        Self {
            ppf: BX_ICACHE_INVALID_PHY_ADDRESS,
            e: BxICacheEntry {
                p_addr: IcacheAddress::Invalid,
                trace_mask: 0,
                tlen: 0,
                i: Instruction::default(),
                mpool_start_idx: 0,
                first_bytes: [0; 8],
            },
        }
    }
}

impl Default for BxICache {
    fn default() -> Self {
        Self::new()
    }
}

impl BxICache {
    pub fn new() -> Self {
        Self {
            entry: core::array::from_fn(|_| BxICacheEntry {
                p_addr: IcacheAddress::Invalid,
                trace_mask: 0,
                tlen: 0,
                i: Instruction::default(),
                mpool_start_idx: 0,
                first_bytes: [0; 8],
            }),
            // Allocate on heap to avoid 15 MB stack allocation
            // vec![val; size] is efficient and heap-allocated
            mpool: vec![Instruction::default(); BX_ICACHE_MEM_POOL],
            mpindex: 0,
            next_page_split_index: 0,
            page_split_index: core::array::from_fn(|_| PageSplitEntry::default()),
            // 4MB heap allocation for 1M pages covering full 4GB physical address space
            page_write_stamps: vec![0u32; PHY_MEM_PAGES],
        }
    }

    pub fn alloc_trace(&mut self, entry_idx: usize) {
        let entry = &mut self.entry[entry_idx];
        if entry.p_addr != IcacheAddress::Invalid {
            flush_smc(entry);
        }
    }

    pub fn commit_trace(&mut self, _tlen: usize) {
        // Update mpindex to point past the last instruction in the trace
        // In C++, this is handled by the pointer arithmetic on entry->i
        // Here, we track it explicitly with mpindex
    }

    pub fn commit_page_split_trace(&mut self, p_addr: BxPhyAddress, e: BxICacheEntry) {
        // Store the page split entry
        self.page_split_index[self.next_page_split_index].ppf = p_addr;
        self.page_split_index[self.next_page_split_index].e = e.clone();
        self.next_page_split_index = (self.next_page_split_index + 1) % BX_ICACHE_ENTRIES;
    }

    pub fn get_entry(&self, p_addr: BxPhyAddress, fetch_mode_mask: u64) -> BxICacheEntry {
        let index = Self::hash(p_addr, fetch_mode_mask);
        self.entry[index as usize].clone()
    }

    #[inline]
    pub fn get_entry_mut(
        &mut self,
        p_addr: BxPhyAddress,
        fetch_mode_mask: u64,
    ) -> &mut BxICacheEntry {
        let index = Self::hash(p_addr, fetch_mode_mask);
        &mut self.entry[index as usize]
    }

    pub(super) fn hash(p_addr: BxPhyAddress, fetch_mode_mask: u64) -> u32 {
        // Bochs icache.h:143 — (pAddr & (BxICacheEntries-1)) ^ fetchModeMask
        let hash = (p_addr as u32) ^ (fetch_mode_mask as u32);
        hash & ((BX_ICACHE_ENTRIES - 1) as u32)
    }

    pub(super) fn find_trace_start(
        &self,
        _entry: &BxICacheEntry,
        _entry_idx: usize,
    ) -> Option<usize> {
        // Find where the trace starts in mpool
        // This is a simplified implementation - in C++, entry->i is a pointer
        // Here we need to search for the first instruction

        // For now, return None to indicate trace not found
        // A more sophisticated implementation would track trace locations
        // TODO: Use algebraic types for clarity?
        None
    }

    pub(super) fn find_entry(
        &self,
        p_addr: BxPhyAddress,
        fetch_mode_mask: u64,
    ) -> Option<BxICacheEntry> {
        let e = self.get_entry(p_addr, fetch_mode_mask);
        if BxPhyAddress::from(e.p_addr) != p_addr {
            return None;
        }
        Some(e)
    }

    fn handle_smc(&mut self, p_addr: BxPhyAddress, mask: u32) {
        let index = Self::hash(p_addr, 0) as usize;
        let entry = &mut self.entry[index];

        if let IcacheAddress::Address(addr) = entry.p_addr {
            if addr == p_addr
                && entry.trace_mask & mask != 0 {
                    flush_smc(entry);
                }
        }

        // Check page split entries
        let ppf = ppf_of(p_addr);
        for i in 0..BX_ICACHE_ENTRIES {
            if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS
                && ppf_of(self.page_split_index[i].ppf) == ppf {
                    flush_smc(&mut self.page_split_index[i].e);
                }
        }
    }

    pub fn flush_page(&mut self, ppf: BxPhyAddress) {
        let index = Self::hash(ppf, 0) as usize;
        let entry = &mut self.entry[index];

        if let IcacheAddress::Address(addr) = entry.p_addr {
            if ppf_of(addr) == ppf {
                flush_smc(entry);
            }
        }

        // Flush page split entries
        for i in 0..BX_ICACHE_ENTRIES {
            if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS
                && ppf_of(self.page_split_index[i].ppf) == ppf {
                    flush_smc(&mut self.page_split_index[i].e);
                }
        }
    }

    /// Bochs icache.h:191 — breakLinks()
    /// Called on every TLB flush (CR3 write, INVLPG, CR0/CR4 write).
    /// Invalidates page-split icache entries so page-boundary instructions
    /// don't serve stale bytes from old physical pages after remapping.
    pub fn break_links(&mut self) {
        // Bochs: invalidatePageSplitICacheEntries()
        for entry in &mut self.page_split_index {
            if entry.ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                entry.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                flush_smc(&mut entry.e);
            }
        }
        self.next_page_split_index = 0;
    }

    pub fn flush_all(&mut self) {
        for entry in &mut self.entry {
            flush_smc(entry);
        }
        for entry in &mut self.page_split_index {
            if entry.ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                entry.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                flush_smc(&mut entry.e);
            }
        }

        // Reset mpool write pointer so new traces can be allocated from the start
        self.mpindex = 0;

        // Clear all page write stamps since no cached entries remain
        self.reset_write_stamps();
    }

    pub fn invalidate_page(&mut self, ppf: BxPhyAddress) {
        let index = Self::hash(ppf, 0) as usize;
        let entry = &mut self.entry[index];

        if let IcacheAddress::Address(addr) = entry.p_addr {
            if ppf_of(addr) == ppf {
                entry.p_addr = IcacheAddress::Invalid;
            }
        }

        // Invalidate page split entries
        for i in 0..BX_ICACHE_ENTRIES {
            if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS
                && ppf_of(self.page_split_index[i].ppf) == ppf {
                    self.page_split_index[i].ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                    flush_smc(&mut self.page_split_index[i].e);
                }
        }
    }

    pub fn invalidate_all(&mut self) {
        for entry in &mut self.entry {
            entry.p_addr = IcacheAddress::Invalid;
        }
        for entry in &mut self.page_split_index {
            if entry.ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                entry.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                flush_smc(&mut entry.e);
            }
        }
    }

    // =========================================================================
    // Bochs-style pageWriteStampTable SMC detection
    // Reference: cpp_orig/bochs/cpu/icache.h lines 29-101
    // =========================================================================

    /// Hash physical address to page write stamp index.
    /// Matching Bochs `bxPageWriteStampTable::hash()`.
    #[inline]
    fn stamp_hash(p_addr: BxPhyAddress) -> usize {
        ((p_addr as u32) >> 12) as usize
    }

    /// Compute the 128-byte cache line bitmask for a physical address range.
    /// Each bit represents one of the 32 cache lines in a 4KB page.
    /// Matching Bochs `markICache()` / `decWriteStamp()` mask computation.
    #[inline]
    fn cache_line_mask(p_addr: BxPhyAddress, len: u32) -> u32 {
        let page_off = (p_addr as u32) & 0xFFF;
        let shift1 = (page_off >> 7).min(31);
        let shift2 = ((page_off.wrapping_add(len - 1)) >> 7).min(31);
        (1u32 << shift1) | (1u32 << shift2)
    }

    /// Called when creating a new trace entry in serve_icache_miss().
    /// Marks the page as having icache entries covering the given physical address range.
    /// Matching Bochs `bxPageWriteStampTable::markICache()`.
    pub fn mark_icache(&mut self, p_addr: BxPhyAddress, len: u32) {
        let index = Self::stamp_hash(p_addr);
        if index < self.page_write_stamps.len() {
            let mask = Self::cache_line_mask(p_addr, len);
            self.page_write_stamps[index] |= mask;
        }
    }

    /// Mark icache with a pre-computed cache line mask.
    /// Matching Bochs `bxPageWriteStampTable::markICacheMask()`.
    pub fn mark_icache_mask(&mut self, p_addr: BxPhyAddress, mask: u32) {
        let index = Self::stamp_hash(p_addr);
        if index < self.page_write_stamps.len() {
            self.page_write_stamps[index] |= mask;
        }
    }

    /// Called on every memory write. Checks if the write overlaps a page with
    /// cached instructions, and if so, invalidates affected icache entries.
    /// Matching Bochs `bxPageWriteStampTable::decWriteStamp(pAddr, len)`.
    ///
    /// This replaces the old `invalidate_page()` approach which only checked
    /// one hash index per page and missed entries at other offsets.
    pub fn smc_write_check(&mut self, p_addr: BxPhyAddress, len: u32) {
        let index = Self::stamp_hash(p_addr);
        if index >= self.page_write_stamps.len() {
            return;
        }
        if self.page_write_stamps[index] == 0 {
            return; // Fast path: no cached instructions on this page
        }

        let mask = Self::cache_line_mask(p_addr, len);
        if self.page_write_stamps[index] & mask == 0 {
            return; // Write doesn't overlap any cached cache lines
        }

        // SMC detected — invalidate affected icache entries
        self.handle_smc_scan(p_addr, mask);
        self.page_write_stamps[index] &= !mask;
    }

    /// Scan icache entries and invalidate any whose physical page matches and
    /// whose trace_mask overlaps the written cache lines.
    /// Matching Bochs `bxICache_c::handleSMC()`.
    fn handle_smc_scan(&mut self, p_addr: BxPhyAddress, mask: u32) {
        let target_page_index = Self::stamp_hash(p_addr);

        tracing::debug!(
            "SMC detected: p_addr={:#x}, page_index={:#x}, mask={:#010b}",
            p_addr,
            target_page_index,
            mask
        );

        // Scan all icache entries for ones that belong to the affected page
        // and have overlapping trace_mask bits.
        for entry in &mut self.entry {
            if let IcacheAddress::Address(entry_addr) = entry.p_addr {
                if Self::stamp_hash(entry_addr) == target_page_index
                    && (entry.trace_mask & mask) != 0
                {
                    flush_smc(entry);
                }
            }
        }

        // Also check page split entries — a write to the first cache line could
        // affect traces that start on the previous page and spill into this one.
        if mask & 0x1 != 0 {
            for pse in &mut self.page_split_index {
                if pse.ppf != BX_ICACHE_INVALID_PHY_ADDRESS
                    && Self::stamp_hash(pse.ppf) == target_page_index
                {
                    pse.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                    flush_smc(&mut pse.e);
                }
            }
        }
    }

    /// Reset all page write stamps (e.g., on full icache flush).
    pub fn reset_write_stamps(&mut self) {
        self.page_write_stamps.fill(0);
    }
}

fn flush_smc(e: &mut BxICacheEntry) {
    // Matching C++ line 64-74: flushSMC
    if let IcacheAddress::Address(_) = e.p_addr {
        e.p_addr = IcacheAddress::Invalid;

        // If handlers chaining speedups are enabled, generate dummy entry
        // (matching C++ line 66-72)
        #[cfg(feature = "bx_support_handlers_chaining_speedups")]
        {
            // TODO: Check if debugger is active (matching C++ line 67)
            // For now, always generate dummy entry
            gen_dummy_icache_entry(&mut e.i);
        }
    }
}

fn gen_dummy_icache_entry(i: &mut Instruction) {
    // Matching C++ line 88-90: genDummyICacheEntry
    i.set_ilen(0);
    i.set_ia_opcode(Opcode::InsertedOpcode);
    // Note: In C++, execute1 is set to &BX_CPU_C::BxEndTrace
    // In Rust, we check for Opcode::InsertedOpcode in cpu_loop_n and set async_event
}

/// Check if an opcode is a trace-ending instruction (control flow change).
/// Matching Bochs BxTraceEnd flag: branches, jumps, calls, returns, loops,
/// interrupts, IRET, HLT, and system call instructions all end the trace.
fn is_trace_end_opcode(opcode: Opcode) -> bool {
    matches!(
        opcode,
        // Jumps (near)
        Opcode::JmpEd | Opcode::JmpEw | Opcode::JmpJw | Opcode::JmpJbw |
        Opcode::JmpJd | Opcode::JmpJbd |
        // Jumps (far)
        Opcode::JmpfAp | Opcode::JmpfOp16Ep | Opcode::JmpfOp32Ep |
        // Jumps (64-bit)
        Opcode::JmpJq | Opcode::JmpJbq | Opcode::JmpEq | Opcode::JmpfOp64Ep |
        // Calls (near)
        Opcode::CallEd | Opcode::CallEw | Opcode::CallJd | Opcode::CallJw |
        // Calls (far)
        Opcode::CallfOp16Ap | Opcode::CallfOp32Ap |
        Opcode::CallfOp16Ep | Opcode::CallfOp32Ep |
        // Calls (64-bit)
        Opcode::CallJq | Opcode::CallEq | Opcode::CallfOp64Ep |
        // Returns (near)
        Opcode::RetOp16 | Opcode::RetOp16Iw | Opcode::RetOp32 | Opcode::RetOp32Iw |
        Opcode::RetOp64 | Opcode::RetOp64Iw |
        // Returns (far)
        Opcode::RetfOp16 | Opcode::RetfOp16Iw | Opcode::RetfOp32 | Opcode::RetfOp32Iw |
        Opcode::RetfOp64 | Opcode::RetfOp64Iw |
        // Conditional jumps (16-bit relative)
        Opcode::JoJw | Opcode::JnoJw | Opcode::JbJw | Opcode::JnbJw |
        Opcode::JzJw | Opcode::JnzJw | Opcode::JbeJw | Opcode::JnbeJw |
        Opcode::JsJw | Opcode::JnsJw | Opcode::JpJw | Opcode::JnpJw |
        Opcode::JlJw | Opcode::JnlJw | Opcode::JleJw | Opcode::JnleJw |
        // Conditional jumps (8-bit, 16-bit mode)
        Opcode::JoJbw | Opcode::JnoJbw | Opcode::JbJbw | Opcode::JnbJbw |
        Opcode::JzJbw | Opcode::JnzJbw | Opcode::JbeJbw | Opcode::JnbeJbw |
        Opcode::JsJbw | Opcode::JnsJbw | Opcode::JpJbw | Opcode::JnpJbw |
        Opcode::JlJbw | Opcode::JnlJbw | Opcode::JleJbw | Opcode::JnleJbw |
        // Conditional jumps (32-bit relative)
        Opcode::JoJd | Opcode::JnoJd | Opcode::JbJd | Opcode::JnbJd |
        Opcode::JzJd | Opcode::JnzJd | Opcode::JbeJd | Opcode::JnbeJd |
        Opcode::JsJd | Opcode::JnsJd | Opcode::JpJd | Opcode::JnpJd |
        Opcode::JlJd | Opcode::JnlJd | Opcode::JleJd | Opcode::JnleJd |
        // Conditional jumps (8-bit, 32-bit mode)
        Opcode::JoJbd | Opcode::JnoJbd | Opcode::JbJbd | Opcode::JnbJbd |
        Opcode::JzJbd | Opcode::JnzJbd | Opcode::JbeJbd | Opcode::JnbeJbd |
        Opcode::JsJbd | Opcode::JnsJbd | Opcode::JpJbd | Opcode::JnpJbd |
        Opcode::JlJbd | Opcode::JnlJbd | Opcode::JleJbd | Opcode::JnleJbd |
        // Conditional jumps (64-bit relative)
        Opcode::JoJq | Opcode::JnoJq | Opcode::JbJq | Opcode::JnbJq |
        Opcode::JzJq | Opcode::JnzJq | Opcode::JbeJq | Opcode::JnbeJq |
        Opcode::JsJq | Opcode::JnsJq | Opcode::JpJq | Opcode::JnpJq |
        Opcode::JlJq | Opcode::JnlJq | Opcode::JleJq | Opcode::JnleJq |
        // Conditional jumps (8-bit, 64-bit mode)
        Opcode::JoJbq | Opcode::JnoJbq | Opcode::JbJbq | Opcode::JnbJbq |
        Opcode::JzJbq | Opcode::JnzJbq | Opcode::JbeJbq | Opcode::JnbeJbq |
        Opcode::JsJbq | Opcode::JnsJbq | Opcode::JpJbq | Opcode::JnpJbq |
        Opcode::JlJbq | Opcode::JnlJbq | Opcode::JleJbq | Opcode::JnleJbq |
        // Loops
        Opcode::LoopJbw | Opcode::LoopeJbw | Opcode::LoopneJbw |
        Opcode::LoopJbd | Opcode::LoopeJbd | Opcode::LoopneJbd |
        Opcode::LoopJbq | Opcode::LoopeJbq | Opcode::LoopneJbq |
        // JCXZ/JECXZ/JRCXZ
        Opcode::JcxzJbw | Opcode::JecxzJbd | Opcode::JrcxzJbq |
        // Interrupts
        Opcode::IntIb | Opcode::Int0 |
        // Interrupt returns
        Opcode::IretOp16 | Opcode::IretOp32 | Opcode::IretOp64 |
        // Halt
        Opcode::Hlt |
        // System calls
        Opcode::Syscall | Opcode::Sysret |
        Opcode::SyscallLegacy | Opcode::SysretLegacy |
        Opcode::Sysenter | Opcode::Sysexit
    )
}

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    fn bx_end_trace(&mut self) {
        self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
    }

    pub(super) fn serve_icache_miss(
        &mut self,
        eip_biased: u32,
        p_addr: BxPhyAddress,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
        page_write_stamp_table: &mut BxPageWriteStampTable,
    ) -> Result<BxICacheEntry> {
        // Get entry index first to avoid borrow conflicts
        let entry_idx = BxICache::hash(p_addr, self.fetch_mode_mask.bits().into()) as usize;

        // Matching C++ icache.cc:106-107 - use eip_biased directly
        // Safety check: ensure eip_biased is within bounds (defensive programming)
        if eip_biased >= self.eip_page_window_size {
            tracing::error!(
                "serve_icache_miss: eip_biased ({}) >= eip_page_window_size ({}), pAddr={:#x}",
                eip_biased,
                self.eip_page_window_size,
                p_addr
            );
            return Err(crate::cpu::CpuError::CpuNotInitialized);
        }

        let remaining_in_page = self.eip_page_window_size - eip_biased;
        let fetch_ptr_slice = self
            .eip_fetch_ptr
            .ok_or(crate::cpu::CpuError::CpuNotInitialized)?;
        if eip_biased as usize >= fetch_ptr_slice.len() {
            tracing::error!(
                "serve_icache_miss: eip_biased ({}) >= fetch_ptr_slice.len ({}), pAddr={:#x}",
                eip_biased,
                fetch_ptr_slice.len(),
                p_addr
            );
            return Err(crate::cpu::CpuError::CpuNotInitialized);
        }
        let fetch_ptr = &fetch_ptr_slice[eip_biased as usize..];
        let page_offset = page_offset(p_addr as u32);

        let mut trace_mask = 0u32;

        // Check if this is the stack page and invalidate stack cache if needed
        // (matching C++ line 115-118: #if BX_SUPPORT_SMP == 0)
        // Note: In SMP mode, this check is skipped in C++
        #[cfg(not(feature = "bx_support_smp"))]
        {
            if ppf_of(p_addr) == self.p_addr_stack_page {
                self.invalidate_stack_cache();
            }
        }

        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        let is_32_bit_mode = self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
            .cache
            .u
            .segment_d_b();
        let quantum = BX_MAX_TRACE_LENGTH;

        // Matching Bochs: when mpool is nearly full, flush all icache entries and
        // reset mpindex to 0. Without this, once mpindex reaches BX_ICACHE_MEM_POOL,
        // all new traces get tlen=0 and point to stale decoded instructions, causing
        // the CPU to execute wrong opcodes (e.g., RET decoded as POP).
        if self.i_cache.mpindex + BX_MAX_TRACE_LENGTH >= BX_ICACHE_MEM_POOL {
            tracing::debug!(
                "mpool nearly full (mpindex={}), flushing icache and resetting",
                self.i_cache.mpindex
            );
            self.i_cache.flush_all();
        }

        let mut current_mpindex = self.i_cache.mpindex;

        // Initialize entry
        self.i_cache.alloc_trace(entry_idx);
        let trace_start_idx = current_mpindex; // Store where this trace starts in mpool
        {
            let entry = &mut self.i_cache.entry[entry_idx];
            entry.p_addr = IcacheAddress::Address(p_addr);
            entry.trace_mask = 0;
            entry.mpool_start_idx = trace_start_idx;
            // Store first 8 bytes of instruction stream for SMC detection.
            // Checking just the first byte is insufficient — if code is loaded
            // at offset+1 (e.g., kernel fill loop at 0x3261 with padding 0x00 at 0x3260),
            // the first byte matches but subsequent bytes are stale.
            let copy_len = fetch_ptr.len().min(8);
            entry.first_bytes[..copy_len].copy_from_slice(&fetch_ptr[..copy_len]);
            if copy_len < 8 {
                entry.first_bytes[copy_len..].fill(0);
            }
        }

        let mut current_p_addr = p_addr;
        let mut current_page_offset = page_offset;
        let mut current_fetch_ptr = fetch_ptr;
        // Preserve original remaining_in_page for boundary_fetch
        let _original_remaining_in_page = remaining_in_page;
        let mut remaining = remaining_in_page;
        let mut tlen = 0usize;

        for n in 0..quantum {
            // Check bounds before accessing mpool
            if current_mpindex >= BX_ICACHE_MEM_POOL {
                // Only log once per trace to reduce spam - mpool full is handled gracefully
                if current_mpindex == BX_ICACHE_MEM_POOL {
                    tracing::debug!(
                        "mpool full, stopping trace (this is normal if cache is heavily used)"
                    );
                }
                break;
            }

            // Decode instruction based on CPU mode — Bochs style: write directly into mpool slot
            let long64 = self.long64_mode();
            let decode_result = if long64 {
                decode64::fetch_decode64(current_fetch_ptr).map(|instr| {
                    self.i_cache.mpool[current_mpindex] = instr;
                })
            } else {
                // Bochs fetchDecode32(fetchPtr, &mpool[mpindex], remain) — inplace, no copy
                decode32::fetch_decode32_inplace(
                    current_fetch_ptr,
                    is_32_bit_mode,
                    &mut self.i_cache.mpool[current_mpindex],
                )
            };

            match decode_result {
                Ok(()) => {

                    // Instruction is already in mpool[current_mpindex] — get its length
                    let i_len = { self.i_cache.mpool[current_mpindex].ilen() as u32 };

                    // Call assignHandler during trace creation (matching C++ line 169)
                    // This checks feature flags and determines if trace should stop
                    // Note: In C++, handlers are stored in instruction structure (i->execute1, i->handlers.execute2)
                    // In Rust, we can't store function pointers in instruction structure (it's in decoder crate),
                    // so we call assign_handler again during execution to get the handler.
                    // But we still call it here to check if tracing should stop (matching original behavior).
                    // Check if this instruction ends the trace (matching Bochs assignHandler
                    // BxTraceEnd check). Control-flow instructions (branches, jumps, calls,
                    // returns, loops, interrupts) must end the trace so that the next
                    // get_icache_entry call looks up the branch TARGET address, not the
                    // next sequential address.
                    let stop_trace_indication =
                        is_trace_end_opcode(self.i_cache.mpool[current_mpindex].get_ia_opcode());

                    // TODO: Implement BX_INSTR_STORE_OPCODE_BYTES if needed (matching C++ line 175-177)
                    // TODO: Implement BX_INSTR_OPCODE if needed (matching C++ line 178-179)

                    // Update trace mask
                    // Clamp shift amounts to 31 to prevent overflow (u32 has 32 bits, so max shift is 31)
                    let shift1 = (current_page_offset >> 7).min(31);
                    let shift2 = ((current_page_offset + i_len - 1) >> 7).min(31);
                    trace_mask |= 1 << shift1;
                    trace_mask |= 1 << shift2;

                    tlen += 1;
                    current_mpindex += 1;

                    // Check bounds again after increment
                    if current_mpindex >= BX_ICACHE_MEM_POOL {
                        // tracing::warn!("mpool full after increment, stopping trace");
                        break;
                    }

                    // Continue to next instruction
                    remaining = remaining.saturating_sub(i_len);

                    // Check if we should stop tracing (matching C++ line 188)
                    // Stop if: stop trace indication OR remaining in page is 0
                    if stop_trace_indication || remaining == 0 {
                        break;
                    }

                    current_p_addr += i_len as u64;
                    current_page_offset = (current_page_offset + i_len) & 0xfff;
                    current_fetch_ptr = &current_fetch_ptr[i_len as usize..];


                    // Try to find a trace starting from current pAddr and merge
                    // TODO: Check if debugger is active (matching C++ line 194)
                    if remaining >= 15u32 {
                        // avoid merging with page split trace
                        if self.merge_traces_internal(
                            entry_idx,
                            current_mpindex,
                            current_p_addr,
                            tlen,
                        ) {
                            // Update entry and commit
                            {
                                let first_instr = self.i_cache.mpool[trace_start_idx];
                                let entry = &mut self.i_cache.entry[entry_idx];
                                entry.tlen = tlen;
                                entry.trace_mask |= trace_mask;
                                entry.i = first_instr;
                            }
                            page_write_stamp_table.mark_icache_mask(current_p_addr, trace_mask);
                            self.i_cache.mark_icache_mask(current_p_addr, trace_mask);
                            self.i_cache.mpindex = current_mpindex;
                            self.i_cache.commit_trace(tlen);
                            let entry = self.i_cache.entry[entry_idx].clone();
                            return Ok(entry);
                        }
                    }
                }
                Err(decode_err) => {
                    // Fetching instruction on segment/page boundary (matching C++ line 138)
                    // If this is not the first instruction (n > 0), drop the boundary instruction and stop tracing
                    if n > 0 {
                        // The trace is already valid, it has several instructions inside,
                        // in this case just drop the boundary instruction and stop tracing (matching C++ line 140-144)
                        break;
                    }

                    // Calculate remaining bytes for THIS instruction position
                    // For n=0, this equals original_remaining_in_page
                    // For later instructions (if we ever get here), it would be decremented
                    let current_remaining = remaining as usize;
                    tracing::debug!(
                        "DECODE-ERR n=0: remaining={} RIP={:#x} p_addr={:#x} err={:?}",
                        current_remaining,
                        self.rip(),
                        current_p_addr,
                        decode_err
                    );

                    // If there are >= 15 bytes remaining, the instruction SHOULD have fit
                    // in the page. Decode failure with >= 15 bytes means it's NOT a boundary
                    // issue - it's an invalid/unsupported instruction.
                    if current_remaining >= 15 {
                        tracing::error!(
                            "Decode failed with {} bytes remaining (not a boundary issue)",
                            current_remaining,
                        );
                        tracing::error!(
                            "DECODE-FAIL: remaining={} RIP={:#x} CS.base={:#x} EIP={:#x} icount={}",
                            current_remaining,
                            self.rip(),
                            self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
                                .cache
                                .u
                                .segment_base(),
                            self.eip(),
                            self.icount,
                        );
                        tracing::error!("DECODE-FAIL: decode_err={:?}", decode_err);
                        tracing::error!(
                            "DECODE-FAIL: first 32 bytes @ fetch_ptr: {:02x?}",
                            &current_fetch_ptr[..core::cmp::min(32, current_fetch_ptr.len())]
                        );

                        // Check if this is an illegal opcode - if so, generate #UD exception
                        // Based on Bochs exception.cc:937 and cpu.h:248 (Exception::Ud = 6)
                        use crate::cpu::decoder::DecodeError;
                        use rusty_box_decoder::decoder::tables::BxDecodeError;
                        match &decode_err {
                            DecodeError::Decoder(BxDecodeError::BxIllegalOpcode) => {
                                // Bochs: store IaError in trace, don't raise #UD here.
                                // The IaError instruction will be executed normally in the
                                // inner trace loop, where prev_rip is correctly set.
                                // This matches Bochs fetchdecode behavior.
                                tracing::debug!(
                                    "Illegal opcode at RIP={:#x}, storing IaError in trace",
                                    self.rip()
                                );
                                self.i_cache.mpool[current_mpindex].set_ia_opcode(Opcode::IaError);
                                self.i_cache.mpool[current_mpindex].set_ilen(1);
                                // Set trace length to include this IaError entry
                                self.i_cache.entry[entry_idx].tlen = 1;
                                return Ok(self.i_cache.entry[entry_idx].clone());
                            }
                            _ => {
                                // Other decode errors are returned as-is
                                return Err(crate::cpu::CpuError::Decoder(decode_err));
                            }
                        }
                    }

                    // First instruction is boundary fetch, leave the trace cache entry
                    // invalid for now because boundaryFetch() can fault (matching C++ line 146-149)
                    {
                        let entry = &mut self.i_cache.entry[entry_idx];
                        entry.p_addr = IcacheAddress::Invalid; // Mark as invalid temporarily (~entry->pAddr in C++)
                        entry.tlen = 1;
                        entry.mpool_start_idx = current_mpindex; // Store where this trace starts
                    }

                    // Debug logging before boundary_fetch
                    tracing::debug!(
                        "boundary_fetch: n={}, current_remaining={}, p_addr={:#x}",
                        n,
                        current_remaining,
                        current_p_addr
                    );

                    // Call boundary_fetch (matching C++ line 150)
                    // Pass the current remaining bytes to page boundary
                    let boundary_instr =
                        self.boundary_fetch(current_fetch_ptr, current_remaining, mem, cpus)?;

                    // Store instruction in mpool (check bounds first)
                    if current_mpindex >= BX_ICACHE_MEM_POOL {
                        tracing::debug!("mpool full before boundary_instr, stopping trace");
                        break;
                    }
                    self.i_cache.mpool[current_mpindex] = boundary_instr;
                    current_mpindex += 1;

                    // Add the instruction to trace cache (matching C++ line 152-154)
                    {
                        let entry = &mut self.i_cache.entry[entry_idx];
                        entry.p_addr = IcacheAddress::Address(p_addr); // Restore pAddr (~entry->pAddr in C++)
                        entry.trace_mask = 0x80000000; /* last line in page */
                        entry.i = boundary_instr; // Set first instruction for ilen cache hit check
                                                  // Note: tlen is already set to 1 above, no need to set it again
                                                  // mpool_start_idx was already set above
                    }

                    page_write_stamp_table.mark_icache_mask(p_addr, 0x80000000);
                    page_write_stamp_table.mark_icache_mask(self.p_addr_fetch_page, 0x1);
                    self.i_cache.mark_icache_mask(p_addr, 0x80000000);
                    self.i_cache.mark_icache_mask(self.p_addr_fetch_page, 0x1);

                    // Add end-of-trace opcode if not in debugger (matching C++ line 158-163)
                    // TODO: Check debugger active state
                    #[cfg(feature = "bx_support_handlers_chaining_speedups")]
                    {
                        if current_mpindex < BX_ICACHE_MEM_POOL {
                            let entry = &mut self.i_cache.entry[entry_idx];
                            entry.tlen += 1; /* Add the inserted end of trace opcode */
                            gen_dummy_icache_entry(&mut self.i_cache.mpool[current_mpindex]);
                            current_mpindex += 1;
                        }
                    }

                    self.i_cache.mpindex = current_mpindex;
                    let entry = self.i_cache.entry[entry_idx].clone();
                    self.i_cache
                        .commit_page_split_trace(self.p_addr_fetch_page, entry.clone());
                    return Ok(entry);
                }
            }
        }

        // Update entry with final trace mask (matching C++ line 206-208)
        {
            let entry = &mut self.i_cache.entry[entry_idx];
            entry.trace_mask |= trace_mask;
        }
        page_write_stamp_table.mark_icache_mask(current_p_addr, trace_mask);
        self.i_cache.mark_icache_mask(current_p_addr, trace_mask);

        // Add end-of-trace opcode if not in debugger (matching C++ line 210-214)
        // TODO: Check debugger active state
        #[cfg(feature = "bx_support_handlers_chaining_speedups")]
        {
            // Check bounds before accessing mpool
            if current_mpindex < BX_ICACHE_MEM_POOL {
                // Note: tlen will be incremented here, then used below
                gen_dummy_icache_entry(&mut self.i_cache.mpool[current_mpindex]);
                current_mpindex += 1;
                tlen += 1; /* Add the inserted end of trace opcode */
            }
        }

        // Update entry tlen and first instruction (matching C++ line 217)
        // In C++, entry->i is a pointer to the first instruction in mpool.
        // In Rust, we store a copy of the first instruction in entry.i so that
        // find_entry can check i.length != 0 for cache hit validation.
        {
            let first_instr = self.i_cache.mpool[trace_start_idx];
            let entry = &mut self.i_cache.entry[entry_idx];
            entry.tlen = tlen;
            entry.i = first_instr;
        }
        self.i_cache.mpindex = current_mpindex;
        self.i_cache.commit_trace(tlen);

        Ok(self.i_cache.entry[entry_idx].clone())
    }

    fn boundary_fetch(
        &mut self,
        fetch_ptr: &[u8],
        remaining_in_page: usize,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
    ) -> Result<Instruction> {
        let mut fetch_buffer = [0u8; 32];

        tracing::debug!(
            "boundary_fetch: remaining_in_page={} RIP={:#x} icount={}",
            remaining_in_page,
            self.rip(),
            self.icount
        );

        // Based on BX_CPU_C::boundaryFetch in icache.cc
        // If remainingInPage >= 15, instruction should fit in current page
        // This condition indicates too many instruction prefixes -> #GP(0)
        if remaining_in_page >= 15 {
            tracing::error!(
                "boundaryFetch #GP(0): too many instruction prefixes\n\
                 remainingInPage={}, RIP={:#x}, CS.base={:#x}, EIP={:#x}\n\
                 This indicates the instruction has too many prefixes (>15 bytes)\n\
                 or boundary_fetch was called with an incorrect remaining_in_page value.",
                remaining_in_page,
                self.rip(),
                self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
                    .cache
                    .u
                    .segment_base(),
                self.eip()
            );
            self.exception(crate::cpu::cpu::Exception::Gp, 0)?;
        }

        // Read all leftover bytes in current page up to boundary
        fetch_buffer[..remaining_in_page].copy_from_slice(&fetch_ptr[..remaining_in_page]);

        // The 2nd chunk of the instruction is on the next page.
        // Set RIP to the 0th byte of the 2nd page, and force a prefetch
        // (matching C++ line 274-275)
        self.set_rip(self.rip() + remaining_in_page as u64);
        // Call prefetch directly - same lifetime as serve_icache_miss
        self.prefetch(mem, cpus)?;

        let fetch_buffer_limit = (self.eip_page_window_size as usize).min(15);

        // We can fetch straight from the 0th byte, which is eipFetchPtr
        let next_page_fetch_ptr = self
            .eip_fetch_ptr
            .ok_or(crate::cpu::CpuError::CpuNotInitialized)?;

        // Read leftover bytes in next page (matching C++ line 287-289)
        fetch_buffer[remaining_in_page..remaining_in_page + fetch_buffer_limit]
            .copy_from_slice(&next_page_fetch_ptr[..fetch_buffer_limit]);

        // Get is_32_bit_mode from CS segment descriptor d_b flag
        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        let is_32_bit_mode = self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
            .cache
            .u
            .segment_d_b();

        // Decode instruction from combined buffer (matching C++ line 291-296)
        let total_bytes = remaining_in_page + fetch_buffer_limit;
        let decode_result = if self.long64_mode() {
            decode64::fetch_decode64(&fetch_buffer[..total_bytes])
        } else {
            decode32::fetch_decode32(&fetch_buffer[..total_bytes], is_32_bit_mode)
        };

        let instr = match decode_result {
            Ok(i) => i,
            Err(e) => {
                tracing::error!(
                    "boundary_fetch FATAL: total_bytes={} remaining_in_page={} fetch_buffer_limit={} eip_page_window_size={} RIP={:#x} err={:?} bytes={:02x?}",
                    total_bytes, remaining_in_page, fetch_buffer_limit, self.eip_page_window_size,
                    self.rip(), e, &fetch_buffer[..total_bytes.min(16)]
                );
                return Err(e.into());
            }
        };

        // assignHandler is a no-op in Rust (matching C++ line 303)
        // In C++, assignHandler can return non-zero, but we don't check it here

        // Restore EIP since we fudged it to start at the 2nd page boundary.
        // (matching C++ line 306: RIP = BX_CPU_THIS_PTR prev_rip)
        self.set_rip(self.prev_rip);

        // TODO: Implement BX_INSTR_STORE_OPCODE_BYTES if needed (matching C++ line 314-316)
        // TODO: Implement BX_INSTR_OPCODE if needed (matching C++ line 318-319)

        Ok(instr)
    }

    fn merge_traces_internal(
        &mut self,
        current_entry_idx: usize,
        current_mpindex: usize,
        p_addr: BxPhyAddress,
        current_tlen: usize,
    ) -> bool {
        // TODO: Check if debugger is active - should assert !debugger_active

        // Find entry in cache
        let cache_entry_idx = BxICache::hash(p_addr, self.fetch_mode_mask.bits().into()) as usize;

        // Extract cached entry info without holding borrow
        // Check if entry exists and matches (matching C++ line 226-228: if (e != NULL))
        let cached_entry = &self.i_cache.entry[cache_entry_idx];

        // Check if entry is valid (not Invalid address)
        let cached_p_addr = match cached_entry.p_addr {
            IcacheAddress::Address(addr) => addr,
            IcacheAddress::Invalid => return false, // Entry doesn't exist (NULL in C++)
        };

        // Check if this is the right entry (matching pAddr)
        if cached_p_addr != p_addr {
            return false;
        }

        let cached_tlen = cached_entry.tlen;
        let cached_trace_mask = cached_entry.trace_mask;
        let cached_first_instr = cached_entry.i;

        // determine max amount of instruction to take from another entry (matching C++ line 231)
        let max_length = cached_tlen;

        #[cfg(feature = "bx_support_handlers_chaining_speedups")]
        {
            if max_length + current_tlen > BX_MAX_TRACE_LENGTH {
                return false;
            }
        }
        #[cfg(not(feature = "bx_support_handlers_chaining_speedups"))]
        {
            if max_length + current_tlen > BX_MAX_TRACE_LENGTH {
                max_length = BX_MAX_TRACE_LENGTH - current_tlen;
            }
            if max_length == 0 {
                return false;
            }
        }

        // Find where the cached entry's trace starts in mpool
        // Create a temporary entry for searching
        let cached_entry_for_search = BxICacheEntry {
            p_addr: IcacheAddress::Address(cached_p_addr),
            trace_mask: cached_trace_mask,
            tlen: cached_tlen,
            i: cached_first_instr,
            mpool_start_idx: 0, // Not used for search, just need valid struct
            first_bytes: [0; 8],
        };

        if let Some(source_start_idx) = self
            .i_cache
            .find_trace_start(&cached_entry_for_search, cache_entry_idx)
        {
            // Copy instructions from found entry to current trace (matching C++ line 242: memcpy)
            for i in 0..max_length {
                if current_mpindex + i < BX_ICACHE_MEM_POOL
                    && source_start_idx + i < BX_ICACHE_MEM_POOL
                {
                    self.i_cache.mpool[current_mpindex + i] =
                        self.i_cache.mpool[source_start_idx + i];
                }
            }

            // Update current entry (matching C++ line 243-246)
            let current_entry = &mut self.i_cache.entry[current_entry_idx];
            current_entry.tlen += max_length;
            debug_assert!(current_entry.tlen <= BX_MAX_TRACE_LENGTH); // Matching C++ line 244: BX_ASSERT

            current_entry.trace_mask |= cached_trace_mask; // Matching C++ line 246

            return true;
        }

        false
    }
}
