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
//! Every floating-point result is computed by SoftFloat 3e against a status
//! word seeded from MXCSR, exactly as Bochs does. That is what makes
//! MXCSR.RC, the MXCSR sticky exception flags, #XM and DAZ/FTZ observable.
//! Bochs runs `check_exceptionsSSE` *before* writing the destination, so an
//! unmasked exception leaves the destination untouched; the `?` on
//! [`Self::check_exceptions_sse`] reproduces that.

use super::simd_pfp::{
    xmm_addps, xmm_addps_mask, xmm_addpd, xmm_addpd_mask, xmm_addsubpd, xmm_addsubps, xmm_cmppd,
    xmm_cmpps, xmm_divpd, xmm_divps, xmm_haddpd, xmm_haddps, xmm_hsubpd, xmm_hsubps, xmm_maxpd,
    xmm_maxps, xmm_minpd, xmm_minps, xmm_mulpd, xmm_mulpd_mask, xmm_mulps, xmm_mulps_mask,
    xmm_shufpd, xmm_shufps, xmm_sqrtpd, xmm_sqrtps, xmm_subpd, xmm_subps,
};
use super::softfloat3e::f32_addsub::f32_add;
use super::softfloat3e::f32_compare::{f32_compare, f32_compare_quiet, f32_max, f32_min};
use super::softfloat3e::f32_div::f32_div;
use super::softfloat3e::f32_mul::f32_mul;
use super::softfloat3e::f32_roundToInt::f32_round_to_int;
use super::softfloat3e::f32_sqrt::f32_sqrt;
use super::softfloat3e::f32_to_f64::f32_to_f64;
use super::softfloat3e::f32_to_int::{f32_to_i32, f32_to_i32_r_min_mag, f32_to_i64, f32_to_i64_r_min_mag};
use super::softfloat3e::f64_addsub::f64_add;
use super::softfloat3e::f64_compare::{f64_compare, f64_compare_quiet, f64_max, f64_min};
use super::softfloat3e::f64_div::f64_div;
use super::softfloat3e::f64_mul::f64_mul;
use super::softfloat3e::f64_roundToInt::f64_round_to_int;
use super::softfloat3e::f64_sqrt::f64_sqrt;
use super::softfloat3e::f64_to_f32::f64_to_f32;
use super::softfloat3e::f64_to_int::{f64_to_i32, f64_to_i32_r_min_mag, f64_to_i64, f64_to_i64_r_min_mag};
use super::softfloat3e::int_to_float::{i32_to_f32, i32_to_f64, i64_to_f32, i64_to_f64};
use super::softfloat3e::softfloat::{
    softfloat_getExceptionFlags, softfloat_getRoundingMode, SoftFloatStatus, FLAG_INEXACT,
};
use super::softfloat3e::softfloat_types::{float32, float64};
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    sse_fp::mxcsr_to_softfloat_status_word,
    xmm::BxPackedXmmRegister,
};

/// Bochs sse_pfp.cc `mxcsr_to_softfloat_status_word_imm_override` — the
/// SSE4.1 ROUNDxx imm8 overrides the MXCSR rounding mode unless imm8[2] is
/// set, and imm8[3] suppresses the precision exception.
#[inline]
pub(super) fn mxcsr_to_softfloat_status_word_imm_override(
    status: &mut SoftFloatStatus,
    control: u8,
) {
    if (control & 0x4) == 0 {
        status.softfloat_roundingMode = control & 0x3;
    }
    if (control & 0x8) != 0 {
        status.softfloat_suppressException |= FLAG_INEXACT;
    }
}

/// One 128-bit lane of DPPS, without the intermediate exception checks that
/// the legacy SSE handler performs. Bochs avx_pfp.cc VDPPS_VpsHpsWpsIbR
/// runs exactly this sequence per lane and checks once at the end.
#[inline]
pub(super) fn dpps_lane(
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    mask: u8,
    status: &mut SoftFloatStatus,
) -> BxPackedXmmRegister {
    // op1: [A, B, C, D]   op2: [E, F, G, H]
    let mut a = *op1;
    let mut b = *op2;
    // after multiplication: a = [AE, BF, CG, DH]
    xmm_mulps_mask(&mut a, &b, status, (mask >> 4) as u32);
    // shuffle b = [BF, AE, DH, CG]
    let a_copy = a;
    xmm_shufps(&mut b, &a_copy, &a_copy, 0xb1);
    // b = [(BF+AE), (AE+BF), (DH+CG), (CG+DH)]
    xmm_addps(&mut b, &a, status);
    // shuffle a = [(DH+CG), (CG+DH), (BF+AE), (AE+BF)]
    let b_copy = b;
    xmm_shufpd(&mut a, &b_copy, &b_copy, 0x1);
    xmm_addps_mask(&mut b, &a, status, mask as u32);
    b
}

