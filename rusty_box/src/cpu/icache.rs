#![allow(dead_code)]

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

/// Number of entries in the machine-wide SMC page-write-stamp table.
/// Bochs icache.h `bxPageWriteStampTable` allocates PHY_MEM_PAGES_IN_4G_SPACE
/// = 1M entries (4GB / 4KB); physical addresses beyond the table's coverage
/// alias into it ("can share writeStamps between multiple pages if >32 bit
/// phy address"). The no-alloc build uses a smaller power-of-two table:
/// heavier aliasing only makes SMC flushing MORE conservative (extra
/// false-positive flushes), never less correct.
#[cfg(feature = "alloc")]
pub(crate) const SMC_STAMP_ENTRIES: usize = 1024 * 1024;
#[cfg(not(feature = "alloc"))]
pub(crate) const SMC_STAMP_ENTRIES: usize = 8192;

/// Bochs icache.h `bxPageWriteStampTable::hash` — stamp-table index of a
/// physical address (page number, aliased into the table).
#[inline]
pub(crate) fn smc_page_index(p_addr: BxPhyAddress) -> usize {
    (((p_addr as u32) >> 12) as usize) & (SMC_STAMP_ENTRIES - 1)
}

/// Bochs icache.h `markICache`/`decWriteStamp` mask computation: one bit per
/// 128-byte line of the 4KB page touched by `[p_addr, p_addr + len)`.
///
/// Callers split cross-page writes before reaching this helper.
#[inline]
pub(crate) fn smc_cache_line_mask(p_addr: BxPhyAddress, len: u32) -> u32 {
    if len == 0 {
        return 0;
    }

    let page_off = (p_addr as u32) & 0x0fff;
    let first_line = page_off >> 7;
    let last_byte = page_off.saturating_add(len - 1).min(0x0fff);
    let last_line = last_byte >> 7;
    let line_count = last_line - first_line + 1;
    let range = if line_count == 32 {
        u32::MAX
    } else {
        (1u32 << line_count) - 1
    };
    range << first_line
}

/// One queued cross-CPU SMC invalidation — a write hit 128-byte lines with
/// cached traces, and every CPU's icache must flush them (Bochs icache.cc
/// `handleSMC` loops over BX_SMP_PROCESSORS synchronously; the queue defers
/// the sibling flushes to the round-robin slice boundary, which no other CPU
/// can execute before).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct PendingSmc {
    pub(crate) p_addr: BxPhyAddress,
    pub(crate) mask: u32,
}

/// Capacity of the pending cross-CPU SMC queue. Overflow falls back to a
/// full icache flush on every CPU that has not caught up — conservative but
/// correct.
pub(crate) const SMC_PENDING_CAP: usize = 64;

const BX_ICACHE_INVALID_PHY_ADDRESS: BxPhyAddress = BxPhyAddress::MAX;
// Bochs icache.h: BxICacheEntries = (64 * 1024). Must be a power of 2.
const BX_ICACHE_ENTRIES: usize = 64 * 1024;
// Bochs icache.h: BX_ICACHE_PAGE_SPLIT_ENTRIES = 8. Must be a power of 2.
const BX_ICACHE_PAGE_SPLIT_ENTRIES: usize = 8;
pub(super) const BX_ICACHE_MEM_POOL: usize = 576 * 1024;
const BX_MAX_TRACE_LENGTH: usize = 32;

#[derive(Debug, Clone)]
pub struct BxICacheEntry {
    /// Physical address of the trace's first instruction. Bochs `bxICacheEntry_c::pAddr`
    /// (icache.h): a raw physical address whose invalid state is the all-ones sentinel
    /// `BX_ICACHE_INVALID_PHY_ADDRESS`, NOT a separate tag. An entry is valid iff
    /// `p_addr == pAddr` for the looked-up address (Bochs `find_entry`).
    pub(super) p_addr: BxPhyAddress,
    pub(super) trace_mask: u32,
    pub(super) tlen: u32, // Bochs bxICacheEntry_c::tlen (Bit32u) — trace length in instructions
    /// Index of the trace's first instruction in `mpool`. This is Rust's stand-in for
    /// Bochs `bxInstruction_c *i` (a pointer): the trace runs `mpool[mpool_start_idx..][..tlen]`.
    /// Bochs stores the pointer; we store the index. (A redundant full `Instruction`
    /// copy used to be kept here for an ilen validity check — removed; validity now
    /// comes from the `p_addr` sentinel, exactly like Bochs `find_entry`.)
    pub(super) mpool_start_idx: usize,
    /// First 8 bytes of this trace's instruction stream (for SMC detection).
    /// Compared against current memory on icache lookup to detect stale entries
    /// when code memory has been overwritten (e.g., LILO loading kernel via REP INSW).
    /// Bochs uses per-page write stamps; this is a simpler but effective alternative.
    pub(super) first_bytes: [u8; 8],
}

