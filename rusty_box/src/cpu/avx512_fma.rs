//! AVX-512F Fused Multiply-Add (FMA) instruction handlers
//!
//! Implements VFMADD, VFMSUB, VFNMADD, VFNMSUB in all three forms (132, 213, 231)
//! for both packed single-precision (PS) and packed double-precision (PD).
//!
//! Uses `f32::mul_add` / `f64::mul_add` for fused multiply-add precision.
//!
//! Mirrors Bochs `cpu/avx/avx512_fma.cc`.

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
        unsafe { cpu.opmask[k as usize].rrx }
    }
}

/// Read ZMM register as a ZMM-width value
#[inline]
fn read_zmm<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, reg: u8) -> BxPackedZmmRegister {
    unsafe { cpu.vmm[reg as usize] }
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
    unsafe {
        let dst = &mut cpu.vmm[reg as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                dst.zmm32u[i] = result.zmm32u[i];
            } else if zero_masking {
                dst.zmm32u[i] = 0;
            }
            // else: merge masking — keep original value
        }
        // Zero upper elements beyond VL (EVEX always clears upper)
        for i in nelements..16 {
            dst.zmm32u[i] = 0;
        }
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
    unsafe {
        let dst = &mut cpu.vmm[reg as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                dst.zmm64u[i] = result.zmm64u[i];
            } else if zero_masking {
                dst.zmm64u[i] = 0;
            }
        }
        // Zero upper elements beyond VL
        for i in nelements..8 {
            dst.zmm64u[i] = 0;
        }
    }
}

/// Read src2 as packed dwords from register or memory
fn read_src2_ps<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let nelements = dword_elements(vl);
        let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        for i in 0..nelements {
            let val = cpu.v_read_dword(seg, laddr + (i * 4) as u64)?;
            unsafe { tmp.zmm32u[i] = val; }
        }
        Ok(tmp)
    }
}

