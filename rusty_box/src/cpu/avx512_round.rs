//! AVX-512F rounding, scale, exponent, and mantissa instruction handlers
//!
//! Implements VRNDSCALE, VSCALEF, VGETEXP, VGETMANT (packed and scalar).
//! Mirrors Bochs `cpu/avx/avx512_pfpmisc.cc`.

use super::softfloat3e::f32_addsub::f32_sub;
use super::softfloat3e::f32_range::f32_range;
use super::softfloat3e::f64_addsub::f64_sub;
use super::softfloat3e::f64_range::f64_range;
use super::softfloat3e::specialize::{
    FLOAT32_NEGATIVE_INF, FLOAT32_POSITIVE_INF, FLOAT64_NEGATIVE_INF, FLOAT64_POSITIVE_INF,
};
use super::sse_pfp::mxcsr_to_softfloat_status_word_imm_override;
use super::softfloat3e::f32_range::{f32_get_exp, f32_get_mant, f32_scalef};
use super::softfloat3e::f32_round_to_int::f32_round_to_int_scaled;
use super::softfloat3e::f64_range::{f64_get_exp, f64_get_mant, f64_scalef};
use super::softfloat3e::f64_round_to_int::f64_round_to_int_scaled;
use super::softfloat3e::softfloat::{
    softfloat_suppress_exception, FLAG_DENORMAL, FLAG_OVERFLOW, FLAG_UNDERFLOW,
    softfloat_get_exception_flags, softfloat_get_rounding_mode, SoftFloatStatus, FLAG_INEXACT,
};
use super::softfloat3e::softfloat_types::{Float32, Float64};
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    xmm::BxPackedZmmRegister,
};
// Load-bearing in pure no-std builds (core f32/f64 lack these inherent
// methods there); redundant in unit graphs where std is linked, so the
// unused-import lint is allowed rather than losing the no-std resolution.
#[cfg(not(feature = "std"))]
#[allow(unused_imports)]
use crate::cpu::float::FloatExt;

/// Number of 32-bit elements per vector length: VL0=4, VL1=8, VL2=16
#[inline]
fn dword_elements(vl: u8) -> usize {
    match vl {
        0 => 4,
        1 => 8,
        _ => 16,
    }
}

/// Number of 64-bit elements per vector length: VL0=2, VL1=4, VL2=8
#[inline]
fn qword_elements(vl: u8) -> usize {
    match vl {
        0 => 2,
        1 => 4,
        _ => 8,
    }
}

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

/// Read ZMM register as a ZMM-width value
#[inline]
fn read_zmm<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &BxCpuC<'_, I, T>,
    reg: u8,
) -> BxPackedZmmRegister {
    cpu.vmm[reg as usize]
}

/// Write ZMM register with dword-granularity masking, zeroing upper beyond VL
fn write_zmm_masked<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nelements = dword_elements(vl);
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..nelements {
        if (mask >> i) & 1 != 0 {
            dst.set_zmm32u(i, result.zmm32u(i));
        } else if zero_masking {
            dst.set_zmm32u(i, 0);
        }
    }
    for i in nelements..16 {
        dst.set_zmm32u(i, 0);
    }
}

