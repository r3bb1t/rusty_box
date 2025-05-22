// cpu/cpudb/intel/corei7_skylake-x.cc

use crate::cpu::cpuid::BxCpuIdTrait;

#[derive(Debug)]
pub(crate) struct Corei7SkylakeX {}

impl BxCpuIdTrait for Corei7SkylakeX {
    const NAME: &'static str = "corei7_skylake_x";
}
