use core::ops::Deref;

use crate::{
    config::BxPhyAddress,
    cpu::{
        cpu::BX_ASYNC_EVENT_STOP_TRACE,
        decoder::{
            fetch_decode32_chatgpt_generated_instr, Opcode, BxInstructionGenerated,
        },
        tlb::{lpf_of, page_offset},
        BxCpuC, BxCpuIdTrait,
    },
};

struct BxPageWriteStampTable {
    fine_granularity_mapping: [u32; Self::PHY_MEM_PAGES_IN_4G_SPACE],
}

impl BxPageWriteStampTable {
    const PHY_MEM_PAGES_IN_4G_SPACE: usize = 1024 * 1024;

    #[inline]
    fn hash(p_addr: BxPhyAddress) -> u32 {
        // can share writeStamps between multiple pages if >32 bit phy address
        p_addr as u32 >> 12
    }

    #[inline]
    fn get_fine_granularity_mapping(&self, p_addr: BxPhyAddress) -> u32 {
        self.fine_granularity_mapping[Self::hash(p_addr) as usize]
    }

    #[inline]
    fn mark_icache(&mut self, p_addr: BxPhyAddress, len: u32) {
        let mut mask: u32 = 1 << (page_offset(p_addr as u32) >> 7);
        mask |= 1 << page_offset(p_addr as u32 + len - 1) >> 7;

        self.fine_granularity_mapping[Self::hash(p_addr) as usize] |= mask;
    }

    #[inline]
    fn mark_icache_mask(&mut self, p_addr: BxPhyAddress, mask: u32) {
        self.fine_granularity_mapping[Self::hash(p_addr) as usize] |= mask;
    }

    #[inline]
    fn dec_write_stamp<I: BxCpuIdTrait>(&mut self, cpus: &mut [BxCpuC<I>], p_addr: BxPhyAddress) {
        let index = Self::hash(p_addr) as usize;

        if self.fine_granularity_mapping[index] != 0 {
            handle_smc(cpus, p_addr, 0xffffffff);
            self.fine_granularity_mapping[index] = 0;
        }
    }

    // assumption: write does not split 4K page
    #[inline]
    pub fn dec_write_stamp_with_len<I: BxCpuIdTrait>(
        &mut self,
        cpus: &mut [BxCpuC<I>],
        p_addr: BxPhyAddress,
        len: u32,
    ) {
        let index = Self::hash(p_addr) as usize;

        if self.fine_granularity_mapping[index] != 0 {
            let mut mask: u32 = 1 << (page_offset(p_addr as u32) >> 7);
            mask |= 1 << page_offset(p_addr as u32 + len - 1) >> 7;

            if self.fine_granularity_mapping[index] & mask != 0 {
                handle_smc(cpus, p_addr, mask);
                self.fine_granularity_mapping[index] &= !mask;
            }
        }
    }

    #[inline]
    fn reset_write_stamps(&mut self) {
        for i in 0..Self::PHY_MEM_PAGES_IN_4G_SPACE {
            self.fine_granularity_mapping[i] = 0
        }
    }
}

fn handle_smc<I: BxCpuIdTrait>(cpus: &mut [BxCpuC<I>], p_addr: BxPhyAddress, mask: u32) {
    // INC_SMC_STAT(smc);

    for cpu in cpus {
        cpu.async_event |= BxCpuC::<I>::BX_ASYNC_EVENT_STOP_TRACE;
        cpu.i_cache.handle_smc(p_addr, mask);
    }
}

const BX_ICACHE_ENTRIES: usize = 64 * 1024; // Must be a power of 2.
const BX_ICACHE_MEM_POOL: usize = 576 * 1024;
const BX_MAX_TRACE_LENGTH: usize = 32;
const BX_ICACHE_INVALID_PHY_ADDRESS: BxPhyAddress = BxPhyAddress::MAX;

