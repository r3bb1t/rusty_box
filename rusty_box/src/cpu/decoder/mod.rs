// Re-export the entire decoder crate's public API.
pub use rusty_box_decoder::*;

// Flatten key types so callers can write `decoder::Instruction` etc.
pub use rusty_box_decoder::instruction::{
    AddressSize, GprIndex, Instruction, InstructionFlags, Operands, OperandSize, RepPrefix,
};
pub use rusty_box_decoder::opcode::Opcode;
pub use rusty_box_decoder::features::X86Feature;

// BxCpuC impl block requires alloc (BxCpuC lives behind alloc gate)
#[cfg(feature = "alloc")]
use crate::cpu::{BxCpuC, BxCpuIdTrait};

#[cfg(feature = "alloc")]
impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Validate CPU feature bitmask and configure decode tables.
    ///
    /// Bochs fetchdecode32.cc: loops all opcodes and disables those
    /// whose ISA feature isn't in ia_extensions_bitmask. Also handles special
    /// cases like LZCNT→BSR and TZCNT→BSF fallback.
    ///
    /// Our decoder uses const tables so we can't patch them at runtime.
    /// Instead, unsupported opcodes hit the dispatcher catch-all which raises #UD.
    /// This function validates the bitmask is populated and logs the configuration.
    pub(in crate::cpu) fn init_fetch_decode_tables(&mut self) -> crate::cpu::Result<()> {
        // Bochs panics if bitmask is empty (fetchdecode32.cc)
        if self.ia_extensions_bitmask[0] == 0 {
            return Err(crate::cpu::CpuError::UnimplementedInstruction);
        }

        // Log key ISA feature status for debugging
        let has_sse = self.bx_cpuid_support_isa_extension(X86Feature::IsaSse);
        let has_sse2 = self.bx_cpuid_support_isa_extension(X86Feature::IsaSse2);
        let has_avx = self.bx_cpuid_support_isa_extension(X86Feature::IsaAvx);
        let has_avx2 = self.bx_cpuid_support_isa_extension(X86Feature::IsaAvx2);
        let has_bmi1 = self.bx_cpuid_support_isa_extension(X86Feature::IsaBmi1);
        let has_bmi2 = self.bx_cpuid_support_isa_extension(X86Feature::IsaBmi2);
        let has_aes = self.bx_cpuid_support_isa_extension(X86Feature::IsaAesPclmulqdq);
        let has_long_mode = self.bx_cpuid_support_isa_extension(X86Feature::IsaLongMode);
        let has_lzcnt = self.bx_cpuid_support_isa_extension(X86Feature::IsaLzcnt);

        tracing::debug!(
            "CPU ISA features: SSE={} SSE2={} AVX={} AVX2={} BMI1={} BMI2={} AES={} LM={} LZCNT={}",
            has_sse, has_sse2, has_avx, has_avx2, has_bmi1, has_bmi2, has_aes, has_long_mode, has_lzcnt
        );

        // LZCNT/TZCNT fallback (Bochs fetchdecode32.cc):
        // When LZCNT not supported, F3 0F BD decodes as BSR (REP prefix ignored).
        // When BMI1 (TZCNT) not supported, F3 0F BC decodes as BSF.
        // Our CPUID reports both as supported for Skylake-X, so no fallback needed.
        // If a different CPU model doesn't support them, the decoder will still
        // decode LZCNT/TZCNT, and they'll execute correctly (our handlers exist).
        // The only difference from Bochs is we won't alias them to BSR/BSF.

        Ok(())
    }
}