/// Read src2 as packed qwords from register or memory
fn read_src2_pd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let nelements = qword_elements(vl);
        let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        for i in 0..nelements {
            let lo = cpu.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
            let hi = cpu.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
            unsafe { tmp.zmm64u[i] = lo | (hi << 32); }
        }
        Ok(tmp)
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // VFMADD — Fused Multiply-Add: dst = a * b + c
    // ========================================================================

    /// VFMADD132PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 98
    /// dst[i] = dst[i] * src2[i] + src1[i]
    pub fn evex_vfmadd132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(dst_val.zmm32u[i]);
                let b = f32::from_bits(src2.zmm32u[i]);
                let c = f32::from_bits(src1.zmm32u[i]);
                result.zmm32u[i] = a.mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD132PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 98
    /// dst[i] = dst[i] * src2[i] + src1[i]
    pub fn evex_vfmadd132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(dst_val.zmm64u[i]);
                let b = f64::from_bits(src2.zmm64u[i]);
                let c = f64::from_bits(src1.zmm64u[i]);
                result.zmm64u[i] = a.mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD213PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 A8
    /// dst[i] = src1[i] * dst[i] + src2[i]
    pub fn evex_vfmadd213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(src1.zmm32u[i]);
                let b = f32::from_bits(dst_val.zmm32u[i]);
                let c = f32::from_bits(src2.zmm32u[i]);
                result.zmm32u[i] = a.mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD213PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 A8
    /// dst[i] = src1[i] * dst[i] + src2[i]
    pub fn evex_vfmadd213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(src1.zmm64u[i]);
                let b = f64::from_bits(dst_val.zmm64u[i]);
                let c = f64::from_bits(src2.zmm64u[i]);
                result.zmm64u[i] = a.mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD231PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 B8
    /// dst[i] = src1[i] * src2[i] + dst[i]
    pub fn evex_vfmadd231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(src1.zmm32u[i]);
                let b = f32::from_bits(src2.zmm32u[i]);
                let c = f32::from_bits(dst_val.zmm32u[i]);
                result.zmm32u[i] = a.mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD231PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 B8
    /// dst[i] = src1[i] * src2[i] + dst[i]
    pub fn evex_vfmadd231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(src1.zmm64u[i]);
                let b = f64::from_bits(src2.zmm64u[i]);
                let c = f64::from_bits(dst_val.zmm64u[i]);
                result.zmm64u[i] = a.mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VFMSUB — Fused Multiply-Subtract: dst = a * b - c
    // ========================================================================

    /// VFMSUB132PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 9A
    /// dst[i] = dst[i] * src2[i] - src1[i]
    pub fn evex_vfmsub132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(dst_val.zmm32u[i]);
                let b = f32::from_bits(src2.zmm32u[i]);
                let c = f32::from_bits(src1.zmm32u[i]);
                result.zmm32u[i] = a.mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB132PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 9A
    /// dst[i] = dst[i] * src2[i] - src1[i]
    pub fn evex_vfmsub132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(dst_val.zmm64u[i]);
                let b = f64::from_bits(src2.zmm64u[i]);
                let c = f64::from_bits(src1.zmm64u[i]);
                result.zmm64u[i] = a.mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB213PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 AA
    /// dst[i] = src1[i] * dst[i] - src2[i]
    pub fn evex_vfmsub213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(src1.zmm32u[i]);
                let b = f32::from_bits(dst_val.zmm32u[i]);
                let c = f32::from_bits(src2.zmm32u[i]);
                result.zmm32u[i] = a.mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB213PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 AA
    /// dst[i] = src1[i] * dst[i] - src2[i]
    pub fn evex_vfmsub213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(src1.zmm64u[i]);
                let b = f64::from_bits(dst_val.zmm64u[i]);
                let c = f64::from_bits(src2.zmm64u[i]);
                result.zmm64u[i] = a.mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB231PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 BA
    /// dst[i] = src1[i] * src2[i] - dst[i]
    pub fn evex_vfmsub231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(src1.zmm32u[i]);
                let b = f32::from_bits(src2.zmm32u[i]);
                let c = f32::from_bits(dst_val.zmm32u[i]);
                result.zmm32u[i] = a.mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB231PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 BA
    /// dst[i] = src1[i] * src2[i] - dst[i]
    pub fn evex_vfmsub231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(src1.zmm64u[i]);
                let b = f64::from_bits(src2.zmm64u[i]);
                let c = f64::from_bits(dst_val.zmm64u[i]);
                result.zmm64u[i] = a.mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VFNMADD — Fused Negative Multiply-Add: dst = -(a * b) + c
    // ========================================================================

    /// VFNMADD132PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 9C
    /// dst[i] = -(dst[i] * src2[i]) + src1[i]
    pub fn evex_vfnmadd132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(dst_val.zmm32u[i]);
                let b = f32::from_bits(src2.zmm32u[i]);
                let c = f32::from_bits(src1.zmm32u[i]);
                result.zmm32u[i] = (-a).mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD132PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 9C
    /// dst[i] = -(dst[i] * src2[i]) + src1[i]
    pub fn evex_vfnmadd132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(dst_val.zmm64u[i]);
                let b = f64::from_bits(src2.zmm64u[i]);
                let c = f64::from_bits(src1.zmm64u[i]);
                result.zmm64u[i] = (-a).mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD213PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 AC
    /// dst[i] = -(src1[i] * dst[i]) + src2[i]
    pub fn evex_vfnmadd213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(src1.zmm32u[i]);
                let b = f32::from_bits(dst_val.zmm32u[i]);
                let c = f32::from_bits(src2.zmm32u[i]);
                result.zmm32u[i] = (-a).mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD213PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 AC
    /// dst[i] = -(src1[i] * dst[i]) + src2[i]
    pub fn evex_vfnmadd213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(src1.zmm64u[i]);
                let b = f64::from_bits(dst_val.zmm64u[i]);
                let c = f64::from_bits(src2.zmm64u[i]);
                result.zmm64u[i] = (-a).mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD231PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 BC
    /// dst[i] = -(src1[i] * src2[i]) + dst[i]
    pub fn evex_vfnmadd231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(src1.zmm32u[i]);
                let b = f32::from_bits(src2.zmm32u[i]);
                let c = f32::from_bits(dst_val.zmm32u[i]);
                result.zmm32u[i] = (-a).mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD231PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 BC
    /// dst[i] = -(src1[i] * src2[i]) + dst[i]
    pub fn evex_vfnmadd231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(src1.zmm64u[i]);
                let b = f64::from_bits(src2.zmm64u[i]);
                let c = f64::from_bits(dst_val.zmm64u[i]);
                result.zmm64u[i] = (-a).mul_add(b, c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VFNMSUB — Fused Negative Multiply-Subtract: dst = -(a * b) - c
    // ========================================================================

    /// VFNMSUB132PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 9E
    /// dst[i] = -(dst[i] * src2[i]) - src1[i]
    pub fn evex_vfnmsub132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(dst_val.zmm32u[i]);
                let b = f32::from_bits(src2.zmm32u[i]);
                let c = f32::from_bits(src1.zmm32u[i]);
                result.zmm32u[i] = (-a).mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB132PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 9E
    /// dst[i] = -(dst[i] * src2[i]) - src1[i]
    pub fn evex_vfnmsub132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(dst_val.zmm64u[i]);
                let b = f64::from_bits(src2.zmm64u[i]);
                let c = f64::from_bits(src1.zmm64u[i]);
                result.zmm64u[i] = (-a).mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB213PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 AE
    /// dst[i] = -(src1[i] * dst[i]) - src2[i]
    pub fn evex_vfnmsub213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(src1.zmm32u[i]);
                let b = f32::from_bits(dst_val.zmm32u[i]);
                let c = f32::from_bits(src2.zmm32u[i]);
                result.zmm32u[i] = (-a).mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB213PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 AE
    /// dst[i] = -(src1[i] * dst[i]) - src2[i]
    pub fn evex_vfnmsub213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(src1.zmm64u[i]);
                let b = f64::from_bits(dst_val.zmm64u[i]);
                let c = f64::from_bits(src2.zmm64u[i]);
                result.zmm64u[i] = (-a).mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB231PS Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 BE
    /// dst[i] = -(src1[i] * src2[i]) - dst[i]
    pub fn evex_vfnmsub231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f32::from_bits(src1.zmm32u[i]);
                let b = f32::from_bits(src2.zmm32u[i]);
                let c = f32::from_bits(dst_val.zmm32u[i]);
                result.zmm32u[i] = (-a).mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB231PD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 BE
    /// dst[i] = -(src1[i] * src2[i]) - dst[i]
    pub fn evex_vfnmsub231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_val = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let a = f64::from_bits(src1.zmm64u[i]);
                let b = f64::from_bits(src2.zmm64u[i]);
                let c = f64::from_bits(dst_val.zmm64u[i]);
                result.zmm64u[i] = (-a).mul_add(b, -c).to_bits();
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}
