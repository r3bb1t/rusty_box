use super::BxCpuC;
use crate::memory::memory_stub::Block;

impl BxCpuC {
    #[cfg(feature = "bx_large_ram_file")]
    pub fn check_addr_in_tlb_buffers(&self, addr: &Block, end: &*const u8) -> bool {
        unimplemented!()
    }
}
