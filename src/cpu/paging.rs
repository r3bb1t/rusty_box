use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait};
use crate::memory::Block;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    #[cfg(feature = "bx_large_ram_file")]
    pub(crate) fn check_addr_in_tlb_buffers(&self, addr: &Block, end: usize) -> bool {
        unimplemented!()
    }
}
