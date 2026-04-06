#![allow(unused_assignments, dead_code)]

use crate::cpu::decoder::features::X86Feature;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub struct BxParams {
    pub(crate) cpu_nthreads: u32,
    pub(crate) cpu_ncores: u32,
    pub(crate) cpu_nprocessors: u32,

    // TODO: use bitflags
    pub(crate) cpu_include_features: Vec<X86Feature>,
    pub(crate) cpu_exclude_features: Vec<X86Feature>,
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
