//! VEX-encoded (AVX) packed and scalar floating-point handlers.
//!
//! Mirrors Bochs `cpu/avx/avx_pfp.cc` (arithmetic, min/max, sqrt, compare,
//! add/sub) and the FP shuffle/unpack forms of `cpu/avx/avx.cc`.
//!
//! Operand convention in this port: `instr.dst()` is the destination,
//! `instr.src2()` is VEX.vvvv (Bochs `i->src1()`, the "H" operand), and
//! `instr.src1()` is the modrm.rm operand (Bochs `i->src2()`, the "W"
//! operand), read via `vex_read_src2_*` / `sse_pfp_read_op2_*`.
//!
//! VEX register writes always clear the destination above VL
//! (`write_xmm_reg` / `write_ymm_reg` — Bochs `BX_WRITE_XMM_REG_CLEAR_HIGH`
//! / `BX_WRITE_YMM_REGZ`).
//!
//! Element arithmetic uses host IEEE-754 floats like the rest of this
//! port's SSE/AVX FP handlers (`sse_pfp.rs`, `avx512_scalar.rs`): correct
//! for round-to-nearest, but MXCSR rounding-mode/DAZ/FTZ and exception
//! flags are not modeled — a known, codebase-wide divergence from Bochs'
//! softfloat, called out in `sse_pfp.rs` as well.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::{BxPackedXmmRegister, BxPackedYmmRegister},
};
#[cfg(not(feature = "std"))]
use crate::cpu::float::FloatExt;

/// Element operation selector for the packed/scalar FP families.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VexPfpOp {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

/// x86 MINSS/MINSD semantics (Bochs softfloat f32_min/f64_min): if either
/// operand is NaN, or the operands compare equal (covers -0.0 vs +0.0),
/// return the second (source) operand.
#[inline]
pub(super) fn sse_min_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        b
    } else if a < b {
        a
    } else {
        b
    }
}

#[inline]
pub(super) fn sse_min_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        b
    } else if a < b {
        a
    } else {
        b
    }
}

/// x86 MAXSS/MAXSD semantics (Bochs softfloat f32_max/f64_max): NaN or
/// equal operands (including ±0.0) return the second operand.
#[inline]
pub(super) fn sse_max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        b
    } else if a > b {
        a
    } else {
        b
    }
}

#[inline]
pub(super) fn sse_max_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        b
    } else if a > b {
        a
    } else {
        b
    }
}

#[inline]
fn pfp_op_f32(op: VexPfpOp, a: f32, b: f32) -> f32 {
    match op {
        VexPfpOp::Add => a + b,
        VexPfpOp::Sub => a - b,
        VexPfpOp::Mul => a * b,
        VexPfpOp::Div => a / b,
        VexPfpOp::Min => sse_min_f32(a, b),
        VexPfpOp::Max => sse_max_f32(a, b),
    }
}

#[inline]
fn pfp_op_f64(op: VexPfpOp, a: f64, b: f64) -> f64 {
    match op {
        VexPfpOp::Add => a + b,
        VexPfpOp::Sub => a - b,
        VexPfpOp::Mul => a * b,
        VexPfpOp::Div => a / b,
        VexPfpOp::Min => sse_min_f64(a, b),
        VexPfpOp::Max => sse_max_f64(a, b),
    }
}

/// AVX compare predicate (imm8[4:0], Bochs avx cmp handler tables).
/// Predicates 0x10..0x1F repeat the relations of 0x00..0x0F; the "S"/"Q"
/// suffix only changes QNaN signaling behavior, which affects exception
/// flags this port does not model.
#[inline]
fn avx_cmp_relation(predicate: u8, lt: bool, eq: bool, unord: bool) -> bool {
    let gt = !unord && !lt && !eq;
    match predicate & 0x0F {
        0x0 => eq,               // EQ_OQ / EQ_OS
        0x1 => lt,               // LT_OS / LT_OQ
        0x2 => lt || eq,         // LE_OS / LE_OQ
        0x3 => unord,            // UNORD_Q / UNORD_S
        0x4 => !eq,              // NEQ_UQ / NEQ_US (unordered => true)
        0x5 => !lt,              // NLT_US / NLT_UQ (unordered => true)
        0x6 => !(lt || eq),      // NLE_US / NLE_UQ (unordered => true)
        0x7 => !unord,           // ORD_Q / ORD_S
        0x8 => eq || unord,      // EQ_UQ / EQ_US
        0x9 => lt || unord,      // NGE_US / NGE_UQ
        0xA => !gt,              // NGT_US / NGT_UQ (unordered => true)
        0xB => false,            // FALSE_OQ / FALSE_OS
        0xC => !eq && !unord,    // NEQ_OQ / NEQ_OS
        0xD => gt || eq,         // GE_OS / GE_OQ
        0xE => gt,               // GT_OS / GT_OQ
        0xF => true,             // TRUE_UQ / TRUE_US
        _ => unreachable!("predicate & 0x0F cannot exceed 0xF"),
    }
}

