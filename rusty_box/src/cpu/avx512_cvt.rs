#![allow(unused_unsafe, dead_code)]

//! AVX-512F floating-point conversion instruction handlers
//!
//! Implements packed integer <-> floating-point conversions with EVEX opmask support.
//! Handlers work for 128/256/512-bit via `get_vl()` (EVEX.L'L field).
//!
//! Mirrors Bochs `cpu/avx/avx512_cvt.cc`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedZmmRegister,
};

// ============================================================================
// Helper functions (duplicated from avx512.rs — module-private there)
// ============================================================================

/// Number of 32-bit elements per vector length: VL0=4, VL1=8, VL2=16
#[inline]
fn dword_elements(vl: u8) -> usize {
    match vl {
        0 => 4,   // 128-bit
        1 => 8,   // 256-bit
        _ => 16,  // 512-bit
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

/// Byte size for vector length: VL0=16, VL1=32, VL2=64
#[inline]
fn vl_bytes(vl: u8) -> usize {
    match vl {
        0 => 16,
        1 => 32,
        _ => 64,
    }
}

/// Read opmask value for masking. k0 returns all-ones (no masking).
#[inline]
fn read_opmask_for_write<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, instr: &Instruction) -> u64 {
    let k = instr.opmask();
    if k == 0 {
        u64::MAX // k0 = all elements active
    } else {
        // SAFETY: opmask register union always valid for rrx (full 64-bit) access
        unsafe { cpu.opmask[k as usize].rrx() }
    }
}

/// Read ZMM register as a ZMM-width value
#[inline]
fn read_zmm<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, reg: u8) -> BxPackedZmmRegister {
    cpu.vmm[reg as usize]
}

/// Write ZMM register with dword-granularity masking, zeroing upper bits beyond VL
fn write_zmm_masked<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
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
        // else: merge masking — keep original value
    }
    // Zero upper elements beyond VL (EVEX always clears upper)
    for i in nelements..16 {
        dst.set_zmm32u(i, 0);
    }
}

/// Write ZMM register with qword-granularity masking
fn write_zmm_masked_q<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
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
    // Zero upper elements beyond VL
    for i in nelements..8 {
        dst.set_zmm64u(i, 0);
    }
}

/// Read source as dword vector from register or memory
fn read_src_dword<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    nelements: usize,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src()))
    } else {
        let mut tmp = BxPackedZmmRegister::default();
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        for i in 0..nelements {
            let val = cpu.v_read_dword(seg, laddr + (i * 4) as u64)?;
            tmp.set_zmm32u(i, val);
        }
        Ok(tmp)
    }
}

/// Read source as qword vector from register or memory
fn read_src_qword<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    nelements: usize,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src()))
    } else {
        let mut tmp = BxPackedZmmRegister::default();
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        for i in 0..nelements {
            let lo = cpu.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
            let hi = cpu.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
            tmp.set_zmm64u(i, lo | (hi << 32));
        }
        Ok(tmp)
    }
}

/// Round an f32 to nearest integer as i32, matching MXCSR rounding mode.
/// MXCSR RC: 0=nearest, 1=down, 2=up, 3=truncate
/// Intel integer indefinite: 0x80000000 for ALL out-of-range signed conversions
/// (positive overflow, negative overflow, NaN). Matches Bochs SoftFloat behavior.
const I32_INDEFINITE: i32 = i32::MIN; // 0x80000000

/// Intel integer indefinite: 0xFFFFFFFF for ALL out-of-range unsigned conversions.
const U32_INDEFINITE: u32 = u32::MAX; // 0xFFFFFFFF

fn round_f32_to_i32(val: f32, rc: u8) -> i32 {
    if val.is_nan() { return I32_INDEFINITE; }
    let rounded = match rc {
        0 => val.round_ties_even(),
        1 => val.floor(),
        2 => val.ceil(),
        _ => val.trunc(),
    };
    // Check overflow BEFORE cast (Rust saturates, Intel returns 0x80000000)
    if rounded >= (i32::MAX as f32 + 1.0) || rounded < (i32::MIN as f32) {
        I32_INDEFINITE
    } else {
        rounded as i32
    }
}

