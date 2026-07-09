//! SSE/SSE2 packed floating-point instruction handlers
//!
//! Based on Bochs cpu/sse_pfp.cc
//!
//! Implements SSE/SSE2 packed and scalar floating-point operations:
//! - Arithmetic: ADD, SUB, MUL, DIV, SQRT, MIN, MAX (ps/pd/ss/sd)
//! - Bitwise logical: AND, ANDN, OR, XOR (ps/pd)
//! - Compare: CMPPS/PD/SS/SD (8 predicates), COMISS/COMISD, UCOMISS/UCOMISD
//! - Conversions: CVTSI2SS/SD, CVTSS2SI/SD2SI, CVTTSS2SI/CVTTSD2SI,
//!   CVTPS2PD, CVTPD2PS, CVTSS2SD, CVTSD2SS, CVTDQ2PS, CVTPS2DQ,
//!   CVTTPS2DQ, CVTDQ2PD, CVTPD2DQ, CVTTPD2DQ
//! - Shuffle: SHUFPS/PD, UNPCKLPS/PD, UNPCKHPS/PD
//!
//! Uses native Rust f32/f64 operations for floating-point math. While Bochs
//! uses SoftFloat3e, native FP is sufficient since we run on x86 host with
//! the same FP behavior. SoftFloat integration can be added later if needed.

#[cfg(not(feature = "std"))]
use crate::cpu::float::FloatExt;

/// Round-to-nearest-ties-even for f64 (no_std compatible).
/// IEEE 754 default rounding: if exactly halfway, round to even.
/// (The f32 conversions go through the exact f64 intermediate, so a
/// dedicated f32 variant is not needed.)
#[cfg(not(feature = "std"))]
#[inline]
pub(super) fn round_ties_even_f64(val: f64) -> f64 {
    let trunc = val as i64;
    let frac = val - trunc as f64;
    let abs_frac = if frac >= 0.0 { frac } else { -frac };
    if abs_frac == 0.5 {
        if trunc % 2 == 0 {
            trunc as f64
        } else if val > 0.0 {
            (trunc + 1) as f64
        } else {
            (trunc - 1) as f64
        }
    } else if abs_frac > 0.5 {
        if val > 0.0 {
            (trunc + 1) as f64
        } else {
            (trunc - 1) as f64
        }
    } else {
        trunc as f64
    }
}

/// Round-to-nearest-ties-even, dispatching on the `std` feature.
#[inline]
fn round_ties_even(val: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        val.round_ties_even()
    }
    #[cfg(not(feature = "std"))]
    {
        round_ties_even_f64(val)
    }
}

/// f64 → i32 with x86 semantics (Bochs softfloat f64_to_i32 /
/// f64_to_i32_round_to_zero): round or truncate FIRST, then range-check the
/// resulting INTEGER; NaN or out-of-range yields the integer indefinite
/// value 0x8000_0000. Checking the unrounded float against `i32::MAX as f64`
/// misclassifies the boundary band (e.g. 2147483647.4 truncates to a valid
/// 2147483647).
#[inline]
pub(super) fn cvt_f64_to_i32(val: f64, truncate: bool) -> i32 {
    // NaN (comparisons are false), ±inf and everything far outside i32 range
    // is invalid regardless of rounding; only the boundary band needs the
    // integer-domain check below.
    if !(val > -2_147_483_650.0 && val < 2_147_483_650.0) {
        return i32::MIN;
    }
    let rounded = if truncate { val } else { round_ties_even(val) };
    // `as` truncates toward zero — exact for every in-band value.
    let int = rounded as i64;
    if int < i32::MIN as i64 || int > i32::MAX as i64 {
        i32::MIN
    } else {
        int as i32
    }
}

/// f32 → i32 with x86 semantics; the f64 intermediate is exact for every
/// f32, so the boundary analysis of [`cvt_f64_to_i32`] carries over
/// (Bochs softfloat f32_to_i32 / f32_to_i32_round_to_zero).
#[inline]
pub(super) fn cvt_f32_to_i32(val: f32, truncate: bool) -> i32 {
    cvt_f64_to_i32(val as f64, truncate)
}

/// f64 → i64 with x86 semantics (Bochs softfloat f64_to_i64 /
/// f64_to_i64_round_to_zero). f64 values at or beyond ±2^63 are invalid;
/// inside that band the spacing near the edges is ≥ 1024 (integral), so no
/// rounded value can leave the band.
#[inline]
pub(super) fn cvt_f64_to_i64(val: f64, truncate: bool) -> i64 {
    if !(val >= -9_223_372_036_854_775_808.0 && val < 9_223_372_036_854_775_808.0) {
        return i64::MIN;
    }
    let rounded = if truncate { val } else { round_ties_even(val) };
    rounded as i64
}

/// f32 → i64 with x86 semantics via the exact f64 intermediate
/// (Bochs softfloat f32_to_i64 / f32_to_i64_round_to_zero).
#[inline]
pub(super) fn cvt_f32_to_i64(val: f32, truncate: bool) -> i64 {
    cvt_f64_to_i64(val as f64, truncate)
}

use super::{
    avx_pfp::{sse_max_f32, sse_max_f64, sse_min_f32, sse_min_f64},
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedXmmRegister,
};

// ============================================================================
// Compare predicate helper
// ============================================================================

