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
use super::softfloat3e::softfloat::softfloat_get_exception_flags;
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
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
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
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
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

    // ════════════════════════════════════════════════════════════════════
    // Alternating add/subtract FMA. VFMADDSUB subtracts on the even
    // elements and adds on the odd ones; VFMSUBADD is the inverse
    // (Bochs simd_pfp.h xmm_fmaddsubps / xmm_fmsubaddps). The parity is
    // taken on the element index, which agrees with Bochs's per-128-bit-lane
    // primitives because every lane holds an even number of elements.
    // ════════════════════════════════════════════════════════════════════

    /// VFMADDSUB132PS — EVEX.66.0F38.W0 96
    /// result[i] = V[i] * W[i] -+ H[i]
    pub fn evex_vfmaddsub132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F132, VexPackedFmaOp::FmaddSub)
    }

    /// VFMADDSUB132PD — EVEX.66.0F38.W1 96
    /// result[i] = V[i] * W[i] -+ H[i]
    pub fn evex_vfmaddsub132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F132, VexPackedFmaOp::FmaddSub)
    }

    /// VFMADDSUB213PS — EVEX.66.0F38.W0 A6
    /// result[i] = H[i] * V[i] -+ W[i]
    pub fn evex_vfmaddsub213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F213, VexPackedFmaOp::FmaddSub)
    }

    /// VFMADDSUB213PD — EVEX.66.0F38.W1 A6
    /// result[i] = H[i] * V[i] -+ W[i]
    pub fn evex_vfmaddsub213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F213, VexPackedFmaOp::FmaddSub)
    }

    /// VFMADDSUB231PS — EVEX.66.0F38.W0 B6
    /// result[i] = H[i] * W[i] -+ V[i]
    pub fn evex_vfmaddsub231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F231, VexPackedFmaOp::FmaddSub)
    }

    /// VFMADDSUB231PD — EVEX.66.0F38.W1 B6
    /// result[i] = H[i] * W[i] -+ V[i]
    pub fn evex_vfmaddsub231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F231, VexPackedFmaOp::FmaddSub)
    }

    /// VFMSUBADD132PS — EVEX.66.0F38.W0 97
    /// result[i] = V[i] * W[i] +- H[i]
    pub fn evex_vfmsubadd132ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F132, VexPackedFmaOp::FmsubAdd)
    }

    /// VFMSUBADD132PD — EVEX.66.0F38.W1 97
    /// result[i] = V[i] * W[i] +- H[i]
    pub fn evex_vfmsubadd132pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F132, VexPackedFmaOp::FmsubAdd)
    }

    /// VFMSUBADD213PS — EVEX.66.0F38.W0 A7
    /// result[i] = H[i] * V[i] +- W[i]
    pub fn evex_vfmsubadd213ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F213, VexPackedFmaOp::FmsubAdd)
    }

    /// VFMSUBADD213PD — EVEX.66.0F38.W1 A7
    /// result[i] = H[i] * V[i] +- W[i]
    pub fn evex_vfmsubadd213pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F213, VexPackedFmaOp::FmsubAdd)
    }

    /// VFMSUBADD231PS — EVEX.66.0F38.W0 B7
    /// result[i] = H[i] * W[i] +- V[i]
    pub fn evex_vfmsubadd231ps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_ps(instr, VexFmaForm::F231, VexPackedFmaOp::FmsubAdd)
    }

    /// VFMSUBADD231PD — EVEX.66.0F38.W1 B7
    /// result[i] = H[i] * W[i] +- V[i]
    pub fn evex_vfmsubadd231pd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_fma_packed_pd(instr, VexFmaForm::F231, VexPackedFmaOp::FmsubAdd)
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    //! VFMADDSUB and VFMSUBADD differ from the plain FMA forms only in which
    //! elements subtract, so an inverted parity produces results that are
    //! individually plausible — every element is still a valid fused
    //! multiply-add. Only comparing the two mnemonics against each other, on
    //! the same inputs, pins the parity down.

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::{BxSegregs, Instruction};
    use crate::cpu::xmm::MXCSR_RESET;
    use rusty_box_decoder::opcode::Opcode;

    /// Register-form EVEX.128, no masking. The 213 form computes
    /// H * V + W, with dst doubling as V.
    fn evex_reg(opcode: Opcode) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0); // dst = V
        i.set_src_reg(1, 1); // rm = W
        i.set_src_reg(2, 2); // vvvv = H
        i.set_opmask(0);
        i.set_vex(true);
        i.set_vl(0);
        i.set_seg(BxSegregs::Ds);
        i.init(0, 0, 1, 1);
        i.assert_mod_c0();
        i
    }

    /// H=2.0, V=3.0, W=1.0 in every element, so H*V = 6.0 and the two
    /// possible results are 6+1=7 and 6-1=5 — far apart and exact.
    fn seed<I: crate::cpu::cpuid::BxCpuIdTrait>(
        cpu: &mut crate::cpu::cpu::BxCpuC<'_, I, ()>,
    ) {
        cpu.mxcsr.mxcsr = MXCSR_RESET;
        for n in 0..4 {
            cpu.vmm[0].set_zmm32u(n, 3.0f32.to_bits()); // V
            cpu.vmm[1].set_zmm32u(n, 1.0f32.to_bits()); // W
            cpu.vmm[2].set_zmm32u(n, 2.0f32.to_bits()); // H
        }
    }

    #[test]
    fn fmaddsub_subtracts_on_even_elements_and_fmsubadd_is_its_inverse() {
        let add = 7.0f32.to_bits();
        let sub = 5.0f32.to_bits();

        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        seed(&mut cpu);
        cpu.execute_instruction(&evex_reg(Opcode::EvexVfmaddsub213psVpsHpsWps))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), sub, "FMADDSUB element 0 must subtract");
        assert_eq!(cpu.vmm[0].zmm32u(1), add, "FMADDSUB element 1 must add");
        assert_eq!(cpu.vmm[0].zmm32u(2), sub);
        assert_eq!(cpu.vmm[0].zmm32u(3), add);

        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        seed(&mut cpu);
        cpu.execute_instruction(&evex_reg(Opcode::EvexVfmsubadd213psVpsHpsWps))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), add, "FMSUBADD element 0 must add");
        assert_eq!(cpu.vmm[0].zmm32u(1), sub, "FMSUBADD element 1 must subtract");
        assert_eq!(cpu.vmm[0].zmm32u(2), add);
        assert_eq!(cpu.vmm[0].zmm32u(3), sub);
    }

    #[test]
    fn fmaddsub_double_precision_keeps_the_same_parity() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.mxcsr.mxcsr = MXCSR_RESET;
        for n in 0..2 {
            cpu.vmm[0].set_zmm64u(n, 3.0f64.to_bits()); // V
            cpu.vmm[1].set_zmm64u(n, 1.0f64.to_bits()); // W
            cpu.vmm[2].set_zmm64u(n, 2.0f64.to_bits()); // H
        }
        cpu.execute_instruction(&evex_reg(Opcode::EvexVfmaddsub213pdVpdHpdWpd))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm64u(0), 5.0f64.to_bits());
        assert_eq!(cpu.vmm[0].zmm64u(1), 7.0f64.to_bits());
    }
}
