//! SSE reciprocal and reciprocal square root approximation instructions
//!
//! Based on Bochs cpu/sse_rcp.cc
//!
//! RCPPS/RCPSS and RSQRTPS/RSQRTSS are *approximate*: the architecture
//! specifies a relative error bound of about 1.5·2⁻¹², not a correctly
//! rounded result. Bochs reproduces the hardware result exactly from three
//! precomputed tables, and so does this port — computing a full-precision
//! `1.0 / x` instead would disagree with real hardware (and with Bochs) in
//! the low mantissa bits.
//!
//! None of these instructions raise SSE exceptions or consult MXCSR, so no
//! SoftFloat status word is involved (Bochs `approximate_rcp` /
//! `approximate_rsqrt` take none either).

use super::sse_rcp_tables::{RCP_TABLE, RSQRT_TABLE0, RSQRT_TABLE1};
use super::softfloat3e::f32_class::f32_class;
use super::softfloat3e::internals::pack_to_f32;
use super::softfloat3e::softfloat::{f32_sign, SoftFloatClass};
use super::softfloat3e::softfloat_types::float32;
use super::softfloat3e::specialize::{FLOAT32_DEFAULT_NAN, FLOAT32_EXP_BIAS};
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedXmmRegister,
};

/// Bochs xmm.h `convert_to_QNaN` — force the quiet bit on a NaN.
#[inline]
fn convert_to_qnan(op: float32) -> float32 {
    op | 0x7FC0_0000
}

/// Bochs sse_rcp.cc `approximate_rcp`.
///
/// Computes 1/1.yyyyyyyyyyy1 rounded to the 12th fraction bit by
/// round-to-nearest regardless of the current rounding mode, from a
/// 2048-entry table.
pub(super) fn approximate_rcp(op: float32) -> float32 {
    let sign = f32_sign(op);
    match f32_class(op) {
        SoftFloatClass::Zero | SoftFloatClass::Denormal => return pack_to_f32(sign, 0xFF, 0),
        SoftFloatClass::NegativeInf | SoftFloatClass::PositiveInf => {
            return pack_to_f32(sign, 0, 0)
        }
        SoftFloatClass::SNaN | SoftFloatClass::QNaN => return convert_to_qnan(op),
        SoftFloatClass::Normalized => {}
    }

    let fraction = super::softfloat3e::softfloat::f32_fraction(op);
    let exp = super::softfloat3e::softfloat::f32_exp(op);

    let exp = 2 * FLOAT32_EXP_BIAS as i16 - 1 - exp;
    // Underflow: the reciprocal of a huge normal is not representable.
    if exp <= 0 {
        return pack_to_f32(sign, 0, 0);
    }

    pack_to_f32(sign, exp, (RCP_TABLE[(fraction >> 12) as usize] as u32) << 8)
}

