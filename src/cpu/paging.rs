use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, Result};
#[cfg(feature = "bx_large_ram_file")]
use crate::memory::Block;
use crate::{
    config::{BxAddress, BxPhyAddress},
    cpu::{
        rusty_box::MemoryAccessType,
        tlb::{BxHostpageaddr, TLBEntry},
    },
};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    #[cfg(feature = "bx_large_ram_file")]
    pub(crate) fn check_addr_in_tlb_buffers(&self, _addr: &Block, _end: usize) -> bool {
        unimplemented!()
    }
}

pub(super) fn translate_linear(
    tlb_entry: &TLBEntry,
    laddr: BxAddress,
    user: bool,
    rw: MemoryAccessType,
) -> BxPhyAddress {
    todo!()
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    fn is_virtual_apic_page(&self, p_addr: &BxPhyAddress) -> bool {
        if self.in_vmx_guest {
            let vm = &self.vmcs;
        }

        false
    }
    fn getHostMemAddr(p_addr: BxPhyAddress, rw: MemoryAccessType) -> Option<BxHostpageaddr> {
        //#[cfg(feature = "bx_support_vmx")]
        //if self.i
        //
        todo!()
    }
}
