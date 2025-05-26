// cpu/cpudb/intel/corei7_skylake-x.cc

use crate::cpu::cpuid::BxCpuIdTrait;

#[derive(Debug)]
pub(crate) struct Corei7SkylakeX {}

impl BxCpuIdTrait for Corei7SkylakeX {
    //const NAME: &'static str = "corei7_skylake_x";
    fn get_name(&self) -> &'static str {
        "corei7_skylake_x"
    }

    fn get_vmx_extensions_bitmask(&self) -> Option<crate::cpu::cpuid::VMXExtensions> {
        todo!()
    }

    fn get_svm_extensions_bitmask(&self) -> Option<crate::cpu::cpuid::SVMExtensions> {
        todo!()
    }

    fn sanity_checks(&self) -> crate::cpu::error::Result<()> {
        todo!()
    }
}
