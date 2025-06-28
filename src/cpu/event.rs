use super::{cpuid::BxCpuIdTrait, BxCpuC};

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    pub(super) fn handle_async_event(&mut self) {
        unimplemented!()
    }
}