// This entry is loaded on every icache lookup (a top hot-path cost), so its size
// drives the lookup cache-miss rate. It mirrors Bochs `bxICacheEntry_c` (24 bytes:
// pAddr + traceMask + tlen + the `i` pointer, which our `mpool_start_idx` stands in
// for) plus our 8-byte `first_bytes` SMC guard. Guard against accidental bloat.
const _: () = assert!(core::mem::size_of::<BxICacheEntry>() == 32);

pub struct BxICache {
    pub(crate) entry: [BxICacheEntry; BX_ICACHE_ENTRIES],
    /// Large array (~15 MB) — struct should be heap-allocated (e.g. via Box).
    pub(crate) mpool: [Instruction; BX_ICACHE_MEM_POOL],
    pub(crate) mpindex: usize,
    next_page_split_index: usize,
    page_split_index: [PageSplitEntry; BX_ICACHE_PAGE_SPLIT_ENTRIES],
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
                p_addr: BX_ICACHE_INVALID_PHY_ADDRESS,
                trace_mask: 0,
                tlen: 0,
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
                p_addr: BX_ICACHE_INVALID_PHY_ADDRESS,
                trace_mask: 0,
                tlen: 0,
                mpool_start_idx: 0,
                first_bytes: [0; 8],
            }),
            mpool: core::array::from_fn(|_| Instruction::default()),
            mpindex: 0,
            next_page_split_index: 0,
            page_split_index: core::array::from_fn(|_| PageSplitEntry::default()),
        }
    }

    pub fn alloc_trace(&mut self, entry_idx: usize) {
        let entry = &mut self.entry[entry_idx];
        if entry.p_addr != BX_ICACHE_INVALID_PHY_ADDRESS {
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
        self.next_page_split_index =
            (self.next_page_split_index + 1) % BX_ICACHE_PAGE_SPLIT_ENTRIES;
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
        // Bochs icache.h — (pAddr & (BxICacheEntries-1)) ^ fetchModeMask
        let hash = (p_addr as u32) ^ (fetch_mode_mask as u32);
        hash & ((BX_ICACHE_ENTRIES - 1) as u32)
    }

    pub(super) fn find_entry(
        &self,
        p_addr: BxPhyAddress,
        fetch_mode_mask: u64,
    ) -> Option<BxICacheEntry> {
        let e = self.get_entry(p_addr, fetch_mode_mask);
        if e.p_addr != p_addr {
            return None;
        }
        Some(e)
    }

    pub fn flush_page(&mut self, ppf: BxPhyAddress) {
        let index = Self::hash(ppf, 0) as usize;
        let entry = &mut self.entry[index];

        if entry.p_addr != BX_ICACHE_INVALID_PHY_ADDRESS && ppf_of(entry.p_addr) == ppf {
            flush_smc(entry);
        }

        // Flush page split entries
        for i in 0..BX_ICACHE_PAGE_SPLIT_ENTRIES {
            if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS
                && ppf_of(self.page_split_index[i].ppf) == ppf
            {
                flush_smc(&mut self.page_split_index[i].e);
            }
        }
    }

    /// Bochs icache.h — breakLinks()
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
        // NOTE: the machine-wide SMC write-stamp table (Bochs
        // pageWriteStampTable, owned by BxMemoryStubC) is deliberately NOT
        // cleared here — other CPUs' traces are still marked in it, and
        // over-marked stamps only cause conservative extra flushes.
    }

    pub fn invalidate_page(&mut self, ppf: BxPhyAddress) {
        let index = Self::hash(ppf, 0) as usize;
        let entry = &mut self.entry[index];

        if entry.p_addr != BX_ICACHE_INVALID_PHY_ADDRESS && ppf_of(entry.p_addr) == ppf {
            entry.p_addr = BX_ICACHE_INVALID_PHY_ADDRESS;
        }

        // Invalidate page split entries
        for i in 0..BX_ICACHE_PAGE_SPLIT_ENTRIES {
            if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS
                && ppf_of(self.page_split_index[i].ppf) == ppf
            {
                self.page_split_index[i].ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                flush_smc(&mut self.page_split_index[i].e);
            }
        }
    }

    pub fn invalidate_all(&mut self) {
        for entry in &mut self.entry {
            entry.p_addr = BX_ICACHE_INVALID_PHY_ADDRESS;
        }
        for entry in &mut self.page_split_index {
            if entry.ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                entry.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                flush_smc(&mut entry.e);
            }
        }
    }

    /// Bochs icache.h `bxICache_c::handleSMC` — the per-CPU flush body of the
    /// all-processors loop in icache.cc `handleSMC`.
    ///
    /// Every trace STARTING in the written page hashes into the contiguous
    /// 4096-entry window at `hash(LPF(pAddr), 0)`: the entry hash is
    /// `(pAddr ^ fetchModeMask) & (entries - 1)` and FetchModeMask only uses
    /// bits 0-7, so the XOR never moves an entry out of its page-aligned
    /// 0x1000 window. Bochs scans 128 entries per 128-byte line and stops
    /// after the highest written line (`line_mask > mask`) — NOT the whole
    /// entry table; this bound is what keeps SMC-heavy phases (boot-time
    /// code patching, code near data) fast.
    ///
    /// Page identity is compared by the SHARED stamp-table index (Bochs:
    /// "pageWriteStampTable wrap — multiple physical addresses could be
    /// mapped into a single entry and all of them have to be invalidated
    /// here now").
    pub(crate) fn handle_smc_scan(&mut self, p_addr: BxPhyAddress, mask: u32) {
        let target_page_index = smc_page_index(p_addr);

        // Bochs handleSMC: breakLinks() first — invalidates all page-split
        // traces (a page-split trace may spill into the written page). This
        // also covers Bochs's separate `mask & 0x1` page-split pass, which
        // re-checks entries breakLinks already invalidated.
        self.break_links();

        // Bochs: bxICacheEntry_c *e = get_entry(LPFOf(pAddr), 0);
        // LPF has its low 12 bits clear, so `start` is a multiple of 0x1000
        // and `start + 4096 <= BX_ICACHE_ENTRIES` — the window never wraps.
        let start = Self::hash(lpf_of(p_addr), 0) as usize;
        let mut idx = start;
        // "go over 32 'cache lines' of 128 byte each"
        for n in 0..32u32 {
            let line_mask = 1u32 << n;
            if line_mask > mask {
                break;
            }
            for _ in 0..128 {
                let entry = &mut self.entry[idx];
                if entry.p_addr != BX_ICACHE_INVALID_PHY_ADDRESS
                    && smc_page_index(entry.p_addr) == target_page_index
                    && (entry.trace_mask & mask) != 0
                {
                    flush_smc(entry);
                }
                idx += 1;
            }
        }
    }
}