#[inline]
fn avx_compare_f32(a: f32, b: f32, predicate: u8) -> bool {
    let unord = a.is_nan() || b.is_nan();
    avx_cmp_relation(predicate, !unord && a < b, !unord && a == b, unord)
}

#[inline]
fn avx_compare_f64(a: f64, b: f64, predicate: u8) -> bool {
    let unord = a.is_nan() || b.is_nan();
    avx_cmp_relation(predicate, !unord && a < b, !unord && a == b, unord)
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ════════════════════════════════════════════════════════════════════
    // Scalar arithmetic — VADDSS/VSUBSS/VMULSS/VDIVSS/VMINSS/VMAXSS and
    // the SD twins. Bochs avx_pfp.cc AVX_SCALAR_SINGLE_FP /
    // AVX_SCALAR_DOUBLE_FP: low element = op(vvvv.low, rm.low), remaining
    // xmm elements pass through from vvvv, upper bits cleared.
    // ════════════════════════════════════════════════════════════════════

    fn vex_scalar_pfp_ss(&mut self, instr: &Instruction, op: VexPfpOp) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm32f(0, pfp_op_f32(op, result.xmm32f(0), w));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    fn vex_scalar_pfp_sd(&mut self, instr: &Instruction, op: VexPfpOp) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64f(0, pfp_op_f64(op, result.xmm64f(0), w));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vaddss(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_ss(i, VexPfpOp::Add)
    }
    pub(super) fn vaddsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_sd(i, VexPfpOp::Add)
    }
    pub(super) fn vsubss(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_ss(i, VexPfpOp::Sub)
    }
    pub(super) fn vsubsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_sd(i, VexPfpOp::Sub)
    }
    pub(super) fn vmulss(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_ss(i, VexPfpOp::Mul)
    }
    pub(super) fn vmulsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_sd(i, VexPfpOp::Mul)
    }
    pub(super) fn vdivss(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_ss(i, VexPfpOp::Div)
    }
    pub(super) fn vdivsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_sd(i, VexPfpOp::Div)
    }
    pub(super) fn vminss(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_ss(i, VexPfpOp::Min)
    }
    pub(super) fn vminsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_sd(i, VexPfpOp::Min)
    }
    pub(super) fn vmaxss(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_ss(i, VexPfpOp::Max)
    }
    pub(super) fn vmaxsd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_scalar_pfp_sd(i, VexPfpOp::Max)
    }

    // ════════════════════════════════════════════════════════════════════
    // Packed arithmetic — VADDPS/VSUBPS/VMULPS/VDIVPS/VMINPS/VMAXPS and
    // the PD twins. Bochs avx_pfp.cc AVX_PACKED_FP: element-wise
    // op(vvvv[i], rm[i]) over VL, upper bits cleared.
    // ════════════════════════════════════════════════════════════════════

    fn vex_packed_pfp_ps(&mut self, instr: &Instruction, op: VexPfpOp) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32f(i, pfp_op_f32(op, op1.ymm32f(i), op2.ymm32f(i)));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32f(i, pfp_op_f32(op, op1.xmm32f(i), op2.xmm32f(i)));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    fn vex_packed_pfp_pd(&mut self, instr: &Instruction, op: VexPfpOp) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64f(i, pfp_op_f64(op, op1.ymm64f(i), op2.ymm64f(i)));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64f(i, pfp_op_f64(op, op1.xmm64f(i), op2.xmm64f(i)));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vaddps(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_ps(i, VexPfpOp::Add)
    }
    pub(super) fn vaddpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_pd(i, VexPfpOp::Add)
    }
    pub(super) fn vsubps(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_ps(i, VexPfpOp::Sub)
    }
    pub(super) fn vsubpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_pd(i, VexPfpOp::Sub)
    }
    pub(super) fn vmulps(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_ps(i, VexPfpOp::Mul)
    }
    pub(super) fn vmulpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_pd(i, VexPfpOp::Mul)
    }
    pub(super) fn vdivps(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_ps(i, VexPfpOp::Div)
    }
    pub(super) fn vdivpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_pd(i, VexPfpOp::Div)
    }
    pub(super) fn vminps(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_ps(i, VexPfpOp::Min)
    }
    pub(super) fn vminpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_pd(i, VexPfpOp::Min)
    }
    pub(super) fn vmaxps(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_ps(i, VexPfpOp::Max)
    }
    pub(super) fn vmaxpd(&mut self, i: &Instruction) -> super::Result<()> {
        self.vex_packed_pfp_pd(i, VexPfpOp::Max)
    }

    // ════════════════════════════════════════════════════════════════════
    // Square root — VSQRTPS/VSQRTPD (no vvvv operand) and VSQRTSS/VSQRTSD
    // (low element from rm, upper elements from vvvv). Bochs avx_pfp.cc
    // VSQRTPS_VpsWpsR / VSQRTSD_VsdHpdWsdR.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vsqrtps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32f(i, op2.ymm32f(i).sqrt());
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32f(i, op2.xmm32f(i).sqrt());
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vsqrtpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64f(i, op2.ymm64f(i).sqrt());
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64f(i, op2.xmm64f(i).sqrt());
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vsqrtss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm32f(0, w.sqrt());
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vsqrtsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64f(0, w.sqrt());
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Compare — VCMPPS/VCMPPD/VCMPSS/VCMPSD with the 32-entry AVX
    // predicate set (Bochs avx_pfp.cc VCMPPS_VpsHpsWpsIbR et al.).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vcmpps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let predicate = instr.ib();
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                let m = avx_compare_f32(op1.ymm32f(i), op2.ymm32f(i), predicate);
                result.set_ymm32u(i, if m { 0xFFFF_FFFF } else { 0 });
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                let m = avx_compare_f32(op1.xmm32f(i), op2.xmm32f(i), predicate);
                result.set_xmm32u(i, if m { 0xFFFF_FFFF } else { 0 });
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vcmppd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let predicate = instr.ib();
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                let m = avx_compare_f64(op1.ymm64f(i), op2.ymm64f(i), predicate);
                result.set_ymm64u(i, if m { u64::MAX } else { 0 });
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                let m = avx_compare_f64(op1.xmm64f(i), op2.xmm64f(i), predicate);
                result.set_xmm64u(i, if m { u64::MAX } else { 0 });
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vcmpss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let predicate = instr.ib();
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let m = avx_compare_f32(result.xmm32f(0), w, predicate);
        result.set_xmm32u(0, if m { 0xFFFF_FFFF } else { 0 });
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcmpsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let predicate = instr.ib();
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        let m = avx_compare_f64(result.xmm64f(0), w, predicate);
        result.set_xmm64u(0, if m { u64::MAX } else { 0 });
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Shuffle — VSHUFPS/VSHUFPD (per-128-bit-lane, Bochs avx.cc
    // VSHUFPS_VpsHpsWpsIbR via xmm_shufps).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vshufps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let order = instr.ib() as usize;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 4;
                result.set_ymm32u(base, op1.ymm32u(base + (order & 3)));
                result.set_ymm32u(base + 1, op1.ymm32u(base + ((order >> 2) & 3)));
                result.set_ymm32u(base + 2, op2.ymm32u(base + ((order >> 4) & 3)));
                result.set_ymm32u(base + 3, op2.ymm32u(base + ((order >> 6) & 3)));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, op1.xmm32u(order & 3));
            result.set_xmm32u(1, op1.xmm32u((order >> 2) & 3));
            result.set_xmm32u(2, op2.xmm32u((order >> 4) & 3));
            result.set_xmm32u(3, op2.xmm32u((order >> 6) & 3));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vshufpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let order = instr.ib() as usize;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 2;
                let sel = order >> (lane * 2);
                result.set_ymm64u(base, op1.ymm64u(base + (sel & 1)));
                result.set_ymm64u(base + 1, op2.ymm64u(base + ((sel >> 1) & 1)));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, op1.xmm64u(order & 1));
            result.set_xmm64u(1, op2.xmm64u((order >> 1) & 1));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // VADDSUBPS/VADDSUBPD — subtract on even elements, add on odd
    // (Bochs avx_pfp.cc VADDSUBPS_VpsHpsWpsR via xmm_addsubps).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vaddsubps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                let v = if i & 1 == 0 {
                    op1.ymm32f(i) - op2.ymm32f(i)
                } else {
                    op1.ymm32f(i) + op2.ymm32f(i)
                };
                result.set_ymm32f(i, v);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                let v = if i & 1 == 0 {
                    op1.xmm32f(i) - op2.xmm32f(i)
                } else {
                    op1.xmm32f(i) + op2.xmm32f(i)
                };
                result.set_xmm32f(i, v);
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vaddsubpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                let v = if i & 1 == 0 {
                    op1.ymm64f(i) - op2.ymm64f(i)
                } else {
                    op1.ymm64f(i) + op2.ymm64f(i)
                };
                result.set_ymm64f(i, v);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64f(0, op1.xmm64f(0) - op2.xmm64f(0));
            result.set_xmm64f(1, op1.xmm64f(1) + op2.xmm64f(1));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // FP unpack — VUNPCKLPS/VUNPCKHPS/VUNPCKLPD/VUNPCKHPD, per-128-bit
    // lane (Bochs avx.cc VUNPCKLPS_VpsHpsWpsR via xmm_unpcklps).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vunpcklps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 4;
                result.set_ymm32u(base, op1.ymm32u(base));
                result.set_ymm32u(base + 1, op2.ymm32u(base));
                result.set_ymm32u(base + 2, op1.ymm32u(base + 1));
                result.set_ymm32u(base + 3, op2.ymm32u(base + 1));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, op1.xmm32u(0));
            result.set_xmm32u(1, op2.xmm32u(0));
            result.set_xmm32u(2, op1.xmm32u(1));
            result.set_xmm32u(3, op2.xmm32u(1));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vunpckhps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 4;
                result.set_ymm32u(base, op1.ymm32u(base + 2));
                result.set_ymm32u(base + 1, op2.ymm32u(base + 2));
                result.set_ymm32u(base + 2, op1.ymm32u(base + 3));
                result.set_ymm32u(base + 3, op2.ymm32u(base + 3));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32u(0, op1.xmm32u(2));
            result.set_xmm32u(1, op2.xmm32u(2));
            result.set_xmm32u(2, op1.xmm32u(3));
            result.set_xmm32u(3, op2.xmm32u(3));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vunpcklpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 2;
                result.set_ymm64u(base, op1.ymm64u(base));
                result.set_ymm64u(base + 1, op2.ymm64u(base));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, op1.xmm64u(0));
            result.set_xmm64u(1, op2.xmm64u(0));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Scalar moves — VMOVSS/VMOVSD. Register forms merge the low element
    // into vvvv's upper elements (Bochs avx.cc VMOVSS_VssHpsWssR); memory
    // loads zero-extend; memory stores write the low element only.
    // ════════════════════════════════════════════════════════════════════

    /// VMOVSS xmm1, xmm2, xmm3 (0F 10 reg) / VMOVSS xmm1, m32 (0F 10 mem).
    pub(super) fn vmovss_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            let mut result = self.read_xmm_reg(instr.src2());
            result.set_xmm32u(0, self.read_xmm_reg(instr.src1()).xmm32u(0));
            self.write_xmm_reg(instr.dst(), result);
        } else {
            let w = self.sse_pfp_read_op2_ss(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm32f(0, w);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VMOVSD xmm1, xmm2, xmm3 (F2 0F 10 reg) / VMOVSD xmm1, m64 (mem).
    pub(super) fn vmovsd_load(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            let mut result = self.read_xmm_reg(instr.src2());
            result.set_xmm64u(0, self.read_xmm_reg(instr.src1()).xmm64u(0));
            self.write_xmm_reg(instr.dst(), result);
        } else {
            let w = self.sse_pfp_read_op2_sd(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64f(0, w);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VMOVSS xmm1, xmm2, xmm3 (0F 11 reg — rm register is the destination,
    /// roles normalized by decode) / VMOVSS m32, xmm1 (mem store).
    pub(super) fn vmovss_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            let mut result = self.read_xmm_reg(instr.src2());
            result.set_xmm32u(0, self.read_xmm_reg(instr.src1()).xmm32u(0));
            self.write_xmm_reg(instr.dst(), result);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            let val = self.read_xmm_reg(instr.src1()).xmm32u(0);
            self.v_write_dword(seg, eaddr, val)?;
        }
        Ok(())
    }

    /// VMOVSD store form (F2 0F 11).
    pub(super) fn vmovsd_store(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.mod_c0() {
            let mut result = self.read_xmm_reg(instr.src2());
            result.set_xmm64u(0, self.read_xmm_reg(instr.src1()).xmm64u(0));
            self.write_xmm_reg(instr.dst(), result);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            let val = self.read_xmm_reg(instr.src1()).xmm64u(0);
            self.v_write_qword(seg, eaddr, val)?;
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Half-register moves — VMOVLPS/LPD (low qword from m64, high from
    // vvvv), VMOVHPS/HPD (high qword from m64, low from vvvv), VMOVHLPS,
    // VMOVLHPS. Bochs avx.cc VMOVLPD_VpdHpdMq / VMOVHLPS_VpsHpsWps.
    // ════════════════════════════════════════════════════════════════════

    /// VMOVLPS/VMOVLPD xmm1, xmm2, m64.
    pub(super) fn vmovlp(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.v_read_qword(seg, eaddr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64u(0, val);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// VMOVHPS/VMOVHPD xmm1, xmm2, m64.
    pub(super) fn vmovhp(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.v_read_qword(seg, eaddr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64u(1, val);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// VMOVHLPS xmm1, xmm2, xmm3 — low qword = xmm3 high qword.
    pub(super) fn vmovhlps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64u(0, self.read_xmm_reg(instr.src1()).xmm64u(1));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    /// VMOVLHPS xmm1, xmm2, xmm3 — high qword = xmm3 low qword.
    pub(super) fn vmovlhps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64u(1, self.read_xmm_reg(instr.src1()).xmm64u(0));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Duplicating moves — VMOVSLDUP/VMOVSHDUP/VMOVDDUP (no vvvv; Bochs
    // avx.cc VMOVSLDUP_VpsWpsR, VMOVDDUP_VpdWpdR). VL=128 VMOVDDUP reads
    // only 64 bits from memory.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vmovsldup(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm32u(i * 2, op2.ymm32u(i * 2));
                result.set_ymm32u(i * 2 + 1, op2.ymm32u(i * 2));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm32u(i * 2, op2.xmm32u(i * 2));
                result.set_xmm32u(i * 2 + 1, op2.xmm32u(i * 2));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vmovshdup(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm32u(i * 2, op2.ymm32u(i * 2 + 1));
                result.set_ymm32u(i * 2 + 1, op2.ymm32u(i * 2 + 1));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm32u(i * 2, op2.xmm32u(i * 2 + 1));
                result.set_xmm32u(i * 2 + 1, op2.xmm32u(i * 2 + 1));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    /// VMOVDDUP — VL=128 duplicates the low qword (memory form reads m64);
    /// VL=256 duplicates the even qwords per 128-bit lane.
    pub(super) fn vmovddup(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let v = op2.ymm64u(lane * 2);
                result.set_ymm64u(lane * 2, v);
                result.set_ymm64u(lane * 2 + 1, v);
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let low = if instr.mod_c0() {
                self.read_xmm_reg(instr.src1()).xmm64u(0)
            } else {
                let seg = BxSegregs::from(instr.seg());
                let eaddr = self.resolve_addr(instr);
                self.v_read_qword(seg, eaddr)?
            };
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, low);
            result.set_xmm64u(1, low);
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Scalar conversions with vvvv upper-element pass-through
    // (Bochs avx_cvt.cc VCVTSS2SD_VsdWssR, VCVTSI2SD_VsdEdR, ...).
    // Host-float casts are round-to-nearest, matching default MXCSR like
    // the legacy handlers.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vcvtss2sd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64f(0, w as f64);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtsd2ss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm32f(0, w as f32);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtsi2sd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src1().into()) as i32
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)? as i32
        };
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64f(0, op2 as f64);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtsi2sd_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src1() as usize) as i64
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)? as i64
        };
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64f(0, op2 as f64);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtsi2ss_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src1().into()) as i32
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_dword(seg, eaddr)? as i32
        };
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm32f(0, op2 as f32);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vcvtsi2ss_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src1() as usize) as i64
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)? as i64
        };
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm32f(0, op2 as f32);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Rounding — VROUNDPS/PD (no vvvv) and VROUNDSS/SD (vvvv upper
    // pass-through). Bochs avx_pfp.cc VROUNDPS_VpsWpsIbR.
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vroundps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let imm8 = instr.ib();
        let rc = self.mxcsr.rounding_mode();
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32f(i, super::sse_pfp::sse_round_f32(op2.ymm32f(i), imm8, rc));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32f(i, super::sse_pfp::sse_round_f32(op2.xmm32f(i), imm8, rc));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vroundpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let imm8 = instr.ib();
        let rc = self.mxcsr.rounding_mode();
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..4 {
                result.set_ymm64f(i, super::sse_pfp::sse_round_f64(op2.ymm64f(i), imm8, rc));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..2 {
                result.set_xmm64f(i, super::sse_pfp::sse_round_f64(op2.xmm64f(i), imm8, rc));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vroundss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let imm8 = instr.ib();
        let rc = self.mxcsr.rounding_mode();
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm32f(0, super::sse_pfp::sse_round_f32(w, imm8, rc));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vroundsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let imm8 = instr.ib();
        let rc = self.mxcsr.rounding_mode();
        let w = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm64f(0, super::sse_pfp::sse_round_f64(w, imm8, rc));
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // Reciprocal approximations — VRCPPS/VRSQRTPS (no vvvv) and
    // VRCPSS/VRSQRTSS (vvvv upper pass-through). Full-precision host math,
    // same documented divergence as legacy sse_rcp.rs (real hardware is
    // ~12-bit approximate).
    // ════════════════════════════════════════════════════════════════════

    pub(super) fn vrcpps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32f(i, 1.0f32 / op2.ymm32f(i));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32f(i, 1.0f32 / op2.xmm32f(i));
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vrsqrtps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for i in 0..8 {
                result.set_ymm32f(i, 1.0f32 / op2.ymm32f(i).sqrt());
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            for i in 0..4 {
                result.set_xmm32f(i, 1.0f32 / op2.xmm32f(i).sqrt());
            }
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }

    pub(super) fn vrcpss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm32f(0, 1.0f32 / w);
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vrsqrtss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let w = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.src2());
        result.set_xmm32f(0, 1.0f32 / w.sqrt());
        self.write_xmm_reg(instr.dst(), result);
        Ok(())
    }

    pub(super) fn vunpckhpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        if instr.get_vl() >= 1 {
            let op1 = self.read_ymm_reg(instr.src2());
            let op2 = self.vex_read_src2_ymm(instr)?;
            let mut result = BxPackedYmmRegister::default();
            for lane in 0..2 {
                let base = lane * 2;
                result.set_ymm64u(base, op1.ymm64u(base + 1));
                result.set_ymm64u(base + 1, op2.ymm64u(base + 1));
            }
            self.write_ymm_reg(instr.dst(), result);
        } else {
            let op1 = self.read_xmm_reg(instr.src2());
            let op2 = self.vex_read_src2_xmm(instr)?;
            let mut result = BxPackedXmmRegister::default();
            result.set_xmm64u(0, op1.xmm64u(1));
            result.set_xmm64u(1, op2.xmm64u(1));
            self.write_xmm_reg(instr.dst(), result);
        }
        Ok(())
    }
}
