// Re-export the entire decoder crate's public API.
pub use rusty_box_decoder::*;

// Flatten key types so callers can write `decoder::Instruction` etc.
pub use rusty_box_decoder::features::X86Feature;
pub use rusty_box_decoder::instruction::{
    AddressSize, GprIndex, Instruction, InstructionFlags, OperandSize, Operands, RepPrefix,
};
pub use rusty_box_decoder::opcode::Opcode;

use crate::cpu::{BxCpuC, BxCpuIdTrait};

/// The ISA gate itself is needed by the icache fill path, which compiles
/// without `alloc`, so it lives outside the alloc-gated block below.
impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Resolve a freshly decoded opcode against this CPU's CPUID feature set.
    ///
    /// Returns the opcode to place in the trace: the original when the model
    /// supports it, the legacy alias for LZCNT/TZCNT when their feature bit is
    /// absent, or [`Opcode::IaError`] (which dispatches to #UD) otherwise.
    ///
    /// Bochs does the equivalent once at init in
    /// `cpu/decoder/fetchdecode32.cc` `init_FetchDecodeTables()`, by rewriting
    /// a **process-global** `BxOpcodesTable` so unsupported opcodes point at
    /// `BxError`. That construct is not thread safe and cannot represent two
    /// CPUs with different models, so rusty_box keeps the generated table
    /// immutable (`opcode_isa::OPCODE_ISA`, shared read-only across threads)
    /// and consults each CPU's own `ia_extensions_bitmask` instead. Same
    /// observable behaviour, no shared mutable state.
    ///
    /// Called from the icache fill path, so the dispatch loop pays nothing.
    pub(in crate::cpu) fn isa_resolve_opcode(&self, opcode: Opcode) -> Opcode {
        let feature = rusty_box_decoder::opcode_isa::opcode_isa_feature(opcode);
        if feature == rusty_box_decoder::opcode_isa::ISA_ALWAYS {
            return opcode;
        }
        if self.isa_feature_index_enabled(feature) {
            return opcode;
        }

        // Bochs init_FetchDecodeTables special case 1: these MMX-era opcodes
        // are also available when 3DNow! Extensions are present, even though
        // their declared feature (SSE) is not.
        if self.bx_cpuid_support_isa_extension(X86Feature::Isa3dnowExt)
            && matches!(
                opcode,
                Opcode::MaskmovqPqNq
                    | Opcode::MovntqMqPq
                    | Opcode::PavgbPqQq
                    | Opcode::PavgwPqQq
                    | Opcode::PextrwGdNqIb
                    | Opcode::PinsrwPqEwIb
                    | Opcode::PmaxswPqQq
                    | Opcode::PmaxubPqQq
                    | Opcode::PminswPqQq
                    | Opcode::PminubPqQq
                    | Opcode::PmovmskbGdNq
                    | Opcode::PmulhuwPqQq
                    | Opcode::PsadbwPqQq
                    | Opcode::PshufwPqQqIb
                    | Opcode::Sfence
            )
        {
            return opcode;
        }

        // Bochs special case 2: AVX10.1 subsumes every AVX-512 sub-extension,
        // so a model advertising it may run them without the individual bits.
        if self.bx_cpuid_support_isa_extension(X86Feature::IsaAvx10_1)
            && Self::is_avx512_subfeature(feature)
        {
            return opcode;
        }

        // Bochs special case 3: without LZCNT/BMI1 the F3-prefixed encodings
        // are architecturally BSR/BSF, not #UD — Bochs copies the BSR/BSF
        // table entry over the LZCNT/TZCNT one.
        match opcode {
            Opcode::LzcntGwEw => return Opcode::BsrGwEw,
            Opcode::LzcntGdEd => return Opcode::BsrGdEd,
            Opcode::LzcntGqEq => return Opcode::BsrGqEq,
            Opcode::TzcntGwEw => return Opcode::BsfGwEw,
            Opcode::TzcntGdEd => return Opcode::BsfGdEd,
            Opcode::TzcntGqEq => return Opcode::BsfGqEq,
            _ => {}
        }

        Opcode::IaError
    }

    /// Resolve a decoded opcode against the CPU state the guest has enabled.
    ///
    /// Bochs tags each instruction with the state it needs — the `BX_PREPARE_*`
    /// field of `bx_define_opcode` — and `assignHandler`
    /// (`cpu/decoder/fetchdecode32.cc`) swaps the handler for `BxNoAVX` or
    /// `BxNoEVEX` when the matching `BX_FETCH_MODE_*_OK` bit is clear. Those
    /// handlers raise #UD when the state is unavailable and #NM when CR0.TS is
    /// set, which is why this returns a sentinel opcode rather than
    /// [`Opcode::IaError`]: only the handler can tell the two faults apart.
    ///
    /// Applied at icache fill next to [`Self::isa_resolve_opcode`], so the
    /// dispatch loop pays nothing. That is sound because the icache is keyed on
    /// `fetch_mode_mask` (see `BxICache::hash`), so a trace decoded while AVX
    /// was disabled cannot be reused after the guest enables it.
    ///
    /// Only the AVX and AVX-512 classes are resolved here. `PREPARE_SSE`,
    /// `PREPARE_MMX` and `PREPARE_FPU` are enforced inside their handlers by
    /// `prepare_sse` / `prepare_fpu`, which is already correct for them; AMX is
    /// not implemented and its opcodes are stopped by the ISA gate above.
    pub(in crate::cpu) fn state_resolve_opcode(&self, opcode: Opcode) -> Opcode {
        use super::opcodes_table::FetchModeMask;
        use rusty_box_decoder::opcode_isa::{opcode_prepare_class, STATE_AVX, STATE_EVEX};

        match opcode_prepare_class(opcode) {
            STATE_AVX if !self.fetch_mode_mask.contains(FetchModeMask::AVX_OK) => {
                Opcode::NoAvxState
            }
            STATE_EVEX if !self.fetch_mode_mask.contains(FetchModeMask::EVEX_OK) => {
                Opcode::NoEvexState
            }
            _ => opcode,
        }
    }

    /// True when the raw `X86Feature` discriminant is set for this CPU.
    /// Companion to [`Self::bx_cpuid_support_isa_extension`] for the generated
    /// table, which stores discriminants rather than enum values.
    #[inline]
    fn isa_feature_index_enabled(&self, feature: u16) -> bool {
        let index = feature as usize;
        match self.ia_extensions_bitmask.get(index / 32) {
            Some(word) => (word & (1 << (index % 32))) != 0,
            None => false,
        }
    }

    /// The AVX-512 sub-extensions that AVX10.1 implies (Bochs
    /// `init_FetchDecodeTables` switch).
    fn is_avx512_subfeature(feature: u16) -> bool {
        const SUBFEATURES: [X86Feature; 12] = [
            X86Feature::IsaAvx512,
            X86Feature::IsaAvx512Dq,
            X86Feature::IsaAvx512Bw,
            X86Feature::IsaAvx512Cd,
            X86Feature::IsaAvx512Vbmi,
            X86Feature::IsaAvx512Vbmi2,
            X86Feature::IsaAvx512Ifma52,
            X86Feature::IsaAvx512Vpopcntdq,
            X86Feature::IsaAvx512Vnni,
            X86Feature::IsaAvx512Bitalg,
            X86Feature::IsaAvx512Bf16,
            X86Feature::IsaAvx512Fp16,
        ];
        SUBFEATURES.iter().any(|f| *f as u16 == feature)
    }
}

// The remaining init-time reporting is only reachable from the alloc build.
#[cfg(feature = "alloc")]
impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Validate CPU feature bitmask and configure decode tables.
    ///
    /// Bochs fetchdecode32.cc: loops all opcodes and disables those
    /// whose ISA feature isn't in ia_extensions_bitmask. Also handles special
    /// cases like LZCNT→BSR and TZCNT→BSF fallback.
    ///
    /// rusty_box applies the same gate per decoded instruction in
    /// [`Self::isa_resolve_opcode`] rather than by patching a global table;
    /// this function keeps Bochs's "bitmask must be populated" panic and logs
    /// the resulting configuration.
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
            has_sse,
            has_sse2,
            has_avx,
            has_avx2,
            has_bmi1,
            has_bmi2,
            has_aes,
            has_long_mode,
            has_lzcnt
        );

        tracing::debug!(
            "ISA opcode gate active: {} of {} opcodes carry a CPUID feature",
            rusty_box_decoder::opcode_isa::GATED_OPCODE_COUNT,
            rusty_box_decoder::opcode_isa::OPCODE_ISA.len()
        );

        Ok(())
    }
}
