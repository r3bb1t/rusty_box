

//! AVX-512F rounding, scale, exponent, and mantissa instruction handlers
//!
//! Implements VRNDSCALE, VSCALEF, VGETEXP, VGETMANT (packed and scalar).
//! Mirrors Bochs `cpu/avx/avx512_pfpmisc.cc`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedZmmRegister,
};

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
fn read_opmask_for_write<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(cpu: &BxCpuC<'_, I, T>, instr: &Instruction) -> u64 {
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
fn read_zmm<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(cpu: &BxCpuC<'_, I, T>, reg: u8) -> BxPackedZmmRegister {
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

// ============================================================================
// Rounding helpers
// ============================================================================

/// Round f32 according to imm8[1:0] rounding mode.
/// 0 = nearest even, 1 = floor, 2 = ceil, 3 = truncate
#[inline]
/// Round f32 with scale support (Bochs f32_roundToInt).
/// `mode`: rounding mode (0=nearest-even, 1=floor, 2=ceil, 3=trunc).
/// `scale`: number of fraction bits to preserve (imm8[7:4]).
/// Rounds to nearest multiple of 2^(-scale).
fn round_f32(val: f32, mode: u8, scale: u8) -> f32 {
    if val.is_nan() || val.is_infinite() { return val; }
    // Scale factor: multiply by 2^scale, round to int, divide by 2^scale
    let factor = (2.0f32).powi(scale as i32);
    let scaled = val * factor;
    let rounded = match mode & 0x3 {
        0 => scaled.round_ties_even(),
        1 => scaled.floor(),
        2 => scaled.ceil(),
        _ => scaled.trunc(),
    };
    rounded / factor
}

/// Round f64 according to imm8[1:0] rounding mode.
/// 0 = nearest even, 1 = floor, 2 = ceil, 3 = truncate
#[inline]
fn round_f64(val: f64, mode: u8, scale: u8) -> f64 {
    if val.is_nan() || val.is_infinite() { return val; }
    let factor = (2.0f64).powi(scale as i32);
    let scaled = val * factor;
    let rounded = match mode & 0x3 {
        0 => scaled.round_ties_even(),
        1 => scaled.floor(),
        2 => scaled.ceil(),
        _ => scaled.trunc(),
    };
    rounded / factor
}

// ============================================================================
// VGETEXP helpers
// ============================================================================

/// Get unbiased exponent of f32 as float (logb).
/// Returns floor(log2(|src|)) for normal values.
/// Special cases: 0 -> -inf, inf -> +inf, NaN -> NaN.
#[inline]
fn getexp_f32(val: f32) -> f32 {
    let bits = val.to_bits();
    let exp_field = (bits >> 23) & 0xFF;
    if exp_field == 0 {
        if (bits & 0x007F_FFFF) == 0 {
            // +/- zero -> -inf
            f32::NEG_INFINITY
        } else {
            // Denormal: normalize and compute exponent
            // Count leading zeros in mantissa to find the effective exponent
            let mantissa = bits & 0x007F_FFFF;
            let shift = mantissa.leading_zeros() - 8; // 8 = 32 - 24 (mantissa is 23 bits)
            let effective_exp = -126i32 - shift as i32;
            effective_exp as f32
        }
    } else if exp_field == 0xFF {
        if (bits & 0x007F_FFFF) == 0 {
            // +/- infinity -> +inf
            f32::INFINITY
        } else {
            // NaN -> NaN (propagate)
            f32::NAN
        }
    } else {
        // Normal: unbiased exponent
        (exp_field as i32 - 127) as f32
    }
}

/// Get unbiased exponent of f64 as float (logb).
/// Returns floor(log2(|src|)) for normal values.
/// Special cases: 0 -> -inf, inf -> +inf, NaN -> NaN.
#[inline]
fn getexp_f64(val: f64) -> f64 {
    let bits = val.to_bits();
    let exp_field = (bits >> 52) & 0x7FF;
    if exp_field == 0 {
        if (bits & 0x000F_FFFF_FFFF_FFFF) == 0 {
            f64::NEG_INFINITY
        } else {
            let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;
            let shift = mantissa.leading_zeros() - 11; // 11 = 64 - 53
            let effective_exp = -1022i32 - shift as i32;
            effective_exp as f64
        }
    } else if exp_field == 0x7FF {
        if (bits & 0x000F_FFFF_FFFF_FFFF) == 0 {
            f64::INFINITY
        } else {
            f64::NAN
        }
    } else {
        (exp_field as i32 - 1023) as f64
    }
}

// ============================================================================
// VSCALEF helpers
// ============================================================================

/// Scale f32: result = src1 * 2^floor(src2)
/// Special handling for large/small exponents clamped to avoid overflow.
#[inline]
fn scalef_f32(src1: f32, src2: f32) -> f32 {
    if src2.is_nan() || src1.is_nan() {
        return f32::NAN;
    }
    // Bochs: inf * 2^(-inf) = NaN (invalid), 0 * 2^(+inf) = NaN (invalid)
    if src1.is_infinite() && src2.is_infinite() && src2.is_sign_negative() {
        return f32::NAN; // inf * 2^(-inf)
    }
    if src1 == 0.0 && src2.is_infinite() && src2.is_sign_positive() {
        return f32::NAN; // 0 * 2^(+inf)
    }
    if src1.is_infinite() {
        return src1;
    }
    if src1 == 0.0 {
        return src1;
    }
    let n = src2.floor();
    // Clamp exponent to prevent overflow in powi
    let n_clamped = if n > 127.0 {
        127
    } else if n < -149.0 {
        -149
    } else {
        n as i32
    };
    src1 * (2.0f32).powi(n_clamped)
}

/// Scale f64: result = src1 * 2^floor(src2)
#[inline]
fn scalef_f64(src1: f64, src2: f64) -> f64 {
    if src2.is_nan() || src1.is_nan() {
        return f64::NAN;
    }
    if src1.is_infinite() && src2.is_infinite() && src2.is_sign_negative() {
        return f64::NAN;
    }
    if src1 == 0.0 && src2.is_infinite() && src2.is_sign_positive() {
        return f64::NAN;
    }
    if src1.is_infinite() {
        return src1;
    }
    if src1 == 0.0 {
        return src1;
    }
    let n = src2.floor();
    let n_clamped = if n > 1023.0 {
        1023
    } else if n < -1074.0 {
        -1074
    } else {
        n as i32
    };
    src1 * (2.0f64).powi(n_clamped)
}

// ============================================================================
// Read helpers for memory operands
// ============================================================================

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// Read packed SP source: register or memory, dword-element granularity
    #[inline]
    fn read_src_ps(&mut self, instr: &Instruction, nelements: usize) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src()))
        } else {
            let mut tmp = BxPackedZmmRegister::default();
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                tmp.set_zmm32u(i, self.v_read_dword(seg, laddr + (i * 4) as u64)?);
            }
            Ok(tmp)
        }
    }

    /// Read packed DP source: register or memory, qword-element granularity
    #[inline]
    fn read_src_pd(&mut self, instr: &Instruction, nelements: usize) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src()))
        } else {
            let mut tmp = BxPackedZmmRegister::default();
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let lo = self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
                let hi = self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
                tmp.set_zmm64u(i, lo | (hi << 32));
            }
            Ok(tmp)
        }
    }

    /// Read 2-operand packed SP source (src2 for 3-operand instructions)
    #[inline]
    fn read_src2_ps(&mut self, instr: &Instruction, nelements: usize) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src2()))
        } else {
            let mut tmp = BxPackedZmmRegister::default();
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                tmp.set_zmm32u(i, self.v_read_dword(seg, laddr + (i * 4) as u64)?);
            }
            Ok(tmp)
        }
    }

    /// Read 2-operand packed DP source (src2 for 3-operand instructions)
    #[inline]
    fn read_src2_pd(&mut self, instr: &Instruction, nelements: usize) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src2()))
        } else {
            let mut tmp = BxPackedZmmRegister::default();
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let lo = self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
                let hi = self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
                tmp.set_zmm64u(i, lo | (hi << 32));
            }
            Ok(tmp)
        }
    }

    /// Read scalar f32 from src or memory
    #[inline]
    fn read_scalar_ss(&mut self, instr: &Instruction) -> super::Result<f32> {
        if instr.mod_c0() {
            let src = read_zmm(self, instr.src());
            Ok(src.zmm32f(0))
        } else {
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.v_read_dword(seg, laddr)?;
            Ok(f32::from_bits(val))
        }
    }

    /// Read scalar f64 from src or memory
    #[inline]
    fn read_scalar_sd(&mut self, instr: &Instruction) -> super::Result<f64> {
        if instr.mod_c0() {
            let src = read_zmm(self, instr.src());
            Ok(src.zmm64f(0))
        } else {
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.v_read_qword(seg, laddr)?;
            Ok(f64::from_bits(val))
        }
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
        let imm8 = instr.ib();
        // imm8[1:0] = rounding mode, imm8[2] = RC source (0=imm, 1=MXCSR)
        // imm8[3] = suppress inexact, imm8[7:4] = scale (fraction bits)
        let rc = if (imm8 & 0x04) != 0 { 0 } else { imm8 & 0x03 }; // TODO: read MXCSR.RC when bit2=1
        let scale = (imm8 >> 4) & 0x0F;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32f(i, round_f32(src.zmm32f(i), rc, scale));
        }
        let mask = read_opmask_for_write(self, instr);
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
        let imm8 = instr.ib();
        let rc = if (imm8 & 0x04) != 0 { 0 } else { imm8 & 0x03 };
        let scale = (imm8 >> 4) & 0x0F;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64f(i, round_f64(src.zmm64f(i), rc, scale));
        }
        let mask = read_opmask_for_write(self, instr);
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
        let imm8 = instr.ib();
        let rc = if (imm8 & 0x04) != 0 { 0 } else { imm8 & 0x03 };
        let scale = (imm8 >> 4) & 0x0F;
        let rounded = round_f32(src_val, rc, scale);

        // Start with src1 (vvvv) to preserve upper elements
        let mut result = read_zmm(self, instr.src1());
        result.set_zmm32f(0, rounded);

        let mask = read_opmask_for_write(self, instr);
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
        let imm8 = instr.ib();
        let rc = if (imm8 & 0x04) != 0 { 0 } else { imm8 & 0x03 };
        let scale = (imm8 >> 4) & 0x0F;
        let rounded = round_f64(src_val, rc, scale);

        let mut result = read_zmm(self, instr.src1());
        result.set_zmm64f(0, rounded);

        let mask = read_opmask_for_write(self, instr);
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
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32f(i, scalef_f32(src1.zmm32f(i), src2.zmm32f(i)));
        }
        let mask = read_opmask_for_write(self, instr);
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
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64f(i, scalef_f64(src1.zmm64f(i), src2.zmm64f(i)));
        }
        let mask = read_opmask_for_write(self, instr);
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
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32f(i, getexp_f32(src.zmm32f(i)));
        }
        let mask = read_opmask_for_write(self, instr);
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
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64f(i, getexp_f64(src.zmm64f(i)));
        }
        let mask = read_opmask_for_write(self, instr);
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
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32f(i, getmant_f32(src.zmm32f(i), imm8));
        }
        let mask = read_opmask_for_write(self, instr);
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
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64f(i, getmant_f64(src.zmm64f(i), imm8));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}

