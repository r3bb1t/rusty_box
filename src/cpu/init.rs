use alloc::vec::Vec;

use super::Result;
use crate::{
    cpu::{decoder::features::X86Feature, svm::VmcbCache, BxCpuC},
    params::BxParams,
};

use super::{
    cpudb::intel::core_i7_skylake::Corei7SkylakeX, cpuid::BxCpuIdTrait, decoder::X86FeatureName,
};

pub(super) fn cpuid_factory() -> impl BxCpuIdTrait {
    // Note: hardcode this for now
    Corei7SkylakeX {}
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub fn initialize(&mut self, config: BxParams) -> Result<()> {
        tracing::info!("Initialized cpu model {}", self.cpuid.get_name());

        let _cpuid_features: Vec<X86FeatureName> = config
            .cpu_include_features
            .iter()
            .cloned()
            .filter(|feature| config.cpu_exclude_features.contains(feature))
            .collect();

        self.svm_extensions_bitmask = self.cpuid.get_svm_extensions_bitmask();
        self.svm_extensions_bitmask = self.cpuid.get_svm_extensions_bitmask();

        self.sanity_checks()?;

        self.init_fetch_decode_tables()?;

        self.xsave_xrestor_init();

        #[cfg(feature = "bx_support_amx")]
        {
            self.amx = if self
                .ia_extensions_bitmask
                .contains(&(X86Feature::IsaAMX as _))
            {
                Some(AMX::default())
            } else {
                None
            };
        }

        self.vmcb = if self
            .ia_extensions_bitmask
            .contains(&(X86Feature::IsaSVM as _))
        {
            Some(VmcbCache::default())
        } else {
            None
        };

        self.init_msrs();

        self.smram_map = Self::init_smram()?;

        // Skip msrs stuff for now
        self.init_vmcs();

        self.init_statistics();

        Ok(())
    }

    fn init_statistics(&mut self) {
        // Not now
    }

    fn sanity_checks(&mut self) -> Result<()> {
        // Late
        Ok(())
    }
}