/// Bochs sse_rcp.cc `approximate_rsqrt`.
///
/// Computes 1/sqrt(1.yyyyyyyyyy1) rounded to the 11th fraction bit by
/// round-to-nearest regardless of the current rounding mode, from two
/// 1024-entry tables selected by the low bit of the exponent.
pub(super) fn approximate_rsqrt(op: float32) -> float32 {
    let sign = f32_sign(op);
    match f32_class(op) {
        SoftFloatClass::Zero | SoftFloatClass::Denormal => return pack_to_f32(sign, 0xFF, 0),
        SoftFloatClass::PositiveInf => return 0,
        SoftFloatClass::NegativeInf => return FLOAT32_DEFAULT_NAN,
        SoftFloatClass::SNaN | SoftFloatClass::QNaN => return convert_to_qnan(op),
        SoftFloatClass::Normalized => {}
    }

    // sqrt of a negative normal is invalid.
    if sign {
        return FLOAT32_DEFAULT_NAN;
    }

    let fraction = super::softfloat3e::softfloat::f32_fraction(op);
    let exp = super::softfloat3e::softfloat::f32_exp(op);

    let table: &[u16; 1024] = if (exp & 1) != 0 {
        &RSQRT_TABLE1
    } else {
        &RSQRT_TABLE0
    };
    let exp = 0x7E - ((exp - 0x7F) >> 1);

    pack_to_f32(false, exp, (table[(fraction >> 13) as usize] as u32) << 8)
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Shared source read for the packed forms.
    #[inline]
    fn sse_rcp_read_op(&mut self, instr: &Instruction) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src1()))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_xmmword(seg, eaddr)
        }
    }

    // ========================================================================
    // RCPPS — Reciprocal of Packed Single-Precision (approximate)
    // Bochs: RCPPS_VpsWps in sse_rcp.cc
    // ========================================================================

    pub(super) fn rcpps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_rcp_read_op(instr)?;
        for i in 0..4 {
            op.set_xmm32u(i, approximate_rcp(op.xmm32u(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    // ========================================================================
    // RCPSS — Reciprocal of Scalar Single-Precision (approximate)
    // Bochs: RCPSS_VssWss in sse_rcp.cc
    // ========================================================================

    pub(super) fn rcpss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32u(0, approximate_rcp(op));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // RSQRTPS — Reciprocal Square Root of Packed Single-Precision (approximate)
    // Bochs: RSQRTPS_VpsWps in sse_rcp.cc
    // ========================================================================

    pub(super) fn rsqrtps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_rcp_read_op(instr)?;
        for i in 0..4 {
            op.set_xmm32u(i, approximate_rsqrt(op.xmm32u(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    // ========================================================================
    // RSQRTSS — Reciprocal Square Root of Scalar Single-Precision (approximate)
    // Bochs: RSQRTSS_VssWss in sse_rcp.cc
    // ========================================================================

    pub(super) fn rsqrtss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32u(0, approximate_rsqrt(op));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rcp(x: f32) -> f32 {
        f32::from_bits(approximate_rcp(x.to_bits()))
    }
    fn rsqrt(x: f32) -> f32 {
        f32::from_bits(approximate_rsqrt(x.to_bits()))
    }

    // The architectural bound is |relative error| < 1.5 * 2^-12.
    #[test]
    fn approximations_stay_inside_the_architectural_error_bound() {
        const BOUND: f64 = 1.5 * (1.0 / 4096.0);
        for &x in &[
            1.0f32, 2.0, 3.0, 0.5, 0.1, 7.0, 1234.5, 1e10, 1e-10, 65504.0, 1.9999999,
        ] {
            let want = 1.0f64 / x as f64;
            let got = rcp(x) as f64;
            assert!(
                ((got - want) / want).abs() < BOUND,
                "rcp({x}) = {got}, want ~{want}"
            );

            let want = 1.0f64 / (x as f64).sqrt();
            let got = rsqrt(x) as f64;
            assert!(
                ((got - want) / want).abs() < BOUND,
                "rsqrt({x}) = {got}, want ~{want}"
            );
        }
    }

    // Being approximate is the point: an exact reciprocal would be a
    // divergence from both hardware and Bochs. Even the powers of two come
    // back short, because the table stores 1/1.yyyyyyyyyyy1 — the extra
    // trailing 1 bit biases every entry slightly low.
    #[test]
    fn approximations_are_not_correctly_rounded() {
        assert_ne!(rcp(3.0).to_bits(), (1.0f32 / 3.0).to_bits());
        assert_ne!(rsqrt(3.0).to_bits(), (1.0f32 / 3.0f32.sqrt()).to_bits());

        // Exact bit patterns Bochs produces from its tables.
        assert_eq!(rcp(1.0).to_bits(), 0x3F7F_F000);
        assert_eq!(rcp(2.0).to_bits(), 0x3EFF_F000);
        assert_eq!(rcp(0.5).to_bits(), 0x3FFF_F000);
        assert_eq!(rsqrt(1.0).to_bits(), 0x3F7F_F000);
        assert_eq!(rsqrt(4.0).to_bits(), 0x3EFF_F000);
        // Odd exponent selects the second table.
        assert_eq!(rsqrt(2.0).to_bits(), 0x3F34_F800);
    }

    #[test]
    fn special_values_follow_bochs() {
        assert_eq!(rcp(0.0).to_bits(), f32::INFINITY.to_bits());
        assert_eq!(rcp(-0.0).to_bits(), f32::NEG_INFINITY.to_bits());
        assert_eq!(rcp(f32::INFINITY).to_bits(), 0.0f32.to_bits());
        assert_eq!(rcp(f32::NEG_INFINITY).to_bits(), (-0.0f32).to_bits());
        assert!(rcp(f32::NAN).is_nan());

        assert_eq!(rsqrt(0.0).to_bits(), f32::INFINITY.to_bits());
        assert_eq!(rsqrt(f32::INFINITY).to_bits(), 0.0f32.to_bits());
        assert!(rsqrt(-1.0).is_nan());
        assert!(rsqrt(f32::NEG_INFINITY).is_nan());
    }
}
