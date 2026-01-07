use crate::cpu::decoder::X86FeatureName;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub struct BxParams {
    pub cpu_nthreads: u32,
    pub cpu_ncores: u32,
    pub cpu_nprocessors: u32,

    // TODO: use bitflags
    pub cpu_include_features: Vec<X86FeatureName>,
    pub cpu_exclude_features: Vec<X86FeatureName>,
}

impl Default for BxParams {
    fn default() -> Self {
        Self {
            cpu_nthreads: 1,
            cpu_ncores: 1,
            cpu_nprocessors: 1,
            cpu_include_features: Vec::new(),
            cpu_exclude_features: Vec::new(),
        }
    }
}
