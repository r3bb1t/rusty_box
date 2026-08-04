//! VEX-encoded (AVX) packed and scalar floating-point handlers.
//!
//! Mirrors Bochs `cpu/avx/avx_pfp.cc` (arithmetic, min/max, sqrt, compare,
//! add/sub) and the FP shuffle/unpack forms of `cpu/avx/avx.cc`.
//!
//! Operand convention in this port: `instr.dst()` is the destination,
//! `instr.src2()` is VEX.vvvv (Bochs `i->src1()`, the "H" operand), and
//! `instr.src1()` is the modrm.rm operand (Bochs `i->src2()`, the "W"
//! operand), read via `vex_read_src2_*` / `sse_pfp_read_op2_*`.
//!
//! VEX register writes always clear the destination above VL
//! (`write_xmm_reg` / `write_ymm_reg` — Bochs `BX_WRITE_XMM_REG_CLEAR_HIGH`
//! / `BX_WRITE_YMM_REGZ`).
//!
//! Element arithmetic goes through SoftFloat 3e against a status word
//! seeded from MXCSR, exactly as Bochs does, so MXCSR.RC, the MXCSR sticky
//! exception flags, #XM and DAZ/FTZ are all observable. Bochs applies the
//! per-128-bit-lane `xmm_*` primitives from `simd_pfp.h` through the
//! templates in `cpu_templates_pfp.h`; [`Self::avx_pfp_2op`] and its
//! siblings are the same shape.

