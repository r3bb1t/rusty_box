
//! AVX-512F floating-point conversion instruction handlers
//!
//! Implements packed integer <-> floating-point conversions with EVEX opmask support.
//! Handlers work for 128/256/512-bit via `get_vl()` (EVEX.L'L field).
//!
//! Mirrors Bochs `cpu/avx/avx512_cvt.cc`.

use super::softfloat3e::f32_to_f64::f32_to_f64;
use super::softfloat3e::f32_to_int::{
    f32_to_i32, f32_to_i32_r_min_mag, f32_to_i64, f32_to_i64_r_min_mag,
};
use super::softfloat3e::f64_to_f32::f64_to_f32;
use super::softfloat3e::f64_to_int::{
    f64_to_i32, f64_to_i32_r_min_mag, f64_to_i64, f64_to_i64_r_min_mag,
};
use super::softfloat3e::int_to_float::{i32_to_f32, i32_to_f64, i64_to_f32, i64_to_f64};
use super::softfloat3e::uint64_convert::{
    f32_to_ui64, f32_to_ui64_r_min_mag, f64_to_ui64, f64_to_ui64_r_min_mag, ui64_to_f32,
    ui64_to_f64,
};
use super::softfloat3e::softfloat::{softfloat_get_exception_flags, softfloat_get_rounding_mode};
use super::softfloat3e::uint_convert::{
    f32_to_ui32, f32_to_ui32_r_min_mag, f64_to_ui32, f64_to_ui32_r_min_mag, ui32_to_f32,
    ui32_to_f64,
};
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

// ============================================================================
// Helper functions (duplicated from avx512.rs — module-private there)
// ============================================================================

/// Number of 32-bit elements per vector length: VL0=4, VL1=8, VL2=16
#[inline]
fn dword_elements(vl: u8) -> usize {
    match vl {
        0 => 4,  // 128-bit
        1 => 8,  // 256-bit
        _ => 16, // 512-bit
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
        u64::MAX // k0 = all elements active
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

/// Write ZMM register with dword-granularity masking, zeroing upper bits beyond VL
fn write_zmm_masked<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    write_zmm_masked_n(cpu, reg, result, mask, zero_masking, dword_elements(vl));
}

/// Write exactly `nelements` dwords with masking, then zero everything above.
///
/// The half-width-output converts (VCVTPD2DQ, VCVTPD2PS, VCVTPD2UDQ,
/// VCVTQQ2PS …) produce as many dwords as they consumed qwords, which is *half*
/// a full vector of dwords. Deriving the count from an output VL instead
/// overshoots at VL128, where 2 qwords yield 2 dwords but `dword_elements(0)`
/// is 4: Bochs writes that case with `BX_WRITE_XMM_REG_LO_QWORD_CLEAR_HIGH`,
/// clearing dwords 2 and 3 outright rather than leaving them to the opmask, so
/// under merge masking with those bits clear the destination must not keep its
/// old contents.
fn write_zmm_masked_n<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    nelements: usize,
) {
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
    // Zero upper elements beyond VL
    for i in nelements..8 {
        dst.set_zmm64u(i, 0);
    }
}

/// Read source as dword vector from register or memory
fn read_src_dword<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    _nelements: usize,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src()))
    } else {
        cpu.evex_load_bcst_d_pair(instr)
    }
}

/// Read source as a *half*-width dword vector — for the widening converts
/// (VCVTDQ2PD, VCVTPS2PD), whose def entries pair
/// `LOAD_BROADCAST_Half_VectorD` with `LOAD_BROADCAST_MASK_Half_VectorD`
/// because the source holds half as many elements as the destination.
fn read_src_half_dword<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src()))
    } else {
        cpu.evex_load_bcst_half_d_pair(instr)
    }
}

