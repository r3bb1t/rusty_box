use super::{cpu::BxCpuC, cpuid::BxCpuTrait, Result};
use crate::config::BxPhyAddress;

impl<'c, I: BxCpuTrait> BxCpuC<'_, I> {
    pub fn is_monitor(&self, begin_addr: BxPhyAddress, len: u32) -> bool {
        unimplemented!()
    }

    pub fn check_monitor(&mut self, begin_addr: BxPhyAddress, len: u32) -> Result<()> {
        unimplemented!()
    }
}
