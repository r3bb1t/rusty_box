use super::{BxCpuC, Result};
use crate::config::BxPhyAddress;

use std::os::raw::c_uint;

impl BxCpuC {
    pub fn is_monitor(&self, begin_addr: BxPhyAddress, len: c_uint) -> bool {
        unimplemented!()
    }

    pub fn check_monitor(&mut self, begin_addr: BxPhyAddress, len: c_uint) -> Result<()> {
        unimplemented!()
    }
}
