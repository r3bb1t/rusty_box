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
fn read_opmask_for_write<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, instr: &Instruction) -> u64 {
    let k = instr.opmask();
    if k == 0 {
        u64::MAX
    } else {
        unsafe { cpu.opmask[k as usize].rrx }
    }
}

/// Read ZMM register as a ZMM-width value
#[inline]
fn read_zmm<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, reg: u8) -> BxPackedZmmRegister {
    unsafe { cpu.vmm[reg as usize] }
}

/// Write ZMM register with dword-granularity masking, zeroing upper beyond VL
fn write_zmm_masked<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nelements = dword_elements(vl);
    unsafe {
        let dst = &mut cpu.vmm[reg as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                dst.zmm32u[i] = result.zmm32u[i];
            } else if zero_masking {
                dst.zmm32u[i] = 0;
            }
        }
        for i in nelements..16 {
            dst.zmm32u[i] = 0;
        }
    }
}

/// Write ZMM register with qword-granularity masking, zeroing upper beyond VL
fn write_zmm_masked_q<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nelements = qword_elements(vl);
    unsafe {
        let dst = &mut cpu.vmm[reg as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                dst.zmm64u[i] = result.zmm64u[i];
            } else if zero_masking {
                dst.zmm64u[i] = 0;
            }
        }
        for i in nelements..8 {
            dst.zmm64u[i] = 0;
        }
    }
}

// ============================================================================
// Rounding helpers
// ============================================================================

/// Round f32 according to imm8[1:0] rounding mode.
/// 0 = nearest even, 1 = floor, 2 = ceil, 3 = truncate
#[inline]
fn round_f32(val: f32, mode: u8) -> f32 {
    match mode & 0x3 {
        0 => {
            // Round to nearest even: Rust's f32::round() rounds half-away-from-zero,
            // so use the banker's rounding approach.
            let rounded = val.round();
            // Check half-integer case: if fract == 0.5, round to even
            if (val - val.floor()).abs() == 0.5 {
                let floor = val.floor();
                let ceil = val.ceil();
                if (floor as i64) % 2 == 0 { floor } else { ceil }
            } else {
                rounded
            }
        }
        1 => val.floor(),
        2 => val.ceil(),
        3 => val.trunc(),
        _ => unreachable!(),
    }
}

