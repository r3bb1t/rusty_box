#![allow(unused_assignments, dead_code)]

use crate::cpu::decoder::features::X86Feature;

const MAX_FEATURES: usize = 64;

/// Fixed-capacity list of CPU features, replacing `Vec<X86Feature>` for no-alloc support.
#[derive(Debug, Clone, Copy)]
pub struct FeatureList {
    features: [X86Feature; MAX_FEATURES],
    len: usize,
}

impl FeatureList {
    pub const fn new() -> Self {
        // X86Feature is repr-safe; use first variant as filler (never read past `len`).
        Self {
            features: [X86Feature::Isa386; MAX_FEATURES],
            len: 0,
        }
    }

    pub fn push(&mut self, feature: X86Feature) {
        assert!(self.len < MAX_FEATURES, "FeatureList overflow");
        self.features[self.len] = feature;
        self.len += 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = &X86Feature> {
        self.features[..self.len].iter()
    }

    pub fn contains(&self, feature: &X86Feature) -> bool {
        self.features[..self.len].contains(feature)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

impl Default for FeatureList {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct BxParams {
    pub(crate) cpu_nthreads: u32,
    pub(crate) cpu_ncores: u32,
    pub(crate) cpu_nprocessors: u32,

    pub(crate) cpu_include_features: FeatureList,
    pub(crate) cpu_exclude_features: FeatureList,
}

impl Default for BxParams {
    fn default() -> Self {
        Self {
            cpu_nthreads: 1,
            cpu_ncores: 1,
            cpu_nprocessors: 1,
            cpu_include_features: FeatureList::new(),
            cpu_exclude_features: FeatureList::new(),
        }
    }
}
