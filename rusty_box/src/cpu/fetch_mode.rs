//! The decode-time view of the processor's mode: which CPU state the guest has
//! enabled, and how much of the vector register file XCR0 exposes.
//!
//! Bochs keeps both on the CPU (`cpu.h` `fetchModeMask`, `decoder.h`
//! `bx_avx_vector_length`). The icache keys its traces on the mask, and the
//! fill path's state gate (`state_resolve_opcode`) reads it to decide whether
//! an instruction runs or faults.

use bitflags::bitflags;

bitflags! {
    /// Bochs `cpu.h` `BX_FETCH_MODE_*`: the code-segment size and long-mode
    /// bits that select the decoder, and the CPU-state bits Bochs
    /// `assignHandler` tests against an opcode's `BX_PREPARE_*` class.
    /// Recomputed by the `handle_*_mode_change` family whenever CS, CR0,
    /// CR4 or XCR0 changes what they derive from.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub(crate) struct FetchModeMask: u32 {
        /// CS.D_B — 32-bit default operand/address size
        const D_B = 1 << 0;

        /// Long64 mode active (CS.L=1 in long mode)
        const LONG64 = 1 << 1;

        /// FPU/MMX available
        const FPU_MMX_OK = 1 << 2;

        /// SSE available (CPU_LEVEL >= 6)
        const SSE_OK = 1 << 3;

        /// AVX available (BX_SUPPORT_AVX)
        const AVX_OK = 1 << 4;

        /// Opmask available (BX_SUPPORT_EVEX)
        const OPMASK_OK = 1 << 5;

        /// EVEX available (BX_SUPPORT_EVEX)
        const EVEX_OK = 1 << 6;

        /// AMX available (BX_SUPPORT_AMX)
        const AMX_OK = 1 << 7;
    }
}

/// The maximum architecturally-visible vector length, as enabled by XCR0.
/// Bochs decoder.h `bx_avx_vector_length`; the discriminants are upstream's
/// so that comparisons read the same way (`maxvl > Vl256`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum BxAvxVectorLength {
    /// XCR0 enables neither YMM nor ZMM state: writes clear nothing above 128.
    #[default]
    Vl128 = 1,
    /// XCR0.YMM only.
    Vl256 = 2,
    /// XCR0 also enables OPMASK / ZMM_HI256 / HI_ZMM.
    Vl512 = 4,
}
