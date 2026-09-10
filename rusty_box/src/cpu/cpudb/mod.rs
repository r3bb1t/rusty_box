pub(crate) mod amd;
pub(crate) mod intel;

use super::cpuid::{BxCpuIdTrait, CpuidFreq, SVMExtensions, VMXExtensions};
use super::decoder::BX_ISA_EXTENSIONS_ARRAY_SIZE;
use super::Result;
use amd::amd_ryzen::AmdRyzen;
use intel::core_i7_skylake::Corei7SkylakeX;

/// Runtime-selected CPU model.
///
/// Bochs resolves the cpudb model at runtime through the virtual base class
/// `bx_cpuid_t` (Bochs cpuid.h); this enum is the same architecture without
/// the vtable — a closed set of models with per-variant delegation. Model
/// behavior differences flow through the ISA extensions bitmask cached on the
/// CPU at init (Bochs cpu.h ia_extensions_bitmask), never through this type,
/// so the delegation below is only exercised on cold paths (init, the CPUID
/// instruction, diagnostics).
#[derive(Debug)]
pub enum CpuModel {
    Corei7SkylakeX(Corei7SkylakeX),
    AmdRyzen(AmdRyzen),
}

impl CpuModel {
    /// Power-on Skylake-X model, const-constructible for static placement.
    pub const fn corei7_skylake_x() -> Self {
        Self::Corei7SkylakeX(Corei7SkylakeX::INIT)
    }

    /// Power-on Ryzen model, const-constructible for static placement.
    pub const fn amd_ryzen() -> Self {
        Self::AmdRyzen(AmdRyzen::INIT)
    }
}

impl Default for CpuModel {
    fn default() -> Self {
        Self::INIT
    }
}

/// Per-variant delegation to the concrete model — the enum counterpart of
/// Bochs's `bx_cpuid_t` virtual dispatch.
macro_rules! delegate_to_model {
    ($self:expr, $model:ident => $body:expr) => {
        match $self {
            CpuModel::Corei7SkylakeX($model) => $body,
            CpuModel::AmdRyzen($model) => $body,
        }
    };
}

impl BxCpuIdTrait for CpuModel {
    const INIT: Self = Self::Corei7SkylakeX(Corei7SkylakeX::INIT);

    fn get_name(&self) -> &'static str {
        delegate_to_model!(self, m => m.get_name())
    }

    fn init(&mut self) {
        delegate_to_model!(self, m => m.init())
    }

    fn get_cpu_extensions(&self, extensions: &[u32]) {
        delegate_to_model!(self, m => m.get_cpu_extensions(extensions))
    }

    fn get_isa_extensions_bitmask(&self) -> [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE] {
        delegate_to_model!(self, m => m.get_isa_extensions_bitmask())
    }

    fn get_vmx_extensions_bitmask(&self) -> Option<VMXExtensions> {
        delegate_to_model!(self, m => m.get_vmx_extensions_bitmask())
    }

    fn get_svm_extensions_bitmask(&self) -> Option<SVMExtensions> {
        delegate_to_model!(self, m => m.get_svm_extensions_bitmask())
    }

    fn sanity_checks(&self) -> Result<()> {
        delegate_to_model!(self, m => m.sanity_checks())
    }

    fn new() -> Self {
        Self::INIT
    }

    fn get_cpuid_leaf(&self, eax: u32, ecx: u32) -> (u32, u32, u32, u32) {
        delegate_to_model!(self, m => m.get_cpuid_leaf(eax, ecx))
    }

    fn set_cpuid_freq(&mut self, freq: CpuidFreq, ips: u32) {
        delegate_to_model!(self, m => m.set_cpuid_freq(freq, ips))
    }
}
