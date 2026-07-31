//! AVX-512F rounding, scale, exponent, and mantissa instruction handlers
//!
//! Implements VRNDSCALE, VSCALEF, VGETEXP, VGETMANT (packed and scalar).
//! Mirrors Bochs `cpu/avx/avx512_pfpmisc.cc`.

use super::softfloat3e::f32_range::{f32_get_exp, f32_get_mant, f32_scalef};
use super::softfloat3e::f32_roundToInt::f32_round_to_int_scaled;
use super::softfloat3e::f64_range::{f64_get_exp, f64_get_mant, f64_scalef};
use super::softfloat3e::f64_roundToInt::f64_round_to_int_scaled;
use super::softfloat3e::softfloat::{
    softfloat_getExceptionFlags, softfloat_getRoundingMode, SoftFloatStatus, FLAG_INEXACT,
};
use super::softfloat3e::softfloat_types::{float32, float64};
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
    fn read_src2_ps(
        &mut self,
        instr: &Instruction,
        _nelements: usize,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src2()))
        } else {
            self.evex_load_bcst_d_pair(instr)
        }
    }

    /// Read 2-operand packed DP source (src2 for 3-operand instructions)
    #[inline]
    fn read_src2_pd(
        &mut self,
        instr: &Instruction,
        _nelements: usize,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src2()))
        } else {
            self.evex_load_bcst_q_pair(instr)
        }
    }

    /// Read scalar f32 from src or memory
    #[inline]
    fn read_scalar_ss(&mut self, instr: &Instruction) -> super::Result<float32> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src()).zmm32u(0))
        } else {
            Ok(self.evex_load_wss_pair(instr)?.zmm32u(0))
        }
    }

    /// Read scalar f64 from src or memory
    #[inline]
    fn read_scalar_sd(&mut self, instr: &Instruction) -> super::Result<float64> {
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
            status.softfloat_roundingMode = imm8 & 0x03;
        }
        if (imm8 & 0x08) != 0 {
            status.softfloat_suppressException |= FLAG_INEXACT;
        }
        let rc = softfloat_getRoundingMode(&status);
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
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
            self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
            self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
        let src1 = read_zmm(self, instr.src1());
        let src2 = self.read_src2_ps(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, f32_scalef(src1.zmm32u(i), src2.zmm32u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
        let src1 = read_zmm(self, instr.src1());
        let src2 = self.read_src2_pd(instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm64u(i, f64_scalef(src1.zmm64u(i), src2.zmm64u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}