/// Round f64 according to imm8[1:0] rounding mode.
/// 0 = nearest even, 1 = floor, 2 = ceil, 3 = truncate
#[inline]
fn round_f64(val: f64, mode: u8) -> f64 {
    match mode & 0x3 {
        0 => {
            let rounded = val.round();
            if (val - val.floor()).abs() == 0.5 {
                let floor = val.floor();
                let ceil = val.ceil();
                if (floor as i64) % 2 == 0 { floor } else { ceil }
            } else {
                rounded
            }
        }
        1 => val.floor(),
        2 => val.ceil(),
        3 => val.trunc(),
        _ => unreachable!(),
    }
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
    if src1.is_infinite() {
        return src1; // inf * 2^n = inf (with same sign)
    }
    if src1 == 0.0 {
        return src1; // 0 * 2^n = 0 (with same sign)
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

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// Read packed SP source: register or memory, dword-element granularity
    #[inline]
    fn read_src_ps(&mut self, instr: &Instruction, nelements: usize) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src()))
        } else {
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
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
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let lo = self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
                let hi = self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
                unsafe { tmp.zmm64u[i] = lo | (hi << 32); }
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
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                unsafe { tmp.zmm32u[i] = self.v_read_dword(seg, laddr + (i * 4) as u64)?; }
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
            let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            for i in 0..nelements {
                let lo = self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
                let hi = self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
                unsafe { tmp.zmm64u[i] = lo | (hi << 32); }
            }
            Ok(tmp)
        }
    }

    /// Read scalar f32 from src or memory
    #[inline]
    fn read_scalar_ss(&mut self, instr: &Instruction) -> super::Result<f32> {
        if instr.mod_c0() {
            let src = read_zmm(self, instr.src());
            Ok(unsafe { src.zmm32f[0] })
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
            Ok(unsafe { src.zmm64f[0] })
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
        let rc = imm8 & 0x3; // rounding control from bits [1:0]
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32f[i] = round_f32(src.zmm32f[i], rc);
            }
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
        let rc = imm8 & 0x3;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64f[i] = round_f64(src.zmm64f[i], rc);
            }
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
        let rc = imm8 & 0x3;
        let rounded = round_f32(src_val, rc);

        // Start with src1 (vvvv) to preserve upper elements
        let mut result = read_zmm(self, instr.src1());
        unsafe {
            result.zmm32f[0] = rounded;
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        // Scalar: only element 0 is masked; elements 1..3 always from src1
        if (mask & 1) == 0 {
            if zmask {
                unsafe { result.zmm32u[0] = 0; }
            } else {
                // Merge: keep original dst element 0
                let orig = read_zmm(self, instr.dst());
                unsafe { result.zmm32u[0] = orig.zmm32u[0]; }
            }
        }
        // Zero upper 256 bits (EVEX scalar zeroes upper)
        unsafe {
            for i in 4..16 {
                result.zmm32u[i] = 0;
            }
        }
        unsafe { self.vmm[instr.dst() as usize] = result; }
        Ok(())
    }

    // ========================================================================
    // VRNDSCALESD — Round scalar double-precision, EVEX.66.0F3A.W1 0B
    // ========================================================================

    /// VRNDSCALESD Vsd{k}, Hsd, Wsd, imm8
    pub fn evex_vrndscalesd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src_val = self.read_scalar_sd(instr)?;
        let imm8 = instr.ib();
        let rc = imm8 & 0x3;
        let rounded = round_f64(src_val, rc);

        let mut result = read_zmm(self, instr.src1());
        unsafe {
            result.zmm64f[0] = rounded;
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        if (mask & 1) == 0 {
            if zmask {
                unsafe { result.zmm64u[0] = 0; }
            } else {
                let orig = read_zmm(self, instr.dst());
                unsafe { result.zmm64u[0] = orig.zmm64u[0]; }
            }
        }
        unsafe {
            for i in 2..8 {
                result.zmm64u[i] = 0;
            }
        }
        unsafe { self.vmm[instr.dst() as usize] = result; }
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
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32f[i] = scalef_f32(src1.zmm32f[i], src2.zmm32f[i]);
            }
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
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64f[i] = scalef_f64(src1.zmm64f[i], src2.zmm64f[i]);
            }
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
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32f[i] = getexp_f32(src.zmm32f[i]);
            }
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
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64f[i] = getexp_f64(src.zmm64f[i]);
            }
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
        let interval = imm8 & 0x3;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32f[i] = getmant_f32(src.zmm32f[i], interval);
            }
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
        let interval = imm8 & 0x3;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64f[i] = getmant_f64(src.zmm64f[i], interval);
            }
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
fn getmant_f32(val: f32, interval: u8) -> f32 {
    let bits = val.to_bits();
    let sign = bits & 0x8000_0000;
    let exp_field = (bits >> 23) & 0xFF;
    let mantissa = bits & 0x007F_FFFF;

    // Special cases
    if exp_field == 0xFF {
        // Inf -> 1.0 (with sign), NaN -> NaN
        if mantissa == 0 {
            return f32::from_bits(sign | 0x3F80_0000); // +/- 1.0
        } else {
            return f32::NAN;
        }
    }
    if exp_field == 0 && mantissa == 0 {
        // Zero -> zero (with sign)
        return val;
    }

    match interval {
        0 | 1 => {
            // [1, 2): set exponent to 127 (biased), keep mantissa bits
            if exp_field == 0 {
                // Denormal: normalize first
                let normalized = val.abs();
                let nbits = normalized.to_bits();
                let nmant = nbits & 0x007F_FFFF;
                f32::from_bits(sign | (127 << 23) | nmant)
            } else {
                f32::from_bits(sign | (127 << 23) | mantissa)
            }
        }
        2 => {
            // (0.5, 1]: set exponent to 126, keep mantissa
            if exp_field == 0 {
                let normalized = val.abs();
                let nbits = normalized.to_bits();
                let nmant = nbits & 0x007F_FFFF;
                f32::from_bits(sign | (126 << 23) | nmant)
            } else {
                f32::from_bits(sign | (126 << 23) | mantissa)
            }
        }
        3 | _ => {
            // [0.75, 1.5): if mantissa >= 0.5 (bit 22 set), use [0.5,1) form (exp=126),
            // otherwise use [1,2) form (exp=127)
            let target_exp = if (mantissa & 0x0040_0000) != 0 { 126u32 } else { 127u32 };
            if exp_field == 0 {
                let normalized = val.abs();
                let nbits = normalized.to_bits();
                let nmant = nbits & 0x007F_FFFF;
                f32::from_bits(sign | (target_exp << 23) | nmant)
            } else {
                f32::from_bits(sign | (target_exp << 23) | mantissa)
            }
        }
    }
}

/// Get normalized mantissa of f64.
#[inline]
fn getmant_f64(val: f64, interval: u8) -> f64 {
    let bits = val.to_bits();
    let sign = bits & 0x8000_0000_0000_0000;
    let exp_field = (bits >> 52) & 0x7FF;
    let mantissa = bits & 0x000F_FFFF_FFFF_FFFF;

    if exp_field == 0x7FF {
        if mantissa == 0 {
            return f64::from_bits(sign | 0x3FF0_0000_0000_0000); // +/- 1.0
        } else {
            return f64::NAN;
        }
    }
    if exp_field == 0 && mantissa == 0 {
        return val;
    }

    match interval {
        0 | 1 => {
            // [1, 2): exponent = 1023
            if exp_field == 0 {
                let normalized = val.abs();
                let nbits = normalized.to_bits();
                let nmant = nbits & 0x000F_FFFF_FFFF_FFFF;
                f64::from_bits(sign | (1023u64 << 52) | nmant)
            } else {
                f64::from_bits(sign | (1023u64 << 52) | mantissa)
            }
        }
        2 => {
            // (0.5, 1]: exponent = 1022
            if exp_field == 0 {
                let normalized = val.abs();
                let nbits = normalized.to_bits();
                let nmant = nbits & 0x000F_FFFF_FFFF_FFFF;
                f64::from_bits(sign | (1022u64 << 52) | nmant)
            } else {
                f64::from_bits(sign | (1022u64 << 52) | mantissa)
            }
        }
        3 | _ => {
            let target_exp = if (mantissa & 0x0008_0000_0000_0000) != 0 { 1022u64 } else { 1023u64 };
            if exp_field == 0 {
                let normalized = val.abs();
                let nbits = normalized.to_bits();
                let nmant = nbits & 0x000F_FFFF_FFFF_FFFF;
                f64::from_bits(sign | (target_exp << 52) | nmant)
            } else {
                f64::from_bits(sign | (target_exp << 52) | mantissa)
            }
        }
    }
}
