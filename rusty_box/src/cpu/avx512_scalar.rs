//! AVX-512F scalar floating-point instruction handlers
//!
//! Implements EVEX-encoded scalar FP operations (VADDSS/SD, VSUBSS/SD,
//! VMULSS/SD, VDIVSS/SD, VSQRTSS/SD, VMAXSS/SD, VMINSS/SD, VMOVSS/SD).
//!
//! Scalar instructions operate on element [0] only. Upper elements come from
//! src1 (the VEX.vvvv operand). Opmask bit 0 controls merging/zeroing of
//! the scalar result element.
//!
//! Mirrors Bochs `cpu/avx/avx512_pfp.cc`.

use super::avx512_round::{f32_fixupimm, f64_fixupimm};
use super::softfloat3e::f32_addsub::{f32_add, f32_sub};
use super::softfloat3e::f32_compare::{f32_max, f32_min};
use super::softfloat3e::f32_range::{f32_get_exp, f32_get_mant, f32_range, f32_scalef};
use super::softfloat3e::f32_div::f32_div;
use super::softfloat3e::f32_mul::f32_mul;
use super::softfloat3e::f32_sqrt::f32_sqrt;
use super::softfloat3e::f64_addsub::{f64_add, f64_sub};
use super::softfloat3e::f64_compare::{f64_max, f64_min};
use super::softfloat3e::f64_range::{f64_get_exp, f64_get_mant, f64_range, f64_scalef};
use super::softfloat3e::f64_div::f64_div;
use super::softfloat3e::f64_mul::f64_mul;
use super::softfloat3e::f64_sqrt::f64_sqrt;
use super::avx512_round::{f32_reduce, f64_reduce, range_control};
use super::softfloat3e::f32_to_f64::f32_to_f64;
use super::softfloat3e::f64_to_f32::f64_to_f32;
use super::softfloat3e::int_to_float::{i32_to_f32, i32_to_f64, i64_to_f32, i64_to_f64};
use super::softfloat3e::uint64_convert::{f32_to_ui64, f32_to_ui64_r_min_mag, f64_to_ui64,
    f64_to_ui64_r_min_mag, ui64_to_f32, ui64_to_f64};
use super::softfloat3e::uint_convert::{f32_to_ui32, f32_to_ui32_r_min_mag, f64_to_ui32,
    f64_to_ui32_r_min_mag, ui32_to_f32, ui32_to_f64};
use super::softfloat3e::softfloat::softfloat_get_rounding_mode;
use super::softfloat3e::softfloat::{
    softfloat_get_exception_flags, softfloat_suppress_exception, SoftFloatStatus,
    FLAG_DENORMAL, FLAG_OVERFLOW, FLAG_UNDERFLOW,
};
use super::sse_pfp::mxcsr_to_softfloat_status_word_imm_override;
use super::softfloat3e::softfloat_types::{Float32, Float64};
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedZmmRegister,
};

/// Read opmask value for masking. k0 returns all-ones (no masking).
#[inline]
fn read_opmask_for_write<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &BxCpuC<'_, I, T>,
    instr: &Instruction,
) -> u64 {
    let k = instr.opmask();
    if k == 0 {
        u64::MAX
    } else {
        // SAFETY: opmask register union always valid for rrx (full 64-bit) access
        cpu.opmask_rrx(k as usize)
    }
}

/// Read ZMM register as a ZMM-width value.
#[inline]
fn read_zmm<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &BxCpuC<'_, I, T>,
    reg: u8,
) -> BxPackedZmmRegister {
    cpu.vmm[reg as usize]
}

/// Write scalar f32 result to dst ZMM register.
///
/// Element [0] is the result, subject to opmask bit 0 merge/zero masking.
/// Elements [1..3] come from src1. Elements [4..15] are zeroed (EVEX clears
/// upper bits).
fn write_scalar_ss<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    dst_reg: u8,
    src1: &BxPackedZmmRegister,
    result_elem0: Float32,
    mask: u64,
    zero_masking: bool,
) {
    let dst = &mut cpu.vmm[dst_reg as usize];
    // Element [0]: apply opmask bit 0
    if (mask & 1) != 0 {
        dst.set_zmm32u(0, result_elem0);
    } else if zero_masking {
        dst.set_zmm32u(0, 0);
    }
    // else: merge masking — keep original dst[0]

    // Elements [1..3] from src1
    dst.set_zmm32u(1, src1.zmm32u(1));
    dst.set_zmm32u(2, src1.zmm32u(2));
    dst.set_zmm32u(3, src1.zmm32u(3));

    // Zero upper elements [4..15] (EVEX always clears upper)
    for i in 4..16 {
        dst.set_zmm32u(i, 0);
    }
}