/// One 128-bit lane of DPPD. Bochs sse_pfp.cc DPPD_VpdHpdWpdIbR.
#[inline]
pub(super) fn dppd_lane(
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    mask: u8,
    status: &mut SoftFloatStatus,
) -> BxPackedXmmRegister {
    // op1: [A, B]   op2: [C, D]
    let mut a = *op1;
    let mut b = *op2;
    // after multiplication: a = [AC, BD]
    xmm_mulpd_mask(&mut a, &b, status, (mask >> 4) as u32);
    // shuffle b = [BD, AC]
    let a_copy = a;
    xmm_shufpd(&mut b, &a_copy, &a_copy, 0x1);
    // a = [AC+BD, BD+AC]
    xmm_addpd_mask(&mut a, &b, status, mask as u32);
    a
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // SSE FP helpers: status word and source-operand reads
    // ========================================================================

    /// Seed a SoftFloat status word from the live MXCSR.
    /// Bochs `mxcsr_to_softfloat_status_word(MXCSR)`.
    #[inline]
    pub(super) fn sse_status(&self) -> SoftFloatStatus {
        mxcsr_to_softfloat_status_word(self.mxcsr)
    }

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

    /// Read source operand as a raw float32 (for SS scalar single ops).
    /// Register form: lowest dword of XMM src1. Memory form: a dword.
    #[inline]
    pub(super) fn sse_pfp_read_op2_ss(&mut self, instr: &Instruction) -> super::Result<float32> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src1()).xmm32u(0))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)
        }
    }

    /// Read source operand as a raw float64 (for SD scalar double ops).
    /// Register form: lowest qword of XMM src1. Memory form: a qword.
    #[inline]
    pub(super) fn sse_pfp_read_op2_sd(&mut self, instr: &Instruction) -> super::Result<float64> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src1()).xmm64u(0))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_qword(seg, eaddr)
        }
    }

    /// Run one packed 128-bit two-operand SSE FP op.
    /// Bochs cpu_templates_pfp.h `HANDLE_SSE_PFP_2OP`.
    #[inline]
    fn sse_pfp_2op(
        &mut self,
        instr: &Instruction,
        func: fn(&mut BxPackedXmmRegister, &BxPackedXmmRegister, &mut SoftFloatStatus),
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        func(&mut op1, &op2, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// Run one packed 128-bit single-operand SSE FP op (SQRTPS/SQRTPD).
    /// Bochs cpu_templates_pfp.h `HANDLE_SSE_PFP_1OP`.
    #[inline]
    fn sse_pfp_1op(
        &mut self,
        instr: &Instruction,
        func: fn(&mut BxPackedXmmRegister, &mut SoftFloatStatus),
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        func(&mut op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// Run one scalar single-precision SSE FP op.
    /// Bochs sse_pfp.cc `SSE_SCALAR_SINGLE_FP_CPU_LEVEL6`.
    #[inline]
    fn sse_scalar_ss(
        &mut self,
        instr: &Instruction,
        func: fn(float32, float32, &mut SoftFloatStatus) -> float32,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        let mut status = self.sse_status();
        let value = func(result.xmm32u(0), op2, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        result.set_xmm32u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// Run one scalar double-precision SSE FP op.
    /// Bochs sse_pfp.cc `SSE_SCALAR_DOUBLE_FP_CPU_LEVEL6`.
    #[inline]
    fn sse_scalar_sd(
        &mut self,
        instr: &Instruction,
        func: fn(float64, float64, &mut SoftFloatStatus) -> float64,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        let mut status = self.sse_status();
        let value = func(result.xmm64u(0), op2, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        result.set_xmm64u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// One 128-bit bitwise-logical SSE op (ANDPS/ANDNPD/ORPS/XORPD …).
    /// These touch no FP status.
    #[inline]
    fn sse_pfp_logic(
        &mut self,
        instr: &Instruction,
        func: fn(u64, u64) -> u64,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, func(op1.xmm64u(0), op2.xmm64u(0)));
        result.set_xmm64u(1, func(op1.xmm64u(1), op2.xmm64u(1)));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: ADDPS/PD/SS/SD
    // Bochs: ADDPS_VpsWps, ADDPD_VpdWpd, ADDSS_VssWss, ADDSD_VsdWsd
    // ========================================================================

    /// ADDPS — Add Packed Single-Precision (4 x f32)
    pub(super) fn addps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_addps)
    }

    /// ADDPD — Add Packed Double-Precision (2 x f64)
    pub(super) fn addpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_addpd)
    }

    /// ADDSS — Add Scalar Single-Precision (lowest f32 only)
    pub(super) fn addss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_ss(instr, f32_add)
    }

    /// ADDSD — Add Scalar Double-Precision (lowest f64 only)
    pub(super) fn addsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_sd(instr, f64_add)
    }

    // ========================================================================
    // Arithmetic: SUBPS/PD/SS/SD
    // Bochs: SUBPS_VpsWps, SUBPD_VpdWpd, SUBSS_VssWss, SUBSD_VsdWsd
    // ========================================================================

    /// SUBPS — Subtract Packed Single-Precision (4 x f32)
    pub(super) fn subps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_subps)
    }

    /// SUBPD — Subtract Packed Double-Precision (2 x f64)
    pub(super) fn subpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_subpd)
    }

    /// SUBSS — Subtract Scalar Single-Precision (lowest f32 only)
    pub(super) fn subss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_ss(instr, super::softfloat3e::f32_addsub::f32_sub)
    }

    /// SUBSD — Subtract Scalar Double-Precision (lowest f64 only)
    pub(super) fn subsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_sd(instr, super::softfloat3e::f64_addsub::f64_sub)
    }

    // ========================================================================
    // Arithmetic: MULPS/PD/SS/SD
    // Bochs: MULPS_VpsWps, MULPD_VpdWpd, MULSS_VssWss, MULSD_VsdWsd
    // ========================================================================

    /// MULPS — Multiply Packed Single-Precision (4 x f32)
    pub(super) fn mulps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_mulps)
    }

    /// MULPD — Multiply Packed Double-Precision (2 x f64)
    pub(super) fn mulpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_mulpd)
    }

    /// MULSS — Multiply Scalar Single-Precision (lowest f32 only)
    pub(super) fn mulss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_ss(instr, f32_mul)
    }

    /// MULSD — Multiply Scalar Double-Precision (lowest f64 only)
    pub(super) fn mulsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_sd(instr, f64_mul)
    }

    // ========================================================================
    // Arithmetic: DIVPS/PD/SS/SD
    // Bochs: DIVPS_VpsWps, DIVPD_VpdWpd, DIVSS_VssWss, DIVSD_VsdWsd
    // ========================================================================

    /// DIVPS — Divide Packed Single-Precision (4 x f32)
    pub(super) fn divps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_divps)
    }

    /// DIVPD — Divide Packed Double-Precision (2 x f64)
    pub(super) fn divpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_divpd)
    }

    /// DIVSS — Divide Scalar Single-Precision (lowest f32 only)
    pub(super) fn divss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_ss(instr, f32_div)
    }

    /// DIVSD — Divide Scalar Double-Precision (lowest f64 only)
    pub(super) fn divsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_sd(instr, f64_div)
    }

    // ========================================================================
    // Arithmetic: SQRTPS/PD/SS/SD
    // Bochs: SQRTPS_VpsWps, SQRTPD_VpdWpd, SQRTSS_VssWss, SQRTSD_VsdWsd
    // ========================================================================

    /// SQRTPS — Square Root of Packed Single-Precision (4 x f32)
    pub(super) fn sqrtps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_1op(instr, xmm_sqrtps)
    }

    /// SQRTPD — Square Root of Packed Double-Precision (2 x f64)
    pub(super) fn sqrtpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_1op(instr, xmm_sqrtpd)
    }

    /// SQRTSS — Square Root of Scalar Single-Precision (lowest f32 only)
    pub(super) fn sqrtss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let mut status = self.sse_status();
        let value = f32_sqrt(op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SQRTSD — Square Root of Scalar Double-Precision (lowest f64 only)
    pub(super) fn sqrtsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let mut status = self.sse_status();
        let value = f64_sqrt(op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Rounding: ROUNDPS/PD/SS/SD (SSE4.1)
    // Bochs: ROUNDPS_VpsWpsIb, ROUNDPD_VpdWpdIb, ROUNDSS_VssWssIb,
    //        ROUNDSD_VsdWsdIb
    // imm8[1:0] = rounding mode when imm8[2] is clear; imm8[2] = use MXCSR.RC;
    // imm8[3] suppresses the precision exception.
    // ========================================================================

    pub(super) fn roundps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        mxcsr_to_softfloat_status_word_imm_override(&mut status, instr.ib());
        for i in 0..4 {
            op.set_xmm32u(i, f32_round_to_int(op.xmm32u(i), &mut status));
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    pub(super) fn roundpd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        mxcsr_to_softfloat_status_word_imm_override(&mut status, instr.ib());
        for i in 0..2 {
            op.set_xmm64u(i, f64_round_to_int(op.xmm64u(i), &mut status));
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    pub(super) fn roundss_vss_wss_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let mut status = self.sse_status();
        mxcsr_to_softfloat_status_word_imm_override(&mut status, instr.ib());
        let value = f32_round_to_int(op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    pub(super) fn roundsd_vsd_wsd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let mut status = self.sse_status();
        mxcsr_to_softfloat_status_word_imm_override(&mut status, instr.ib());
        let value = f64_round_to_int(op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: MINPS/PD/SS/SD and MAXPS/PD/SS/SD
    // Bochs: MINPS_VpsWps … MAXSD_VsdWsd
    // SSE MIN/MAX return the second operand whenever the comparison is not
    // strictly less/greater, which is how NaN and ±0.0 ties resolve.
    // ========================================================================

    /// MINPS — Minimum of Packed Single-Precision (4 x f32)
    pub(super) fn minps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_minps)
    }

    /// MINPD — Minimum of Packed Double-Precision (2 x f64)
    pub(super) fn minpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_minpd)
    }

    /// MINSS — Minimum of Scalar Single-Precision (lowest f32 only)
    pub(super) fn minss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_ss(instr, f32_min)
    }

    /// MINSD — Minimum of Scalar Double-Precision (lowest f64 only)
    pub(super) fn minsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_sd(instr, f64_min)
    }

    /// MAXPS — Maximum of Packed Single-Precision (4 x f32)
    pub(super) fn maxps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_maxps)
    }

    /// MAXPD — Maximum of Packed Double-Precision (2 x f64)
    pub(super) fn maxpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_maxpd)
    }

    /// MAXSS — Maximum of Scalar Single-Precision (lowest f32 only)
    pub(super) fn maxss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_ss(instr, f32_max)
    }

    /// MAXSD — Maximum of Scalar Double-Precision (lowest f64 only)
    pub(super) fn maxsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_scalar_sd(instr, f64_max)
    }

    // ========================================================================
    // Bitwise Logical: ANDPS/ANDPD, ANDNPS/ANDNPD, ORPS/ORPD, XORPS/XORPD
    // Bochs: ANDPS_VpsWps … XORPD_VpdWpd
    // ========================================================================

    /// ANDPS — Bitwise AND of Packed Single-Precision (128-bit)
    pub(super) fn andps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_logic(instr, |a, b| a & b)
    }

    /// ANDPD — Bitwise AND of Packed Double-Precision (128-bit)
    pub(super) fn andpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_logic(instr, |a, b| a & b)
    }

    /// ANDNPS — Bitwise AND NOT of Packed Single-Precision: NOT(op1) AND op2
    pub(super) fn andnps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_logic(instr, |a, b| !a & b)
    }

    /// ANDNPD — Bitwise AND NOT of Packed Double-Precision: NOT(op1) AND op2
    pub(super) fn andnpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_logic(instr, |a, b| !a & b)
    }

    /// ORPS — Bitwise OR of Packed Single-Precision (128-bit)
    pub(super) fn orps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_logic(instr, |a, b| a | b)
    }

    /// ORPD — Bitwise OR of Packed Double-Precision (128-bit)
    pub(super) fn orpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_logic(instr, |a, b| a | b)
    }

    /// XORPS — Bitwise XOR of Packed Single-Precision (128-bit)
    pub(super) fn xorps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_logic(instr, |a, b| a ^ b)
    }

    /// XORPD — Bitwise XOR of Packed Double-Precision (128-bit)
    pub(super) fn xorpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_logic(instr, |a, b| a ^ b)
    }

    // ========================================================================
    // Compare: CMPPS/CMPPD/CMPSS/CMPSD (8 predicates via imm8)
    // Bochs: CMPPS_VpsWpsIb, CMPPD_VpdWpdIb, CMPSS_VssWssIb, CMPSD_VsdWsdIb
    // Result: all-ones mask if true, all-zeros if false. Legacy SSE encodes
    // only predicates 0..7, hence the `& 7` Bochs applies to Ib().
    // ========================================================================

    /// CMPPS — Compare Packed Single-Precision (4 x f32) with imm8 predicate
    pub(super) fn cmpps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        xmm_cmpps(&mut op1, &op2, instr.ib() & 7, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// CMPPD — Compare Packed Double-Precision (2 x f64) with imm8 predicate
    pub(super) fn cmppd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        xmm_cmppd(&mut op1, &op2, instr.ib() & 7, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }

    /// CMPSS — Compare Scalar Single-Precision (lowest f32) with imm8 predicate
    pub(super) fn cmpss_vss_wss_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        let mut status = self.sse_status();
        let hit = super::softfloat3e::softfloat_compare::f32_compare_predicate(
            instr.ib() & 7,
            result.xmm32u(0),
            op2,
            &mut status,
        );
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        result.set_xmm32u(0, if hit { 0xFFFF_FFFF } else { 0 });
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CMPSD — Compare Scalar Double-Precision (lowest f64) with imm8 predicate
    pub(super) fn cmpsd_vsd_wsd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        let mut status = self.sse_status();
        let hit = super::softfloat3e::softfloat_compare::f64_compare_predicate(
            instr.ib() & 7,
            result.xmm64u(0),
            op2,
            &mut status,
        );
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        result.set_xmm64u(0, if hit { 0xFFFF_FFFF_FFFF_FFFF } else { 0 });
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Compare: COMISS/COMISD (signalling) and UCOMISS/UCOMISD (quiet)
    // Bochs: COMISS_VssWss, COMISD_VsdWsd, UCOMISS_VssWss, UCOMISD_VsdWsd
    // Sets ZF, PF, CF; clears OF, SF, AF. COMISx raises #I for *any* NaN,
    // UCOMISx only for a signalling NaN.
    // ========================================================================

    /// COMISS — Ordered Compare Scalar Single-Precision to EFLAGS
    pub(super) fn comiss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_comis_ss(instr, f32_compare)
    }

    /// COMISD — Ordered Compare Scalar Double-Precision to EFLAGS
    pub(super) fn comisd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_comis_sd(instr, f64_compare)
    }

    /// UCOMISS — Unordered Compare Scalar Single-Precision to EFLAGS
    pub(super) fn ucomiss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_comis_ss(instr, f32_compare_quiet)
    }

    /// UCOMISD — Unordered Compare Scalar Double-Precision to EFLAGS
    pub(super) fn ucomisd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_comis_sd(instr, f64_compare_quiet)
    }

    #[inline]
    fn sse_comis_ss(
        &mut self,
        instr: &Instruction,
        compare: fn(float32, float32, &mut SoftFloatStatus) -> i32,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst()).xmm32u(0);
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = compare(op1, op2, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_eflags_fpu_compare(rc);
        Ok(())
    }

    #[inline]
    fn sse_comis_sd(
        &mut self,
        instr: &Instruction,
        compare: fn(float64, float64, &mut SoftFloatStatus) -> i32,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst()).xmm64u(0);
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = compare(op1, op2, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_eflags_fpu_compare(rc);
        Ok(())
    }

    // ========================================================================
    // Conversions: Int32 to Float
    // Bochs: CVTSI2SS_VssEd, CVTSI2SD_VsdEd
    // ========================================================================

    /// Read the 32-bit integer source of a CVTSI2xx.
    #[inline]
    fn cvtsi_read_src32(&mut self, instr: &Instruction) -> super::Result<i32> {
        if instr.mod_c0() {
            Ok(self.get_gpr32(instr.src1().into()) as i32)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            Ok(self.v_read_dword(seg, eaddr)? as i32)
        }
    }

    /// Read the 64-bit integer source of a CVTSI2xx (long mode).
    #[inline]
    fn cvtsi_read_src64(&mut self, instr: &Instruction) -> super::Result<i64> {
        if instr.mod_c0() {
            Ok(self.get_gpr64(instr.src1() as usize) as i64)
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            Ok(self.read_virtual_qword_64(seg, eaddr)? as i64)
        }
    }

    /// CVTSI2SS — Convert Int32 to Scalar Single-Precision
    pub(super) fn cvtsi2ss_vss_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.cvtsi_read_src32(instr)?;
        let mut status = self.sse_status();
        let value = i32_to_f32(op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSI2SD — Convert Int32 to Scalar Double-Precision.
    /// Exact for every i32, so Bochs performs no exception check here.
    pub(super) fn cvtsi2sd_vsd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.cvtsi_read_src32(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64u(0, i32_to_f64(op));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSI2SS — Convert Int64 to Scalar Single-Precision (64-bit mode)
    pub(super) fn cvtsi2ss_vss_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.cvtsi_read_src64(instr)?;
        let mut status = self.sse_status();
        let value = i64_to_f32(op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSI2SD — Convert Int64 to Scalar Double-Precision (64-bit mode)
    pub(super) fn cvtsi2sd_vsd_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.cvtsi_read_src64(instr)?;
        let mut status = self.sse_status();
        let value = i64_to_f64(op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Float to integer
    // Bochs: CVTSS2SI_GdWss, CVTSD2SI_GdWsd, CVTTSS2SI_GdWss, CVTTSD2SI_GdWsd
    //        and their 64-bit-mode Gq counterparts.
    // CVTxx2SI round with MXCSR.RC (or the EVEX embedded RC); CVTTxx2SI
    // always truncate toward zero.
    // ========================================================================

    /// CVTSS2SI — Convert Scalar Single-Precision to Int32 (MXCSR rounding)
    pub(super) fn cvtss2si_gd_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_getRoundingMode(&status);
        let result = f32_to_i32(op, rc, true, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.set_gpr32(instr.dst().into(), result as u32);
        Ok(())
    }

    /// CVTSD2SI — Convert Scalar Double-Precision to Int32 (MXCSR rounding)
    pub(super) fn cvtsd2si_gd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_getRoundingMode(&status);
        let result = f64_to_i32(op, rc, true, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.set_gpr32(instr.dst().into(), result as u32);
        Ok(())
    }

    /// CVTTSS2SI — Convert Scalar Single-Precision to Int32 (truncate)
    pub(super) fn cvttss2si_gd_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let result = f32_to_i32_r_min_mag(op, true, false, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.set_gpr32(instr.dst().into(), result as u32);
        Ok(())
    }

    /// CVTTSD2SI — Convert Scalar Double-Precision to Int32 (truncate)
    pub(super) fn cvttsd2si_gd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let result = f64_to_i32_r_min_mag(op, true, false, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.set_gpr32(instr.dst().into(), result as u32);
        Ok(())
    }

    /// CVTTSS2SI — Convert Scalar Single-Precision to Int64 (truncate, 64-bit mode)
    pub(super) fn cvttss2si_gq_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let result = f32_to_i64_r_min_mag(op, true, false, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.set_gpr64(instr.dst() as usize, result as u64);
        Ok(())
    }

    /// CVTTSD2SI — Convert Scalar Double-Precision to Int64 (truncate, 64-bit mode)
    pub(super) fn cvttsd2si_gq_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let result = f64_to_i64_r_min_mag(op, true, false, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.set_gpr64(instr.dst() as usize, result as u64);
        Ok(())
    }

    /// CVTSS2SI — Convert Scalar Single-Precision to Int64 (MXCSR rounding, 64-bit mode)
    pub(super) fn cvtss2si_gq_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_getRoundingMode(&status);
        let result = f32_to_i64(op, rc, true, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.set_gpr64(instr.dst() as usize, result as u64);
        Ok(())
    }

    /// CVTSD2SI — Convert Scalar Double-Precision to Int64 (MXCSR rounding, 64-bit mode)
    pub(super) fn cvtsd2si_gq_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_getRoundingMode(&status);
        let result = f64_to_i64(op, rc, true, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.set_gpr64(instr.dst() as usize, result as u64);
        Ok(())
    }

    // ========================================================================
    // Conversions: Float precision conversions
    // Bochs: CVTPS2PD, CVTPD2PS, CVTSS2SD, CVTSD2SS
    // ========================================================================

    /// Read the low 64 bits of the source as a zero-extended XMM value —
    /// the half-width source form of CVTPS2PD and CVTDQ2PD.
    #[inline]
    fn sse_pfp_read_op2_lo_qword(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src1()))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let lo = self.v_read_qword(seg, eaddr)?;
            let mut tmp = BxPackedXmmRegister::default();
            tmp.set_xmm64u(0, lo);
            Ok(tmp)
        }
    }

    /// CVTPS2PD — Convert 2 Packed Singles to 2 Packed Doubles
    pub(super) fn cvtps2pd_vpd_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_lo_qword(instr)?;
        let mut status = self.sse_status();
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, f32_to_f64(op.xmm32u(0), &mut status));
        result.set_xmm64u(1, f32_to_f64(op.xmm32u(1), &mut status));
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTPD2PS — Convert 2 Packed Doubles to 2 Packed Singles
    pub(super) fn cvtpd2ps_vps_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        let lo = f64_to_f32(op.xmm64u(0), &mut status);
        let hi = f64_to_f32(op.xmm64u(1), &mut status);
        op.set_xmm32u(0, lo);
        op.set_xmm32u(1, hi);
        op.set_xmm64u(1, 0);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// CVTSS2SD — Convert Scalar Single to Scalar Double
    pub(super) fn cvtss2sd_vsd_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let mut status = self.sse_status();
        let value = f32_to_f64(op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm64u(0, value);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSD2SS — Convert Scalar Double to Scalar Single
    pub(super) fn cvtsd2ss_vss_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let mut status = self.sse_status();
        let value = f64_to_f32(op, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let mut result = self.read_xmm_reg(instr.dst());
        result.set_xmm32u(0, value);
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
        let mut op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        for i in 0..4 {
            op.set_xmm32u(i, i32_to_f32(op.xmm32s(i), &mut status));
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// CVTPS2DQ — Convert 4 Packed Singles to 4 Packed Int32 (MXCSR rounding)
    pub(super) fn cvtps2dq_vdq_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        let rc = softfloat_getRoundingMode(&status);
        for i in 0..4 {
            op.set_xmm32s(i, f32_to_i32(op.xmm32u(i), rc, true, &mut status));
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// CVTTPS2DQ — Convert 4 Packed Singles to 4 Packed Int32 (truncate)
    pub(super) fn cvttps2dq_vdq_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        for i in 0..4 {
            op.set_xmm32s(i, f32_to_i32_r_min_mag(op.xmm32u(i), true, false, &mut status));
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// CVTDQ2PD — Convert 2 Packed Int32 to 2 Packed Doubles.
    /// Exact for every i32, so Bochs performs no exception check here.
    pub(super) fn cvtdq2pd_vpd_wq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_lo_qword(instr)?;
        let mut result = BxPackedXmmRegister::default();
        result.set_xmm64u(0, i32_to_f64(op.xmm32s(0)));
        result.set_xmm64u(1, i32_to_f64(op.xmm32s(1)));
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTPD2DQ — Convert 2 Packed Doubles to 2 Packed Int32 (MXCSR rounding).
    /// Result occupies the low 64 bits; the high 64 bits are zeroed.
    pub(super) fn cvtpd2dq_vq_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        let rc = softfloat_getRoundingMode(&status);
        let lo = f64_to_i32(op.xmm64u(0), rc, true, &mut status);
        let hi = f64_to_i32(op.xmm64u(1), rc, true, &mut status);
        op.set_xmm32s(0, lo);
        op.set_xmm32s(1, hi);
        op.set_xmm64u(1, 0);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    /// CVTTPD2DQ — Convert 2 Packed Doubles to 2 Packed Int32 (truncate).
    /// Result occupies the low 64 bits; the high 64 bits are zeroed.
    pub(super) fn cvttpd2dq_vq_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut status = self.sse_status();
        let lo = f64_to_i32_r_min_mag(op.xmm64u(0), true, false, &mut status);
        let hi = f64_to_i32_r_min_mag(op.xmm64u(1), true, false, &mut status);
        op.set_xmm32s(0, lo);
        op.set_xmm32s(1, hi);
        op.set_xmm64u(1, 0);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        self.write_xmm_reg_lo128(instr.dst(), op);
        Ok(())
    }

    // ========================================================================
    // Shuffle: SHUFPS/SHUFPD — pure data movement, no FP status
    // Bochs: SHUFPS_VpsWpsIb, SHUFPD_VpdWpdIb
    // ========================================================================

    /// SHUFPS — Shuffle Packed Single-Precision (imm8 selects lanes)
    pub(super) fn shufps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        xmm_shufps(&mut result, &op1, &op2, instr.ib());
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SHUFPD — Shuffle Packed Double-Precision (imm8 selects lanes)
    pub(super) fn shufpd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        xmm_shufpd(&mut result, &op1, &op2, instr.ib());
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Shuffle: UNPCKLPS/UNPCKHPS/UNPCKLPD/UNPCKHPD
    // Bochs: UNPCKLPS_VpsWps, UNPCKHPS_VpsWps, UNPCKLPD_VpdWpd, UNPCKHPD_VpdWpd
    // ========================================================================

    /// UNPCKLPS — Interleave Low Single-Precision: { op1[0], op2[0], op1[1], op2[1] }
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

    /// UNPCKHPS — Interleave High Single-Precision: { op1[2], op2[2], op1[3], op2[3] }
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

    /// UNPCKLPD — Interleave Low Double-Precision: { op1[0], op2[0] }
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

    /// UNPCKHPD — Interleave High Double-Precision: { op1[1], op2[1] }
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
    // SSE3 horizontal add/sub and ADDSUBPS/PD
    // Bochs: HANDLE_SSE_PFP_2OP<xmm_haddps> etc. (ia_opcodes.def)
    // ========================================================================

    /// HADDPS — Packed Single-FP Horizontal Add (F2 0F 7C)
    pub(super) fn haddps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_haddps)
    }

    /// HADDPD — Packed Double-FP Horizontal Add (66 0F 7C)
    pub(super) fn haddpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_haddpd)
    }

    /// HSUBPS — Packed Single-FP Horizontal Subtract (F2 0F 7D)
    pub(super) fn hsubps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_hsubps)
    }

    /// HSUBPD — Packed Double-FP Horizontal Subtract (66 0F 7D)
    pub(super) fn hsubpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_hsubpd)
    }

    /// ADDSUBPS — Packed Single-FP Add/Subtract (F2 0F D0)
    pub(super) fn addsubps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_addsubps)
    }

    /// ADDSUBPD — Packed Double-FP Add/Subtract (66 0F D0)
    pub(super) fn addsubpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.sse_pfp_2op(instr, xmm_addsubpd)
    }

    // ========================================================================
    // SSE4.1 dot products: DPPS/DPPD
    // Bochs: DPPS_VpsWpsIbR / DPPD_VpdHpdWpdIbR (sse_pfp.cc)
    // Unlike the VEX forms, the legacy handlers check for exceptions after
    // *each* arithmetic step, so an unmasked exception in the multiply
    // aborts before the reduction runs.
    // ========================================================================

    /// DPPS — Dot Product of Packed Single-FP (66 0F 3A 40)
    pub(super) fn dpps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let mut op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mask = instr.ib();
        let mut status = self.sse_status();

        // op1: [A, B, C, D]   op2: [E, F, G, H]
        // after multiplication: op1 = [AE, BF, CG, DH]
        xmm_mulps_mask(&mut op1, &op2, &mut status, (mask >> 4) as u32);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;

        // shuffle op2 = [BF, AE, DH, CG]
        let op1_copy = op1;
        xmm_shufps(&mut op2, &op1_copy, &op1_copy, 0xb1);

        // op2 = [(BF+AE), (AE+BF), (DH+CG), (CG+DH)]
        xmm_addps(&mut op2, &op1, &mut status);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;

        // shuffle op1 = [(DH+CG), (CG+DH), (BF+AE), (AE+BF)]
        let op2_copy = op2;
        xmm_shufpd(&mut op1, &op2_copy, &op2_copy, 0x1);

        xmm_addps_mask(&mut op2, &op1, &mut status, mask as u32);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;

        self.write_xmm_reg_lo128(instr.dst(), op2);
        Ok(())
    }

    /// DPPD — Dot Product of Packed Double-FP (66 0F 3A 41)
    pub(super) fn dppd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut op1 = self.read_xmm_reg(instr.dst());
        let mut op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mask = instr.ib();
        let mut status = self.sse_status();

        // op1: [A, B]   op2: [C, D]   after multiplication: op1 = [AC, BD]
        xmm_mulpd_mask(&mut op1, &op2, &mut status, (mask >> 4) as u32);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;

        // shuffle op2 = [BD, AC]
        let op1_copy = op1;
        xmm_shufpd(&mut op2, &op1_copy, &op1_copy, 0x1);

        // op1 = [AC+BD, BD+AC]
        xmm_addpd_mask(&mut op1, &op2, &mut status, mask as u32);
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;

        self.write_xmm_reg_lo128(instr.dst(), op1);
        Ok(())
    }
}