// ============================================================================
// VGETMANT helpers
// ============================================================================

/// Get normalized mantissa of f32.
/// interval selects normalization range:
///   0: [1, 2)   — set exponent bits to 127 (biased 0)
///   1: (0.5, 2) — same as 0 but if mantissa < 1.0, use [0.5, 1) form
///   2: (0.5, 1] — set exponent bits to 126 (biased -1), mantissa in [1,2) -> [0.5,1)
///   3: [0.75, 1.5) — adjust based on mantissa magnitude
/// Simplified implementation: handles intervals 0 and 2 exactly; 1 and 3 approximate.
#[inline]
/// Get mantissa of f32 (Bochs f32_getMant).
/// `imm8[1:0]` = interval, `imm8[3:2]` = sign_ctrl.
/// sign_ctrl: bit 0 = force positive when set, bit 1 = NaN on negative input.
fn getmant_f32(val: f32, imm8: u8) -> f32 {
    let interval = imm8 & 0x03;
    let sign_ctrl = (imm8 >> 2) & 0x03;
    let bits = val.to_bits();
    let sign_bit = bits & 0x8000_0000;
    let is_negative = sign_bit != 0;
    let exp_field = (bits >> 23) & 0xFF;
    let mantissa = bits & 0x007F_FFFF;

    // Output sign: if sign_ctrl bit 0 set, force positive
    let out_sign = if (sign_ctrl & 1) != 0 { 0u32 } else { sign_bit };

    // NaN
    if exp_field == 0xFF && mantissa != 0 { return f32::NAN; }
    // Infinity: negative inf with sign_ctrl bit 1 → NaN
    if exp_field == 0xFF && mantissa == 0 {
        if is_negative && (sign_ctrl & 2) != 0 { return f32::NAN; }
        return f32::from_bits(out_sign | 0x3F80_0000); // 1.0 with output sign
    }
    // Zero → 1.0 with output sign (Bochs: packToF32UI(~sign_ctrl & signA, 0x7F, 0))
    if exp_field == 0 && mantissa == 0 {
        return f32::from_bits(out_sign | 0x3F80_0000);
    }
    // Negative input with sign_ctrl bit 1 → NaN
    if is_negative && (sign_ctrl & 2) != 0 { return f32::NAN; }

    // Get normalized exponent and mantissa
    let (norm_exp, norm_mant) = if exp_field == 0 {
        // Denormal: normalize
        let shift = mantissa.leading_zeros() - 8; // leading zeros in 23-bit field
        let normalized_mant = (mantissa << shift) & 0x007F_FFFF;
        let norm_exp = 1u32.wrapping_sub(shift); // unbiased exponent
        (norm_exp, normalized_mant)
    } else {
        (exp_field, mantissa)
    };

    // Select target exponent based on interval
    let target_exp = match interval {
        0 => 127u32, // [1, 2)
        1 => {
            // [1/2, 2): Bochs: expA -= 0x7F; expA = 0x7F - (expA & 0x1)
            let unbiased = norm_exp.wrapping_sub(127);
            127 - (unbiased & 1)
        }
        2 => 126, // [0.5, 1)
        _ => {
            // [3/4, 3/2): Bochs: expA = 0x7F - ((sigA >> 22) & 0x1)
            if (norm_mant & 0x0040_0000) != 0 { 126 } else { 127 }
        }
    };

    f32::from_bits(out_sign | (target_exp << 23) | norm_mant)
}

