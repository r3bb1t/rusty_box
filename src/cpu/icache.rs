use crate::config::BxPhyAddress;

use super::decoder::instr::BxInstruction;

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

#[derive(Debug)]
pub struct BxIcacheEntry {
    p_addr: BxPhyAddress, // Physical address of the instruction
    trace_mask: u32,

    tlen: u32, // Trace length in instructions
    i: BxInstruction,
}

#[derive(Debug)]
pub struct BxIcache {
    pub entry: [BxIcacheEntry; BX_ICACHE_ENTRIES],
    pub mpool: [BxInstruction; BX_ICACHE_MEM_POOL],
    pub mpindex: u32,

    pub trace_link_time_stamp: u32,
}
