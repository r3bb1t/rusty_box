use alloc::{boxed::Box, vec::Vec};

use super::Result;
use crate::params::BxParams;

use super::{
    cpudb::intel::core_i7_skylake::Corei7SkylakeX, cpuid::BxCpuIdTrait,
    decoder::X86FeatureName,
};

fn cpuid_factory() -> Box<dyn BxCpuIdTrait> {
    // Note: hardcode this for now
    Box::new(Corei7SkylakeX {})
}

pub fn initialize<I: BxCpuIdTrait>(config: BxParams, _cpuid: I) -> Result<()> {
    let cpuid = cpuid_factory();
    tracing::info!("Initialized cpu model {}", cpuid.get_name());

    let _cpuid_features: Vec<X86FeatureName> = config
        .cpu_include_features
        .iter()
        .cloned()
        .filter(|feature| config.cpu_exclude_features.contains(feature))
        .collect();

    let _vmx_extensions_bitmask = cpuid.get_vmx_extensions_bitmask();
    let _svm_extensions_bitmask = cpuid.get_svm_extensions_bitmask();

    cpuid.sanity_checks()?;

    todo!()
}
