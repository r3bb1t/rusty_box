//! AVX-512F Fused Multiply-Add (FMA) instruction handlers
//!
//! Implements VFMADD, VFMSUB, VFNMADD, VFNMSUB in all three forms (132, 213, 231)
//! for both packed single-precision (PS) and packed double-precision (PD).
//!
//! Every element goes through SoftFloat `f32_mul_add` / `f64_mul_add`
//! against an MXCSR-seeded status word with the EVEX embedded rounding
//! control applied, exactly as Bochs does. Elements the writemask disables
//! are not computed at all, so they raise no exception.
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

use super::avx::{packed_fma_flags, vex_fma_operands_u32, vex_fma_operands_u64, VexFmaForm,
    VexPackedFmaOp};
use super::softfloat3e::f32_mul_add::f32_mul_add;
use super::softfloat3e::f64_mul_add::f64_mul_add;
use super::softfloat3e::softfloat::softfloat_getExceptionFlags;
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    xmm::BxPackedZmmRegister,
};

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

/// Read rm operand (W) as packed dwords from register or memory.
/// Register form: reads src1() (rm register = W).
/// Memory form: reads from memory at resolved address.
fn read_rm_ps<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    _vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src1()))
    } else {
        cpu.evex_load_bcst_d_pair(instr)
    }
}

