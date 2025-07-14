#[cfg(feature = "std")]
use std::sync::OnceLock;

use crate::config::BxPhyAddress;
#[cfg(not(feature = "std"))]
use spin::once::Once;

#[cfg(feature = "std")]
static BX_PC_SYSTEM_LOCK: OnceLock<BxPcSystemC> = OnceLock::new();

#[cfg(not(feature = "std"))]
static BX_PC_SYSTEM_LOCK: Once<BxPcSystemC> = Once::new();

pub fn bx_pc_system() -> &'static BxPcSystemC {
    #[cfg(feature = "std")]
    return BX_PC_SYSTEM_LOCK.get_or_init(BxPcSystemC::new);
    #[cfg(not(feature = "std"))]
    BX_PC_SYSTEM_LOCK.call_once(BxPcSystemC::new)
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
