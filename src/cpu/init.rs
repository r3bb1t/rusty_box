use super::{cpu::BxCpuC, cpudb::intel::core_i7_skylake::Corei7SkylakeX, cpuid::BxCpuTrait};

fn cpuid_factory<I: BxCpuTrait>(cpu: &BxCpuC<I>) -> impl BxCpuTrait {
    // Note: hardcode this for now
    Corei7SkylakeX {}
}

pub fn initialize() {}
