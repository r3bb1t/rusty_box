use super::{cpuid::BxCpuIdTrait, BxCpuC};

// VMCS pointer is always 64-bit variable
pub(super) const BX_INVALID_VMCSPTR: u64 = 0xFFFFFFFFFFFFFFFF;

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    pub(super) fn init_vmcs(&mut self) {

        // Skip for now
    }
}
