use std::os::raw::c_uint;

use crate::config::BxPhyAddress;

pub struct BxPageWriteStampTable<'a> {
    fine_granularity_mapping: &'a [u32],
}

impl BxPageWriteStampTable<'_> {
    const PHY_MEM_PAGES_IN_4G_SPACE: u32 = 1024 * 1024;

    const BX_ICACHE_ENTRIES: usize = (64 * 1024); // Must be a power of 2.
    const BX_ICACHE_MEM_POOL: usize = (576 * 1024);

    pub fn dec_write_stamp(&mut self, p_addr: BxPhyAddress) {
        unimplemented!()
    }

    pub fn dec_write_stamp_with_len(&mut self, p_addr: BxPhyAddress, len: c_uint) {
        unimplemented!()
    }
}
