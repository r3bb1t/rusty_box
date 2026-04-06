#![allow(unused_unsafe)]

//! AVX-512F Fused Multiply-Add (FMA) instruction handlers
//!
//! Implements VFMADD, VFMSUB, VFNMADD, VFNMSUB in all three forms (132, 213, 231)
//! for both packed single-precision (PS) and packed double-precision (PD).
//!
//! Uses `f32::mul_add` / `f64::mul_add` for fused multiply-add precision.
//!
//! Decoder convention:
//!   dst()  = nnn = V (destination register, also an input)
//!   src1() = rm  = W (ModRM r/m operand - register or memory)
//!   src2() = vvvv = H (VEX.vvvv operand)
//!
//! FMA operand forms:
//!   132: result = V * W + H
//!   213: result = H * V + W
//!   231: result = H * W + V
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

/// Read rm operand (W) as packed dwords from register or memory.
/// Register form: reads src1() (rm register = W).
/// Memory form: reads from memory at resolved address.
fn read_rm_ps<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src1()))
    } else {
        let nelements = dword_elements(vl);
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

/// Read rm operand (W) as packed qwords from register or memory.
/// Register form: reads src1() (rm register = W).
/// Memory form: reads from memory at resolved address.
fn read_rm_pd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src1()))
    } else {
        let nelements = qword_elements(vl);
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

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // VFMADD — Fused Multiply-Add
    //   132: V * W + H
    //   213: H * V + W
    //   231: H * W + V
    // ========================================================================

    /// VFMADD132PS — EVEX.66.0F38.W0 98
    /// result[i] = V[i] * W[i] + H[i]
    pub fn evex_vfmadd132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());       // V = nnn (destination)
        let h = read_zmm(self, instr.src2());       // H = vvvv
        let w = read_rm_ps(self, instr, vl)?;       // W = rm/memory
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let vf = f32::from_bits(v.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            let hf = f32::from_bits(h.zmm32u(i));
            result.set_zmm32u(i, vf.mul_add(wf, hf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD132PD — EVEX.66.0F38.W1 98
    /// result[i] = V[i] * W[i] + H[i]
    pub fn evex_vfmadd132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let vf = f64::from_bits(v.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            let hf = f64::from_bits(h.zmm64u(i));
            result.set_zmm64u(i, vf.mul_add(wf, hf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD213PS — EVEX.66.0F38.W0 A8
    /// result[i] = H[i] * V[i] + W[i]
    pub fn evex_vfmadd213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f32::from_bits(h.zmm32u(i));
            let vf = f32::from_bits(v.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            result.set_zmm32u(i, hf.mul_add(vf, wf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD213PD — EVEX.66.0F38.W1 A8
    /// result[i] = H[i] * V[i] + W[i]
    pub fn evex_vfmadd213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f64::from_bits(h.zmm64u(i));
            let vf = f64::from_bits(v.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            result.set_zmm64u(i, hf.mul_add(vf, wf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD231PS — EVEX.66.0F38.W0 B8
    /// result[i] = H[i] * W[i] + V[i]
    pub fn evex_vfmadd231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f32::from_bits(h.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            let vf = f32::from_bits(v.zmm32u(i));
            result.set_zmm32u(i, hf.mul_add(wf, vf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD231PD — EVEX.66.0F38.W1 B8
    /// result[i] = H[i] * W[i] + V[i]
    pub fn evex_vfmadd231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f64::from_bits(h.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            let vf = f64::from_bits(v.zmm64u(i));
            result.set_zmm64u(i, hf.mul_add(wf, vf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VFMSUB — Fused Multiply-Subtract (negate the addend)
    //   132: V * W - H    = V * W + (-H)
    //   213: H * V - W    = H * V + (-W)
    //   231: H * W - V    = H * W + (-V)
    // ========================================================================

    /// VFMSUB132PS — EVEX.66.0F38.W0 9A
    /// result[i] = V[i] * W[i] - H[i]
    pub fn evex_vfmsub132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let vf = f32::from_bits(v.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            let hf = f32::from_bits(h.zmm32u(i));
            result.set_zmm32u(i, vf.mul_add(wf, -hf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB132PD — EVEX.66.0F38.W1 9A
    /// result[i] = V[i] * W[i] - H[i]
    pub fn evex_vfmsub132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let vf = f64::from_bits(v.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            let hf = f64::from_bits(h.zmm64u(i));
            result.set_zmm64u(i, vf.mul_add(wf, -hf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB213PS — EVEX.66.0F38.W0 AA
    /// result[i] = H[i] * V[i] - W[i]
    pub fn evex_vfmsub213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f32::from_bits(h.zmm32u(i));
            let vf = f32::from_bits(v.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            result.set_zmm32u(i, hf.mul_add(vf, -wf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB213PD — EVEX.66.0F38.W1 AA
    /// result[i] = H[i] * V[i] - W[i]
    pub fn evex_vfmsub213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f64::from_bits(h.zmm64u(i));
            let vf = f64::from_bits(v.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            result.set_zmm64u(i, hf.mul_add(vf, -wf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB231PS — EVEX.66.0F38.W0 BA
    /// result[i] = H[i] * W[i] - V[i]
    pub fn evex_vfmsub231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f32::from_bits(h.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            let vf = f32::from_bits(v.zmm32u(i));
            result.set_zmm32u(i, hf.mul_add(wf, -vf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMSUB231PD — EVEX.66.0F38.W1 BA
    /// result[i] = H[i] * W[i] - V[i]
    pub fn evex_vfmsub231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f64::from_bits(h.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            let vf = f64::from_bits(v.zmm64u(i));
            result.set_zmm64u(i, hf.mul_add(wf, -vf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VFNMADD — Fused Negative Multiply-Add (negate the product)
    //   132: -(V * W) + H  = (-V) * W + H
    //   213: -(H * V) + W  = (-H) * V + W
    //   231: -(H * W) + V  = (-H) * W + V
    // ========================================================================

    /// VFNMADD132PS — EVEX.66.0F38.W0 9C
    /// result[i] = -(V[i] * W[i]) + H[i]
    pub fn evex_vfnmadd132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let vf = f32::from_bits(v.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            let hf = f32::from_bits(h.zmm32u(i));
            result.set_zmm32u(i, (-vf).mul_add(wf, hf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD132PD — EVEX.66.0F38.W1 9C
    /// result[i] = -(V[i] * W[i]) + H[i]
    pub fn evex_vfnmadd132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let vf = f64::from_bits(v.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            let hf = f64::from_bits(h.zmm64u(i));
            result.set_zmm64u(i, (-vf).mul_add(wf, hf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD213PS — EVEX.66.0F38.W0 AC
    /// result[i] = -(H[i] * V[i]) + W[i]
    pub fn evex_vfnmadd213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f32::from_bits(h.zmm32u(i));
            let vf = f32::from_bits(v.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            result.set_zmm32u(i, (-hf).mul_add(vf, wf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD213PD — EVEX.66.0F38.W1 AC
    /// result[i] = -(H[i] * V[i]) + W[i]
    pub fn evex_vfnmadd213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f64::from_bits(h.zmm64u(i));
            let vf = f64::from_bits(v.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            result.set_zmm64u(i, (-hf).mul_add(vf, wf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD231PS — EVEX.66.0F38.W0 BC
    /// result[i] = -(H[i] * W[i]) + V[i]
    pub fn evex_vfnmadd231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f32::from_bits(h.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            let vf = f32::from_bits(v.zmm32u(i));
            result.set_zmm32u(i, (-hf).mul_add(wf, vf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMADD231PD — EVEX.66.0F38.W1 BC
    /// result[i] = -(H[i] * W[i]) + V[i]
    pub fn evex_vfnmadd231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f64::from_bits(h.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            let vf = f64::from_bits(v.zmm64u(i));
            result.set_zmm64u(i, (-hf).mul_add(wf, vf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VFNMSUB — Fused Negative Multiply-Subtract (negate both product and addend)
    //   132: -(V * W) - H  = (-V) * W + (-H)
    //   213: -(H * V) - W  = (-H) * V + (-W)
    //   231: -(H * W) - V  = (-H) * W + (-V)
    // ========================================================================

    /// VFNMSUB132PS — EVEX.66.0F38.W0 9E
    /// result[i] = -(V[i] * W[i]) - H[i]
    pub fn evex_vfnmsub132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let vf = f32::from_bits(v.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            let hf = f32::from_bits(h.zmm32u(i));
            result.set_zmm32u(i, (-vf).mul_add(wf, -hf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB132PD — EVEX.66.0F38.W1 9E
    /// result[i] = -(V[i] * W[i]) - H[i]
    pub fn evex_vfnmsub132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let vf = f64::from_bits(v.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            let hf = f64::from_bits(h.zmm64u(i));
            result.set_zmm64u(i, (-vf).mul_add(wf, -hf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB213PS — EVEX.66.0F38.W0 AE
    /// result[i] = -(H[i] * V[i]) - W[i]
    pub fn evex_vfnmsub213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f32::from_bits(h.zmm32u(i));
            let vf = f32::from_bits(v.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            result.set_zmm32u(i, (-hf).mul_add(vf, -wf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB213PD — EVEX.66.0F38.W1 AE
    /// result[i] = -(H[i] * V[i]) - W[i]
    pub fn evex_vfnmsub213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f64::from_bits(h.zmm64u(i));
            let vf = f64::from_bits(v.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            result.set_zmm64u(i, (-hf).mul_add(vf, -wf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB231PS — EVEX.66.0F38.W0 BE
    /// result[i] = -(H[i] * W[i]) - V[i]
    pub fn evex_vfnmsub231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_ps(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f32::from_bits(h.zmm32u(i));
            let wf = f32::from_bits(w.zmm32u(i));
            let vf = f32::from_bits(v.zmm32u(i));
            result.set_zmm32u(i, (-hf).mul_add(wf, -vf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFNMSUB231PD — EVEX.66.0F38.W1 BE
    /// result[i] = -(H[i] * W[i]) - V[i]
    pub fn evex_vfnmsub231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let hf = f64::from_bits(h.zmm64u(i));
            let wf = f64::from_bits(w.zmm64u(i));
            let vf = f64::from_bits(v.zmm64u(i));
            result.set_zmm64u(i, (-hf).mul_add(wf, -vf).to_bits());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}
