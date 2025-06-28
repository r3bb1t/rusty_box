use super::{cpuid::BxCpuIdTrait, BxCpuC};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn init_vmcs(&mut self) {

        // Skip for now
    }
}