/// Read source as qword vector from register or memory. Callers (VCVTPD2DQ,
/// VCVTTPD2DQ, VCVTPD2PS) pair `LOAD_BROADCAST_VectorQ` with
/// `LOAD_BROADCAST_MASK_VectorQ`.
fn read_src_qword<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    _nelements: usize,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src()))
    } else {
        cpu.evex_load_bcst_q_pair(instr)
    }
}

/// Round an f32 to nearest integer as i32, matching MXCSR rounding mode.
/// MXCSR RC: 0=nearest, 1=down, 2=up, 3=truncate
impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // VCVTDQ2PS — Convert packed signed dwords to SP FP
    // EVEX.0F.W0 5B /r
    // ========================================================================

    /// VCVTDQ2PS Vps{k}, Wdq — convert packed signed int32 to Float32
    pub fn evex_vcvtdq2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, i32_to_f32(src.zmm32s(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTPS2DQ — Convert SP FP to packed signed dwords (round per MXCSR)
    // EVEX.66.0F.W0 5B /r
    // ========================================================================

    /// VCVTPS2DQ Vdq{k}, Wps — convert Float32 to signed int32 (MXCSR rounding)
    pub fn evex_vcvtps2dq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_get_rounding_mode(&status);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32s(i, f32_to_i32(src.zmm32u(i), rc, true, &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTTPS2DQ — Convert SP FP to packed signed dwords (truncate)
    // EVEX.F3.0F.W0 5B /r
    // ========================================================================

    /// VCVTTPS2DQ Vdq{k}, Wps — convert Float32 to signed int32 (truncation)
    pub fn evex_vcvttps2dq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32s(
                    i,
                    f32_to_i32_r_min_mag(src.zmm32u(i), true, false, &mut status),
                );
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
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

    /// VCVTDQ2PD Vpd{k}, Wdq — convert packed signed int32 to Float64
    pub fn evex_vcvtdq2pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl); // number of output qword elements
                                            // Source is half the width: nelements dwords
        let src = read_src_half_dword(self, instr)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, i32_to_f64(src.zmm32s(i)));
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

    /// VCVTPD2DQ Vdq{k}, Wpd — convert Float64 to signed int32 (MXCSR rounding)
    pub fn evex_vcvtpd2dq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl); // number of input qword elements
        let src = read_src_qword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_get_rounding_mode(&status);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32s(i, f64_to_i32(src.zmm64u(i), rc, true, &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        // Output is dword-masked but only nelements dwords are active;
        // upper dword slots (beyond nelements) are zeroed by write_zmm_masked
        // because we use the full VL for zeroing.
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        // Write with dword masking. The result vector has nelements active dwords
        // in the lower half and zeros in the upper half. We write at half VL
        // (the output width) and zero everything beyond that.
        write_zmm_masked_n(self, instr.dst(), &result, mask, zmask, nelements);
        Ok(())
    }

    // ========================================================================
    // VCVTTPD2DQ — Convert DP FP to packed signed dwords (truncate)
    // EVEX.66.0F.W1 E6 /r
    // ========================================================================

    /// VCVTTPD2DQ Vdq{k}, Wpd — convert Float64 to signed int32 (truncation)
    pub fn evex_vcvttpd2dq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_src_qword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32s(
                    i,
                    f64_to_i32_r_min_mag(src.zmm64u(i), true, false, &mut status),
                );
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_n(self, instr.dst(), &result, mask, zmask, nelements);
        Ok(())
    }

    // ========================================================================
    // VCVTPS2PD — Convert SP FP to DP FP
    // EVEX.0F.W0 5A /r
    // Source is half width (same as VCVTDQ2PD).
    // ========================================================================

    /// VCVTPS2PD Vpd{k}, Wps — convert Float32 to Float64
    pub fn evex_vcvtps2pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl); // output qword count
                                            // Source is half width: nelements dwords (Float32)
        let src = read_src_half_dword(self, instr)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm64u(i, f32_to_f64(src.zmm32u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTPD2PS — Convert DP FP to SP FP
    // EVEX.66.0F.W1 5A /r
    // Output is half width (same as VCVTPD2DQ).
    // ========================================================================

    /// VCVTPD2PS Vps{k}, Wpd — convert Float64 to Float32
    pub fn evex_vcvtpd2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl); // input qword count
        let src = read_src_qword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, f64_to_f32(src.zmm64u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_n(self, instr.dst(), &result, mask, zmask, nelements);
        Ok(())
    }

    // ========================================================================
    // VCVTUDQ2PS — Convert packed unsigned dwords to SP FP
    // EVEX.F2.0F.W0 7A /r
    // ========================================================================

    /// VCVTUDQ2PS Vps{k}, Wdq — convert packed unsigned int32 to Float32
    pub fn evex_vcvtudq2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, ui32_to_f32(src.zmm32u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTPS2UDQ — Convert SP FP to packed unsigned dwords (round per MXCSR)
    // EVEX.0F.W0 79 /r
    // ========================================================================

    /// VCVTPS2UDQ Vdq{k}, Wps — convert Float32 to unsigned int32 (MXCSR rounding)
    pub fn evex_vcvtps2udq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_get_rounding_mode(&status);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, f32_to_ui32(src.zmm32u(i), rc, true, &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTTPS2UDQ — Convert SP FP to packed unsigned dwords (truncate)
    // EVEX.0F.W0 78 /r
    // ========================================================================

    /// VCVTTPS2UDQ Vdq{k}, Wps — convert Float32 to unsigned int32 (truncation)
    pub fn evex_vcvttps2udq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_src_dword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(
                    i,
                    f32_to_ui32_r_min_mag(src.zmm32u(i), true, false, &mut status),
                );
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VCVTPD2UDQ / VCVTTPD2UDQ — packed double to unsigned dword.
    // Same half-width output shape as VCVTPD2DQ: n qword inputs produce n
    // dwords in the low half, and everything above is zeroed.
    // ========================================================================

    /// The shared body; `truncate` selects round-toward-zero over MXCSR.RC.
    fn evex_cvt_pd2udq(&mut self, instr: &Instruction, truncate: bool) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_src_qword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_get_rounding_mode(&status);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                let v = if truncate {
                    f64_to_ui32_r_min_mag(src.zmm64u(i), true, false, &mut status)
                } else {
                    f64_to_ui32(src.zmm64u(i), rc, true, &mut status)
                };
                result.set_zmm32u(i, v);
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_n(self, instr.dst(), &result, mask, zmask, nelements);
        Ok(())
    }

    /// VCVTPD2UDQ Vdq{k}, Wpd — EVEX.0F.W1 79 (MXCSR rounding)
    pub fn evex_vcvtpd2udq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_pd2udq(instr, false)
    }

    /// VCVTTPD2UDQ Vdq{k}, Wpd — EVEX.0F.W1 78 (truncate)
    pub fn evex_vcvttpd2udq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_pd2udq(instr, true)
    }
    // ========================================================================
    // AVX512_DQ qword <-> floating-point.
    //
    // Four shapes, all from Bochs avx512_cvt.cc:
    //   64 -> 64  AVX512_CVT64_TO_64      full width, qword in and out
    //   64 -> 32  AVX512_CVT64_TO_32_MASK half-width output
    //   32 -> 64  AVX512_CVT32_TO_64_MASK half-width input
    // ========================================================================

    /// VCVTPD2QQ / VCVTTPD2QQ / VCVTPD2UQQ / VCVTTPD2UQQ — packed double to
    /// packed qword. `truncate` selects round-toward-zero over MXCSR.RC.
    /// Bochs avx512_cvt.cc `AVX512_CVT64_TO_64`.
    fn evex_cvt_pd2qq(
        &mut self,
        instr: &Instruction,
        unsigned: bool,
        truncate: bool,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_src_qword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_get_rounding_mode(&status);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                let a = src.zmm64u(i);
                let v = match (unsigned, truncate) {
                    (true, true) => f64_to_ui64_r_min_mag(a, true, false, &mut status),
                    (true, false) => f64_to_ui64(a, rc, true, &mut status),
                    (false, true) => f64_to_i64_r_min_mag(a, true, false, &mut status) as u64,
                    (false, false) => f64_to_i64(a, rc, true, &mut status) as u64,
                };
                result.set_zmm64u(i, v);
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VCVTPD2QQ Vdq{k}, Wpd — EVEX.66.0F.W1 7B
    pub fn evex_vcvtpd2qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_pd2qq(instr, false, false)
    }

    /// VCVTTPD2QQ Vdq{k}, Wpd — EVEX.66.0F.W1 7A
    pub fn evex_vcvttpd2qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_pd2qq(instr, false, true)
    }

    /// VCVTPD2UQQ Vdq{k}, Wpd — EVEX.66.0F.W1 79
    pub fn evex_vcvtpd2uqq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_pd2qq(instr, true, false)
    }

    /// VCVTTPD2UQQ Vdq{k}, Wpd — EVEX.66.0F.W1 78
    pub fn evex_vcvttpd2uqq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_pd2qq(instr, true, true)
    }

    /// VCVTQQ2PD / VCVTUQQ2PD — packed qword to packed double.
    fn evex_cvt_qq2pd(&mut self, instr: &Instruction, unsigned: bool) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_src_qword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                let v = if unsigned {
                    ui64_to_f64(src.zmm64u(i), &mut status)
                } else {
                    i64_to_f64(src.zmm64s(i), &mut status)
                };
                result.set_zmm64u(i, v);
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VCVTQQ2PD Vpd{k}, Wdq — EVEX.F3.0F.W1 E6
    pub fn evex_vcvtqq2pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_qq2pd(instr, false)
    }

    /// VCVTUQQ2PD Vpd{k}, Wdq — EVEX.F3.0F.W1 7A
    pub fn evex_vcvtuqq2pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_qq2pd(instr, true)
    }

    /// VCVTQQ2PS / VCVTUQQ2PS — packed qword to packed single. Half-width
    /// output: n qwords in, n dwords out, everything above cleared.
    fn evex_cvt_qq2ps(&mut self, instr: &Instruction, unsigned: bool) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_src_qword(self, instr, nelements)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                let v = if unsigned {
                    ui64_to_f32(src.zmm64u(i), &mut status)
                } else {
                    i64_to_f32(src.zmm64s(i), &mut status)
                };
                result.set_zmm32u(i, v);
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_n(self, instr.dst(), &result, mask, zmask, nelements);
        Ok(())
    }

    /// VCVTQQ2PS Vps{k}, Wdq — EVEX.0F.W1 5B
    pub fn evex_vcvtqq2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_qq2ps(instr, false)
    }

    /// VCVTUQQ2PS Vps{k}, Wdq — EVEX.F2.0F.W1 7A
    pub fn evex_vcvtuqq2ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_qq2ps(instr, true)
    }

    /// VCVTPS2QQ / VCVTTPS2QQ / VCVTPS2UQQ / VCVTTPS2UQQ — packed single to
    /// packed qword. Half-width input: the source holds n dwords for n qword
    /// results, so it uses the `Half_VectorD` loader.
    fn evex_cvt_ps2qq(
        &mut self,
        instr: &Instruction,
        unsigned: bool,
        truncate: bool,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_src_half_dword(self, instr)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let rc = softfloat_get_rounding_mode(&status);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                let a = src.zmm32u(i);
                let v = match (unsigned, truncate) {
                    (true, true) => f32_to_ui64_r_min_mag(a, true, false, &mut status),
                    (true, false) => f32_to_ui64(a, rc, true, &mut status),
                    (false, true) => f32_to_i64_r_min_mag(a, true, false, &mut status) as u64,
                    (false, false) => f32_to_i64(a, rc, true, &mut status) as u64,
                };
                result.set_zmm64u(i, v);
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VCVTPS2QQ Vdq{k}, Wps — EVEX.66.0F.W0 7B
    pub fn evex_vcvtps2qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_ps2qq(instr, false, false)
    }

    /// VCVTTPS2QQ Vdq{k}, Wps — EVEX.66.0F.W0 7A
    pub fn evex_vcvttps2qq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_ps2qq(instr, false, true)
    }

    /// VCVTPS2UQQ Vdq{k}, Wps — EVEX.66.0F.W0 79
    pub fn evex_vcvtps2uqq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_ps2qq(instr, true, false)
    }

    /// VCVTTPS2UQQ Vdq{k}, Wps — EVEX.66.0F.W0 78
    pub fn evex_vcvttps2uqq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cvt_ps2qq(instr, true, true)
    }

    /// VCVTUDQ2PD Vpd{k}, Wdq — EVEX.F3.0F.W0 7A. The unsigned twin of
    /// VCVTDQ2PD; exact for every u32, so it raises nothing and needs no
    /// status word.
    pub fn evex_vcvtudq2pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_src_half_dword(self, instr)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, ui32_to_f64(src.zmm32u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::super::softfloat3e::softfloat::SoftFloatStatus;
    use super::*;

    // Boundary semantics of the truncating (VCVTT*) conversion path the
    // vcvttps2dq / vcvttpd2dq / vcvttps2udq handlers delegate to, now that
    // it is Bochs softfloat3e f{32,64}_to_{i,ui}32_r_minMag itself: truncate
    // first, then let the integer-range check decide indefinite.

    fn t_f64_i32(v: f64) -> i32 {
        let mut st = SoftFloatStatus::default();
        f64_to_i32_r_min_mag(v.to_bits(), true, false, &mut st)
    }
    fn t_f32_i32(v: f32) -> i32 {
        let mut st = SoftFloatStatus::default();
        f32_to_i32_r_min_mag(v.to_bits(), true, false, &mut st)
    }
    fn t_f32_u32(v: f32) -> u32 {
        let mut st = SoftFloatStatus::default();
        f32_to_ui32_r_min_mag(v.to_bits(), true, false, &mut st)
    }

    #[test]
    fn vcvttpd2dq_negative_boundary_truncates_to_i32_min() {
        // -2147483648.9 truncates to exactly -2^31, which IS representable;
        // a raw-value range check would return integer-indefinite here.
        assert_eq!(t_f64_i32(-2147483648.9), i32::MIN);
        // Just past the negative edge: truncates to -2^31-1, out of range.
        assert_eq!(t_f64_i32(-2147483649.0), i32::MIN); // integer indefinite
        // Positive: 2^31 does not fit a signed i32 → indefinite.
        assert_eq!(t_f64_i32(2147483648.0), i32::MIN);
        // Largest in-range value, fractional part truncated away.
        assert_eq!(t_f64_i32(2147483647.9), i32::MAX);
        assert_eq!(t_f64_i32(f64::NAN), i32::MIN);
    }

    #[test]
    fn vcvttps2dq_boundary_matches_f32_grid() {
        // No f32 exists strictly between -2^31-256 and -2^31, so the exact
        // edge value truncates to i32::MIN and the next lower f32 overflows.
        assert_eq!(t_f32_i32(-2147483648.0), i32::MIN);
        assert_eq!(t_f32_i32(-2147483904.0), i32::MIN); // integer indefinite
        assert_eq!(t_f32_i32(f32::NAN), i32::MIN);
    }

    #[test]
    fn vcvttps2udq_small_negative_truncates_to_zero() {
        // Magnitude < 1 truncates to 0 (valid unsigned); Bochs returns 0
        // before its sign test.
        assert_eq!(t_f32_u32(-0.9), 0);
        assert_eq!(t_f32_u32(0.9), 0);
        // At/under -1 the truncated magnitude is negative → indefinite.
        assert_eq!(t_f32_u32(-1.5), u32::MAX);
        // Full unsigned range: 2^32 does not fit → indefinite.
        assert_eq!(t_f32_u32(4294967296.0), u32::MAX);
    }

    // ---- AVX512_DQ qword <-> float, driven through the dispatcher ----------

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::BxSegregs;
    use crate::cpu::xmm::MXCSR_RESET;
    use rusty_box_decoder::opcode::Opcode;

    /// Register-form EVEX convert: dst=0, src=1, opmask `k` (0 = unmasked).
    fn evex_cvt(opcode: Opcode, vl: u8, k: u8, zero_masking: bool) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0);
        i.set_src_reg(1, 1);
        i.set_opmask(k);
        i.set_zero_masking(zero_masking as u8);
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
    fn qword_to_double_and_back_round_trips_both_signednesses() {
        let mut c = cpu();
        // -3 as signed is 3 below zero; as unsigned it is 2^64-3, which is
        // not representable exactly and rounds to 2^64.
        c.vmm[1].set_zmm64u(0, (-3i64) as u64);
        c.vmm[1].set_zmm64u(1, 5);

        c.execute_instruction(&evex_cvt(Opcode::EvexVcvtqq2pdVpdWdq, 0, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), (-3.0f64).to_bits());
        assert_eq!(c.vmm[0].zmm64u(1), 5.0f64.to_bits());

        c.execute_instruction(&evex_cvt(Opcode::EvexVcvtuqq2pdVpdWdq, 0, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 18446744073709551616.0f64.to_bits());
        assert_eq!(c.vmm[0].zmm64u(1), 5.0f64.to_bits());

        // Back the other way, truncating.
        c.vmm[1].set_zmm64u(0, (-3.9f64).to_bits());
        c.vmm[1].set_zmm64u(1, 5.9f64.to_bits());
        c.execute_instruction(&evex_cvt(Opcode::EvexVcvttpd2qqVdqWpd, 0, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), (-3i64) as u64);
        assert_eq!(c.vmm[0].zmm64u(1), 5);

        // Unsigned truncation of a negative value that truncates to zero is
        // legal; one that does not is the integer indefinite value.
        c.vmm[1].set_zmm64u(0, (-0.5f64).to_bits());
        c.vmm[1].set_zmm64u(1, (-3.9f64).to_bits());
        c.execute_instruction(&evex_cvt(Opcode::EvexVcvttpd2uqqVdqWpd, 0, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 0);
        assert_eq!(c.vmm[0].zmm64u(1), u64::MAX);
    }

    #[test]
    fn pd2qq_rounds_by_mxcsr_while_tpd2qq_truncates() {
        let mut c = cpu();
        c.vmm[1].set_zmm64u(0, 2.5f64.to_bits());
        c.vmm[1].set_zmm64u(1, 3.5f64.to_bits());

        // Default MXCSR.RC is round-to-nearest-even: 2.5 -> 2, 3.5 -> 4.
        c.execute_instruction(&evex_cvt(Opcode::EvexVcvtpd2qqVdqWpd, 0, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 2);
        assert_eq!(c.vmm[0].zmm64u(1), 4);

        // The truncating form ignores MXCSR.RC entirely.
        c.execute_instruction(&evex_cvt(Opcode::EvexVcvttpd2qqVdqWpd, 0, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 2);
        assert_eq!(c.vmm[0].zmm64u(1), 3);

        // Round-toward-plus-infinity moves both up.
        c.mxcsr.mxcsr = MXCSR_RESET | (2 << 13);
        c.execute_instruction(&evex_cvt(Opcode::EvexVcvtpd2qqVdqWpd, 0, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 3);
        assert_eq!(c.vmm[0].zmm64u(1), 4);
    }

    #[test]
    fn single_to_qword_reads_only_the_low_half_of_the_source() {
        let mut c = cpu();
        // VL256 produces 4 qwords from 4 dwords, so the source dwords sit in
        // the low 128 bits even though the destination spans 256.
        for (i, v) in [1.5f32, -2.5, 3.75, 4.0].into_iter().enumerate() {
            c.vmm[1].set_zmm32u(i, v.to_bits());
        }
        c.execute_instruction(&evex_cvt(Opcode::EvexVcvttps2qqVdqWps, 1, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 1);
        assert_eq!(c.vmm[0].zmm64u(1), (-2i64) as u64);
        assert_eq!(c.vmm[0].zmm64u(2), 3);
        assert_eq!(c.vmm[0].zmm64u(3), 4);
        // Above VL: cleared.
        assert_eq!(c.vmm[0].zmm64u(4), 0);

        // Unsigned: the negative element becomes the indefinite value.
        c.execute_instruction(&evex_cvt(Opcode::EvexVcvttps2uqqVdqWps, 1, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 1);
        assert_eq!(c.vmm[0].zmm64u(1), u64::MAX);
    }

    #[test]
    fn qword_to_single_writes_half_a_vector() {
        let mut c = cpu();
        c.vmm[1].set_zmm64u(0, 3);
        c.vmm[1].set_zmm64u(1, u64::MAX); // 2^64-1: not exact in f32
        c.vmm[1].set_zmm64u(2, 7);
        c.vmm[1].set_zmm64u(3, 9);

        c.execute_instruction(&evex_cvt(Opcode::EvexVcvtuqq2psVpsWdq, 1, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 3.0f32.to_bits());
        assert_eq!(c.vmm[0].zmm32u(1), 18446744073709551616.0f32.to_bits());
        assert_eq!(c.vmm[0].zmm32u(2), 7.0f32.to_bits());
        assert_eq!(c.vmm[0].zmm32u(3), 9.0f32.to_bits());
        // 4 qwords in produce 4 dwords out; the rest of the register is gone.
        assert_eq!(c.vmm[0].zmm32u(4), 0);

        // Signed reads the same bits as -1.
        c.execute_instruction(&evex_cvt(Opcode::EvexVcvtqq2psVpsWdq, 1, 0, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(1), (-1.0f32).to_bits());
    }

    #[test]
    fn half_width_output_clears_the_upper_dwords_at_vl128_under_merge_masking() {
        // At VL128 two qwords produce two dwords, and Bochs writes that with
        // BX_WRITE_XMM_REG_LO_QWORD_CLEAR_HIGH — dwords 2 and 3 are cleared
        // outright, not left to the opmask. Deriving the count from an output
        // VL of 128 instead would treat them as maskable and, under merge
        // masking with those bits clear, leave the destination's old contents.
        let mut c = cpu();
        c.opmask[1].set_rrx(0b0011); // elements 0,1 active; 2,3 clear
        for i in 0..4 {
            c.vmm[0].set_zmm32u(i, 0xDEAD_BEEF);
        }
        c.vmm[1].set_zmm64u(0, 1.0f64.to_bits());
        c.vmm[1].set_zmm64u(1, 2.0f64.to_bits());

        c.execute_instruction(&evex_cvt(Opcode::EvexVcvtpd2dqVdqWpdKmask, 0, 1, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 1);
        assert_eq!(c.vmm[0].zmm32u(1), 2);
        assert_eq!(c.vmm[0].zmm32u(2), 0, "dword 2 must be cleared, not merged");
        assert_eq!(c.vmm[0].zmm32u(3), 0, "dword 3 must be cleared, not merged");

        // Merge masking still preserves the destination *within* the two
        // dwords the instruction actually produces.
        c.opmask[1].set_rrx(0b0001);
        c.vmm[0].set_zmm32u(1, 0xDEAD_BEEF);
        c.execute_instruction(&evex_cvt(Opcode::EvexVcvtpd2dqVdqWpdKmask, 0, 1, false))
            .unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 1);
        assert_eq!(c.vmm[0].zmm32u(1), 0xDEAD_BEEF);
    }

}
