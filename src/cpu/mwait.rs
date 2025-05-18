use super::{cpu::BxCpuC, Result};
use crate::config::BxPhyAddress;

use core::ffi::c_uint;

impl<'c> BxCpuC<'_> {
    pub fn is_monitor(&self, begin_addr: BxPhyAddress, len: c_uint) -> bool {
        unimplemented!()
    }

    pub fn check_monitor(&mut self, begin_addr: BxPhyAddress, len: c_uint) -> Result<()> {
        unimplemented!()
    }
}
