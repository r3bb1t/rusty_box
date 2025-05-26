use alloc::vec::Vec;

use crate::cpu::decoder::X86FeatureName;

#[derive(Debug, Clone)]
pub struct BxParams {
    pub cpu_nthreads: u32,
    pub cpu_ncores: u32,
    pub cpu_nprocessors: u32,

    // TODO: use bitflags
    pub cpu_include_features: Vec<X86FeatureName>,
    pub cpu_exclude_features: Vec<X86FeatureName>,
}
