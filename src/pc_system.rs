use std::sync::OnceLock;

use crate::config::BxPhyAddress;

static BX_PC_SYSTEM_LOCK: OnceLock<BxPcSystemC> = OnceLock::new();

pub fn bx_pc_system() -> &'static BxPcSystemC {
    BX_PC_SYSTEM_LOCK.get_or_init(BxPcSystemC::new)
}

pub struct BxPcSystemC {
    a20_mask: BxPhyAddress,
}

impl BxPcSystemC {
    fn new() -> Self {
        todo!()
    }
}

pub fn a20_addr(x: BxPhyAddress) -> BxPhyAddress {
    x & bx_pc_system().a20_mask
}