/// Get normalized mantissa of f64.
#[inline]
fn getmant_f64(val: f64, imm8: u8) -> f64 {
    let interval = imm8 & 0x03;
    let sign_ctrl = (imm8 >> 2) & 0x03;
    let bits = val.to_bits();
    let sign_bit = bits & 0x8000_0000_0000_0000;
    let is_negative = sign_bit != 0;
    let exp_field = (bits >> 52) & 0x7FF;
    let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;

    let out_sign = if (sign_ctrl & 1) != 0 { 0u64 } else { sign_bit };

    if exp_field == 0x7FF && mantissa != 0 { return f64::NAN; }
    if exp_field == 0x7FF && mantissa == 0 {
        if is_negative && (sign_ctrl & 2) != 0 { return f64::NAN; }
        return f64::from_bits(out_sign | 0x3FF0_0000_0000_0000);
    }
    if exp_field == 0 && mantissa == 0 {
        return f64::from_bits(out_sign | 0x3FF0_0000_0000_0000);
    }
    if is_negative && (sign_ctrl & 2) != 0 { return f64::NAN; }

    let (norm_exp, norm_mant) = if exp_field == 0 {
        let shift = mantissa.leading_zeros() - 11;
        let normalized_mant = (mantissa << shift) & 0x000F_FFFF_FFFF_FFFF;
        let norm_exp = 1u64.wrapping_sub(shift as u64);
        (norm_exp, normalized_mant)
    } else {
        (exp_field, mantissa)
    };

    let target_exp = match interval {
        0 => 1023u64,
        1 => {
            let unbiased = norm_exp.wrapping_sub(1023);
            1023 - (unbiased & 1)
        }
        2 => 1022,
        _ => {
            if (norm_mant & 0x0008_0000_0000_0000) != 0 { 1022 } else { 1023 }
        }
    };

    f64::from_bits(out_sign | (target_exp << 52) | norm_mant)
}