/// Write ZMM register with qword-granularity masking, zeroing upper beyond VL
fn write_zmm_masked_q<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nelements = qword_elements(vl);
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..nelements {
        if (mask >> i) & 1 != 0 {
            dst.set_zmm64u(i, result.zmm64u(i));
        } else if zero_masking {
            dst.set_zmm64u(i, 0);
        }
    }
    for i in nelements..8 {
        dst.set_zmm64u(i, 0);
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Read packed SP source: register or memory, dword-element granularity
    #[inline]
    fn read_src_ps(
        &mut self,
        instr: &Instruction,
        _nelements: usize,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src()))
        } else {
            self.evex_load_bcst_d_pair(instr)
        }
    }

    /// Read packed DP source: register or memory, qword-element granularity
    #[inline]
    fn read_src_pd(
        &mut self,
        instr: &Instruction,
        _nelements: usize,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src()))
        } else {
            self.evex_load_bcst_q_pair(instr)
        }
    }

    /// Read 2-operand packed SP source (src2 for 3-operand instructions)
    #[inline]
    /// Read the r/m operand — Bochs calls it `i->src2()`, but this decoder
    /// puts EVEX.vvvv in `src2()` and ModRM.rm in `src1()`.
    fn read_rm_ps(
        &mut self,
        instr: &Instruction,
        _nelements: usize,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src1()))
        } else {
            self.evex_load_bcst_d_pair(instr)
        }
    }

    /// Read 2-operand packed DP source (src2 for 3-operand instructions)
    #[inline]
    /// Qword counterpart of [`Self::read_rm_ps`].
    fn read_rm_pd(
        &mut self,
        instr: &Instruction,
        _nelements: usize,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src1()))
        } else {
            self.evex_load_bcst_q_pair(instr)
        }
    }

    /// Read scalar f32 from src or memory
    #[inline]
    fn read_scalar_ss(&mut self, instr: &Instruction) -> super::Result<Float32> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src()).zmm32u(0))
        } else {
            Ok(self.evex_load_wss_pair(instr)?.zmm32u(0))
        }
    }

    /// Read scalar f64 from src or memory
    #[inline]
    fn read_scalar_sd(&mut self, instr: &Instruction) -> super::Result<Float64> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src()).zmm64u(0))
        } else {
            Ok(self.evex_load_wsd_pair(instr)?.zmm64u(0))
        }
    }


    /// VRNDSCALE imm8: bit 2 keeps MXCSR.RC, bits[1:0] otherwise override it,
    /// and bit 3 suppresses the precision exception. Bochs avx512_pfp.cc
    /// VRNDSCALEPS_MASK_VpsWpsIb via `mxcsr_to_softfloat_status_word_imm_override`.
    #[inline]
    fn rndscale_status(&self, instr: &Instruction) -> (SoftFloatStatus, u8, u8) {
        let imm8 = instr.ib();
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        if (imm8 & 0x04) == 0 {
            status.softfloat_rounding_mode = imm8 & 0x03;
        }
        if (imm8 & 0x08) != 0 {
            status.softfloat_suppress_exception |= FLAG_INEXACT;
        }
        let rc = softfloat_get_rounding_mode(&status);
        (status, rc, (imm8 >> 4) & 0x0F)
    }

    // ========================================================================
    // VRNDSCALEPS — Round packed single-precision, EVEX.66.0F3A.W0 08
    // ========================================================================

    /// VRNDSCALEPS Vps{k}, Wps, imm8
    /// imm8[1:0] = rounding mode, imm8[3:0] = fraction bits M (simplified: use rounding mode)
    pub fn evex_vrndscaleps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = self.read_src_ps(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let (mut status, rc, scale) = self.rndscale_status(instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(
                    i,
                    f32_round_to_int_scaled(src.zmm32u(i), scale, rc, true, &mut status),
                );
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VRNDSCALEPD — Round packed double-precision, EVEX.66.0F3A.W1 09
    // ========================================================================

    /// VRNDSCALEPD Vpd{k}, Wpd, imm8
    pub fn evex_vrndscalepd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = self.read_src_pd(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let (mut status, rc, scale) = self.rndscale_status(instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm64u(
                    i,
                    f64_round_to_int_scaled(src.zmm64u(i), scale, rc, true, &mut status),
                );
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VRNDSCALESS — Round scalar single-precision, EVEX.66.0F3A.W0 0A
    // ========================================================================

    /// VRNDSCALESS Vss{k}, Hss, Wss, imm8
    /// Scalar: rounds element [0] from src, copies [1..3] from src1 (vvvv).
    pub fn evex_vrndscaless(&mut self, instr: &Instruction) -> super::Result<()> {
        let src_val = self.read_scalar_ss(instr)?;
        let mask = read_opmask_for_write(self, instr);

        // Start with src1 (vvvv) to preserve upper elements
        let mut result = read_zmm(self, instr.src1());
        if (mask & 1) != 0 {
            let (mut status, rc, scale) = self.rndscale_status(instr);
            let rounded = f32_round_to_int_scaled(src_val, scale, rc, true, &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            result.set_zmm32u(0, rounded);
        }

        let zmask = instr.is_zero_masking() != 0;
        // Scalar: only element 0 is masked; elements 1..3 always from src1
        if (mask & 1) == 0 {
            if zmask {
                result.set_zmm32u(0, 0);
            } else {
                // Merge: keep original dst element 0
                let orig = read_zmm(self, instr.dst());
                result.set_zmm32u(0, orig.zmm32u(0));
            }
        }
        // Zero upper 256 bits (EVEX scalar zeroes upper)
        for i in 4..16 {
            result.set_zmm32u(i, 0);
        }
        self.vmm[instr.dst() as usize] = result;
        Ok(())
    }

    // ========================================================================
    // VRNDSCALESD — Round scalar double-precision, EVEX.66.0F3A.W1 0B
    // ========================================================================

    /// VRNDSCALESD Vsd{k}, Hsd, Wsd, imm8
    pub fn evex_vrndscalesd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src_val = self.read_scalar_sd(instr)?;
        let mask = read_opmask_for_write(self, instr);

        let mut result = read_zmm(self, instr.src1());
        if (mask & 1) != 0 {
            let (mut status, rc, scale) = self.rndscale_status(instr);
            let rounded = f64_round_to_int_scaled(src_val, scale, rc, true, &mut status);
            self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
            result.set_zmm64u(0, rounded);
        }

        let zmask = instr.is_zero_masking() != 0;
        if (mask & 1) == 0 {
            if zmask {
                result.set_zmm64u(0, 0);
            } else {
                let orig = read_zmm(self, instr.dst());
                result.set_zmm64u(0, orig.zmm64u(0));
            }
        }
        for i in 2..8 {
            result.set_zmm64u(i, 0);
        }
        self.vmm[instr.dst() as usize] = result;
        Ok(())
    }

    // ========================================================================
    // VSCALEFPS — Scale packed SP, EVEX.66.0F38.W0 2C
    // result[i] = src1[i] * 2^floor(src2[i])
    // ========================================================================

    /// VSCALEFPS Vps{k}, Hps, Wps
    pub fn evex_vscalefps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src2()); // EVEX.vvvv = Bochs i->src1()
        let src2 = self.read_rm_ps(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, f32_scalef(src1.zmm32u(i), src2.zmm32u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VSCALEFPD — Scale packed DP, EVEX.66.0F38.W1 2C
    // ========================================================================

    /// VSCALEFPD Vpd{k}, Hpd, Wpd
    pub fn evex_vscalefpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src2()); // EVEX.vvvv = Bochs i->src1()
        let src2 = self.read_rm_pd(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm64u(i, f64_scalef(src1.zmm64u(i), src2.zmm64u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VGETEXPPS — Get exponent of packed SP, EVEX.66.0F38.W0 42
    // result[i] = floor(log2(|src[i]|)) as float
    // ========================================================================

    /// VGETEXPPS Vps{k}, Wps
    pub fn evex_vgetexpps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = self.read_src_ps(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, f32_get_exp(src.zmm32u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VGETEXPPD — Get exponent of packed DP, EVEX.66.0F38.W1 42
    // ========================================================================

    /// VGETEXPPD Vpd{k}, Wpd
    pub fn evex_vgetexppd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = self.read_src_pd(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm64u(i, f64_get_exp(src.zmm64u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VGETMANTPS — Get mantissa of packed SP, EVEX.66.0F3A.W0 26
    // Simplified: returns input unchanged (stub)
    // ========================================================================

    /// VGETMANTPS Vps{k}, Wps, imm8
    /// Simplified stub: returns the normalized mantissa.
    /// imm8[1:0] selects interval: 0=[1,2), 1=(0.5,2), 2=(0.5,1], 3=[0.75,1.5)
    /// imm8[3:2] selects sign control.
    /// For now, extract mantissa in [1,2) range (set exponent to 0 = bias 127).
    pub fn evex_vgetmantps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = self.read_src_ps(instr, nelements)?;
        let imm8 = instr.ib();
        let (sign_ctrl, interv) = (((imm8 >> 2) & 0x3) as i32, (imm8 & 0x3) as i32);
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(
                    i,
                    f32_get_mant(src.zmm32u(i), &mut status, sign_ctrl, interv),
                );
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VGETMANTPD — Get mantissa of packed DP, EVEX.66.0F3A.W1 26
    // ========================================================================

    /// VGETMANTPD Vpd{k}, Wpd, imm8
    pub fn evex_vgetmantpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = self.read_src_pd(instr, nelements)?;
        let imm8 = instr.ib();
        let (sign_ctrl, interv) = (((imm8 >> 2) & 0x3) as i32, (imm8 & 0x3) as i32);
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm64u(
                    i,
                    f64_get_mant(src.zmm64u(i), &mut status, sign_ctrl, interv),
                );
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VREDUCE — a - roundToInt(a, M), i.e. the fractional remainder after
    // rounding away the top M fraction bits. Bochs avx512_pfp.cc
    // float32_reduce / VREDUCEPS_MASK_VpsWpsIbR.
    //
    // Bochs suppresses #D, #U and #O here: the subtraction of a value derived
    // from the operand itself cannot overflow, and any underflow is an
    // artefact of the decomposition rather than of the instruction.
    // ========================================================================

    /// The status word VREDUCE runs under: MXCSR, then embedded-RC, then the
    /// imm8 rounding override, then the three suppressed exceptions.
    fn reduce_status(&self, instr: &Instruction) -> (SoftFloatStatus, u8) {
        let control = instr.ib();
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        mxcsr_to_softfloat_status_word_imm_override(&mut status, control);
        softfloat_suppress_exception(
            &mut status,
            FLAG_DENORMAL | FLAG_UNDERFLOW | FLAG_OVERFLOW,
        );
        (status, control >> 4)
    }

    /// VREDUCEPS Vps{k}, Wps, Ib — EVEX.66.0F3A.W0 56
    pub fn evex_vreduceps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = self.read_src_ps(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let (mut status, scale) = self.reduce_status(instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, f32_reduce(src.zmm32u(i), scale, &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VREDUCEPD Vpd{k}, Wpd, Ib — EVEX.66.0F3A.W1 56
    pub fn evex_vreducepd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = self.read_src_pd(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let (mut status, scale) = self.reduce_status(instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm64u(i, f64_reduce(src.zmm64u(i), scale, &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VRANGE — min/max with imm8-controlled magnitude comparison and result
    // sign. Bochs avx512_pfp.cc VRANGEPS/PD_MASK_*.
    // ========================================================================

    /// VRANGEPS Vps{k}, Hps, Wps, Ib — EVEX.66.0F3A.W0 50
    pub fn evex_vrangeps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src2()); // EVEX.vvvv = Bochs i->src1()
        let src2 = self.read_rm_ps(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let (is_max, is_abs, sign_ctrl) = range_control(instr.ib());
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                let v = f32_range(
                    src1.zmm32u(i),
                    src2.zmm32u(i),
                    is_max,
                    is_abs,
                    sign_ctrl,
                    &mut status,
                );
                result.set_zmm32u(i, v);
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VRANGEPD Vpd{k}, Hpd, Wpd, Ib — EVEX.66.0F3A.W1 50
    pub fn evex_vrangepd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src2()); // EVEX.vvvv = Bochs i->src1()
        let src2 = self.read_rm_pd(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let (is_max, is_abs, sign_ctrl) = range_control(instr.ib());
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                let v = f64_range(
                    src1.zmm64u(i),
                    src2.zmm64u(i),
                    is_max,
                    is_abs,
                    sign_ctrl,
                    &mut status,
                );
                result.set_zmm64u(i, v);
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

}

/// Bochs avx512_pfp.cc `float32_reduce`. An infinite operand reduces to zero;
/// otherwise the result is the operand minus its own scaled rounding.
pub(super) fn f32_reduce(a: Float32, scale: u8, status: &mut SoftFloatStatus) -> Float32 {
    if a == FLOAT32_NEGATIVE_INF || a == FLOAT32_POSITIVE_INF {
        return 0;
    }
    let rc = softfloat_get_rounding_mode(status);
    let tmp = f32_round_to_int_scaled(a, scale, rc, false, status);
    f32_sub(a, tmp, status)
}

/// Bochs avx512_pfp.cc `float64_reduce`.
pub(super) fn f64_reduce(a: Float64, scale: u8, status: &mut SoftFloatStatus) -> Float64 {
    if a == FLOAT64_NEGATIVE_INF || a == FLOAT64_POSITIVE_INF {
        return 0;
    }
    let rc = softfloat_get_rounding_mode(status);
    let tmp = f64_round_to_int_scaled(a, scale, rc, false, status);
    f64_sub(a, tmp, status)
}

/// VRANGE imm8: bit 0 selects max over min, bit 1 compares magnitudes, and
/// bits [3:2] control the sign of the result.
#[inline]
pub(super) fn range_control(imm8: u8) -> (bool, bool, i32) {
    (imm8 & 0x1 != 0, imm8 & 0x2 != 0, ((imm8 >> 2) & 0x3) as i32)
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    //! VREDUCE and VRANGE are both imm8-driven, and in both cases the
    //! interesting behaviour lives in the immediate rather than in the
    //! arithmetic: VREDUCE's imm8 carries a rounding mode *and* a binary
    //! scale, and VRANGE's carries a magnitude flag and a sign override that
    //! can hand back a value whose sign belongs to neither input's ordering.

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::BxSegregs;
    use crate::cpu::xmm::MXCSR_RESET;
    use rusty_box_decoder::opcode::Opcode;

    use super::*;

    /// Register-form EVEX: dst=0, rm=1, vvvv=2, k0 (all elements active).
    fn evex(opcode: Opcode, vl: u8, imm8: u8) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0);
        i.set_src_reg(1, 1);
        i.set_src_reg(2, 2);
        i.set_opmask(0);
        i.set_iq(imm8 as u64);
        i.set_vex(true);
        i.set_vl(vl);
        i.set_seg(BxSegregs::Ds);
        i.init(0, 0, 1, 1);
        i.assert_mod_c0();
        i
    }

    fn cpu() -> alloc::boxed::Box<crate::cpu::cpu::BxCpuC<'static, AmdRyzen>> {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.mxcsr.mxcsr = MXCSR_RESET;
        c
    }

    #[test]
    fn vreduce_subtracts_the_scaled_rounding_of_its_operand() {
        let mut c = cpu();
        c.vmm[1].set_zmm32u(0, 1.75f32.to_bits());

        // scale 0, round-to-nearest-even: 1.75 rounds to 2, remainder -0.25.
        c.execute_instruction(&evex(Opcode::EvexVreducepsVpsWpsIbKmask, 0, 0x00))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), (-0.25f32).to_bits());

        // imm8[1:0] = 3 selects truncation: 1.75 -> 1, remainder 0.75.
        c.execute_instruction(&evex(Opcode::EvexVreducepsVpsWpsIbKmask, 0, 0x03))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 0.75f32.to_bits());

        // imm8[7:4] = 2 rounds to a multiple of 1/4, which 1.75 already is.
        c.execute_instruction(&evex(Opcode::EvexVreducepsVpsWpsIbKmask, 0, 0x20))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 0.0f32.to_bits());

        // An infinite operand reduces to zero rather than to a NaN.
        c.vmm[1].set_zmm32u(0, f32::INFINITY.to_bits());
        c.execute_instruction(&evex(Opcode::EvexVreducepsVpsWpsIbKmask, 0, 0x00))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 0);
        c.vmm[1].set_zmm32u(0, f32::NEG_INFINITY.to_bits());
        c.execute_instruction(&evex(Opcode::EvexVreducepsVpsWpsIbKmask, 0, 0x00))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 0);
    }

    #[test]
    fn vreducepd_matches_the_single_precision_behaviour() {
        let mut c = cpu();
        c.vmm[1].set_zmm64u(0, 1.75f64.to_bits());
        c.execute_instruction(&evex(Opcode::EvexVreducepdVpdWpdIbKmask, 0, 0x00))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), (-0.25f64).to_bits());
        c.execute_instruction(&evex(Opcode::EvexVreducepdVpdWpdIbKmask, 0, 0x03))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 0.75f64.to_bits());
    }

    #[test]
    fn vrange_sign_control_can_override_the_comparison_result() {
        let mut c = cpu();
        // src1 (vvvv) = -3.0, src2 (rm) = 2.0.
        c.vmm[2].set_zmm32u(0, (-3.0f32).to_bits());
        c.vmm[1].set_zmm32u(0, 2.0f32.to_bits());

        // imm8 0: min, signed compare, sign taken from src1. -3 < 2, so the
        // value is -3 and src1's sign is already negative.
        c.execute_instruction(&evex(Opcode::EvexVrangepsVpsHpsWpsIbKmask, 0, 0b0000))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), (-3.0f32).to_bits());

        // imm8 1: max picks 2.0, but sign_ctrl 0 still forces src1's sign.
        c.execute_instruction(&evex(Opcode::EvexVrangepsVpsHpsWpsIbKmask, 0, 0b0001))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), (-2.0f32).to_bits());

        // sign_ctrl 1 preserves the compared value's own sign.
        c.execute_instruction(&evex(Opcode::EvexVrangepsVpsHpsWpsIbKmask, 0, 0b0101))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 2.0f32.to_bits());

        // sign_ctrl 2 forces positive, 3 forces negative.
        c.execute_instruction(&evex(Opcode::EvexVrangepsVpsHpsWpsIbKmask, 0, 0b1001))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 2.0f32.to_bits());
        c.execute_instruction(&evex(Opcode::EvexVrangepsVpsHpsWpsIbKmask, 0, 0b1101))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), (-2.0f32).to_bits());
    }

    #[test]
    fn vrange_magnitude_mode_ignores_the_operand_signs() {
        let mut c = cpu();
        c.vmm[2].set_zmm64u(0, (-3.0f64).to_bits());
        c.vmm[1].set_zmm64u(0, 2.0f64.to_bits());

        // imm8 bit 1 compares |−3| against |2|: the minimum is 2, and
        // sign_ctrl 0 gives it src1's negative sign.
        c.execute_instruction(&evex(Opcode::EvexVrangepdVpdHpdWpdIbKmask, 0, 0b0010))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), (-2.0f64).to_bits());

        // Magnitude maximum is 3, again re-signed from src1.
        c.execute_instruction(&evex(Opcode::EvexVrangepdVpdHpdWpdIbKmask, 0, 0b0011))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), (-3.0f64).to_bits());
    }

    #[test]
    fn vrange_and_vreduce_scalar_forms_touch_only_element_zero() {
        let mut c = cpu();
        // vvvv supplies the upper elements of the destination.
        c.vmm[2].set_zmm32u(0, (-3.0f32).to_bits());
        c.vmm[2].set_zmm32u(1, 0x1111_1111);
        c.vmm[2].set_zmm32u(2, 0x2222_2222);
        c.vmm[2].set_zmm32u(3, 0x3333_3333);
        c.vmm[1].set_zmm32u(0, 2.0f32.to_bits());

        c.execute_instruction(&evex(Opcode::EvexVrangessVssHpsWssIbKmask, 0, 0b0101))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 2.0f32.to_bits());
        assert_eq!(c.vmm[0].zmm32u(1), 0x1111_1111);
        assert_eq!(c.vmm[0].zmm32u(2), 0x2222_2222);
        assert_eq!(c.vmm[0].zmm32u(3), 0x3333_3333);
        assert_eq!(c.vmm[0].zmm32u(4), 0, "EVEX clears above 128 bits");

        // VREDUCESS reduces the rm operand, not vvvv's element 0.
        c.vmm[1].set_zmm32u(0, 1.75f32.to_bits());
        c.execute_instruction(&evex(Opcode::EvexVreducessVssHpsWssIbKmask, 0, 0x00))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), (-0.25f32).to_bits());
        assert_eq!(c.vmm[0].zmm32u(1), 0x1111_1111);
    }

    #[test]
    fn vscalef_scales_vvvv_by_the_rm_operand_not_the_other_way_round() {
        // Bochs VSCALEFPS computes f32_scalef(i->src1(), i->src2()) — that is,
        // vvvv * 2^floor(rm). This decoder puts vvvv in src2() and rm in
        // src1(), so a handler that reads them positionally computes
        // rm * 2^floor(vvvv) instead. scalef is not commutative, so the two
        // give different answers for every asymmetric pair.
        let mut c = cpu();
        c.vmm[2].set_zmm32u(0, 3.0f32.to_bits()); // vvvv: the value
        c.vmm[1].set_zmm32u(0, 2.0f32.to_bits()); // rm:   the exponent
        c.execute_instruction(&evex(Opcode::EvexVscalefpsVpsHpsWps, 0, 0))
            .unwrap();
        // 3 * 2^2 = 12, not 2 * 2^3 = 16.
        assert_eq!(c.vmm[0].zmm32u(0), 12.0f32.to_bits());

        c.vmm[2].set_zmm64u(0, 3.0f64.to_bits());
        c.vmm[1].set_zmm64u(0, 2.0f64.to_bits());
        c.execute_instruction(&evex(Opcode::EvexVscalefpdVpdHpdWpd, 0, 0))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 12.0f64.to_bits());
    }

}
