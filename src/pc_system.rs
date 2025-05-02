use crate::config::BxPhyAddress;

lazy_static::lazy_static!(
    pub static ref bx_pc_system: BxPcSystemC = BxPcSystemC::new();
);

pub struct BxPcSystemC {
    a20_mask: BxPhyAddress,
}

impl BxPcSystemC {
    fn new() -> Self {
        todo!()
    }
}

pub fn a20_addr(x: BxPhyAddress) -> BxPhyAddress {
    x & bx_pc_system.a20_mask
}