#[derive(Debug, PartialEq, Clone, Default, Copy)]
enum IcacheAddress {
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

#[derive(Clone, Default, Copy)]
pub(super) struct BxICacheEntry {
    // p_addr: BxPhyAddress, // Physical address of the instruction
    p_addr: IcacheAddress, // Physical address of the instruction
    trace_mask: u32,
    // orignial replaced
    //tlen: u32, // Trace length in instructions
    tlen: usize, // Trace length in instructions
    i: BxInstructionGenerated,
}

const BX_ICACHE_PAGE_SPLIT_ENTRIES: usize = 8;

#[derive(Clone, Default, Copy)]
struct PageSplitEntryIndex {
    // Physical address of 2nd page of the trace
    ppf: BxPhyAddress,
    e: BxICacheEntry,
}

#[derive(Clone)]
pub(crate) struct BxICache {
    pub(crate) entry: [BxICacheEntry; BX_ICACHE_ENTRIES],
    pub(crate) mpool: [BxInstructionGenerated; BX_ICACHE_MEM_POOL],
    pub(crate) mpindex: usize,
    pub(crate) trace_link_time_stamp: u32,
    pub(crate) page_split_index: [PageSplitEntryIndex; BX_ICACHE_PAGE_SPLIT_ENTRIES],
    pub(crate) next_page_split_index: usize,
}

impl Default for BxICache {
    fn default() -> Self {
        Self {
            entry: [BxICacheEntry::default(); _],
            mpool: [BxInstructionGenerated::default(); _],
            mpindex: 0,
            trace_link_time_stamp: 0,
            page_split_index: [PageSplitEntryIndex::default(); _],
            next_page_split_index: 0,
        }
    }
}

impl BxICache {
    //fn new() -> Self {
    //    Self {
    //        entry: [BxICacheEntry {
    //            p_addr: BX_ICACHE_INVALID_PHY_ADDRESS,
    //            trace_mask: 0,
    //            tlen: 0,
    //            i: None,
    //        }; BX_ICACHE_ENTRIES],
    //        mpool: vec![0; BX_ICACHE_MEM_POOL],
    //        mpindex: 0,
    //        trace_link_time_stamp: 0,
    //        page_split_index: vec![None; BX_ICACHE_PAGE_SPLIT_ENTRIES],
    //        next_page_split_index: 0,
    //    }
    //}

    #[inline]
    const fn hash(p_addr: BxPhyAddress, fetch_mode_mask: u64) -> u64 {
        ((p_addr) & (BX_ICACHE_ENTRIES - 1) as BxPhyAddress) ^ fetch_mode_mask
    }

    #[inline]
    fn alloc_trace(&mut self, e: &mut BxICacheEntry) {
        // took +1 garbend for instruction chaining speedup (end-of-trace opcode)
        if (self.mpindex + BX_MAX_TRACE_LENGTH + 1) > BX_ICACHE_MEM_POOL {
            self.flush_icache_entries();
        }
        e.i = self.mpool[self.mpindex]; // TODO: Check if its okay
        e.tlen = 0;
    }

    #[inline]
    fn commit_trace(&mut self, len: usize) {
        self.mpindex += len
    }

    #[inline]
    pub fn commit_page_split_trace(&mut self, p_addr: BxPhyAddress, e: BxICacheEntry) {
        self.mpindex += e.tlen;

        // register page split entry
        if self.page_split_index[self.next_page_split_index].ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
            self.page_split_index[self.next_page_split_index].e.p_addr = IcacheAddress::Invalid;
        }

        self.page_split_index[self.next_page_split_index].ppf = p_addr;
        self.page_split_index[self.next_page_split_index].e = e;

        self.next_page_split_index =
            (self.next_page_split_index + 1) & (BX_ICACHE_PAGE_SPLIT_ENTRIES - 1);
    }

    #[inline]
    pub fn get_entry(&self, p_addr: BxPhyAddress, fetch_mode_mask: u64) -> BxICacheEntry {
        let index = Self::hash(p_addr, fetch_mode_mask);
        self.entry[index as usize].clone()
    }

    #[inline]
    pub(super) fn find_entry(
        &mut self,
        p_addr: BxPhyAddress,
        fetch_mode_mask: u64,
    ) -> Option<BxICacheEntry> {
        let e = self.get_entry(p_addr, fetch_mode_mask);
        if BxPhyAddress::from(e.p_addr.clone()) != p_addr {
            return None;
        }
        Some(e)
    }

    #[inline]
    fn flush_icache_entries(&mut self) {
        self.entry.iter_mut().for_each(|entry| {
            entry.p_addr = IcacheAddress::Invalid;
            entry.trace_mask = 0;
        });

        self.next_page_split_index = 0;

        for page_slit in &mut self.page_split_index {
            page_slit.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
        }
        self.mpindex = 0;
        self.trace_link_time_stamp = 0;
    }

