use super::BxCpuC;
use crate::memory::Block;

impl BxCpuC {
    #[cfg(feature = "bx_large_ram_file")]
    pub fn check_addr_in_tlb_buffers(&self, addr: &Block, end: usize) -> bool {
        unimplemented!()
    }
}