/// Write scalar f64 result to dst ZMM register.
///
/// Element [0] is the result, subject to opmask bit 0 merge/zero masking.
/// Element [1] comes from src1. Elements [2..7] are zeroed.
fn write_scalar_sd<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    dst_reg: u8,
    src1: &BxPackedZmmRegister,
    result_elem0: Float64,
    mask: u64,
    zero_masking: bool,
) {
    let dst = &mut cpu.vmm[dst_reg as usize];
    // Element [0]: apply opmask bit 0
    if (mask & 1) != 0 {
        dst.set_zmm64u(0, result_elem0);
    } else if zero_masking {
        dst.set_zmm64u(0, 0);
    }
    // else: merge masking — keep original dst[0]

    // Element [1] from src1
    dst.set_zmm64u(1, src1.zmm64u(1));

    // Zero upper elements [2..7]
    for i in 2..8 {
        dst.set_zmm64u(i, 0);
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // Helper: read scalar f32 source operand (register or memory)
    // ========================================================================

    /// Read scalar f32 from src2 (register) or memory.
    /// Register form: returns the low dword of src2.
    /// Memory form: reads 4 bytes from memory.
    #[inline]
    /// Read scalar f32 from rm operand (src1 in our convention).
    /// Register form: XMM element [0] of src1 (rm).
    /// Memory form: reads 4 bytes from memory.
    fn evex_read_rm_ss(&mut self, instr: &Instruction) -> super::Result<Float32> {
        if instr.mod_c0() {
            Ok(self.vmm[instr.src1() as usize].zmm32u(0))
        } else {
            // Callers pair LOAD_Wss with LOAD_MASK_Wss, so a masked-off scalar
            // element must read as zero without touching memory.
            Ok(self.evex_load_wss_pair(instr)?.zmm32u(0))
        }
    }

    /// Read scalar f64 from src2 (register) or memory.
    /// Register form: returns the low qword of src2.
    /// Memory form: reads 8 bytes from memory.
    #[inline]
    /// Read scalar f64 from rm operand (src1 in our convention).
    fn evex_read_rm_sd(&mut self, instr: &Instruction) -> super::Result<Float64> {
        if instr.mod_c0() {
            Ok(self.vmm[instr.src1() as usize].zmm64u(0))
        } else {
            // Callers pair LOAD_Wsd with LOAD_MASK_Wsd.
            Ok(self.evex_load_wsd_pair(instr)?.zmm64u(0))
        }
    }

    // ========================================================================
    // Scalar EVEX FP arithmetic — Bochs avx512_pfp.cc
    // `AVX512_SCALAR_SINGLE_FP_MASK` / `AVX512_SCALAR_DOUBLE_FP_MASK`.
    //
    // When opmask bit 0 is clear the operation is not performed at all, so
    // it raises no exception; the destination element is then either zeroed
    // or merged. Otherwise the result goes through SoftFloat with embedded
    // rounding control applied, and check_exceptionsSSE runs before the
    // write.
    // ========================================================================

    /// The shared body of the single-precision scalar arithmetic handlers.
    fn evex_scalar_ss(
        &mut self,
        instr: &Instruction,
        func: impl Fn(Float32, Float32, &mut SoftFloatStatus) -> Float32,
    ) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_ss(instr)?;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        let mut result = 0;
        if (mask & 1) != 0 {
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            result = func(src1.zmm32u(0), src2_val, &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        }
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// The shared body of the double-precision scalar arithmetic handlers.
    fn evex_scalar_sd(
        &mut self,
        instr: &Instruction,
        func: impl Fn(Float64, Float64, &mut SoftFloatStatus) -> Float64,
    ) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_sd(instr)?;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        let mut result = 0;
        if (mask & 1) != 0 {
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            result = func(src1.zmm64u(0), src2_val, &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        }
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VADDSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vaddss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_ss(instr, f32_add)
    }

    /// VADDSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vaddsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_sd(instr, f64_add)
    }

    /// VSUBSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vsubss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_ss(instr, f32_sub)
    }

    /// VSUBSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vsubsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_sd(instr, f64_sub)
    }

    /// VMULSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vmulss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_ss(instr, f32_mul)
    }

    /// VMULSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vmulsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_sd(instr, f64_mul)
    }

    /// VDIVSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vdivss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_ss(instr, f32_div)
    }

    /// VDIVSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vdivsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_sd(instr, f64_div)
    }

    /// VSQRTSS xmm1{k1}{z}, xmm2, xmm3/m32 — the vvvv operand supplies only
    /// the upper elements, so the first argument is discarded.
    pub fn evex_vsqrtss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_ss(instr, |_, b, status| f32_sqrt(b, status))
    }

    /// VSQRTSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vsqrtsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_sd(instr, |_, b, status| f64_sqrt(b, status))
    }

    /// VMAXSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vmaxss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_ss(instr, f32_max)
    }

    /// VMAXSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vmaxsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_sd(instr, f64_max)
    }

    /// VMINSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vminss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_ss(instr, f32_min)
    }

    /// VMINSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vminsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_sd(instr, f64_min)
    }

    // ========================================================================
    // VMOVSS — Move Scalar Single-Precision
    // EVEX.LIG.F3.0F.W0 10 (load) / EVEX.LIG.F3.0F.W0 11 (store)
    //
    // Memory form load: dst[0] = mem32, dst[1..15] = 0
    // Register form load: dst[0] = src2[0], dst[1..3] = src1[1..3], dst[4..15] = 0
    // Memory form store: mem32 = src[0]
    // Register form store: dst[0] = src[0], dst[1..3] = src1[1..3], dst[4..15] = 0
    // ========================================================================

    // ========================================================================
    // Scalar exponent / mantissa / scale — VGETEXPSS/SD, VSCALEFSS/SD,
    // VGETMANTSS/SD. Same masked scalar shape as the arithmetic above, so
    // they reuse its body; only the element function differs.
    //
    // VGETEXP and VGETMANT take their value from the rm operand alone — the
    // vvvv operand supplies only the upper elements — so their closures
    // discard the first argument the way VSQRTSS does.
    // ========================================================================

    /// VGETEXPSS xmm1{k1}{z}, xmm2, xmm3/m32 — EVEX.66.0F38.W0 43
    pub fn evex_vgetexpss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_ss(instr, |_, b, status| f32_get_exp(b, status))
    }

    /// VGETEXPSD xmm1{k1}{z}, xmm2, xmm3/m64 — EVEX.66.0F38.W1 43
    pub fn evex_vgetexpsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_sd(instr, |_, b, status| f64_get_exp(b, status))
    }

    /// VSCALEFSS xmm1{k1}{z}, xmm2, xmm3/m32 — EVEX.66.0F38.W0 2D
    pub fn evex_vscalefss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_ss(instr, f32_scalef)
    }

    /// VSCALEFSD xmm1{k1}{z}, xmm2, xmm3/m64 — EVEX.66.0F38.W1 2D
    pub fn evex_vscalefsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_scalar_sd(instr, f64_scalef)
    }

    /// VGETMANTSS xmm1{k1}{z}, xmm2, xmm3/m32, Ib — EVEX.66.0F3A.W0 27
    /// imm8[1:0] selects the mantissa interval, imm8[3:2] the sign control.
    pub fn evex_vgetmantss(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm8 = instr.ib();
        let (sign_ctrl, interv) = (((imm8 >> 2) & 0x3) as i32, (imm8 & 0x3) as i32);
        self.evex_scalar_ss(instr, move |_, b, status| {
            f32_get_mant(b, status, sign_ctrl, interv)
        })
    }

    /// VGETMANTSD xmm1{k1}{z}, xmm2, xmm3/m64, Ib — EVEX.66.0F3A.W1 27
    pub fn evex_vgetmantsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm8 = instr.ib();
        let (sign_ctrl, interv) = (((imm8 >> 2) & 0x3) as i32, (imm8 & 0x3) as i32);
        self.evex_scalar_sd(instr, move |_, b, status| {
            f64_get_mant(b, status, sign_ctrl, interv)
        })
    }


    // ========================================================================
    // VREDUCE / VRANGE scalar forms. Bochs avx512_pfp.cc
    // VREDUCESS/SD_MASK_* and VRANGESS/SD_MASK_*.
    //
    // VREDUCE reduces the *rm* operand and takes only the upper elements from
    // vvvv, the way VSQRTSS does; VRANGE uses both.
    // ========================================================================

    /// VREDUCESS xmm1{k1}{z}, xmm2, xmm3/m32, Ib — EVEX.66.0F3A.W0 57
    pub fn evex_vreducess(&mut self, instr: &Instruction) -> super::Result<()> {
        let control = instr.ib();
        self.evex_scalar_ss(instr, move |_, b, status| {
            mxcsr_to_softfloat_status_word_imm_override(status, control);
            softfloat_suppress_exception(status, FLAG_DENORMAL | FLAG_UNDERFLOW | FLAG_OVERFLOW);
            f32_reduce(b, control >> 4, status)
        })
    }

    /// VREDUCESD xmm1{k1}{z}, xmm2, xmm3/m64, Ib — EVEX.66.0F3A.W1 57
    pub fn evex_vreducesd(&mut self, instr: &Instruction) -> super::Result<()> {
        let control = instr.ib();
        self.evex_scalar_sd(instr, move |_, b, status| {
            mxcsr_to_softfloat_status_word_imm_override(status, control);
            softfloat_suppress_exception(status, FLAG_DENORMAL | FLAG_UNDERFLOW | FLAG_OVERFLOW);
            f64_reduce(b, control >> 4, status)
        })
    }

    /// VRANGESS xmm1{k1}{z}, xmm2, xmm3/m32, Ib — EVEX.66.0F3A.W0 51
    pub fn evex_vrangess(&mut self, instr: &Instruction) -> super::Result<()> {
        let (is_max, is_abs, sign_ctrl) = range_control(instr.ib());
        self.evex_scalar_ss(instr, move |a, b, status| {
            f32_range(a, b, is_max, is_abs, sign_ctrl, status)
        })
    }

    /// VRANGESD xmm1{k1}{z}, xmm2, xmm3/m64, Ib — EVEX.66.0F3A.W1 51
    pub fn evex_vrangesd(&mut self, instr: &Instruction) -> super::Result<()> {
        let (is_max, is_abs, sign_ctrl) = range_control(instr.ib());
        self.evex_scalar_sd(instr, move |a, b, status| {
            f64_range(a, b, is_max, is_abs, sign_ctrl, status)
        })
    }


    // ========================================================================
    // Scalar conversions between a GPR and a scalar float, and between the two
    // float widths. Bochs avx512_cvt.cc and avx_cvt.cc.
    //
    // The float -> GPR direction has no vvvv operand, so upstream's EVEX def
    // entries name the legacy handlers and this file only adds the unsigned
    // forms. The GPR -> float direction does have one: the destination's upper
    // elements come from vvvv, which the legacy handler cannot express because
    // it writes the low element of the destination in place.
    // ========================================================================

    /// VCVTSS2USI / VCVTTSS2USI — scalar single to unsigned GPR.
    /// `wide` selects the 64-bit destination, `truncate` round-toward-zero.
    fn evex_cvt_ss2usi(
        &mut self,
        instr: &Instruction,
        wide: bool,
        truncate: bool,
    ) -> super::Result<()> {
        let op = self.evex_read_rm_ss(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_get_rounding_mode(&status);
        if wide {
            let r = if truncate {
                f32_to_ui64_r_min_mag(op, true, false, &mut status)
            } else {
                f32_to_ui64(op, rc, true, &mut status)
            };
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.set_gpr64(instr.dst() as usize, r);
        } else {
            let r = if truncate {
                f32_to_ui32_r_min_mag(op, true, false, &mut status)
            } else {
                f32_to_ui32(op, rc, true, &mut status)
            };
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.set_gpr32(instr.dst().into(), r);
        }
        Ok(())
    }

    /// Double-precision counterpart of [`Self::evex_cvt_ss2usi`].
    fn evex_cvt_sd2usi(
        &mut self,
        instr: &Instruction,
        wide: bool,
        truncate: bool,
    ) -> super::Result<()> {
        let op = self.evex_read_rm_sd(instr)?;
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_get_rounding_mode(&status);
        if wide {
            let r = if truncate {
                f64_to_ui64_r_min_mag(op, true, false, &mut status)
            } else {
                f64_to_ui64(op, rc, true, &mut status)
            };
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.set_gpr64(instr.dst() as usize, r);
        } else {
            let r = if truncate {
                f64_to_ui32_r_min_mag(op, true, false, &mut status)
            } else {
                f64_to_ui32(op, rc, true, &mut status)
            };
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.set_gpr32(instr.dst().into(), r);
        }
        Ok(())
    }

    /// VCVTSS2USI Gd, Wss — EVEX.F3.0F.W0 79
    pub fn evex_vcvtss2usi_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_ss2usi(instr, false, false)
    }
    /// VCVTSS2USI Gq, Wss — EVEX.F3.0F.W1 79
    pub fn evex_vcvtss2usi_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_ss2usi(instr, true, false)
    }
    /// VCVTTSS2USI Gd, Wss — EVEX.F3.0F.W0 78
    pub fn evex_vcvttss2usi_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_ss2usi(instr, false, true)
    }
    /// VCVTTSS2USI Gq, Wss — EVEX.F3.0F.W1 78
    pub fn evex_vcvttss2usi_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_ss2usi(instr, true, true)
    }
    /// VCVTSD2USI Gd, Wsd — EVEX.F2.0F.W0 79
    pub fn evex_vcvtsd2usi_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_sd2usi(instr, false, false)
    }
    /// VCVTSD2USI Gq, Wsd — EVEX.F2.0F.W1 79
    pub fn evex_vcvtsd2usi_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_sd2usi(instr, true, false)
    }
    /// VCVTTSD2USI Gd, Wsd — EVEX.F2.0F.W0 78
    pub fn evex_vcvttsd2usi_gd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_sd2usi(instr, false, true)
    }
    /// VCVTTSD2USI Gq, Wsd — EVEX.F2.0F.W1 78
    pub fn evex_vcvttsd2usi_gq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_sd2usi(instr, true, true)
    }

    /// VCVTSI2SS xmm1, xmm2, r/m32 — EVEX.F3.0F.W0 2A
    pub fn evex_vcvtsi2ss_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op = self.cvtsi_read_src32(instr)?;
        let src1 = read_zmm(self, instr.src2()); // vvvv supplies the upper elements
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = i32_to_f32(op, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        write_scalar_ss(self, instr.dst(), &src1, value, u64::MAX, false);
        Ok(())
    }

    /// VCVTSI2SS xmm1, xmm2, r/m64 — EVEX.F3.0F.W1 2A
    pub fn evex_vcvtsi2ss_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op = self.cvtsi_read_src64(instr)?;
        let src1 = read_zmm(self, instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = i64_to_f32(op, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        write_scalar_ss(self, instr.dst(), &src1, value, u64::MAX, false);
        Ok(())
    }

    /// VCVTSI2SD xmm1, xmm2, r/m32 — EVEX.F2.0F.W0 2A. Exact for every i32,
    /// so upstream raises nothing and uses no status word.
    pub fn evex_vcvtsi2sd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op = self.cvtsi_read_src32(instr)?;
        let src1 = read_zmm(self, instr.src2());
        write_scalar_sd(self, instr.dst(), &src1, i32_to_f64(op), u64::MAX, false);
        Ok(())
    }

    /// VCVTSI2SD xmm1, xmm2, r/m64 — EVEX.F2.0F.W1 2A
    pub fn evex_vcvtsi2sd_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op = self.cvtsi_read_src64(instr)?;
        let src1 = read_zmm(self, instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = i64_to_f64(op, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        write_scalar_sd(self, instr.dst(), &src1, value, u64::MAX, false);
        Ok(())
    }

    /// VCVTUSI2SS xmm1, xmm2, r/m32 — EVEX.F3.0F.W0 7B
    pub fn evex_vcvtusi2ss_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op = self.cvtsi_read_src32(instr)? as u32;
        let src1 = read_zmm(self, instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = ui32_to_f32(op, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        write_scalar_ss(self, instr.dst(), &src1, value, u64::MAX, false);
        Ok(())
    }

    /// VCVTUSI2SS xmm1, xmm2, r/m64 — EVEX.F3.0F.W1 7B
    pub fn evex_vcvtusi2ss_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op = self.cvtsi_read_src64(instr)? as u64;
        let src1 = read_zmm(self, instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = ui64_to_f32(op, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        write_scalar_ss(self, instr.dst(), &src1, value, u64::MAX, false);
        Ok(())
    }

    /// VCVTUSI2SD xmm1, xmm2, r/m32 — EVEX.F2.0F.W0 7B. Exact for every u32.
    pub fn evex_vcvtusi2sd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op = self.cvtsi_read_src32(instr)? as u32;
        let src1 = read_zmm(self, instr.src2());
        write_scalar_sd(self, instr.dst(), &src1, ui32_to_f64(op), u64::MAX, false);
        Ok(())
    }

    /// VCVTUSI2SD xmm1, xmm2, r/m64 — EVEX.F2.0F.W1 7B
    pub fn evex_vcvtusi2sd_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op = self.cvtsi_read_src64(instr)? as u64;
        let src1 = read_zmm(self, instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = ui64_to_f64(op, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        write_scalar_sd(self, instr.dst(), &src1, value, u64::MAX, false);
        Ok(())
    }

    /// VCVTSD2SS xmm1{k1}{z}, xmm2, xmm3/m64 — EVEX.F2.0F.W1 5A.
    /// Narrowing, so the destination is a single and the upper *dwords* come
    /// from vvvv.
    pub fn evex_vcvtsd2ss(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        let mut result = 0;
        if (mask & 1) != 0 {
            let op2 = self.evex_read_rm_sd(instr)?;
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            result = f64_to_f32(op2, &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        }
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VCVTSS2SD xmm1{k1}{z}, xmm2, xmm3/m32 — EVEX.F3.0F.W0 5A
    pub fn evex_vcvtss2sd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        let mut result = 0;
        if (mask & 1) != 0 {
            let op2 = self.evex_read_rm_ss(instr)?;
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            result = f32_to_f64(op2, &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        }
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VMOVSS xmm1{k1}{z}, xmm2, xmm3 (register form load)
    /// VMOVSS xmm1{k1}{z}, m32 (memory form load)
    pub fn evex_vmovss_load(&mut self, instr: &Instruction) -> super::Result<()> {
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        if instr.mod_c0() {
            // Register form: element 0 comes from src2, the upper elements from
            // src1 (VEX.vvvv). Bochs avx512_move.cc VMOVSS_MASK_VssHpsWssR
            // reads `src1()` for the base and `src2()` only for element 0.
            let src1 = read_zmm(self, instr.src2());
            let src2 = read_zmm(self, instr.src1());
            let val = src2.zmm32u(0);
            write_scalar_ss(self, instr.dst(), &src1, val, mask, zmask);
        } else {
            // Memory form: dst[0] = mem32, rest zeroed. Bochs
            // VMOVSS_MASK_VssWssM guards the read on BX_SCALAR_ELEMENT_MASK, so
            // a masked-off scalar performs no access and cannot fault.
            let val = if (mask & 1) != 0 {
                let laddr = self.resolve_addr(instr);
                let seg = BxSegregs::from(instr.seg());
                self.v_read_dword(seg, laddr)?
            } else {
                0
            };

            let dst = &mut self.vmm[instr.dst() as usize];
            // Element [0]: apply opmask bit 0
            if (mask & 1) != 0 {
                dst.set_zmm32u(0, val);
            } else if zmask {
                dst.set_zmm32u(0, 0);
            }
            // Memory form: all other elements zeroed
            for i in 1..16 {
                dst.set_zmm32u(i, 0);
            }
        }
        Ok(())
    }

    /// VMOVSD xmm1{k1}{z}, xmm2, xmm3 (register form load)
    /// VMOVSD xmm1{k1}{z}, m64 (memory form load)
    pub fn evex_vmovsd_load(&mut self, instr: &Instruction) -> super::Result<()> {
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        if instr.mod_c0() {
            // Register form: element 0 from src2, element 1 from src1
            // (VEX.vvvv). Bochs avx512_move.cc VMOVSD_MASK_VsdHpdWsdR.
            let src1 = read_zmm(self, instr.src2());
            let src2 = read_zmm(self, instr.src1());
            let val = src2.zmm64u(0);
            write_scalar_sd(self, instr.dst(), &src1, val, mask, zmask);
        } else {
            // Memory form: dst[0] = mem64, rest zeroed; no access when the
            // scalar element is masked off (Bochs VMOVSD_MASK_VsdWsdM).
            let val = if (mask & 1) != 0 {
                let laddr = self.resolve_addr(instr);
                let seg = BxSegregs::from(instr.seg());
                self.v_read_qword(seg, laddr)?
            } else {
                0
            };

            let dst = &mut self.vmm[instr.dst() as usize];
            // Element [0]: apply opmask bit 0
            if (mask & 1) != 0 {
                dst.set_zmm64u(0, val);
            } else if zmask {
                dst.set_zmm64u(0, 0);
            }
            // Memory form: all other elements zeroed
            for i in 1..8 {
                dst.set_zmm64u(i, 0);
            }
        }
        Ok(())
    }

    /// VMOVSS xmm1/m32{k1}, xmm2 (register form store)
    /// VMOVSS m32{k1}, xmm1 (memory form store)
    pub fn evex_vmovss_store(&mut self, instr: &Instruction) -> super::Result<()> {
        let mask = read_opmask_for_write(self, instr);

        if instr.mod_c0() {
            // Register form store: dst[0] = src[0], dst[1..3] = src1[1..3], zero upper
            let src = read_zmm(self, instr.src());
            let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
            let zmask = instr.is_zero_masking() != 0;
            let val = src.zmm32u(0);
            write_scalar_ss(self, instr.dst(), &src1, val, mask, zmask);
        } else {
            // Memory form store: write element [0] to memory
            if (mask & 1) != 0 {
                let src = read_zmm(self, instr.src());
                let laddr = self.resolve_addr(instr);
                let seg = BxSegregs::from(instr.seg());
                let bits = src.zmm32u(0);
                self.v_write_dword(seg, laddr, bits)?;
            }
        }
        Ok(())
    }

    /// VMOVSD xmm1/m64{k1}, xmm2 (register form store)
    /// VMOVSD m64{k1}, xmm1 (memory form store)
    pub fn evex_vmovsd_store(&mut self, instr: &Instruction) -> super::Result<()> {
        let mask = read_opmask_for_write(self, instr);

        if instr.mod_c0() {
            // Register form store: dst[0] = src[0], dst[1] = src1[1], zero upper
            let src = read_zmm(self, instr.src());
            let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
            let zmask = instr.is_zero_masking() != 0;
            let val = src.zmm64u(0);
            write_scalar_sd(self, instr.dst(), &src1, val, mask, zmask);
        } else {
            // Memory form store: write element [0] to memory
            if (mask & 1) != 0 {
                let src = read_zmm(self, instr.src());
                let laddr = self.resolve_addr(instr);
                let seg = BxSegregs::from(instr.seg());
                let val = src.zmm64u(0);
                let lo = val as u32;
                let hi = (val >> 32) as u32;
                self.v_write_dword(seg, laddr, lo)?;
                self.v_write_dword(seg, laddr + 4, hi)?;
            }
        }
        Ok(())
    }

    /// VFIXUPIMMSS xmm1{k1}{z}, xmm2, xmm3/m32, Ib — EVEX.66.0F3A.W0 55.
    /// The upper elements come from vvvv; the destination supplies only the
    /// per-element fallback value.
    pub fn evex_vfixupimmss(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv
        let dst_elem = read_zmm(self, instr.dst()).zmm32u(0);
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        let mut result = 0;
        if (mask & 1) != 0 {
            let op2 = self.evex_read_rm_ss(instr)?;
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            result = f32_fixupimm(dst_elem, src1.zmm32u(0), op2, instr.ib(), &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        }
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VFIXUPIMMSD xmm1{k1}{z}, xmm2, xmm3/m64, Ib — EVEX.66.0F3A.W1 55
    pub fn evex_vfixupimmsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2());
        let dst_elem = read_zmm(self, instr.dst()).zmm64u(0);
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        let mut result = 0;
        if (mask & 1) != 0 {
            let op2 = self.evex_read_rm_sd(instr)?;
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            result = f64_fixupimm(
                dst_elem,
                src1.zmm64u(0),
                op2 as u32,
                instr.ib(),
                &mut status,
            );
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        }
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    //! VGETEXP, VSCALEF and VGETMANT decompose and reassemble a float, so
    //! each is checked against the identity it is supposed to satisfy rather
    //! than against a hand-computed constant: getexp gives the unbiased
    //! exponent, scalef multiplies by a power of two, and getmant returns
    //! the significand normalised into the interval imm8[1:0] selects.

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::{BxSegregs, Instruction};
    use crate::cpu::xmm::MXCSR_RESET;
    use rusty_box_decoder::opcode::Opcode;

    /// Register-form EVEX scalar: dst=0, rm=1 (the value), vvvv=2 (upper
    /// elements). k0, so element 0 is always active.
    fn evex_scalar(opcode: Opcode) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0);
        i.set_src_reg(1, 1);
        i.set_src_reg(2, 2);
        i.set_opmask(0);
        i.set_vex(true);
        i.set_vl(0);
        i.set_seg(BxSegregs::Ds);
        i.init(0, 0, 1, 1);
        i.assert_mod_c0();
        i
    }

    #[test]
    fn vgetexp_returns_the_unbiased_exponent() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.mxcsr.mxcsr = MXCSR_RESET;
        for (v, want) in [(8.0f32, 3.0f32), (1.0, 0.0), (0.5, -1.0), (12.0, 3.0)] {
            cpu.vmm[1].set_zmm32u(0, v.to_bits());
            cpu.execute_instruction(&evex_scalar(Opcode::EvexVgetexpssVssHpsWss))
                .unwrap();
            assert_eq!(
                cpu.vmm[0].zmm32u(0),
                want.to_bits(),
                "getexp({v}) should be {want}"
            );
        }
        // Zero has no exponent: the architecture returns -inf.
        cpu.vmm[1].set_zmm32u(0, 0.0f32.to_bits());
        cpu.execute_instruction(&evex_scalar(Opcode::EvexVgetexpssVssHpsWss))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), f32::NEG_INFINITY.to_bits());
    }

    #[test]
    fn vscalef_multiplies_by_two_to_the_truncated_exponent() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.mxcsr.mxcsr = MXCSR_RESET;
        // vvvv holds the value, rm the exponent: 3.0 * 2^2 = 12.0.
        cpu.vmm[2].set_zmm32u(0, 3.0f32.to_bits());
        cpu.vmm[1].set_zmm32u(0, 2.0f32.to_bits());
        cpu.execute_instruction(&evex_scalar(Opcode::EvexVscalefssVssHpsWss))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 12.0f32.to_bits());

        // A negative exponent scales down, and the exponent is truncated
        // toward zero, so 2.7 behaves as 2.
        cpu.vmm[2].set_zmm32u(0, 8.0f32.to_bits());
        cpu.vmm[1].set_zmm32u(0, (-1.0f32).to_bits());
        cpu.execute_instruction(&evex_scalar(Opcode::EvexVscalefssVssHpsWss))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 4.0f32.to_bits());

        cpu.vmm[2].set_zmm32u(0, 3.0f32.to_bits());
        cpu.vmm[1].set_zmm32u(0, 2.7f32.to_bits());
        cpu.execute_instruction(&evex_scalar(Opcode::EvexVscalefssVssHpsWss))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 12.0f32.to_bits());
    }

    #[test]
    fn vgetmant_normalises_into_the_interval_the_immediate_selects() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.mxcsr.mxcsr = MXCSR_RESET;
        // 12.0 = 1.5 * 2^3, so the significand is 1.5.
        cpu.vmm[1].set_zmm32u(0, 12.0f32.to_bits());

        let mut i = evex_scalar(Opcode::EvexVgetmantssVssHpsWssIbKmask);
        i.set_iq(0); // interval [1,2)
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 1.5f32.to_bits());

        let mut i = evex_scalar(Opcode::EvexVgetmantssVssHpsWssIbKmask);
        i.set_iq(2); // interval [1/2,1) — halves the same significand
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 0.75f32.to_bits());

        // sign_ctrl imm8[3] set makes a negative input invalid -> QNaN.
        cpu.vmm[1].set_zmm32u(0, (-12.0f32).to_bits());
        let mut i = evex_scalar(Opcode::EvexVgetmantssVssHpsWssIbKmask);
        i.set_iq(0b1000); // sign_ctrl = 0b10
        cpu.execute_instruction(&i).unwrap();
        assert!(f32::from_bits(cpu.vmm[0].zmm32u(0)).is_nan());
    }

    // ---- scalar GPR <-> float conversions -------------------------------

    #[test]
    fn unsigned_destination_conversions_differ_from_the_signed_ones() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.mxcsr.mxcsr = MXCSR_RESET;
        // The signed float -> GPR forms route to the legacy handlers, which
        // begin with prepare_sse(); a builder-made CPU has CR4.OSFXSR clear,
        // so without this they raise #UD before converting anything.
        c.cr4.insert(crate::cpu::crregs::BxCr4::OSFXSR);

        // 3e9 exceeds i32::MAX but fits a u32, so the signed form saturates to
        // the integer indefinite value while the unsigned one converts.
        c.vmm[1].set_zmm32u(0, 3.0e9f32.to_bits());
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvttss2usiGdWss))
            .unwrap();
        assert_eq!(c.get_gpr32(0), 3_000_000_000);
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvttss2siGdWss))
            .unwrap();
        assert_eq!(c.get_gpr32(0), 0x8000_0000, "signed form is out of range");

        // A negative value has no unsigned representation at all.
        c.vmm[1].set_zmm32u(0, (-1.0f32).to_bits());
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvttss2usiGdWss))
            .unwrap();
        assert_eq!(c.get_gpr32(0), 0xFFFF_FFFF);

        // …but a negative fraction truncates to zero first, which is legal.
        c.vmm[1].set_zmm32u(0, (-0.5f32).to_bits());
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvttss2usiGdWss))
            .unwrap();
        assert_eq!(c.get_gpr32(0), 0);
    }

    #[test]
    fn scalar_float_to_gpr_rounds_or_truncates_by_opcode() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.mxcsr.mxcsr = MXCSR_RESET;
        c.vmm[1].set_zmm64u(0, 2.5f64.to_bits());
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvtsd2usiGdWsd))
            .unwrap();
        assert_eq!(c.get_gpr32(0), 2, "round-to-nearest-even");
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvttsd2usiGdWsd))
            .unwrap();
        assert_eq!(c.get_gpr32(0), 2, "truncation agrees here");

        c.vmm[1].set_zmm64u(0, 3.5f64.to_bits());
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvtsd2usiGdWsd))
            .unwrap();
        assert_eq!(c.get_gpr32(0), 4, "ties to even rounds up");
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvttsd2usiGdWsd))
            .unwrap();
        assert_eq!(c.get_gpr32(0), 3, "truncation does not");

        // 64-bit destination.
        c.vmm[1].set_zmm64u(0, 1.0e19f64.to_bits());
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvttsd2usiGqWsd))
            .unwrap();
        assert_eq!(c.get_gpr64(0), 10_000_000_000_000_000_000);
    }

    #[test]
    fn gpr_to_scalar_float_takes_its_upper_elements_from_vvvv() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.mxcsr.mxcsr = MXCSR_RESET;
        // vvvv (= vmm[2]) supplies dwords 1..3; the destination's own previous
        // contents must not survive.
        c.vmm[0].set_zmm32u(1, 0xDEAD_BEEF);
        c.vmm[2].set_zmm32u(1, 0x1111_1111);
        c.vmm[2].set_zmm32u(2, 0x2222_2222);
        c.vmm[2].set_zmm32u(3, 0x3333_3333);
        c.set_gpr32(1, 7); // the r/m operand is src1() = register 1

        c.execute_instruction(&evex_scalar(Opcode::EvexVcvtsi2ssVssEd))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 7.0f32.to_bits());
        assert_eq!(c.vmm[0].zmm32u(1), 0x1111_1111);
        assert_eq!(c.vmm[0].zmm32u(2), 0x2222_2222);
        assert_eq!(c.vmm[0].zmm32u(3), 0x3333_3333);
        assert_eq!(c.vmm[0].zmm32u(4), 0, "EVEX clears above 128 bits");
    }

    #[test]
    fn usi_to_scalar_float_reads_the_gpr_as_unsigned() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.mxcsr.mxcsr = MXCSR_RESET;
        c.set_gpr32(1, 0xFFFF_FFFF);

        c.execute_instruction(&evex_scalar(Opcode::EvexVcvtusi2sdVsdEd))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 4294967295.0f64.to_bits());

        c.execute_instruction(&evex_scalar(Opcode::EvexVcvtsi2sdVsdEd))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), (-1.0f64).to_bits(), "signed reads -1");

        // 64-bit source: 2^64-1 is not exactly representable, so it rounds.
        c.set_gpr64(1, u64::MAX);
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvtusi2sdVsdEq))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 18446744073709551616.0f64.to_bits());
    }

    #[test]
    fn scalar_float_width_conversions_keep_the_vvvv_upper_elements() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.mxcsr.mxcsr = MXCSR_RESET;
        c.vmm[1].set_zmm64u(0, 1.5f64.to_bits()); // rm
        c.vmm[2].set_zmm32u(1, 0xAAAA_AAAA); // vvvv
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvtsd2ssVssWsd))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 1.5f32.to_bits());
        assert_eq!(c.vmm[0].zmm32u(1), 0xAAAA_AAAA);

        c.vmm[1].set_zmm32u(0, 1.5f32.to_bits());
        c.vmm[2].set_zmm64u(1, 0xBBBB_BBBB_BBBB_BBBB);
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvtss2sdVsdWss))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 1.5f64.to_bits());
        assert_eq!(c.vmm[0].zmm64u(1), 0xBBBB_BBBB_BBBB_BBBB);

        // Narrowing a double that has no exact single is inexact but defined.
        c.vmm[1].set_zmm64u(0, 0.1f64.to_bits());
        c.execute_instruction(&evex_scalar(Opcode::EvexVcvtsd2ssVssWsd))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 0.1f32.to_bits());
    }

}