#[inline]
fn round_f64_to_i32(val: f64, rc: u8) -> i32 {
    if val.is_nan() { return I32_INDEFINITE; }
    let rounded = match rc {
        0 => val.round_ties_even(),
        1 => val.floor(),
        2 => val.ceil(),
        _ => val.trunc(),
    };
    if rounded >= (i32::MAX as f64 + 1.0) || rounded < (i32::MIN as f64) {
        I32_INDEFINITE
    } else {
        rounded as i32
    }
}

/// Intel: ALL invalid unsigned conversions (NaN, negative, overflow) return 0xFFFFFFFF.
#[inline]
fn round_f32_to_u32(val: f32, rc: u8) -> u32 {
    if val.is_nan() { return U32_INDEFINITE; }
    let rounded = match rc {
        0 => val.round_ties_even(),
        1 => val.floor(),
        2 => val.ceil(),
        _ => val.trunc(),
    };
    if rounded < 0.0 || rounded >= (u32::MAX as f32 + 1.0) {
        U32_INDEFINITE
    } else {
        rounded as u32
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // VCVTDQ2PS — Convert packed signed dwords to SP FP
    // EVEX.0F.W0 5B /r
    // ========================================================================

    /// VCVTDQ2PS Vps{k}, Wdq — convert packed signed int32 to float32
    pub fn evex_vcvtdq2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32f(i, src.zmm32s(i) as f32);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTPS2DQ — Convert SP FP to packed signed dwords (round per MXCSR)
    // EVEX.66.0F.W0 5B /r
    // ========================================================================

    /// VCVTPS2DQ Vdq{k}, Wps — convert float32 to signed int32 (MXCSR rounding)
    pub fn evex_vcvtps2dq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let rc = self.mxcsr.rounding_mode();
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32s(i, round_f32_to_i32(src.zmm32f(i), rc));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTTPS2DQ — Convert SP FP to packed signed dwords (truncate)
    // EVEX.F3.0F.W0 5B /r
    // ========================================================================

    /// VCVTTPS2DQ Vdq{k}, Wps — convert float32 to signed int32 (truncation)
    pub fn evex_vcvttps2dq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let v = src.zmm32f(i);
            result.set_zmm32s(i, if v.is_nan() || v >= (i32::MAX as f32 + 1.0) || v < (i32::MIN as f32) {
                I32_INDEFINITE
            } else {
                v as i32
            });
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTDQ2PD — Convert packed signed dwords to DP FP
    // EVEX.F3.0F.W0 E6 /r
    // Source is half width: VL=128 reads 2 dwords (64 bits),
    //                       VL=256 reads 4 dwords (128 bits),
    //                       VL=512 reads 8 dwords (256 bits).
    // ========================================================================

    /// VCVTDQ2PD Vpd{k}, Wdq — convert packed signed int32 to float64
    pub fn evex_vcvtdq2pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl); // number of output qword elements
        // Source is half the width: nelements dwords
        let src = read_src_dword(self, instr, nelements)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64f(i, src.zmm32s(i) as f64);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTPD2DQ — Convert DP FP to packed signed dwords (round per MXCSR)
    // EVEX.F2.0F.W1 E6 /r
    // Output is half width: VL=128 writes 2 dwords (zero upper),
    //                       VL=256 writes 4 dwords (zero upper),
    //                       VL=512 writes 8 dwords (zero upper).
    // ========================================================================

    /// VCVTPD2DQ Vdq{k}, Wpd — convert float64 to signed int32 (MXCSR rounding)
    pub fn evex_vcvtpd2dq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl); // number of input qword elements
        let src = read_src_qword(self, instr, nelements)?;
        let rc = self.mxcsr.rounding_mode();
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32s(i, round_f64_to_i32(src.zmm64f(i), rc));
        }
        // Output is dword-masked but only nelements dwords are active;
        // upper dword slots (beyond nelements) are zeroed by write_zmm_masked
        // because we use the full VL for zeroing.
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        // Write with dword masking. The result vector has nelements active dwords
        // in the lower half and zeros in the upper half. We write at half VL
        // (the output width) and zero everything beyond that.
        let out_vl = match vl {
            0 => 0, // 128-bit input -> 64-bit output, but XMM min = 128-bit dest
            1 => 0, // 256-bit input -> 128-bit output
            _ => 1, // 512-bit input -> 256-bit output
        };
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, out_vl);
        Ok(())
    }

    // ========================================================================
    // VCVTTPD2DQ — Convert DP FP to packed signed dwords (truncate)
    // EVEX.66.0F.W1 E6 /r
    // ========================================================================

    /// VCVTTPD2DQ Vdq{k}, Wpd — convert float64 to signed int32 (truncation)
    pub fn evex_vcvttpd2dq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_src_qword(self, instr, nelements)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let v = src.zmm64f(i);
            result.set_zmm32s(i, if v.is_nan() || v >= (i32::MAX as f64 + 1.0) || v < (i32::MIN as f64) {
                I32_INDEFINITE
            } else {
                v as i32
            });
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        let out_vl = match vl {
            0 => 0,
            1 => 0,
            _ => 1,
        };
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, out_vl);
        Ok(())
    }

    // ========================================================================
    // VCVTPS2PD — Convert SP FP to DP FP
    // EVEX.0F.W0 5A /r
    // Source is half width (same as VCVTDQ2PD).
    // ========================================================================

    /// VCVTPS2PD Vpd{k}, Wps — convert float32 to float64
    pub fn evex_vcvtps2pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl); // output qword count
        // Source is half width: nelements dwords (float32)
        let src = read_src_dword(self, instr, nelements)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64f(i, src.zmm32f(i) as f64);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTPD2PS — Convert DP FP to SP FP
    // EVEX.66.0F.W1 5A /r
    // Output is half width (same as VCVTPD2DQ).
    // ========================================================================

    /// VCVTPD2PS Vps{k}, Wpd — convert float64 to float32
    pub fn evex_vcvtpd2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl); // input qword count
        let src = read_src_qword(self, instr, nelements)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32f(i, src.zmm64f(i) as f32);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        let out_vl = match vl {
            0 => 0,
            1 => 0,
            _ => 1,
        };
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, out_vl);
        Ok(())
    }

    // ========================================================================
    // VCVTUDQ2PS — Convert packed unsigned dwords to SP FP
    // EVEX.F2.0F.W0 7A /r
    // ========================================================================

    /// VCVTUDQ2PS Vps{k}, Wdq — convert packed unsigned int32 to float32
    pub fn evex_vcvtudq2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32f(i, src.zmm32u(i) as f32);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTPS2UDQ — Convert SP FP to packed unsigned dwords (round per MXCSR)
    // EVEX.0F.W0 79 /r
    // ========================================================================

    /// VCVTPS2UDQ Vdq{k}, Wps — convert float32 to unsigned int32 (MXCSR rounding)
    pub fn evex_vcvtps2udq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let rc = self.mxcsr.rounding_mode();
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, round_f32_to_u32(src.zmm32f(i), rc));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTTPS2UDQ — Convert SP FP to packed unsigned dwords (truncate)
    // EVEX.0F.W0 78 /r
    // ========================================================================

    /// VCVTTPS2UDQ Vdq{k}, Wps — convert float32 to unsigned int32 (truncation)
    pub fn evex_vcvttps2udq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let val = src.zmm32f(i);
            result.set_zmm32u(i, if val.is_nan() || val < 0.0 || val >= (u32::MAX as f32 + 1.0) {
                U32_INDEFINITE
            } else {
                val as u32
            });
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}
