use super::{cpu::BxCpuC, cpudb::intel::core_i7_skylake::Corei7SkylakeX, cpuid::BxCpuIdTrait};

fn cpuid_factory<I: BxCpuIdTrait>(cpu: &BxCpuC<I>) -> impl BxCpuIdTrait {
    // Note: hardcode this for now
    Corei7SkylakeX {}
}

pub fn initialize() {}
