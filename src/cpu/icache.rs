use crate::{
    config::BxPhyAddress,
    cpu::decoder::{
        fetchdecode32::{self, fetch_decode32_chatgpt_generated_instr},
        instr_generated::BxInstructionGenerated,
    },
};

use super::decoder::instr::BxInstruction;

const BX_ICACHE_INVALID_PHY_ADDRESS: BxPhyAddress = -1 as _;

const BX_ICACHE_PAGE_SPLIT_ENTRIES: usize = 8;

#[derive(Debug)]
pub struct BxPageWriteStampTable<'a> {
    pub fine_granularity_mapping: &'a [u32],
}

impl BxPageWriteStampTable<'_> {
    const PHY_MEM_PAGES_IN_4G_SPACE: u32 = 1024 * 1024;

    const BX_ICACHE_ENTRIES: usize = (64 * 1024); // Must be a power of 2.
    const BX_ICACHE_MEM_POOL: usize = (576 * 1024);

    pub fn dec_write_stamp(&mut self, _p_addr: BxPhyAddress) {
        unimplemented!()
    }

    pub fn dec_write_stamp_with_len(&mut self, _p_addr: BxPhyAddress, _len: u32) {
        unimplemented!()
    }
}

const BX_ICACHE_ENTRIES: usize = 64 * 1024; // Must be a power of 2.
const BX_ICACHE_MEM_POOL: usize = 576 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct BxIcacheEntry {
    pub(super) p_addr: BxPhyAddress, // Physical address of the instruction
    pub(super) trace_mask: u32,

    pub(super) tlen: u32, // Trace length in instructions
    pub(super) i: BxInstructionGenerated,
}

#[derive(Debug, Default)]
struct PageSplitEntry {
    ppf: BxPhyAddress,
    entry_idx: usize,
}

#[derive(Debug)]
pub struct BxIcache {
    pub entry: [BxIcacheEntry; BX_ICACHE_ENTRIES],
    pub mpool: [BxInstructionGenerated; BX_ICACHE_MEM_POOL],

    pub page_split_index: [PageSplitEntry; BX_ICACHE_PAGE_SPLIT_ENTRIES],
    pub mpindex: u32,

    pub trace_link_time_stamp: u32,

    pub next_page_split_index: u32,
}

impl BxIcache {
    #[inline]
    pub fn flush_icache_entries(&mut self) {
        // Invalidate all I-cache entries
        for entry in &mut self.entry {
            entry.p_addr = BX_ICACHE_INVALID_PHY_ADDRESS;
            entry.trace_mask = 0;
            entry.tlen = 0;
            entry.i = fetch_decode32_chatgpt_generated_instr(&[0x90], true).unwrap();
        }

        // Reset page‐split tracking
        self.next_page_split_index = 0;
        for psi in &mut self.page_split_index {
            psi.ppf = BX_ICACHE_INVALID_PHY_ADDRESS;
        }

        // Reset memory‐pool index and trace‐link timestamp
        self.mpindex = 0;
        self.trace_link_time_stamp = 0;
    }
}

impl Default for BxIcacheEntry {
    fn default() -> Self {
        Self {
            p_addr: Default::default(),
            trace_mask: Default::default(),
            tlen: Default::default(),
            i: Default::default(),
        }
    }
}

impl Default for BxIcache {
    fn default() -> Self {
        Self {
            entry: core::array::from_fn(|_| Default::default()),
            mpool: core::array::from_fn(|_| Default::default()),
            page_split_index: Default::default(),
            mpindex: Default::default(),
            trace_link_time_stamp: Default::default(),
            next_page_split_index: Default::default(),
        }
    }
}