/// Read rm operand (W) as packed qwords from register or memory.
/// Register form: reads src1() (rm register = W).
/// Memory form: reads from memory at resolved address.
fn read_rm_pd<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    _vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src1()))
    } else {
        cpu.evex_load_bcst_q_pair(instr)
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /// The shared body of all twelve packed single-precision EVEX FMA
    /// handlers. Bochs avx512_fma.cc `EVEX_FMA_PACKED_SINGLE`.
    fn evex_fma_packed_ps(
        &mut self,
        instr: &Instruction,
        form: VexFmaForm,
        op: VexPackedFmaOp,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let v = read_zmm(self, instr.dst()); // V = nnn (destination, also an input)
        let h = read_zmm(self, instr.src2()); // H = vvvv
        let w = read_rm_ps(self, instr, vl)?; // W = rm/memory
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 == 0 {
                continue;
            }
            let (a, b, c) = vex_fma_operands_u32(form, v.zmm32u(i), h.zmm32u(i), w.zmm32u(i));
            result.set_zmm32u(i, f32_mul_add(a, b, c, packed_fma_flags(op, i), &mut status));
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// The shared body of all twelve packed double-precision EVEX FMA
    /// handlers. Bochs avx512_fma.cc `EVEX_FMA_PACKED_DOUBLE`.
    fn evex_fma_packed_pd(
        &mut self,
        instr: &Instruction,
        form: VexFmaForm,
        op: VexPackedFmaOp,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let v = read_zmm(self, instr.dst());
        let h = read_zmm(self, instr.src2());
        let w = read_rm_pd(self, instr, vl)?;
        let mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            if (mask >> i) & 1 == 0 {
                continue;
            }
            let (a, b, c) = vex_fma_operands_u64(form, v.zmm64u(i), h.zmm64u(i), w.zmm64u(i));
            result.set_zmm64u(i, f64_mul_add(a, b, c, packed_fma_flags(op, i), &mut status));
        }
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VFMADD132PS — EVEX.66.0F38.W0 98
    /// result[i] = V[i] * W[i] + H[i]
    pub fn evex_vfmadd132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F132, VexPackedFmaOp::Fmadd)
    }

    /// VFMADD132PD — EVEX.66.0F38.W1 98
    /// result[i] = V[i] * W[i] + H[i]
    pub fn evex_vfmadd132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F132, VexPackedFmaOp::Fmadd)
    }

    /// VFMADD213PS — EVEX.66.0F38.W0 A8
    /// result[i] = H[i] * V[i] + W[i]
    pub fn evex_vfmadd213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F213, VexPackedFmaOp::Fmadd)
    }

    /// VFMADD213PD — EVEX.66.0F38.W1 A8
    /// result[i] = H[i] * V[i] + W[i]
    pub fn evex_vfmadd213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F213, VexPackedFmaOp::Fmadd)
    }

    /// VFMADD231PS — EVEX.66.0F38.W0 B8
    /// result[i] = H[i] * W[i] + V[i]
    pub fn evex_vfmadd231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F231, VexPackedFmaOp::Fmadd)
    }

    /// VFMADD231PD — EVEX.66.0F38.W1 B8
    /// result[i] = H[i] * W[i] + V[i]
    pub fn evex_vfmadd231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F231, VexPackedFmaOp::Fmadd)
    }

    /// VFMSUB132PS — EVEX.66.0F38.W0 9A
    /// result[i] = V[i] * W[i] - H[i]
    pub fn evex_vfmsub132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F132, VexPackedFmaOp::Fmsub)
    }

    /// VFMSUB132PD — EVEX.66.0F38.W1 9A
    /// result[i] = V[i] * W[i] - H[i]
    pub fn evex_vfmsub132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F132, VexPackedFmaOp::Fmsub)
    }

    /// VFMSUB213PS — EVEX.66.0F38.W0 AA
    /// result[i] = H[i] * V[i] - W[i]
    pub fn evex_vfmsub213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F213, VexPackedFmaOp::Fmsub)
    }

    /// VFMSUB213PD — EVEX.66.0F38.W1 AA
    /// result[i] = H[i] * V[i] - W[i]
    pub fn evex_vfmsub213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F213, VexPackedFmaOp::Fmsub)
    }

    /// VFMSUB231PS — EVEX.66.0F38.W0 BA
    /// result[i] = H[i] * W[i] - V[i]
    pub fn evex_vfmsub231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F231, VexPackedFmaOp::Fmsub)
    }

    /// VFMSUB231PD — EVEX.66.0F38.W1 BA
    /// result[i] = H[i] * W[i] - V[i]
    pub fn evex_vfmsub231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F231, VexPackedFmaOp::Fmsub)
    }

    /// VFNMADD132PS — EVEX.66.0F38.W0 9C
    /// result[i] = -(V[i] * W[i]) + H[i]
    pub fn evex_vfnmadd132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F132, VexPackedFmaOp::Fnmadd)
    }

    /// VFNMADD132PD — EVEX.66.0F38.W1 9C
    /// result[i] = -(V[i] * W[i]) + H[i]
    pub fn evex_vfnmadd132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F132, VexPackedFmaOp::Fnmadd)
    }

    /// VFNMADD213PS — EVEX.66.0F38.W0 AC
    /// result[i] = -(H[i] * V[i]) + W[i]
    pub fn evex_vfnmadd213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F213, VexPackedFmaOp::Fnmadd)
    }

    /// VFNMADD213PD — EVEX.66.0F38.W1 AC
    /// result[i] = -(H[i] * V[i]) + W[i]
    pub fn evex_vfnmadd213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F213, VexPackedFmaOp::Fnmadd)
    }

    /// VFNMADD231PS — EVEX.66.0F38.W0 BC
    /// result[i] = -(H[i] * W[i]) + V[i]
    pub fn evex_vfnmadd231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F231, VexPackedFmaOp::Fnmadd)
    }

    /// VFNMADD231PD — EVEX.66.0F38.W1 BC
    /// result[i] = -(H[i] * W[i]) + V[i]
    pub fn evex_vfnmadd231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F231, VexPackedFmaOp::Fnmadd)
    }

    /// VFNMSUB132PS — EVEX.66.0F38.W0 9E
    /// result[i] = -(V[i] * W[i]) - H[i]
    pub fn evex_vfnmsub132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F132, VexPackedFmaOp::Fnmsub)
    }

    /// VFNMSUB132PD — EVEX.66.0F38.W1 9E
    /// result[i] = -(V[i] * W[i]) - H[i]
    pub fn evex_vfnmsub132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F132, VexPackedFmaOp::Fnmsub)
    }

    /// VFNMSUB213PS — EVEX.66.0F38.W0 AE
    /// result[i] = -(H[i] * V[i]) - W[i]
    pub fn evex_vfnmsub213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F213, VexPackedFmaOp::Fnmsub)
    }

    /// VFNMSUB213PD — EVEX.66.0F38.W1 AE
    /// result[i] = -(H[i] * V[i]) - W[i]
    pub fn evex_vfnmsub213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F213, VexPackedFmaOp::Fnmsub)
    }

    /// VFNMSUB231PS — EVEX.66.0F38.W0 BE
    /// result[i] = -(H[i] * W[i]) - V[i]
    pub fn evex_vfnmsub231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F231, VexPackedFmaOp::Fnmsub)
    }

    /// VFNMSUB231PD — EVEX.66.0F38.W1 BE
    /// result[i] = -(H[i] * W[i]) - V[i]
    pub fn evex_vfnmsub231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F231, VexPackedFmaOp::Fnmsub)
    }
}