/// Evaluate SSE compare predicate (imm8 bits[2:0]) for f32 operands.
/// Returns true if the comparison is satisfied.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
#[inline]
fn sse_compare_f32(op1: f32, op2: f32, predicate: u8) -> bool {
    match predicate & 7 {
        0 => op1 == op2,                                 // EQ
        1 => op1 < op2,                                  // LT
        2 => op1 <= op2,                                 // LE
        3 => op1.is_nan() || op2.is_nan(),               // UNORD
        4 => op1 != op2 || op1.is_nan() || op2.is_nan(), // NEQ (unordered or not equal)
        5 => !(op1 < op2),                               // NLT (not less than)
        6 => !(op1 <= op2),                              // NLE (not less than or equal)
        7 => !op1.is_nan() && !op2.is_nan(),             // ORD
        _ => unreachable!("SSE compare predicate & 7 cannot exceed 7"),
    }
}

/// Evaluate SSE compare predicate (imm8 bits[2:0]) for f64 operands.
/// Returns true if the comparison is satisfied.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
#[inline]
fn sse_compare_f64(op1: f64, op2: f64, predicate: u8) -> bool {
    match predicate & 7 {
        0 => op1 == op2,                                 // EQ
        1 => op1 < op2,                                  // LT
        2 => op1 <= op2,                                 // LE
        3 => op1.is_nan() || op2.is_nan(),               // UNORD
        4 => op1 != op2 || op1.is_nan() || op2.is_nan(), // NEQ
        5 => !(op1 < op2),                               // NLT
        6 => !(op1 <= op2),                              // NLE
        7 => !op1.is_nan() && !op2.is_nan(),             // ORD
        _ => unreachable!("SSE compare predicate & 7 cannot exceed 7"),
    }
}

#[inline]
fn sse_round_mode(imm8: u8, mxcsr_rc: u8) -> u8 {
    if (imm8 & 0x04) == 0 {
        imm8 & 0x03
    } else {
        mxcsr_rc & 0x03
    }
}

#[inline]
pub(super) fn sse_round_f32(val: f32, imm8: u8, mxcsr_rc: u8) -> f32 {
    if val.is_nan() || val.is_infinite() {
        return val;
    }
    match sse_round_mode(imm8, mxcsr_rc) {
        0 => val.round_ties_even(),
        1 => val.floor(),
        2 => val.ceil(),
        _ => val.trunc(),
    }
}

#[inline]
pub(super) fn sse_round_f64(val: f64, imm8: u8, mxcsr_rc: u8) -> f64 {
    if val.is_nan() || val.is_infinite() {
        return val;
    }
    match sse_round_mode(imm8, mxcsr_rc) {
        0 => val.round_ties_even(),
        1 => val.floor(),
        2 => val.ceil(),
        _ => val.trunc(),
    }
}

// ============================================================================
// SSE3 horizontal add/sub lane helpers (Bochs simd_pfp.h xmm_haddps,
// xmm_haddpd, xmm_hsubps, xmm_hsubpd). Shared by the legacy handlers below
// and the per-128-bit-lane VEX handlers in avx_pfp.rs.
// ============================================================================

/// Bochs simd_pfp.h xmm_haddps
#[inline]
pub(super) fn haddps_lane(
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
) -> BxPackedXmmRegister {
    let mut r = BxPackedXmmRegister::default();
    r.set_xmm32f(0, op1.xmm32f(0) + op1.xmm32f(1));
    r.set_xmm32f(1, op1.xmm32f(2) + op1.xmm32f(3));
    r.set_xmm32f(2, op2.xmm32f(0) + op2.xmm32f(1));
    r.set_xmm32f(3, op2.xmm32f(2) + op2.xmm32f(3));
    r
}

/// Bochs simd_pfp.h xmm_haddpd
#[inline]
pub(super) fn haddpd_lane(
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
) -> BxPackedXmmRegister {
    let mut r = BxPackedXmmRegister::default();
    r.set_xmm64f(0, op1.xmm64f(0) + op1.xmm64f(1));
    r.set_xmm64f(1, op2.xmm64f(0) + op2.xmm64f(1));
    r
}

/// Bochs simd_pfp.h xmm_hsubps
#[inline]
pub(super) fn hsubps_lane(
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
) -> BxPackedXmmRegister {
    let mut r = BxPackedXmmRegister::default();
    r.set_xmm32f(0, op1.xmm32f(0) - op1.xmm32f(1));
    r.set_xmm32f(1, op1.xmm32f(2) - op1.xmm32f(3));
    r.set_xmm32f(2, op2.xmm32f(0) - op2.xmm32f(1));
    r.set_xmm32f(3, op2.xmm32f(2) - op2.xmm32f(3));
    r
}

/// Bochs simd_pfp.h xmm_hsubpd
#[inline]
pub(super) fn hsubpd_lane(
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
) -> BxPackedXmmRegister {
    let mut r = BxPackedXmmRegister::default();
    r.set_xmm64f(0, op1.xmm64f(0) - op1.xmm64f(1));
    r.set_xmm64f(1, op2.xmm64f(0) - op2.xmm64f(1));
    r
}

// ============================================================================
// SSE4.1 dot-product lane helpers. One 128-bit lane of DPPS/DPPD following
// Bochs' exact operation order (Bochs sse_pfp.cc DPPS_VpsWpsIbR and
// DPPD_VpdHpdWpdIbR): masked multiply under imm8[7:4] (unselected products
// are 0.0), shuffle+add reduction, then store the sum only to the result
// lanes selected by imm8[3:0] (0.0 elsewhere).
// ============================================================================

