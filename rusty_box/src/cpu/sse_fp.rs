//! SSE/AVX floating-point glue: MXCSR ↔ SoftFloat status conversion and the
//! post-computation SSE exception check. Mirrors Bochs `cpu/sse_pfp.cc`
//! (`mxcsr_to_softfloat_status_word`, `check_exceptionsSSE`).

use super::cpu::{BxCpuC, Exception};
use super::cpuid::BxCpuIdTrait;
use super::instrumentation::Instrumentation;
use super::softfloat3e::softfloat::SoftFloatStatus;
use super::xmm::{BxMxcsr, Mxcsr, MXCSR_EXCEPTIONS};

/// Build a SoftFloat status word from the current MXCSR.
/// Bochs sse_pfp.cc `mxcsr_to_softfloat_status_word`.
pub fn mxcsr_to_softfloat_status_word(mxcsr: BxMxcsr) -> SoftFloatStatus {
    SoftFloatStatus {
        // MXCSR RC (0=nearest,1=down,2=up,3=truncate) is identical to the
        // SoftFloat rounding-mode encoding.
        softfloat_roundingMode: mxcsr.rounding_mode(),
        softfloat_exceptionFlags: 0,
        softfloat_exceptionMasks: mxcsr.exceptions_masks(),
        softfloat_suppressException: 0,
        // Flush-to-zero only applies when underflow is masked (Bochs
        // get_flush_masked_underflow() && get_UM()).
        softfloat_flush_underflow_to_zero: mxcsr.flush_to_zero()
            && mxcsr.is_masked(Mxcsr::UE.bits()),
        softfloat_denormals_are_zeros: mxcsr.daz(),
        // Irrelevant for f32/f64 muladd; kept at the 80-bit default.
        extF80_roundingPrecision: 80,
    }
}

impl<I: BxCpuIdTrait, T: Instrumentation> BxCpuC<'_, I, T> {
    /// Update MXCSR status bits from a SoftFloat exception-flags word and,
    /// if any unmasked exception occurred, raise #XM (or #UD when
    /// CR4.OSXMMEXCPT is clear). Bochs sse_pfp.cc `check_exceptionsSSE`.
    pub(super) fn check_exceptions_sse(&mut self, exception_flags: i32) -> super::Result<()> {
        let mut flags = exception_flags & (MXCSR_EXCEPTIONS as i32);
        let unmasked = !self.mxcsr.exceptions_masks() & flags;
        // An unmasked pre-computation exception (#I/#D/#Z = low 3 bits) makes
        // the post-computation bits (#O/#U/#P) not sticky.
        if unmasked & 0x7 != 0 {
            flags &= 0x7;
        }
        self.mxcsr.set_exceptions(flags);

        if unmasked != 0 {
            if self.cr4.osxmmexcpt() {
                return self.exception(Exception::Xm, 0);
            }
            return self.exception(Exception::Ud, 0);
        }
        Ok(())
    }
}