    #[inline]
    fn invalidate_page_split_icache_entries(&mut self) {
        for entry in &mut self.page_split_index {
            // TODO: Use algebraic types for clarity?
            if entry.ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                entry.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                flush_smc(&mut entry.e);
            }
        }
        // for i in 0..BX_ICACHE_PAGE_SPLIT_ENTRIES {
        //     if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
        //         self.page_split_index[i].ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
        //         flush_smc(&mut *self.page_split_index[i].e);
        //     }
        // }
        self.next_page_split_index = 0;
    }

    #[inline]
    fn handle_smc(&mut self, p_addr: BxPhyAddress, mask: u32) {
        let p_addr_index = BxPageWriteStampTable::hash(p_addr);

        // break all links between traces
        if self.break_links() {
            return;
        }

        // Need to invalidate all traces in the trace cache that might include an
        // instruction that was modified.  But this is not enough, it is possible
        // that some another trace is linked into  invalidated trace and it won't
        // be invalidated. In order to solve this issue  replace all instructions
        // from the invalidated trace with dummy EndOfTrace opcodes.

        // Another corner case that has to be handled - pageWriteStampTable wrap.
        // Multiple physical addresses could be mapped into single pageWriteStampTable
        // entry and all of them have to be invalidated here now.

        if mask & 0x1 != 0 {
            // the store touched 1st cache line in the page, check for
            // page split traces to invalidate.
            for i in 0..BX_ICACHE_PAGE_SPLIT_ENTRIES {
                if self.page_split_index[i].ppf != BX_ICACHE_INVALID_PHY_ADDRESS {
                    if p_addr_index == BxPageWriteStampTable::hash(self.page_split_index[i].ppf) {
                        self.page_split_index[i].ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
                        flush_smc(&mut self.page_split_index[i].e); // Assuming flush_smc is defined elsewhere
                    }
                }
            }
        }

        let mut e = self.get_entry(lpf_of(p_addr), 0);
        // go over 32 "cache lines" of 128 byte each
        for n in 0..32 {
            let line_mask = 1 << n;
            if line_mask > mask {
                break;
            }
            for _ in 0..128 {
                if p_addr_index == BxPageWriteStampTable::hash(BxPhyAddress::from(e.p_addr.clone()))
                    && (e.trace_mask & mask) != 0
                {
                    flush_smc(&mut e);
                }
                // TODO: make sense of this
                // // what is this
                // e = &mut self.entry
                //     [(e as *const _ as usize / core::mem::size_of::<BxICacheEntry>()) + 1];
            }
        }
    }

    #[inline]
    fn break_links(&mut self) -> bool {
        self.invalidate_page_split_icache_entries();

        // break all links between traces
        if self.trace_link_time_stamp == u32::MAX {
            self.flush_icache_entries();
            return true;
        }
        self.trace_link_time_stamp += 1;
        false
    }
}

fn flush_icaches<I: BxCpuIdTrait>(
    cpus: &mut [BxCpuC<I>],
    page_write_stamp_table: &mut BxPageWriteStampTable,
) {
    for cpu in cpus {
        cpu.i_cache.flush_icache_entries();
        cpu.async_event != BX_ASYNC_EVENT_STOP_TRACE;
    }

    page_write_stamp_table.reset_write_stamps();
}

fn flush_smc(e: &mut BxICacheEntry) {
    // if e.p_addr != BX_ICACHE_INVALID_PHY_ADDRESS {
    //     e.p_addr = BX_ICACHE_INVALID_PHY_ADDRESS;
    //     // NOTE: add code here in future
    // }

    if let IcacheAddress::Address(_) = e.p_addr {
        e.p_addr == IcacheAddress::Invalid;
        // NOTE: add code here in future
    }
}

fn gen_dummy_icache_entry(i: &mut BxInstructionGenerated) {
    i.set_ilen(0);
    i.set_ia_opcode(Opcode::InsertedOpcode);
    // FIXME: execute1 is set here
}

impl<'c, I: BxCpuIdTrait> BxCpuC<'_, I> {
    fn bx_end_trace(&mut self) {
        self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
    }

    fn serve_icache_miss(&mut self, eip_biased: u32, p_addr: BxPhyAddress) {
        let mut entry = self.i_cache.get_entry(p_addr, self.fetch_mode_mask.into());

        self.i_cache.alloc_trace(&mut entry);
        // Cache miss. We weren't so lucky, but let's be optimistic - try to build
        // trace from incoming instruction bytes stream !
        entry.p_addr = IcacheAddress::Address(p_addr);
        entry.trace_mask = 0;

        let remaining_in_page = self.eip_page_window_size - eip_biased;
        // let fetch_ptr = self.eip_fetch_ptr.add() + eip_biased;
        let fetch_ptr = &self.eip_fetch_ptr.unwrap()[eip_biased as usize..];
        let mut ret;

        let i = entry.i;

        let page_offset = page_offset(p_addr as u32);
        let trace_mask = 0;

        // #[cfg(not(feature = "bx_support_smp"))]
        let quantum = BX_MAX_TRACE_LENGTH;

        for n in 0..quantum {
            ret = fetch_decode32_chatgpt_generated_instr(&fetch_ptr, true)
        }
    }
}