use super::simd_pfp::{
    xmm_addpd, xmm_addps, xmm_addsubpd, xmm_addsubps, xmm_cmppd, xmm_cmpps, xmm_divpd, xmm_divps,
    xmm_haddpd, xmm_haddps, xmm_hsubpd, xmm_hsubps, xmm_maxpd, xmm_maxps, xmm_minpd, xmm_minps,
    xmm_mulpd, xmm_mulps, xmm_sqrtpd, xmm_sqrtps, xmm_subpd, xmm_subps,
};
use super::softfloat3e::f32_round_to_int::f32_round_to_int;
use super::softfloat3e::f32_sqrt::f32_sqrt;
use super::softfloat3e::f32_to_f64::f32_to_f64;
use super::softfloat3e::f32_to_int::{f32_to_i32, f32_to_i32_r_min_mag};
use super::softfloat3e::f64_round_to_int::f64_round_to_int;
use super::softfloat3e::f64_sqrt::f64_sqrt;
use super::softfloat3e::f64_to_f32::f64_to_f32;
use super::softfloat3e::f64_to_int::{f64_to_i32, f64_to_i32_r_min_mag};
use super::softfloat3e::int_to_float::{i32_to_f32, i32_to_f64, i64_to_f32, i64_to_f64};
use super::softfloat3e::softfloat::{
    softfloat_get_exception_flags, softfloat_get_rounding_mode, SoftFloatStatus,
};
use super::softfloat3e::softfloat_compare::{f32_compare_predicate, f64_compare_predicate};
use super::softfloat3e::softfloat_types::{Float32, Float64};
use super::sse_pfp::{dppd_lane, dpps_lane, mxcsr_to_softfloat_status_word_imm_override};
use super::sse_rcp::{approximate_rcp, approximate_rsqrt};
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::{BxPackedXmmRegister, BxPackedYmmRegister},
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ════════════════════════════════════════════════════════════════════
    // VEX FP templates — Bochs cpu_templates_pfp.h HANDLE_AVX_PFP_2OP /
    // HANDLE_AVX_PFP_1OP and avx_pfp.cc AVX_SCALAR_SINGLE_FP /
    // AVX_SCALAR_DOUBLE_FP. Each applies a `simd_pfp` primitive per
    // 128-bit lane over VL, checks for exceptions once, then writes.
    // ════════════════════════════════════════════════════════════════════

    /// Two-operand packed VEX FP: op1 = vvvv, op2 = rm.
    fn avx_pfp_2op(
        &mut self,
        instr: &Instruction,
        func: impl Fn(&mut BxPackedXmmRegister, &BxPackedXmmRegister, &mut SoftFloatStatus),
    ) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let mut op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            for lane in 0..2 {
                let mut a = op1.ymm128(lane);
                func(&mut a, &op2.ymm128(lane), &mut status);
                op1.set_ymm128(lane, a);
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_ymm_reg(instr.dst(), op1);
        } else {
            let mut op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            func(&mut op1, &op2, &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_xmm_reg(instr.dst(), op1);
        }
        Ok(())
    }

    /// Single-operand packed VEX FP (VSQRTPS/VSQRTPD): no vvvv source.
    fn avx_pfp_1op(
        &mut self,
        instr: &Instruction,
        func: fn(&mut BxPackedXmmRegister, &mut SoftFloatStatus),
    ) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let mut op = self.vex_read_src2_ymm(instr)?;
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            for lane in 0..2 {
                let mut a = op.ymm128(lane);
                func(&mut a, &mut status);
                op.set_ymm128(lane, a);
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_ymm_reg(instr.dst(), op);
        } else {
            let mut op = self.vex_read_src2_xmm(instr)?;
            let mut status = self.sse_status();
            self.softfloat_rc_override(&mut status, instr);
            func(&mut op, &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_xmm_reg(instr.dst(), op);
        }
        Ok(())
    }

    /// Scalar single-precision VEX FP: low element = op(vvvv.low, rm.low),
    /// remaining elements pass through from vvvv.
    fn avx_scalar_ss(
        &mut self,
        instr: &Instruction,
        func: fn(Float32, Float32, &mut SoftFloatStatus) -> Float32,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = func(result.xmm32u(0), w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm32u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// Scalar double-precision VEX FP.
    fn avx_scalar_sd(
        &mut self,
        instr: &Instruction,
        func: fn(Float64, Float64, &mut SoftFloatStatus) -> Float64,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = func(result.xmm64u(0), w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm64u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Packed conversions — Bochs avx_cvt.cc. All are single-source (no
    // vvvv); VL selects lane count and the destination is zeroed above
    // the result width.
    // ════════════════════════════════════════════════════════════════════

    /// VCVTDQ2PS — packed i32 → f32 over VL.
    pub(super) fn vcvtdq2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let mut op = self.vex_read_src2_ymm(instr)?;
            let mut status = self.sse_status();
            for i in 0..8 {
                op.set_ymm32u(i, i32_to_f32(op.ymm32s(i), &mut status));
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_ymm_reg(instr.dst(), op);
        } else {
            let mut op = self.vex_read_src2_xmm(instr)?;
            let mut status = self.sse_status();
            for i in 0..4 {
                op.set_xmm32u(i, i32_to_f32(op.xmm32s(i), &mut status));
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_xmm_reg(instr.dst(), op);
        }
        Ok(())
    }

    /// VCVTPS2DQ / VCVTTPS2DQ — packed f32 → i32 over VL. The rounding form
    /// takes its mode from MXCSR.RC; the truncating form always rounds
    /// toward zero.
    fn vex_cvt_ps2dq(&mut self, instr: &Instruction, truncate: bool) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let mut op = self.vex_read_src2_ymm(instr)?;
            let mut status = self.sse_status();
            let rc = softfloat_get_rounding_mode(&status);
            for i in 0..8 {
                let v = if truncate {
                    f32_to_i32_r_min_mag(op.ymm32u(i), true, false, &mut status)
                } else {
                    f32_to_i32(op.ymm32u(i), rc, true, &mut status)
                };
                op.set_ymm32s(i, v);
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_ymm_reg(instr.dst(), op);
        } else {
            let mut op = self.vex_read_src2_xmm(instr)?;
            let mut status = self.sse_status();
            let rc = softfloat_get_rounding_mode(&status);
            for i in 0..4 {
                let v = if truncate {
                    f32_to_i32_r_min_mag(op.xmm32u(i), true, false, &mut status)
                } else {
                    f32_to_i32(op.xmm32u(i), rc, true, &mut status)
                };
                op.set_xmm32s(i, v);
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_xmm_reg(instr.dst(), op);
        }
        Ok(())
    }

    pub(super) fn vcvtps2dq(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_cvt_ps2dq(i, false)
    }
    pub(super) fn vcvttps2dq(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_cvt_ps2dq(i, true)
    }

    /// Widening source read: VL=128 forms read 64 bits (m64 or low half of
    /// the rm register), VL=256 forms read 128 bits.
    fn vex_read_widening_src(&mut self, instr: &Instruction) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src1()))
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            if instr.get_vl() >= 1 {
                self.v_read_xmmword(seg, eaddr)
            } else {
                let lo = self.v_read_qword(seg, eaddr)?;
                let mut tmp = BxPackedXmmRegister::default();
                tmp.set_xmm64u(0, lo);
                Ok(tmp)
            }
        }
    }

    /// VCVTDQ2PD — 2 (VL=128) or 4 (VL=256) i32 → f64. Exact for every i32,
    /// so Bochs performs no exception check.
    pub(super) fn vcvtdq2pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.vex_read_widening_src(instr)?;
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, i32_to_f64(op2.xmm32s(i)));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, i32_to_f64(op2.xmm32s(0)));
            result.set_xmm64u(1, i32_to_f64(op2.xmm32s(1)));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VCVTPS2PD — 2 (VL=128) or 4 (VL=256) f32 → f64.
    pub(super) fn vcvtps2pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.vex_read_widening_src(instr)?;
        let mut status = self.sse_status();
        if instr.get_vl() >= 1 {
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64u(i, f32_to_f64(op2.xmm32u(i), &mut status));
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, f32_to_f64(op2.xmm32u(0), &mut status));
            result.set_xmm64u(1, f32_to_f64(op2.xmm32u(1), &mut status));
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VCVTPD2PS — 2 (VL=128) or 4 (VL=256) f64 → f32 into an xmm result;
    /// unused upper lanes zeroed.
    pub(super) fn vcvtpd2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut result = BxPackedXmmRegister::default();
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut status = self.sse_status();
            for i in 0..4 {
                result.set_xmm32u(i, f64_to_f32(op2.ymm64u(i), &mut status));
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut status = self.sse_status();
            result.set_xmm32u(0, f64_to_f32(op2.xmm64u(0), &mut status));
            result.set_xmm32u(1, f64_to_f32(op2.xmm64u(1), &mut status));
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        }
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// VCVTPD2DQ / VCVTTPD2DQ — 2 (VL=128) or 4 (VL=256) f64 → i32 into an
    /// xmm result; unused upper lanes zeroed.
    fn vex_cvt_pd2dq(&mut self, instr: &Instruction, truncate: bool) -> super::Result<()> {
        self.prepare_sse()?;
        let mut result = BxPackedXmmRegister::default();
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut status = self.sse_status();
            let rc = softfloat_get_rounding_mode(&status);
            for i in 0..4 {
                let v = if truncate {
                    f64_to_i32_r_min_mag(op2.ymm64u(i), true, false, &mut status)
                } else {
                    f64_to_i32(op2.ymm64u(i), rc, true, &mut status)
                };
                result.set_xmm32s(i, v);
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut status = self.sse_status();
            let rc = softfloat_get_rounding_mode(&status);
            for i in 0..2 {
                let v = if truncate {
                    f64_to_i32_r_min_mag(op2.xmm64u(i), true, false, &mut status)
                } else {
                    f64_to_i32(op2.xmm64u(i), rc, true, &mut status)
                };
                result.set_xmm32s(i, v);
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        }
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtpd2dq(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_cvt_pd2dq(i, false)
    }
    pub(super) fn vcvttpd2dq(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_cvt_pd2dq(i, true)
    }

    // ════════════════════════════════════════════════════════════════════
    // Scalar arithmetic — VADDSS/VSUBSS/VMULSS/VDIVSS/VMINSS/VMAXSS and
    // the SD twins. Bochs avx_pfp.cc AVX_SCALAR_SINGLE_FP /
    // AVX_SCALAR_DOUBLE_FP: low element = op(vvvv.low, rm.low), remaining
    // xmm elements pass through from vvvv, upper bits cleared.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vaddss(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_ss(i, super::softfloat3e::f32_addsub::f32_add)
    }
    pub(super) fn vaddsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_sd(i, super::softfloat3e::f64_addsub::f64_add)
    }
    pub(super) fn vsubss(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_ss(i, super::softfloat3e::f32_addsub::f32_sub)
    }
    pub(super) fn vsubsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_sd(i, super::softfloat3e::f64_addsub::f64_sub)
    }
    pub(super) fn vmulss(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_ss(i, super::softfloat3e::f32_mul::f32_mul)
    }
    pub(super) fn vmulsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_sd(i, super::softfloat3e::f64_mul::f64_mul)
    }
    pub(super) fn vdivss(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_ss(i, super::softfloat3e::f32_div::f32_div)
    }
    pub(super) fn vdivsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_sd(i, super::softfloat3e::f64_div::f64_div)
    }
    pub(super) fn vminss(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_ss(i, super::softfloat3e::f32_compare::f32_min)
    }
    pub(super) fn vminsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_sd(i, super::softfloat3e::f64_compare::f64_min)
    }
    pub(super) fn vmaxss(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_ss(i, super::softfloat3e::f32_compare::f32_max)
    }
    pub(super) fn vmaxsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_scalar_sd(i, super::softfloat3e::f64_compare::f64_max)
    }

    // ════════════════════════════════════════════════════════════════════
    // Packed arithmetic — VADDPS/VSUBPS/VMULPS/VDIVPS/VMINPS/VMAXPS and
    // the PD twins. Bochs avx_pfp.cc AVX_PACKED_FP: element-wise
    // op(vvvv[i], rm[i]) over VL, upper bits cleared.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vaddps(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_addps)
    }
    pub(super) fn vaddpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_addpd)
    }
    pub(super) fn vsubps(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_subps)
    }
    pub(super) fn vsubpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_subpd)
    }
    pub(super) fn vmulps(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_mulps)
    }
    pub(super) fn vmulpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_mulpd)
    }
    pub(super) fn vdivps(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_divps)
    }
    pub(super) fn vdivpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_divpd)
    }
    pub(super) fn vminps(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_minps)
    }
    pub(super) fn vminpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_minpd)
    }
    pub(super) fn vmaxps(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_maxps)
    }
    pub(super) fn vmaxpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(i, xmm_maxpd)
    }

    // ════════════════════════════════════════════════════════════════════
    // Square root — VSQRTPS/VSQRTPD (no vvvv operand) and VSQRTSS/VSQRTSD
    // (low element from rm, upper elements from vvvv). Bochs avx_pfp.cc
    // VSQRTPS_VpsWpsR / VSQRTSD_VsdHpdWsdR.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vsqrtps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_pfp_1op(instr, xmm_sqrtps)
    }

    pub(super) fn vsqrtpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_pfp_1op(instr, xmm_sqrtpd)
    }

    pub(super) fn vsqrtss(&mut self, instr: &Instruction) -> super::Result<()> {
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = f32_sqrt(w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm32u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vsqrtsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = f64_sqrt(w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm64u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Compare — VCMPPS/VCMPPD/VCMPSS/VCMPSD with the 32-entry AVX
    // predicate set (Bochs avx_pfp.cc VCMPPS_VpsHpsWpsIbR et al.).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vcmpps(&mut self, instr: &Instruction) -> super::Result<()> {
        // VEX encodes the full 32-predicate set: Bochs masks Ib() with 0x1F.
        let predicate = instr.ib() & 0x1F;
        self.avx_pfp_2op(instr, move |op1, op2, status| {
            xmm_cmpps(op1, op2, predicate, status)
        })
    }

    pub(super) fn vcmppd(&mut self, instr: &Instruction) -> super::Result<()> {
        let predicate = instr.ib() & 0x1F;
        self.avx_pfp_2op(instr, move |op1, op2, status| {
            xmm_cmppd(op1, op2, predicate, status)
        })
    }

    pub(super) fn vcmpss(&mut self, instr: &Instruction) -> super::Result<()> {
        let predicate = instr.ib() & 0x1F;
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        let hit = f32_compare_predicate(predicate, result.xmm32u(0), w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm32u(0, if hit { 0xFFFF_FFFF } else { 0 });
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcmpsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let predicate = instr.ib() & 0x1F;
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        let hit = f64_compare_predicate(predicate, result.xmm64u(0), w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm64u(0, if hit { u64::MAX } else { 0 });
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Shuffle — VSHUFPS/VSHUFPD (per-128-bit-lane, Bochs avx.cc
    // VSHUFPS_VpsHpsWpsIbR via xmm_shufps).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vshufps(&mut self, instr: &Instruction) -> super::Result<()> {
        let order = instr.ib() as usize;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 4;
                result.set_ymm32u(base, op1.ymm32u(base + (order & 3)));
                result.set_ymm32u(base + 1, op1.ymm32u(base + ((order >> 2) & 3)));
                result.set_ymm32u(base + 2, op2.ymm32u(base + ((order >> 4) & 3)));
                result.set_ymm32u(base + 3, op2.ymm32u(base + ((order >> 6) & 3)));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, op1.xmm32u(order & 3));
            result.set_xmm32u(1, op1.xmm32u((order >> 2) & 3));
            result.set_xmm32u(2, op2.xmm32u((order >> 4) & 3));
            result.set_xmm32u(3, op2.xmm32u((order >> 6) & 3));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vshufpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let order = instr.ib() as usize;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 2;
                let sel = order >> (lane * 2);
                result.set_ymm64u(base, op1.ymm64u(base + (sel & 1)));
                result.set_ymm64u(base + 1, op2.ymm64u(base + ((sel >> 1) & 1)));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, op1.xmm64u(order & 1));
            result.set_xmm64u(1, op2.xmm64u((order >> 1) & 1));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // VADDSUBPS/VADDSUBPD — subtract on even elements, add on odd
    // (Bochs avx_pfp.cc VADDSUBPS_VpsHpsWpsR via xmm_addsubps).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vaddsubps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(instr, xmm_addsubps)
    }

    pub(super) fn vaddsubpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(instr, xmm_addsubpd)
    }

    // ════════════════════════════════════════════════════════════════════
    // VHADDPS/VHADDPD/VHSUBPS/VHSUBPD — horizontal add/subtract, per
    // 128-bit lane (Bochs avx_pfp.cc HANDLE_AVX_PFP_2OP<xmm_haddps> via
    // simd_pfp.h xmm_haddps et al.).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vhaddps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(instr, xmm_haddps)
    }

    pub(super) fn vhaddpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(instr, xmm_haddpd)
    }

    pub(super) fn vhsubps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(instr, xmm_hsubps)
    }

    pub(super) fn vhsubpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_pfp_2op(instr, xmm_hsubpd)
    }

    // ════════════════════════════════════════════════════════════════════
    // VBLENDPS/VBLENDPD — immediate blends (Bochs avx.cc
    // VBLENDPS_VpsHpsWpsIbR via simd_int.h xmm_blendps). The imm8 mask is
    // consumed 4 (ps) / 2 (pd) bits per 128-bit lane.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vblendps(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut mask = instr.ib();
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = op1;
            for lane in 0..2 {
                let mut r = result.ymm128(lane);
                super::sse::blendps_lane(&mut r, &op2.ymm128(lane), mask);
                result.set_ymm128(lane, r);
                mask >>= 4;
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            super::sse::blendps_lane(&mut result, &op2, mask);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vblendpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut mask = instr.ib();
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = op1;
            for lane in 0..2 {
                let mut r = result.ymm128(lane);
                super::sse::blendpd_lane(&mut r, &op2.ymm128(lane), mask);
                result.set_ymm128(lane, r);
                mask >>= 2;
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            super::sse::blendpd_lane(&mut result, &op2, mask);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // VBLENDVPS/VBLENDVPD — variable blends (Bochs avx.cc
    // VBLENDVPS_VpsHpsWpsIbR via simd_int.h xmm_blendvps). Unlike the
    // legacy forms (implicit XMM0), the mask register is is4: imm8[7:4],
    // extracted into src3 by the decoder.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vblendvps(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mask = self.read_ymm_reg(instr.src3());
            let mut result = op1;
            for lane in 0..2 {
                let mut r = result.ymm128(lane);
                super::sse::blendvps_lane(&mut r, &op2.ymm128(lane), &mask.ymm128(lane));
                result.set_ymm128(lane, r);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mask = self.read_xmm_reg(instr.src3());
            super::sse::blendvps_lane(&mut result, &op2, &mask);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vblendvpd(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mask = self.read_ymm_reg(instr.src3());
            let mut result = op1;
            for lane in 0..2 {
                let mut r = result.ymm128(lane);
                super::sse::blendvpd_lane(&mut r, &op2.ymm128(lane), &mask.ymm128(lane));
                result.set_ymm128(lane, r);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let mut result = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mask = self.read_xmm_reg(instr.src3());
            super::sse::blendvpd_lane(&mut result, &op2, &mask);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // VDPPS/VDPPD — dot products (Bochs avx_pfp.cc VDPPS_VpsHpsWpsIbR,
    // sse_pfp.cc DPPD_VpdHpdWpdIbR). VDPPS applies the same imm8 to each
    // 128-bit lane; VDPPD is VL128-only (VEX.256 #UD in the decoder).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vdpps(&mut self, instr: &Instruction) -> super::Result<()> {
        let mask = instr.ib();
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut status = self.sse_status();
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                result.set_ymm128(
                    lane,
                    dpps_lane(&op1.ymm128(lane), &op2.ymm128(lane), mask, &mut status),
                );
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut status = self.sse_status();
            let result = dpps_lane(&op1, &op2, mask, &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vdppd(&mut self, instr: &Instruction) -> super::Result<()> {
        let op1 = self.read_xmm_reg(instr.src2());
        let op2 = self.vex_read_src2_xmm(instr)?;
        let mut status = self.sse_status();
        let result = dppd_lane(&op1, &op2, instr.ib(), &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // FP unpack — VUNPCKLPS/VUNPCKHPS/VUNPCKLPD/VUNPCKHPD, per-128-bit
    // lane (Bochs avx.cc VUNPCKLPS_VpsHpsWpsR via xmm_unpcklps).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vunpcklps(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 4;
                result.set_ymm32u(base, op1.ymm32u(base));
                result.set_ymm32u(base + 1, op2.ymm32u(base));
                result.set_ymm32u(base + 2, op1.ymm32u(base + 1));
                result.set_ymm32u(base + 3, op2.ymm32u(base + 1));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, op1.xmm32u(0));
            result.set_xmm32u(1, op2.xmm32u(0));
            result.set_xmm32u(2, op1.xmm32u(1));
            result.set_xmm32u(3, op2.xmm32u(1));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vunpckhps(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 4;
                result.set_ymm32u(base, op1.ymm32u(base + 2));
                result.set_ymm32u(base + 1, op2.ymm32u(base + 2));
                result.set_ymm32u(base + 2, op1.ymm32u(base + 3));
                result.set_ymm32u(base + 3, op2.ymm32u(base + 3));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, op1.xmm32u(2));
            result.set_xmm32u(1, op2.xmm32u(2));
            result.set_xmm32u(2, op1.xmm32u(3));
            result.set_xmm32u(3, op2.xmm32u(3));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vunpcklpd(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 2;
                result.set_ymm64u(base, op1.ymm64u(base));
                result.set_ymm64u(base + 1, op2.ymm64u(base));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, op1.xmm64u(0));
            result.set_xmm64u(1, op2.xmm64u(0));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Scalar moves — VMOVSS/VMOVSD. Register forms merge the low element
    // into vvvv's upper elements (Bochs avx.cc VMOVSS_VssHpsWssR); memory
    // loads zero-extend; memory stores write the low element only.
    // ════════════════════════════════════════════════════════════════════

    /// VMOVSS xmm1, xmm2, xmm3 (0F 10 reg) / VMOVSS xmm1, m32 (0F 10 mem).
    pub(super) fn vmovss_load(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let mut result = self.read_xmm_reg(instr.src2());
            result.set_xmm32u(0, self.read_xmm_reg(instr.src1()).xmm32u(0));
            self.write_xmm_reg(instr.dst(), result);
        } else {
            let w = self.sse_pfp_read_op2_ss(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, w);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VMOVSD xmm1, xmm2, xmm3 (F2 0F 10 reg) / VMOVSD xmm1, m64 (mem).
    pub(super) fn vmovsd_load(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let mut result = self.read_xmm_reg(instr.src2());
            result.set_xmm64u(0, self.read_xmm_reg(instr.src1()).xmm64u(0));
            self.write_xmm_reg(instr.dst(), result);
        } else {
            let w = self.sse_pfp_read_op2_sd(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, w);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VMOVSS xmm1, xmm2, xmm3 (0F 11 reg — rm register is the destination,
    /// roles normalized by decode) / VMOVSS m32, xmm1 (mem store).
    pub(super) fn vmovss_store(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let mut result = self.read_xmm_reg(instr.src2());
            result.set_xmm32u(0, self.read_xmm_reg(instr.src1()).xmm32u(0));
            self.write_xmm_reg(instr.dst(), result);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            let val = self.read_xmm_reg(instr.src1()).xmm32u(0);
            self.v_write_dword(seg, eaddr, val)?;
        }
        Ok(())
    }

    /// VMOVSD store form (F2 0F 11).
    pub(super) fn vmovsd_store(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.mod_c0() {
            let mut result = self.read_xmm_reg(instr.src2());
            result.set_xmm64u(0, self.read_xmm_reg(instr.src1()).xmm64u(0));
            self.write_xmm_reg(instr.dst(), result);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            let val = self.read_xmm_reg(instr.src1()).xmm64u(0);
            self.v_write_qword(seg, eaddr, val)?;
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Half-register moves — VMOVLPS/LPD (low qword from m64, high from
    // vvvv), VMOVHPS/HPD (high qword from m64, low from vvvv), VMOVHLPS,
    // VMOVLHPS. Bochs avx.cc VMOVLPD_VpdHpdMq / VMOVHLPS_VpsHpsWps.
    // ════════════════════════════════════════════════════════════════════

    /// VMOVLPS/VMOVLPD xmm1, xmm2, m64.
    pub(super) fn vmovlp(&mut self, instr: &Instruction) -> super::Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.v_read_qword(seg, eaddr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64u(0, val);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// VMOVHPS/VMOVHPD xmm1, xmm2, m64.
    pub(super) fn vmovhp(&mut self, instr: &Instruction) -> super::Result<()> {
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.v_read_qword(seg, eaddr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64u(1, val);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// VMOVHLPS xmm1, xmm2, xmm3 — low qword = xmm3 high qword.
    pub(super) fn vmovhlps(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64u(0, self.read_xmm_reg(instr.src1()).xmm64u(1));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// VMOVLHPS xmm1, xmm2, xmm3 — high qword = xmm3 low qword.
    pub(super) fn vmovlhps(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64u(1, self.read_xmm_reg(instr.src1()).xmm64u(0));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Duplicating moves — VMOVSLDUP/VMOVSHDUP/VMOVDDUP (no vvvv; Bochs
    // avx.cc VMOVSLDUP_VpsWpsR, VMOVDDUP_VpdWpdR). VL=128 VMOVDDUP reads
    // only 64 bits from memory.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vmovsldup(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm32u(i * 2, op2.ymm32u(i * 2));
                result.set_ymm32u(i * 2 + 1, op2.ymm32u(i * 2));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm32u(i * 2, op2.xmm32u(i * 2));
                result.set_xmm32u(i * 2 + 1, op2.xmm32u(i * 2));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vmovshdup(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm32u(i * 2, op2.ymm32u(i * 2 + 1));
                result.set_ymm32u(i * 2 + 1, op2.ymm32u(i * 2 + 1));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm32u(i * 2, op2.xmm32u(i * 2 + 1));
                result.set_xmm32u(i * 2 + 1, op2.xmm32u(i * 2 + 1));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VMOVDDUP — VL=128 duplicates the low qword (memory form reads m64);
    /// VL=256 duplicates the even qwords per 128-bit lane.
    pub(super) fn vmovddup(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let v = op2.ymm64u(lane * 2);
                result.set_ymm64u(lane * 2, v);
                result.set_ymm64u(lane * 2 + 1, v);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let low = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1()).xmm64u(0)
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_qword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, low);
            result.set_xmm64u(1, low);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Scalar conversions with vvvv upper-element pass-through
    // (Bochs avx_cvt.cc VCVTSS2SD_VsdWssR, VCVTSI2SD_VsdEdR, ...).
    // Host-float casts are round-to-nearest, matching default MXCSR like
    // the legacy handlers.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vcvtss2sd(&mut self, instr: &Instruction) -> super::Result<()> {
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        let value = f32_to_f64(w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm64u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtsd2ss(&mut self, instr: &Instruction) -> super::Result<()> {
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = f64_to_f32(w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm32u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// Read the 32-bit integer source of a VCVTSI2xx.
    #[inline]
    fn vcvtsi_read_src32(&mut self, instr: &Instruction) -> super::Result<i32> {
        if instr.mod_c0() {
            Ok(self.get_gpr32(instr.src1().into()) as i32)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            Ok(self.v_read_dword(seg, eaddr)? as i32)
        }
    }

    /// Read the 64-bit integer source of a VCVTSI2xx (long mode).
    #[inline]
    fn vcvtsi_read_src64(&mut self, instr: &Instruction) -> super::Result<i64> {
        if instr.mod_c0() {
            Ok(self.get_gpr64(instr.src1() as usize) as i64)
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            Ok(self.read_virtual_qword_64(seg, eaddr)? as i64)
        }
    }

    /// VCVTSI2SD (dword source) — exact for every i32, so no status check.
    pub(super) fn vcvtsi2sd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.vcvtsi_read_src32(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64u(0, i32_to_f64(op2));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtsi2sd_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.vcvtsi_read_src64(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = i64_to_f64(op2, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm64u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtsi2ss_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.vcvtsi_read_src32(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = i32_to_f32(op2, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm32u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtsi2ss_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        let op2 = self.vcvtsi_read_src64(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let value = i64_to_f32(op2, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm32u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Rounding — VROUNDPS/PD (no vvvv) and VROUNDSS/SD (vvvv upper
    // pass-through). Bochs avx_pfp.cc VROUNDPS_VpsWpsIbR.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vroundps(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm8 = instr.ib();
        if instr.get_vl() >= 1 {
            let mut op = self.vex_read_src2_ymm(instr)?;
            let mut status = self.sse_status();
            mxcsr_to_softfloat_status_word_imm_override(&mut status, imm8);
            for i in 0..8 {
                op.set_ymm32u(i, f32_round_to_int(op.ymm32u(i), &mut status));
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_ymm_reg(instr.dst(), op);
        } else {
            let mut op = self.vex_read_src2_xmm(instr)?;
            let mut status = self.sse_status();
            mxcsr_to_softfloat_status_word_imm_override(&mut status, imm8);
            for i in 0..4 {
                op.set_xmm32u(i, f32_round_to_int(op.xmm32u(i), &mut status));
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_xmm_reg(instr.dst(), op);
        }
        Ok(())
    }

    pub(super) fn vroundpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm8 = instr.ib();
        if instr.get_vl() >= 1 {
            let mut op = self.vex_read_src2_ymm(instr)?;
            let mut status = self.sse_status();
            mxcsr_to_softfloat_status_word_imm_override(&mut status, imm8);
            for i in 0..4 {
                op.set_ymm64u(i, f64_round_to_int(op.ymm64u(i), &mut status));
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_ymm_reg(instr.dst(), op);
        } else {
            let mut op = self.vex_read_src2_xmm(instr)?;
            let mut status = self.sse_status();
            mxcsr_to_softfloat_status_word_imm_override(&mut status, imm8);
            for i in 0..2 {
                op.set_xmm64u(i, f64_round_to_int(op.xmm64u(i), &mut status));
            }
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            self.write_xmm_reg(instr.dst(), op);
        }
        Ok(())
    }

    pub(super) fn vroundss(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm8 = instr.ib();
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        mxcsr_to_softfloat_status_word_imm_override(&mut status, imm8);
        let value = f32_round_to_int(w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm32u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vroundsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm8 = instr.ib();
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let mut status = self.sse_status();
        mxcsr_to_softfloat_status_word_imm_override(&mut status, imm8);
        let value = f64_round_to_int(w, &mut status);
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        result.set_xmm64u(0, value);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Reciprocal approximations — VRCPPS/VRSQRTPS (no vvvv) and
    // VRCPSS/VRSQRTSS (vvvv upper pass-through). Full-precision host math,
    // same documented divergence as legacy sse_rcp.rs (real hardware is
    // ~12-bit approximate).
    // ════════════════════════════════════════════════════════════════════

    /// VRCPPS / VRSQRTPS share the same shape: single source, no vvvv, and
    /// no MXCSR interaction at all (Bochs avx_pfp.cc VRCPPS_VpsWpsR).
    fn avx_approx_ps(
        &mut self,
        instr: &Instruction,
        func: fn(Float32) -> Float32,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let mut op = self.vex_read_src2_ymm(instr)?;
            for i in 0..8 {
                op.set_ymm32u(i, func(op.ymm32u(i)));
            }
            self.write_ymm_reg(instr.dst(), op);
        } else {
            let mut op = self.vex_read_src2_xmm(instr)?;
            for i in 0..4 {
                op.set_xmm32u(i, func(op.xmm32u(i)));
            }
            self.write_xmm_reg(instr.dst(), op);
        }
        Ok(())
    }

    /// VRCPSS / VRSQRTSS — low element approximated, upper elements from vvvv.
    fn avx_approx_ss(
        &mut self,
        instr: &Instruction,
        func: fn(Float32) -> Float32,
    ) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm32u(0, func(w));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vrcpps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_approx_ps(instr, approximate_rcp)
    }

    pub(super) fn vrsqrtps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_approx_ps(instr, approximate_rsqrt)
    }

    pub(super) fn vrcpss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_approx_ss(instr, approximate_rcp)
    }

    pub(super) fn vrsqrtss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.avx_approx_ss(instr, approximate_rsqrt)
    }

    pub(super) fn vunpckhpd(&mut self, instr: &Instruction) -> super::Result<()> {
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 2;
                result.set_ymm64u(base, op1.ymm64u(base + 1));
                result.set_ymm64u(base + 1, op2.ymm64u(base + 1));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, op1.xmm64u(1));
            result.set_xmm64u(1, op2.xmm64u(1));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }
}
