use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, Result};
#[cfg(feature = "bx_large_ram_file")]
use crate::memory::Block;
use crate::{
    config::{BxAddress, BxPhyAddress},
    cpu::{
        rusty_box::MemoryAccessType,
        tlb::{BxHostpageaddr, TLBEntry},
    },
    memory::BxMemC,
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
    0 // FIXME: remove that
    // todo!()
}

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    fn is_virtual_apic_page(&self, p_addr: &BxPhyAddress) -> bool {
        if self.in_vmx_guest {
            let vm = &self.vmcs;
            // FIXME: add more here
        }

        false
    }
    pub(crate) fn get_host_mem_addr(
        &self,
        p_addr: BxPhyAddress,
        rw: MemoryAccessType,
        mem: &'c mut BxMemC<'c>,
        // cpus: &[&Self],
    ) -> crate::Result<Option<&'c mut [u8]>> {
        //#[cfg(feature = "bx_support_vmx")]
        //if self.i
        if self.is_virtual_apic_page(&p_addr) {
            return Ok(None); // Do not allow direct access to virtual apic page
        }

        // let addr_option = mem.get_host_mem_addr(Some(self), p_addr, rw, cpus)?;
        let addr_option = mem.get_host_mem_addr(p_addr, rw, &[&self])?;
        // let addr_option = mem.get_host_mem_addr(Some(self), p_addr, rw, &[&self])?;
        Ok(addr_option)
    }
}