fn flush_smc(e: &mut BxICacheEntry) {
    // Bochs icache.cc flushSMC invalidates the entry (pAddr = INVALID) and, under
    // BX_SUPPORT_HANDLERS_CHAINING_SPEEDUPS, also writes an end-of-trace marker into
    // the trace's first mpool slot via genDummyICacheEntry(e->i). We invalidate via
    // the sentinel but deliberately do NOT mirror the mpool write — for a hard Rust
    // aliasing-safety reason (safety trumps Bochs literalness), not because trace
    // linking happens to be unimplemented:
    //
    //   flush_smc runs DURING instruction execution — a self-modifying store goes
    //   access.rs write → smc_write_check → handle_smc_scan → flush_smc. Meanwhile
    //   cpu_loop_n_impl holds a raw `*const Instruction` into mpool across
    //   execute_instruction under the SAFETY invariant "mpool is not written during
    //   execution". Taking `&mut mpool` here to write the marker would invalidate that
    //   raw pointer (Stacked Borrows) → UB, observably so in instrumented builds which
    //   deref it in fire_after_execution. (This is exactly why the pre-shrink code
    //   wrote the dummy into a dead `entry.i` COPY, never into mpool.)
    //
    // The mpool marker is also unnecessary for correctness here: the pAddr sentinel
    // fully invalidates the entry, and every trace is re-entered only through
    // get_icache_entry, which rejects the sentinel and re-decodes. The marker only
    // guards linked traces that jump into an invalidated trace's mpool region without
    // a lookup — a path this design never creates.
    if e.p_addr != BX_ICACHE_INVALID_PHY_ADDRESS {
        e.p_addr = BX_ICACHE_INVALID_PHY_ADDRESS;
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

impl<'c, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'c, I, T> {
    fn bx_end_trace(&mut self) {
        self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
    }

    pub(super) fn serve_icache_miss(
        &mut self,
        eip_biased: u32,
        p_addr: BxPhyAddress,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
    ) -> Result<BxICacheEntry> {
        // Raw pointer for stamp-table marking after `mem` is moved into
        // boundary_fetch below (same reborrow discipline as cpu_loop's
        // mem_ptr; the borrows never overlap).
        let mem_raw: *mut BxMemC<'c> = mem;
        // Get entry index first to avoid borrow conflicts
        let entry_idx = BxICache::hash(p_addr, self.fetch_mode_mask.bits().into()) as usize;

        // Matching C++ icache.cc - use eip_biased directly
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

        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        let is_32_bit_mode = self.sregs[crate::cpu::decoder::BxSegregs::Cs as usize]
            .cache
            .u
            .segment_d_b();
        // Bochs icache.cc getICacheEntry: "Don't allow traces longer than
        // cpu_loop can execute" — with multiple processors the trace is
        // capped by the configured SMP quantum (BXPN_SMP_QUANTUM, default 16,
        // range 1-32), otherwise by BX_MAX_TRACE_LENGTH.
        let quantum = if self.cpu_topology.cpu_count() > 1 {
            (self.smp_trace_quantum.max(1)) as usize
        } else {
            BX_MAX_TRACE_LENGTH
        };

        // Matching Bochs: when mpool is nearly full, flush all icache entries and
        // reset mpindex to 0. Without this, once mpindex reaches BX_ICACHE_MEM_POOL,
        // all new traces get tlen=0 and point to stale decoded instructions, causing
        // the CPU to execute wrong opcodes (e.g., RET decoded as POP).
        if self.i_cache.mpindex + BX_MAX_TRACE_LENGTH >= BX_ICACHE_MEM_POOL {
            tracing::trace!(
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
            entry.p_addr = p_addr;
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
                    tracing::trace!(
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
                    let handler =
                        self.handler_for_instruction(&self.i_cache.mpool[current_mpindex]);
                    self.i_cache_handlers[current_mpindex] = handler;
                    // Instruction is already in mpool[current_mpindex] — get its length
                    let i_len = { self.i_cache.mpool[current_mpindex].ilen() as u32 };

                    // Bochs stores the selected execute handler in each decoded
                    // instruction. Rust keeps the generic function pointer in
                    // the parallel i_cache_handlers pool populated above.
                    // Check if this instruction ends the trace (matching Bochs assignHandler
                    // BxTraceEnd check). Control-flow instructions (branches, jumps, calls,
                    // returns, loops, interrupts) must end the trace so that the next
                    // get_icache_entry call looks up the branch TARGET address, not the
                    // next sequential address.
                    let stop_trace_indication =
                        is_trace_end_opcode(self.i_cache.mpool[current_mpindex].get_ia_opcode());

                    // BX_INSTR_OPCODE (matching C++ icache.cc:178-179)
                    #[cfg(feature = "instrumentation")]
                    if self.instrumentation.active.has_exec() {
                        let rip =
                            self.prev_rip + (current_page_offset as u64 - (page_offset as u64));
                        let bytes = &current_fetch_ptr[..i_len as usize];
                        let size = if long64 {
                            super::instrumentation::CodeSize::Bits64
                        } else if is_32_bit_mode {
                            super::instrumentation::CodeSize::Bits32
                        } else {
                            super::instrumentation::CodeSize::Bits16
                        };
                        let ev = super::instrumentation::OpcodeEvent {
                            rip,
                            instr: &self.i_cache.mpool[current_mpindex],
                            bytes,
                            size,
                        };
                        self.instrumentation.fire_opcode(&ev);
                    }

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
                        if let Some(merged) = self.merge_traces_internal(
                            entry_idx,
                            current_mpindex,
                            current_p_addr,
                            tlen,
                        ) {
                            // merge_traces_internal set entry.tlen = tlen + merged and folded in
                            // the source trace's mask. Add the decoded portion's mask, stamp the
                            // page-write table with the full mask, advance mpindex PAST the spliced
                            // instructions (else the next trace overwrites them), and commit.
                            // (Bochs serveICacheMiss, icache.cc:197-200.)
                            let full_mask = {
                                let entry = &mut self.i_cache.entry[entry_idx];
                                entry.trace_mask |= trace_mask;
                                entry.trace_mask
                            };
                            // SAFETY: no live borrow of mem at this point (see mem_raw above).
                            unsafe { (*mem_raw).smc_mark_icache_mask(current_p_addr, full_mask) };
                            self.i_cache.mpindex = current_mpindex + merged;
                            self.i_cache
                                .commit_trace(self.i_cache.entry[entry_idx].tlen as usize);
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
                    tracing::trace!(
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
                        // Based on Bochs exception.cc and cpu.h (Exception::Ud = 6)
                        use crate::cpu::decoder::DecodeError;
                        use rusty_box_decoder::decoder::tables::BxDecodeError;
                        match &decode_err {
                            DecodeError::Decoder(BxDecodeError::BxIllegalOpcode) => {
                                // Bochs: store IaError in trace, don't raise #UD here.
                                // The IaError instruction will be executed normally in the
                                // inner trace loop, where prev_rip is correctly set.
                                // This matches Bochs fetchdecode behavior.
                                tracing::trace!(
                                    "Illegal opcode at RIP={:#x}, storing IaError in trace",
                                    self.rip()
                                );
                                self.i_cache.mpool[current_mpindex].set_ia_opcode(Opcode::IaError);
                                self.i_cache.mpool[current_mpindex].set_ilen(1);
                                let handler = self
                                    .handler_for_instruction(&self.i_cache.mpool[current_mpindex]);
                                self.i_cache_handlers[current_mpindex] = handler;
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
                        entry.p_addr = BX_ICACHE_INVALID_PHY_ADDRESS; // Mark as invalid temporarily (~entry->pAddr in C++)
                        entry.tlen = 1;
                        entry.mpool_start_idx = current_mpindex; // Store where this trace starts
                    }

                    // Debug logging before boundary_fetch
                    tracing::trace!(
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
                        tracing::trace!("mpool full before boundary_instr, stopping trace");
                        break;
                    }
                    self.i_cache.mpool[current_mpindex] = boundary_instr;
                    let handler =
                        self.handler_for_instruction(&self.i_cache.mpool[current_mpindex]);
                    self.i_cache_handlers[current_mpindex] = handler;
                    current_mpindex += 1;

                    // Add the instruction to trace cache (matching C++ line 152-154)
                    {
                        let entry = &mut self.i_cache.entry[entry_idx];
                        entry.p_addr = p_addr; // Restore pAddr (~entry->pAddr in C++)
                        entry.trace_mask = 0x80000000; /* last line in page */
                        // tlen is already set to 1 above; mpool_start_idx was already set above.
                    }

                    // SAFETY: boundary_fetch's borrow of mem ended above (see mem_raw).
                    unsafe {
                        (*mem_raw).smc_mark_icache_mask(p_addr, 0x80000000);
                        (*mem_raw).smc_mark_icache_mask(self.p_addr_fetch_page, 0x1);
                    }

                    // Add end-of-trace opcode if not in debugger (matching C++ line 158-163)
                    // TODO: Check debugger active state
                    {
                        if current_mpindex < BX_ICACHE_MEM_POOL {
                            let entry = &mut self.i_cache.entry[entry_idx];
                            entry.tlen += 1; /* Add the inserted end of trace opcode */
                            gen_dummy_icache_entry(&mut self.i_cache.mpool[current_mpindex]);
                            let handler =
                                self.handler_for_instruction(&self.i_cache.mpool[current_mpindex]);
                            self.i_cache_handlers[current_mpindex] = handler;
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
        // SAFETY: no live borrow of mem at this point (see mem_raw above).
        unsafe { (*mem_raw).smc_mark_icache_mask(current_p_addr, trace_mask) };

        // Add end-of-trace opcode if not in debugger (matching C++ line 210-214)
        // TODO: Check debugger active state
        {
            // Check bounds before accessing mpool
            if current_mpindex < BX_ICACHE_MEM_POOL {
                // Note: tlen will be incremented here, then used below
                gen_dummy_icache_entry(&mut self.i_cache.mpool[current_mpindex]);
                let handler = self.handler_for_instruction(&self.i_cache.mpool[current_mpindex]);
                self.i_cache_handlers[current_mpindex] = handler;
                current_mpindex += 1;
                tlen += 1; /* Add the inserted end of trace opcode */
            }
        }

        // Update entry tlen (matching C++ line 217). Bochs entry->i points at the
        // first mpool instruction; our mpool_start_idx (set earlier) plays that role.
        {
            let entry = &mut self.i_cache.entry[entry_idx];
            entry.tlen = tlen as u32;
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

        tracing::trace!(
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

        // BX_INSTR_OPCODE (matching C++ icache.cc:318-319)
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_exec() {
            let rip = self.prev_rip;
            let bytes = &fetch_buffer[..instr.ilen() as usize];
            let size = if self.long64_mode() {
                super::instrumentation::CodeSize::Bits64
            } else if is_32_bit_mode {
                super::instrumentation::CodeSize::Bits32
            } else {
                super::instrumentation::CodeSize::Bits16
            };
            let ev = super::instrumentation::OpcodeEvent {
                rip,
                instr: &instr,
                bytes,
                size,
            };
            self.instrumentation.fire_opcode(&ev);
        }

        Ok(instr)
    }

    fn merge_traces_internal(
        &mut self,
        current_entry_idx: usize,
        current_mpindex: usize,
        p_addr: BxPhyAddress,
        current_tlen: usize,
    ) -> Option<usize> {
        // Bochs find_entry(pAddr): invalid entries carry the all-ones sentinel, so a real
        // p_addr never matches one — a single compare covers both "exists" and "pAddr matches".
        let cache_entry_idx = BxICache::hash(p_addr, self.fetch_mode_mask.bits().into()) as usize;
        let e = &self.i_cache.entry[cache_entry_idx];
        if e.p_addr != p_addr {
            return None;
        }

        // Bochs: max_length = e->tlen. With BX_SUPPORT_HANDLERS_CHAINING_SPEEDUPS, only merge if
        // the whole cached trace still fits the max-trace budget (else leave it un-merged).
        let max_length = e.tlen as usize;
        if max_length + current_tlen > BX_MAX_TRACE_LENGTH {
            return None;
        }
        let source_start_idx = e.mpool_start_idx; // Bochs e->i (pointer into mpool)
        let source_trace_mask = e.trace_mask;

        // memcpy(i, e->i, max_length): splice the cached trace's instructions onto the one being
        // built. In-bounds by the budget check above plus serve_icache_miss's mpindex reservation;
        // the source (a trace committed earlier, lower in mpool) and the target (the trace under
        // construction) occupy distinct regions and never overlap.
        debug_assert!(current_mpindex + max_length <= BX_ICACHE_MEM_POOL);
        debug_assert!(source_start_idx + max_length <= BX_ICACHE_MEM_POOL);
        for k in 0..max_length {
            self.i_cache.mpool[current_mpindex + k] = self.i_cache.mpool[source_start_idx + k];
            self.i_cache_handlers[current_mpindex + k] =
                self.i_cache_handlers[source_start_idx + k];
        }

        // Bochs mergeTraces: entry->tlen += max_length; entry->traceMask |= e->traceMask. Our
        // running length is the local current_tlen (the entry's tlen field is not bumped per
        // instruction), so set the total here.
        let entry = &mut self.i_cache.entry[current_entry_idx];
        entry.tlen = (current_tlen + max_length) as u32;
        debug_assert!(entry.tlen as usize <= BX_MAX_TRACE_LENGTH); // Bochs BX_ASSERT (icache.cc:244)
        entry.trace_mask |= source_trace_mask;
        Some(max_length)
    }
}

#[cfg(test)]
mod smc_mask_tests {
    use super::smc_cache_line_mask;

    #[test]
    fn smc_mask_covers_every_touched_cache_line() {
        assert_eq!(smc_cache_line_mask(0x0180, 0), 0);
        assert_eq!(smc_cache_line_mask(0x0180, 1), 1 << 3);
        assert_eq!(smc_cache_line_mask(0x0180, 0x0180), 0b111 << 3);
        assert_eq!(smc_cache_line_mask(0x0000, 0x1000), u32::MAX);
        assert_eq!(smc_cache_line_mask(0x0f80, 0x0080), 1 << 31);
    }
}
