use crate::cpu::{cpuid::BxCpuIdTrait, BxCpuC};

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) fn init_msrs(&mut self) {
        // TODO: implement later
    }
}