/// Bochs sse_pfp.cc DPPS_VpsWpsIbR / avx_pfp.cc VDPPS_VpsHpsWpsIbR (one lane)
#[inline]
pub(super) fn dpps_lane(
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    mask: u8,
) -> BxPackedXmmRegister {
    // xmm_mulps_mask: prod[n] = op1[n]*op2[n] under imm8[7:4], else 0.0
    let mut prod = [0.0f32; 4];
    for (n, p) in prod.iter_mut().enumerate() {
        if (mask >> 4) & (1 << n) != 0 {
            *p = op1.xmm32f(n) * op2.xmm32f(n);
        }
    }
    // xmm_shufps(.., prod, prod, 0xB1) then xmm_addps: pairwise sums
    let sum = [
        prod[1] + prod[0],
        prod[0] + prod[1],
        prod[3] + prod[2],
        prod[2] + prod[3],
    ];
    // xmm_shufpd(.., sum, sum, 0x1) then xmm_addps_mask under imm8[3:0]
    let swapped = [sum[2], sum[3], sum[0], sum[1]];
    let mut r = BxPackedXmmRegister::default();
    for n in 0..4usize {
        if mask & (1 << n) != 0 {
            r.set_xmm32f(n, sum[n] + swapped[n]);
        }
    }
    r
}

