//! SSE reciprocal and reciprocal square root approximation instructions
//!
//! Based on Bochs cpu/sse_rcp.cc
//!
//! RCPPS/RCPSS and RSQRTPS/RSQRTSS are approximate instructions with
//! ~12-bit precision. We use native Rust f32 operations which give full
//! IEEE 754 precision — this is MORE precise than real hardware but
//! functionally correct (programs follow up with Newton-Raphson refinement).

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedXmmRegister,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // RCPPS — Reciprocal of Packed Single-Precision (approximate)
    // Bochs: RCPPS_VpsWps in sse_rcp.cc
    // ========================================================================

    pub(super) fn rcpps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;

        let op = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1())
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_xmmword(seg, eaddr)?
        };

        let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32f(i, 1.0f32 / op.xmm32f(i));
            }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // RCPSS — Reciprocal of Scalar Single-Precision (approximate)
    // Bochs: RCPSS_VssWss in sse_rcp.cc
    // ========================================================================

    pub(super) fn rcpss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;

        let src_f32 = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm32f(0)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.v_read_dword(seg, eaddr)?;
            f32::from_bits(val)
        };

        let mut result = self.read_xmm_reg(instr.dst());
            result.set_xmm32f(0, 1.0f32 / src_f32);
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // RSQRTPS — Reciprocal Square Root of Packed Single-Precision (approximate)
    // Bochs: RSQRTPS_VpsWps in sse_rcp.cc
    // ========================================================================

    pub(super) fn rsqrtps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;

        let op = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1())
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_xmmword(seg, eaddr)?
        };

        let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32f(i, 1.0f32 / op.xmm32f(i).sqrt());
            }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // RSQRTSS — Reciprocal Square Root of Scalar Single-Precision (approximate)
    // Bochs: RSQRTSS_VssWss in sse_rcp.cc
    // ========================================================================

    pub(super) fn rsqrtss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;

        let src_f32 = if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            src.xmm32f(0)
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.v_read_dword(seg, eaddr)?;
            f32::from_bits(val)
        };

        let mut result = self.read_xmm_reg(instr.dst());
            result.set_xmm32f(0, 1.0f32 / src_f32.sqrt());
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }
}
