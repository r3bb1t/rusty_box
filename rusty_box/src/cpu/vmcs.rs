use super::{cpuid::BxCpuIdTrait, BxCpuC};

// VMCS pointer is always 64-bit variable
pub(super) const BX_INVALID_VMCSPTR: u64 = 0xFFFFFFFFFFFFFFFF;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn init_vmcs(&mut self) {

        // Skip for now
    }
}
