use super::{cpu::BxCpuC, cpuid::BxCpuTrait};
use crate::memory::Block;

impl<I: BxCpuTrait> BxCpuC<'_, I> {
    #[cfg(feature = "bx_large_ram_file")]
    pub(crate) fn check_addr_in_tlb_buffers(&self, addr: &Block, end: usize) -> bool {
        unimplemented!()
    }
}