/// Bochs sse_pfp.cc DPPD_VpdHpdWpdIbR (one lane; also VDPPD which is
/// VL128-only)
#[inline]
pub(super) fn dppd_lane(
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    mask: u8,
) -> BxPackedXmmRegister {
    // xmm_mulpd_mask: prod[n] = op1[n]*op2[n] under imm8[5:4], else 0.0
    let mut prod = [0.0f64; 2];
    for (n, p) in prod.iter_mut().enumerate() {
        if (mask >> 4) & (1 << n) != 0 {
            *p = op1.xmm64f(n) * op2.xmm64f(n);
        }
    }
    // xmm_shufpd(.., prod, prod, 0x1) then xmm_addpd_mask under imm8[1:0]
    let swapped = [prod[1], prod[0]];
    let mut r = BxPackedXmmRegister::default();
    for n in 0..2usize {
        if mask & (1 << n) != 0 {
            r.set_xmm64f(n, prod[n] + swapped[n]);
        }
    }
    r
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // SSE FP helpers: read source operand (register or memory)
    // ========================================================================

    /// Read source operand as packed 128-bit XMM (for PS/PD packed ops).
    #[inline]
    fn sse_pfp_read_op2_xmm(&mut self, instr: &Instruction) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src1()))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_xmmword(seg, eaddr)
        }
    }

    /// Read source operand as scalar f32 (for SS scalar single ops).
    /// Register form: read lowest f32 from XMM src1.
    /// Memory form: read dword from memory, reinterpret as f32.
    #[inline]
    pub(super) fn sse_pfp_read_op2_ss(&mut self, instr: &Instruction) -> super::Result<f32> {
        if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            Ok(src.xmm32f(0))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.v_read_dword(seg, eaddr)?;
            Ok(f32::from_bits(val))
        }
    }

    /// Read source operand as scalar f64 (for SD scalar double ops).
    /// Register form: read lowest f64 from XMM src1.
    /// Memory form: read qword from memory, reinterpret as f64.
    #[inline]
    pub(super) fn sse_pfp_read_op2_sd(&mut self, instr: &Instruction) -> super::Result<f64> {
        if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            Ok(src.xmm64f(0))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.v_read_qword(seg, eaddr)?;
            Ok(f64::from_bits(val))
        }
    }

    /// Mirrors Bochs fpu/fpu_compare.cc `write_eflags_fpu_compare(int float_relation)`.
    /// - unordered (NaN): setEFlagsOSZAPC(ZF|PF|CF)
    /// - greater:         clearEFlagsOSZAPC()
    /// - less:            clearEFlagsOSZAPC(); assert_CF()
    /// - equal:           clearEFlagsOSZAPC(); assert_ZF()
    #[inline]
    fn sse_set_eflags_compare(&mut self, unordered: bool, less: bool, equal: bool) {
        if unordered {
            // Bochs: setEFlagsOSZAPC(ZFMask | PFMask | CFMask)
            // = set_oszapc with CF=1, PF=1, ZF=1, others=0
            self.set_eflags_oszapc(
                super::eflags::EFlags::ZF.bits()
                    | super::eflags::EFlags::PF.bits()
                    | super::eflags::EFlags::CF.bits(),
            );
        } else if less {
            // Bochs: clearEFlagsOSZAPC(); assert_CF()
            self.oszapc.set_oszapc_logic_32(1);
            self.oszapc.set_cf(true);
        } else if equal {
            // Bochs: clearEFlagsOSZAPC(); assert_ZF()
            self.oszapc.set_oszapc_logic_32(1);
            self.oszapc.set_zf(true);
        } else {
            // greater: clearEFlagsOSZAPC()
            self.oszapc.set_oszapc_logic_32(1);
        }
    }

    // ========================================================================
    // Arithmetic: ADDPS/PD/SS/SD
    // Bochs: ADDPS_VpsWps, ADDPD_VpdWpd, ADDSS_VssWss, ADDSD_VsdWsd
    // ========================================================================

    /// ADDPS — Add Packed Single-Precision (4 x f32)
    pub(super) fn addps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32f(i, op1.xmm32f(i) + op2.xmm32f(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ADDPD — Add Packed Double-Precision (2 x f64)
    pub(super) fn addpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm64f(i, op1.xmm64f(i) + op2.xmm64f(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ADDSS — Add Scalar Single-Precision (lowest f32 only)
    pub(super) fn addss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, result.xmm32f(0) + op2);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ADDSD — Add Scalar Double-Precision (lowest f64 only)
    pub(super) fn addsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, result.xmm64f(0) + op2);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: SUBPS/PD/SS/SD
    // Bochs: SUBPS_VpsWps, SUBPD_VpdWpd, SUBSS_VssWss, SUBSD_VsdWsd
    // ========================================================================

    /// SUBPS — Subtract Packed Single-Precision (4 x f32)
    pub(super) fn subps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32f(i, op1.xmm32f(i) - op2.xmm32f(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SUBPD — Subtract Packed Double-Precision (2 x f64)
    pub(super) fn subpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm64f(i, op1.xmm64f(i) - op2.xmm64f(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SUBSS — Subtract Scalar Single-Precision (lowest f32 only)
    pub(super) fn subss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, result.xmm32f(0) - op2);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SUBSD — Subtract Scalar Double-Precision (lowest f64 only)
    pub(super) fn subsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, result.xmm64f(0) - op2);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: MULPS/PD/SS/SD
    // Bochs: MULPS_VpsWps, MULPD_VpdWpd, MULSS_VssWss, MULSD_VsdWsd
    // ========================================================================

    /// MULPS — Multiply Packed Single-Precision (4 x f32)
    pub(super) fn mulps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32f(i, op1.xmm32f(i) * op2.xmm32f(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MULPD — Multiply Packed Double-Precision (2 x f64)
    pub(super) fn mulpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm64f(i, op1.xmm64f(i) * op2.xmm64f(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MULSS — Multiply Scalar Single-Precision (lowest f32 only)
    pub(super) fn mulss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, result.xmm32f(0) * op2);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MULSD — Multiply Scalar Double-Precision (lowest f64 only)
    pub(super) fn mulsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, result.xmm64f(0) * op2);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: DIVPS/PD/SS/SD
    // Bochs: DIVPS_VpsWps, DIVPD_VpdWpd, DIVSS_VssWss, DIVSD_VsdWsd
    // ========================================================================

    /// DIVPS — Divide Packed Single-Precision (4 x f32)
    pub(super) fn divps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32f(i, op1.xmm32f(i) / op2.xmm32f(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// DIVPD — Divide Packed Double-Precision (2 x f64)
    pub(super) fn divpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm64f(i, op1.xmm64f(i) / op2.xmm64f(i));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// DIVSS — Divide Scalar Single-Precision (lowest f32 only)
    pub(super) fn divss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, result.xmm32f(0) / op2);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// DIVSD — Divide Scalar Double-Precision (lowest f64 only)
    pub(super) fn divsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, result.xmm64f(0) / op2);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: SQRTPS/PD/SS/SD
    // Bochs: SQRTPS_VpsWps, SQRTPD_VpdWpd, SQRTSS_VssWss, SQRTSD_VsdWsd
    // ========================================================================

    /// SQRTPS — Square Root of Packed Single-Precision (4 x f32)
    pub(super) fn sqrtps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32f(i, op.xmm32f(i).sqrt());
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SQRTPD — Square Root of Packed Double-Precision (2 x f64)
    pub(super) fn sqrtpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm64f(i, op.xmm64f(i).sqrt());
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SQRTSS — Square Root of Scalar Single-Precision (lowest f32 only)
    pub(super) fn sqrtss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, op2.sqrt());
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SQRTSD — Square Root of Scalar Double-Precision (lowest f64 only)
    pub(super) fn sqrtsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, op2.sqrt());
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Rounding: ROUNDPS/PD/SS/SD (SSE4.1)
    // Bochs: ROUNDPS_VpsWpsIb, ROUNDPD_VpdWpdIb, ROUNDSS_VssWssIb,
    //        ROUNDSD_VsdWsdIb
    // imm8[1:0] = rounding mode when imm8[2] is clear; imm8[2] = use MXCSR.RC.
    // imm8[3] suppresses precision exceptions in Bochs; existing SSE FP handlers
    // do not model MXCSR sticky exception updates, so this has no extra state here.
    // ========================================================================

    pub(super) fn roundps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_xmm(instr)?;
        let imm8 = instr.ib();
        let mxcsr_rc = self.mxcsr.rounding_mode();
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32f(i, sse_round_f32(op.xmm32f(i), imm8, mxcsr_rc));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    pub(super) fn roundpd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_xmm(instr)?;
        let imm8 = instr.ib();
        let mxcsr_rc = self.mxcsr.rounding_mode();
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm64f(i, sse_round_f64(op.xmm64f(i), imm8, mxcsr_rc));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    pub(super) fn roundss_vss_wss_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let imm8 = instr.ib();
        let mxcsr_rc = self.mxcsr.rounding_mode();
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, sse_round_f32(op, imm8, mxcsr_rc));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    pub(super) fn roundsd_vsd_wsd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let imm8 = instr.ib();
        let mxcsr_rc = self.mxcsr.rounding_mode();
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, sse_round_f64(op, imm8, mxcsr_rc));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: MINPS/PD/SS/SD
    // Bochs: MINPS_VpsWps, MINPD_VpdWpd, MINSS_VssWss, MINSD_VsdWsd
    // Note: SSE MIN semantics: if either operand is NaN, return op2 (source).
    // If op2 < op1, return op2; else return op1.
    // ========================================================================

    /// MINPS — Minimum of Packed Single-Precision (4 x f32)
    pub(super) fn minps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32f(i, sse_min_f32(op1.xmm32f(i), op2.xmm32f(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MINPD — Minimum of Packed Double-Precision (2 x f64)
    pub(super) fn minpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm64f(i, sse_min_f64(op1.xmm64f(i), op2.xmm64f(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MINSS — Minimum of Scalar Single-Precision (lowest f32 only)
    pub(super) fn minss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, sse_min_f32(result.xmm32f(0), op2));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MINSD — Minimum of Scalar Double-Precision (lowest f64 only)
    pub(super) fn minsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, sse_min_f64(result.xmm64f(0), op2));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: MAXPS/PD/SS/SD
    // Bochs: MAXPS_VpsWps, MAXPD_VpdWpd, MAXSS_VssWss, MAXSD_VsdWsd
    // Note: SSE MAX semantics: if either operand is NaN, return op2 (source).
    // If op2 > op1, return op2; else return op1.
    // ========================================================================

    /// MAXPS — Maximum of Packed Single-Precision (4 x f32)
    pub(super) fn maxps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32f(i, sse_max_f32(op1.xmm32f(i), op2.xmm32f(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MAXPD — Maximum of Packed Double-Precision (2 x f64)
    pub(super) fn maxpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm64f(i, sse_max_f64(op1.xmm64f(i), op2.xmm64f(i)));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MAXSS — Maximum of Scalar Single-Precision (lowest f32 only)
    pub(super) fn maxss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, sse_max_f32(result.xmm32f(0), op2));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MAXSD — Maximum of Scalar Double-Precision (lowest f64 only)
    pub(super) fn maxsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, sse_max_f64(result.xmm64f(0), op2));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Bitwise Logical: ANDPS/ANDPD
    // Bochs: ANDPS_VpsWps, ANDPD_VpdWpd
    // ========================================================================

    /// ANDPS — Bitwise AND of Packed Single-Precision (128-bit)
    pub(super) fn andps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0) & op2.xmm64u(0));
        result.set_xmm64u(1, op1.xmm64u(1) & op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ANDPD — Bitwise AND of Packed Double-Precision (128-bit)
    pub(super) fn andpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0) & op2.xmm64u(0));
        result.set_xmm64u(1, op1.xmm64u(1) & op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Bitwise Logical: ANDNPS/ANDNPD
    // Bochs: ANDNPS_VpsWps, ANDNPD_VpdWpd
    // ========================================================================

    /// ANDNPS — Bitwise AND NOT of Packed Single-Precision (128-bit)
    /// Result = NOT(op1) AND op2
    pub(super) fn andnps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, (!op1.xmm64u(0)) & op2.xmm64u(0));
        result.set_xmm64u(1, (!op1.xmm64u(1)) & op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ANDNPD — Bitwise AND NOT of Packed Double-Precision (128-bit)
    /// Result = NOT(op1) AND op2
    pub(super) fn andnpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, (!op1.xmm64u(0)) & op2.xmm64u(0));
        result.set_xmm64u(1, (!op1.xmm64u(1)) & op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Bitwise Logical: ORPS/ORPD
    // Bochs: ORPS_VpsWps, ORPD_VpdWpd
    // ========================================================================

    /// ORPS — Bitwise OR of Packed Single-Precision (128-bit)
    pub(super) fn orps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0) | op2.xmm64u(0));
        result.set_xmm64u(1, op1.xmm64u(1) | op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ORPD — Bitwise OR of Packed Double-Precision (128-bit)
    pub(super) fn orpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0) | op2.xmm64u(0));
        result.set_xmm64u(1, op1.xmm64u(1) | op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Bitwise Logical: XORPS/XORPD
    // Bochs: XORPS_VpsWps, XORPD_VpdWpd
    // ========================================================================

    /// XORPS — Bitwise XOR of Packed Single-Precision (128-bit)
    pub(super) fn xorps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0) ^ op2.xmm64u(0));
        result.set_xmm64u(1, op1.xmm64u(1) ^ op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// XORPD — Bitwise XOR of Packed Double-Precision (128-bit)
    pub(super) fn xorpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0) ^ op2.xmm64u(0));
        result.set_xmm64u(1, op1.xmm64u(1) ^ op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Compare: CMPPS/CMPPD/CMPSS/CMPSD (8 predicates via imm8)
    // Bochs: CMPPS_VpsWpsIb, CMPPD_VpdWpdIb, CMPSS_VssWssIb, CMPSD_VsdWsdIb
    // Result: all-ones mask if true, all-zeros if false
    // ========================================================================

    /// CMPPS — Compare Packed Single-Precision (4 x f32) with imm8 predicate
    pub(super) fn cmpps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let predicate = instr.ib();
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32u(
                i,
                if sse_compare_f32(op1.xmm32f(i), op2.xmm32f(i), predicate) {
                    0xFFFF_FFFF
                } else {
                    0
                },
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CMPPD — Compare Packed Double-Precision (2 x f64) with imm8 predicate
    pub(super) fn cmppd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let predicate = instr.ib();
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm64u(
                i,
                if sse_compare_f64(op1.xmm64f(i), op2.xmm64f(i), predicate) {
                    0xFFFF_FFFF_FFFF_FFFF
                } else {
                    0
                },
            );
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CMPSS — Compare Scalar Single-Precision (lowest f32) with imm8 predicate
    pub(super) fn cmpss_vss_wss_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        let op1 = result.xmm32f(0);
        let predicate = instr.ib();
        result.set_xmm32u(
            0,
            if sse_compare_f32(op1, op2, predicate) {
                0xFFFF_FFFF
            } else {
                0
            },
        );
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CMPSD — Compare Scalar Double-Precision (lowest f64) with imm8 predicate
    pub(super) fn cmpsd_vsd_wsd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        let op1 = result.xmm64f(0);
        let predicate = instr.ib();
        result.set_xmm64u(
            0,
            if sse_compare_f64(op1, op2, predicate) {
                0xFFFF_FFFF_FFFF_FFFF
            } else {
                0
            },
        );
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Compare: COMISS/COMISD — Ordered Compare Scalar to EFLAGS
    // Bochs: COMISS_VssWss, COMISD_VsdWsd
    // Sets ZF, PF, CF; clears OF, SF, AF
    // Raises #IA for any NaN (SNaN or QNaN)
    // ========================================================================

    /// COMISS — Ordered Compare Scalar Single-Precision to EFLAGS
    pub(super) fn comiss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst()).xmm32f(0);
        let op2 = self.sse_pfp_read_op2_ss(instr)?;

        let unordered = op1.is_nan() || op2.is_nan();
        let less = !unordered && op1 < op2;
        let equal = !unordered && op1 == op2;
        self.sse_set_eflags_compare(unordered, less, equal);
        Ok(())
    }

    /// COMISD — Ordered Compare Scalar Double-Precision to EFLAGS
    pub(super) fn comisd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst()).xmm64f(0);
        let op2 = self.sse_pfp_read_op2_sd(instr)?;

        let unordered = op1.is_nan() || op2.is_nan();
        let less = !unordered && op1 < op2;
        let equal = !unordered && op1 == op2;
        self.sse_set_eflags_compare(unordered, less, equal);
        Ok(())
    }

    // ========================================================================
    // Compare: UCOMISS/UCOMISD — Unordered Compare Scalar to EFLAGS
    // Bochs: UCOMISS_VssWss, UCOMISD_VsdWsd
    // Sets ZF, PF, CF; clears OF, SF, AF
    // Same behavior as COMISS/COMISD but does not raise #IA for QNaN
    // (For our emulator, we don't raise #IA exceptions anyway)
    // ========================================================================

    /// UCOMISS — Unordered Compare Scalar Single-Precision to EFLAGS
    pub(super) fn ucomiss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst()).xmm32f(0);
        let op2 = self.sse_pfp_read_op2_ss(instr)?;

        let unordered = op1.is_nan() || op2.is_nan();
        let less = !unordered && op1 < op2;
        let equal = !unordered && op1 == op2;
        self.sse_set_eflags_compare(unordered, less, equal);
        Ok(())
    }

    /// UCOMISD — Unordered Compare Scalar Double-Precision to EFLAGS
    pub(super) fn ucomisd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst()).xmm64f(0);
        let op2 = self.sse_pfp_read_op2_sd(instr)?;

        let unordered = op1.is_nan() || op2.is_nan();
        let less = !unordered && op1 < op2;
        let equal = !unordered && op1 == op2;
        self.sse_set_eflags_compare(unordered, less, equal);
        Ok(())
    }

    // ========================================================================
    // Conversions: Int32 to Float
    // Bochs: CVTSI2SS_VssEd, CVTSI2SD_VsdEd
    // ========================================================================

    /// CVTSI2SS — Convert Int32 to Scalar Single-Precision
    pub(super) fn cvtsi2ss_vss_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src1().into()) as i32
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)? as i32
        };
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, op2 as f32);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSI2SD — Convert Int32 to Scalar Double-Precision
    pub(super) fn cvtsi2sd_vsd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src1().into()) as i32
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)? as i32
        };
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, op2 as f64);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Float to Int32
    // Bochs: CVTSS2SI_GdWss, CVTSD2SI_GdWsd, CVTTSS2SI_GdWss, CVTTSD2SI_GdWsd
    // Note: CVTSS2SI/CVTSD2SI use MXCSR rounding mode. We use native Rust
    // rounding (round-half-to-even) which matches the default MXCSR mode.
    // CVTTSS2SI/CVTTSD2SI always truncate toward zero.
    // ========================================================================

    /// CVTSS2SI — Convert Scalar Single-Precision to Int32 (MXCSR rounding)
    pub(super) fn cvtss2si_gd_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        // Round-half-to-even (default MXCSR rounding mode), then integer
        // range check — Bochs softfloat f32_to_i32.
        let result = cvt_f32_to_i32(op, false) as u32;
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    /// CVTSD2SI — Convert Scalar Double-Precision to Int32 (MXCSR rounding)
    pub(super) fn cvtsd2si_gd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let result = cvt_f64_to_i32(op, false) as u32;
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    /// CVTTSS2SI — Convert Scalar Single-Precision to Int32 (truncate)
    pub(super) fn cvttss2si_gd_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let result = cvt_f32_to_i32(op, true) as u32;
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    /// CVTTSD2SI — Convert Scalar Double-Precision to Int32 (truncate)
    pub(super) fn cvttsd2si_gd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let result = cvt_f64_to_i32(op, true) as u32;
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Int64 to Float (64-bit mode)
    // Bochs: CVTSI2SS_VssEq, CVTSI2SD_VsdEq
    // ========================================================================

    /// CVTSI2SS — Convert Int64 to Scalar Single-Precision (64-bit mode)
    pub(super) fn cvtsi2ss_vss_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src1() as usize) as i64
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)? as i64
        };
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, op2 as f32);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSI2SD — Convert Int64 to Scalar Double-Precision (64-bit mode)
    pub(super) fn cvtsi2sd_vsd_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src1() as usize) as i64
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)? as i64
        };
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, op2 as f64);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Float to Int64 (64-bit mode)
    // Bochs: CVTTSS2SI_GqWss, CVTTSD2SI_GqWsd, CVTSS2SI_GqWss, CVTSD2SI_GqWsd
    // ========================================================================

    /// CVTTSS2SI — Convert Scalar Single-Precision to Int64 (truncate, 64-bit mode)
    pub(super) fn cvttss2si_gq_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let result = cvt_f32_to_i64(op, true) as u64;
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// CVTTSD2SI — Convert Scalar Double-Precision to Int64 (truncate, 64-bit mode)
    pub(super) fn cvttsd2si_gq_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let result = cvt_f64_to_i64(op, true) as u64;
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// CVTSS2SI — Convert Scalar Single-Precision to Int64 (MXCSR rounding, 64-bit mode)
    pub(super) fn cvtss2si_gq_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let result = cvt_f32_to_i64(op, false) as u64;
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// CVTSD2SI — Convert Scalar Double-Precision to Int64 (MXCSR rounding, 64-bit mode)
    pub(super) fn cvtsd2si_gq_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let result = cvt_f64_to_i64(op, false) as u64;
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Float precision conversions
    // Bochs: CVTPS2PD, CVTPD2PS, CVTSS2SD, CVTSD2SS
    // ========================================================================

    /// CVTPS2PD — Convert 2 Packed Singles to 2 Packed Doubles
    /// Reads low 2 floats from src, converts to 2 doubles in dst
    pub(super) fn cvtps2pd_vpd_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        // Only need low 64 bits (2 x f32) from source
        let op2 = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1())
        } else {
            // Read 64 bits from memory, zero-extend to 128
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let lo = self.v_read_qword(seg, eaddr)?;
            let mut tmp = BxPackedXmmRegister::default();
            tmp.set_xmm64u(0, lo);
            tmp
        };
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64f(0, op2.xmm32f(0) as f64);
        result.set_xmm64f(1, op2.xmm32f(1) as f64);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTPD2PS — Convert 2 Packed Doubles to 2 Packed Singles
    /// Reads 2 doubles from src, converts to 2 singles in low part of dst
    pub(super) fn cvtpd2ps_vps_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm32f(0, op2.xmm64f(0) as f32);
        result.set_xmm32f(1, op2.xmm64f(1) as f32);
        // High 64 bits zeroed (from default())
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSS2SD — Convert Scalar Single to Scalar Double
    pub(super) fn cvtss2sd_vsd_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64f(0, op2 as f64);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSD2SS — Convert Scalar Double to Scalar Single
    pub(super) fn cvtsd2ss_vss_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32f(0, op2 as f32);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Packed Int32 <-> Float
    // Bochs: CVTDQ2PS, CVTPS2DQ, CVTTPS2DQ, CVTDQ2PD, CVTPD2DQ, CVTTPD2DQ
    // ========================================================================

    /// CVTDQ2PS — Convert 4 Packed Int32 to 4 Packed Singles
    pub(super) fn cvtdq2ps_vps_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32f(i, op2.xmm32s(i) as f32);
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTPS2DQ — Convert 4 Packed Singles to 4 Packed Int32 (MXCSR rounding)
    pub(super) fn cvtps2dq_vdq_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32s(i, cvt_f32_to_i32(op2.xmm32f(i), false));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTTPS2DQ — Convert 4 Packed Singles to 4 Packed Int32 (truncate)
    pub(super) fn cvttps2dq_vdq_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..4 {
            result.set_xmm32s(i, cvt_f32_to_i32(op2.xmm32f(i), true));
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTDQ2PD — Convert 2 Packed Int32 to 2 Packed Doubles
    /// Reads low 2 dwords (64 bits) from src, converts to 2 doubles in dst
    pub(super) fn cvtdq2pd_vpd_wq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1())
        } else {
            // Read 64 bits from memory
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let lo = self.v_read_qword(seg, eaddr)?;
            let mut tmp = BxPackedXmmRegister::default();
            tmp.set_xmm64u(0, lo);
            tmp
        };
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64f(0, op2.xmm32s(0) as f64);
        result.set_xmm64f(1, op2.xmm32s(1) as f64);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTPD2DQ — Convert 2 Packed Doubles to 2 Packed Int32 (MXCSR rounding)
    /// Result goes to low 64 bits of dst; high 64 bits zeroed
    pub(super) fn cvtpd2dq_vq_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm32s(i, cvt_f64_to_i32(op2.xmm64f(i), false));
        }
        // High 64 bits zeroed (from default())
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTTPD2DQ — Convert 2 Packed Doubles to 2 Packed Int32 (truncate)
    /// Result goes to low 64 bits of dst; high 64 bits zeroed
    pub(super) fn cvttpd2dq_vq_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        for i in 0..2 {
            result.set_xmm32s(i, cvt_f64_to_i32(op2.xmm64f(i), true));
        }
        // High 64 bits zeroed (from default())
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Shuffle: SHUFPS/SHUFPD
    // Bochs: SHUFPS_VpsWpsIb, SHUFPD_VpdWpdIb
    // ========================================================================

    /// SHUFPS — Shuffle Packed Single-Precision (imm8 selects lanes)
    /// Result[0] = op1[imm8[1:0]], Result[1] = op1[imm8[3:2]],
    /// Result[2] = op2[imm8[5:4]], Result[3] = op2[imm8[7:6]]
    pub(super) fn shufps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let order = instr.ib();
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm32u(0, op1.xmm32u((order & 3) as usize));
        result.set_xmm32u(1, op1.xmm32u(((order >> 2) & 3) as usize));
        result.set_xmm32u(2, op2.xmm32u(((order >> 4) & 3) as usize));
        result.set_xmm32u(3, op2.xmm32u(((order >> 6) & 3) as usize));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SHUFPD — Shuffle Packed Double-Precision (imm8 selects lanes)
    /// Result[0] = op1[imm8[0]], Result[1] = op2[imm8[1]]
    pub(super) fn shufpd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let order = instr.ib();
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u((order & 1) as usize));
        result.set_xmm64u(1, op2.xmm64u(((order >> 1) & 1) as usize));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Shuffle: UNPCKLPS/UNPCKHPS/UNPCKLPD/UNPCKHPD
    // Bochs: UNPCKLPS_VpsWps, UNPCKHPS_VpsWps, UNPCKLPD_VpdWpd, UNPCKHPD_VpdWpd
    // ========================================================================

    /// UNPCKLPS — Interleave Low Single-Precision
    /// Result = { op1[0], op2[0], op1[1], op2[1] }
    pub(super) fn unpcklps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm32u(0, op1.xmm32u(0));
        result.set_xmm32u(1, op2.xmm32u(0));
        result.set_xmm32u(2, op1.xmm32u(1));
        result.set_xmm32u(3, op2.xmm32u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// UNPCKHPS — Interleave High Single-Precision
    /// Result = { op1[2], op2[2], op1[3], op2[3] }
    pub(super) fn unpckhps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm32u(0, op1.xmm32u(2));
        result.set_xmm32u(1, op2.xmm32u(2));
        result.set_xmm32u(2, op1.xmm32u(3));
        result.set_xmm32u(3, op2.xmm32u(3));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// UNPCKLPD — Interleave Low Double-Precision
    /// Result = { op1[0], op2[0] }
    pub(super) fn unpcklpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(0));
        result.set_xmm64u(1, op2.xmm64u(0));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// UNPCKHPD — Interleave High Double-Precision
    /// Result = { op1[1], op2[1] }
    pub(super) fn unpckhpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, op1.xmm64u(1));
        result.set_xmm64u(1, op2.xmm64u(1));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // SSE3 horizontal add/sub: HADDPS/HADDPD/HSUBPS/HSUBPD
    // Bochs: HANDLE_SSE_PFP_2OP<xmm_haddps> etc. (ia_opcodes.def) via
    // simd_pfp.h xmm_haddps/xmm_haddpd/xmm_hsubps/xmm_hsubpd
    // ========================================================================

    /// HADDPS — Packed Single-FP Horizontal Add (F2 0F 7C)
    pub(super) fn haddps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        self.write_xmm_reg_lo128(instr.dst(), haddps_lane(&op1, &op2));
        Ok(())
    }

    /// HADDPD — Packed Double-FP Horizontal Add (66 0F 7C)
    pub(super) fn haddpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        self.write_xmm_reg_lo128(instr.dst(), haddpd_lane(&op1, &op2));
        Ok(())
    }

    /// HSUBPS — Packed Single-FP Horizontal Subtract (F2 0F 7D)
    pub(super) fn hsubps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        self.write_xmm_reg_lo128(instr.dst(), hsubps_lane(&op1, &op2));
        Ok(())
    }

    /// HSUBPD — Packed Double-FP Horizontal Subtract (66 0F 7D)
    pub(super) fn hsubpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        self.write_xmm_reg_lo128(instr.dst(), hsubpd_lane(&op1, &op2));
        Ok(())
    }

    // ========================================================================
    // SSE4.1 dot products: DPPS/DPPD
    // Bochs: DPPS_VpsWpsIbR / DPPD_VpdHpdWpdIbR (sse_pfp.cc)
    // ========================================================================

    /// DPPS — Dot Product of Packed Single-FP (66 0F 3A 40)
    pub(super) fn dpps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let result = dpps_lane(&op1, &op2, instr.ib());
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// DPPD — Dot Product of Packed Double-FP (66 0F 3A 41)
    pub(super) fn dppd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let result = dppd_lane(&op1, &op2, instr.ib());
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }
}
