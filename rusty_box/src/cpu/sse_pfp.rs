//! SSE/SSE2 packed floating-point instruction handlers
//!
//! Based on Bochs cpu/sse_pfp.cc
//! Copyright (C) 2001-2018 The Bochs Project
//!
//! Implements SSE/SSE2 packed and scalar floating-point operations:
//! - Arithmetic: ADD, SUB, MUL, DIV, SQRT, MIN, MAX (ps/pd/ss/sd)
//! - Bitwise logical: AND, ANDN, OR, XOR (ps/pd)
//! - Compare: CMPPS/PD/SS/SD (8 predicates), COMISS/COMISD, UCOMISS/UCOMISD
//! - Conversions: CVTSI2SS/SD, CVTSS2SI/SD2SI, CVTTSS2SI/CVTTSD2SI,
//!   CVTPS2PD, CVTPD2PS, CVTSS2SD, CVTSD2SS, CVTDQ2PS, CVTPS2DQ,
//!   CVTTPS2DQ, CVTDQ2PD, CVTPD2DQ, CVTTPD2DQ
//! - Shuffle: SHUFPS/PD, UNPCKLPS/PD, UNPCKHPS/PD
//!
//! Uses native Rust f32/f64 operations for floating-point math. While Bochs
//! uses SoftFloat3e, native FP is sufficient since we run on x86 host with
//! the same FP behavior. SoftFloat integration can be added later if needed.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
    xmm::BxPackedXmmRegister,
};

// ============================================================================
// Compare predicate helper
// ============================================================================

/// Evaluate SSE compare predicate (imm8 bits[2:0]) for f32 operands.
/// Returns true if the comparison is satisfied.
#[inline]
fn sse_compare_f32(op1: f32, op2: f32, predicate: u8) -> bool {
    match predicate & 7 {
        0 => op1 == op2,                                 // EQ
        1 => op1 < op2,                                  // LT
        2 => op1 <= op2,                                 // LE
        3 => op1.is_nan() || op2.is_nan(),               // UNORD
        4 => op1 != op2 || op1.is_nan() || op2.is_nan(), // NEQ (unordered or not equal)
        5 => !(op1 < op2),                               // NLT (not less than)
        6 => !(op1 <= op2),                              // NLE (not less than or equal)
        7 => !op1.is_nan() && !op2.is_nan(),             // ORD
        _ => unreachable!(),
    }
}

/// Evaluate SSE compare predicate (imm8 bits[2:0]) for f64 operands.
/// Returns true if the comparison is satisfied.
#[inline]
fn sse_compare_f64(op1: f64, op2: f64, predicate: u8) -> bool {
    match predicate & 7 {
        0 => op1 == op2,                                 // EQ
        1 => op1 < op2,                                  // LT
        2 => op1 <= op2,                                 // LE
        3 => op1.is_nan() || op2.is_nan(),               // UNORD
        4 => op1 != op2 || op1.is_nan() || op2.is_nan(), // NEQ
        5 => !(op1 < op2),                               // NLT
        6 => !(op1 <= op2),                              // NLE
        7 => !op1.is_nan() && !op2.is_nan(),             // ORD
        _ => unreachable!(),
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // SSE FP helpers: read source operand (register or memory)
    // ========================================================================

    /// Read source operand as packed 128-bit XMM (for PS/PD packed ops).
    #[inline]
    fn sse_pfp_read_op2_xmm(&mut self, instr: &Instruction) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src1()))
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_xmmword(seg, eaddr)
        }
    }

    /// Read source operand as scalar f32 (for SS scalar single ops).
    /// Register form: read lowest f32 from XMM src1.
    /// Memory form: read dword from memory, reinterpret as f32.
    #[inline]
    fn sse_pfp_read_op2_ss(&mut self, instr: &Instruction) -> super::Result<f32> {
        if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            Ok(unsafe { src.xmm32f[0] })
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.read_virtual_dword(seg, eaddr)?;
            Ok(f32::from_bits(val))
        }
    }

    /// Read source operand as scalar f64 (for SD scalar double ops).
    /// Register form: read lowest f64 from XMM src1.
    /// Memory form: read qword from memory, reinterpret as f64.
    #[inline]
    fn sse_pfp_read_op2_sd(&mut self, instr: &Instruction) -> super::Result<f64> {
        if instr.mod_c0() {
            let src = self.read_xmm_reg(instr.src1());
            Ok(unsafe { src.xmm64f[0] })
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            let val = self.read_virtual_qword(seg, eaddr)?;
            Ok(f64::from_bits(val))
        }
    }

    /// Set EFLAGS for COMISS/COMISD/UCOMISS/UCOMISD comparison.
    /// Clears OF, SF, AF. Sets ZF, PF, CF based on result.
    #[inline]
    fn sse_set_eflags_compare(&mut self, unordered: bool, less: bool, equal: bool) {
        // Clear OF, SF, AF first
        self.eflags.remove(EFlags::OF | EFlags::SF | EFlags::AF);
        // Clear ZF, PF, CF — then set as needed
        self.eflags.remove(EFlags::ZF | EFlags::PF | EFlags::CF);

        if unordered {
            // NaN: ZF=1, PF=1, CF=1
            self.eflags.insert(EFlags::ZF | EFlags::PF | EFlags::CF);
        } else if less {
            // op1 < op2: CF=1
            self.eflags.insert(EFlags::CF);
        } else if equal {
            // op1 == op2: ZF=1
            self.eflags.insert(EFlags::ZF);
        }
        // op1 > op2: all clear (done above)
    }

    // ========================================================================
    // Arithmetic: ADDPS/PD/SS/SD
    // Bochs: ADDPS_VpsWps, ADDPD_VpdWpd, ADDSS_VssWss, ADDSD_VsdWsd
    // ========================================================================

    /// ADDPS — Add Packed Single-Precision (4 x f32)
    pub(super) fn addps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                result.xmm32f[i] = op1.xmm32f[i] + op2.xmm32f[i];
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ADDPD — Add Packed Double-Precision (2 x f64)
    pub(super) fn addpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                result.xmm64f[i] = op1.xmm64f[i] + op2.xmm64f[i];
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ADDSS — Add Scalar Single-Precision (lowest f32 only)
    pub(super) fn addss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] += op2;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ADDSD — Add Scalar Double-Precision (lowest f64 only)
    pub(super) fn addsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] += op2;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: SUBPS/PD/SS/SD
    // Bochs: SUBPS_VpsWps, SUBPD_VpdWpd, SUBSS_VssWss, SUBSD_VsdWsd
    // ========================================================================

    /// SUBPS — Subtract Packed Single-Precision (4 x f32)
    pub(super) fn subps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                result.xmm32f[i] = op1.xmm32f[i] - op2.xmm32f[i];
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SUBPD — Subtract Packed Double-Precision (2 x f64)
    pub(super) fn subpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                result.xmm64f[i] = op1.xmm64f[i] - op2.xmm64f[i];
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SUBSS — Subtract Scalar Single-Precision (lowest f32 only)
    pub(super) fn subss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] -= op2;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SUBSD — Subtract Scalar Double-Precision (lowest f64 only)
    pub(super) fn subsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] -= op2;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: MULPS/PD/SS/SD
    // Bochs: MULPS_VpsWps, MULPD_VpdWpd, MULSS_VssWss, MULSD_VsdWsd
    // ========================================================================

    /// MULPS — Multiply Packed Single-Precision (4 x f32)
    pub(super) fn mulps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                result.xmm32f[i] = op1.xmm32f[i] * op2.xmm32f[i];
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MULPD — Multiply Packed Double-Precision (2 x f64)
    pub(super) fn mulpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                result.xmm64f[i] = op1.xmm64f[i] * op2.xmm64f[i];
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MULSS — Multiply Scalar Single-Precision (lowest f32 only)
    pub(super) fn mulss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] *= op2;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MULSD — Multiply Scalar Double-Precision (lowest f64 only)
    pub(super) fn mulsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] *= op2;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: DIVPS/PD/SS/SD
    // Bochs: DIVPS_VpsWps, DIVPD_VpdWpd, DIVSS_VssWss, DIVSD_VsdWsd
    // ========================================================================

    /// DIVPS — Divide Packed Single-Precision (4 x f32)
    pub(super) fn divps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                result.xmm32f[i] = op1.xmm32f[i] / op2.xmm32f[i];
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// DIVPD — Divide Packed Double-Precision (2 x f64)
    pub(super) fn divpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                result.xmm64f[i] = op1.xmm64f[i] / op2.xmm64f[i];
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// DIVSS — Divide Scalar Single-Precision (lowest f32 only)
    pub(super) fn divss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] /= op2;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// DIVSD — Divide Scalar Double-Precision (lowest f64 only)
    pub(super) fn divsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] /= op2;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: SQRTPS/PD/SS/SD
    // Bochs: SQRTPS_VpsWps, SQRTPD_VpdWpd, SQRTSS_VssWss, SQRTSD_VsdWsd
    // ========================================================================

    /// SQRTPS — Square Root of Packed Single-Precision (4 x f32)
    pub(super) fn sqrtps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                result.xmm32f[i] = op.xmm32f[i].sqrt();
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SQRTPD — Square Root of Packed Double-Precision (2 x f64)
    pub(super) fn sqrtpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                result.xmm64f[i] = op.xmm64f[i].sqrt();
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SQRTSS — Square Root of Scalar Single-Precision (lowest f32 only)
    pub(super) fn sqrtss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] = op2.sqrt();
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SQRTSD — Square Root of Scalar Double-Precision (lowest f64 only)
    pub(super) fn sqrtsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] = op2.sqrt();
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: MINPS/PD/SS/SD
    // Bochs: MINPS_VpsWps, MINPD_VpdWpd, MINSS_VssWss, MINSD_VsdWsd
    // Note: SSE MIN semantics: if either operand is NaN, return op2 (source).
    // If op2 < op1, return op2; else return op1.
    // ========================================================================

    /// MINPS — Minimum of Packed Single-Precision (4 x f32)
    pub(super) fn minps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                result.xmm32f[i] = if op2.xmm32f[i] < op1.xmm32f[i] {
                    op2.xmm32f[i]
                } else {
                    op1.xmm32f[i]
                };
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MINPD — Minimum of Packed Double-Precision (2 x f64)
    pub(super) fn minpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                result.xmm64f[i] = if op2.xmm64f[i] < op1.xmm64f[i] {
                    op2.xmm64f[i]
                } else {
                    op1.xmm64f[i]
                };
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MINSS — Minimum of Scalar Single-Precision (lowest f32 only)
    pub(super) fn minss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] = if op2 < result.xmm32f[0] {
                op2
            } else {
                result.xmm32f[0]
            };
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MINSD — Minimum of Scalar Double-Precision (lowest f64 only)
    pub(super) fn minsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] = if op2 < result.xmm64f[0] {
                op2
            } else {
                result.xmm64f[0]
            };
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Arithmetic: MAXPS/PD/SS/SD
    // Bochs: MAXPS_VpsWps, MAXPD_VpdWpd, MAXSS_VssWss, MAXSD_VsdWsd
    // Note: SSE MAX semantics: if either operand is NaN, return op2 (source).
    // If op2 > op1, return op2; else return op1.
    // ========================================================================

    /// MAXPS — Maximum of Packed Single-Precision (4 x f32)
    pub(super) fn maxps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                result.xmm32f[i] = if op2.xmm32f[i] > op1.xmm32f[i] {
                    op2.xmm32f[i]
                } else {
                    op1.xmm32f[i]
                };
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MAXPD — Maximum of Packed Double-Precision (2 x f64)
    pub(super) fn maxpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                result.xmm64f[i] = if op2.xmm64f[i] > op1.xmm64f[i] {
                    op2.xmm64f[i]
                } else {
                    op1.xmm64f[i]
                };
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MAXSS — Maximum of Scalar Single-Precision (lowest f32 only)
    pub(super) fn maxss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] = if op2 > result.xmm32f[0] {
                op2
            } else {
                result.xmm32f[0]
            };
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// MAXSD — Maximum of Scalar Double-Precision (lowest f64 only)
    pub(super) fn maxsd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] = if op2 > result.xmm64f[0] {
                op2
            } else {
                result.xmm64f[0]
            };
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Bitwise Logical: ANDPS/ANDPD
    // Bochs: ANDPS_VpsWps, ANDPD_VpdWpd
    // ========================================================================

    /// ANDPS — Bitwise AND of Packed Single-Precision (128-bit)
    pub(super) fn andps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[0] & op2.xmm64u[0];
            result.xmm64u[1] = op1.xmm64u[1] & op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ANDPD — Bitwise AND of Packed Double-Precision (128-bit)
    pub(super) fn andpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[0] & op2.xmm64u[0];
            result.xmm64u[1] = op1.xmm64u[1] & op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Bitwise Logical: ANDNPS/ANDNPD
    // Bochs: ANDNPS_VpsWps, ANDNPD_VpdWpd
    // ========================================================================

    /// ANDNPS — Bitwise AND NOT of Packed Single-Precision (128-bit)
    /// Result = NOT(op1) AND op2
    pub(super) fn andnps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = (!op1.xmm64u[0]) & op2.xmm64u[0];
            result.xmm64u[1] = (!op1.xmm64u[1]) & op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ANDNPD — Bitwise AND NOT of Packed Double-Precision (128-bit)
    /// Result = NOT(op1) AND op2
    pub(super) fn andnpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = (!op1.xmm64u[0]) & op2.xmm64u[0];
            result.xmm64u[1] = (!op1.xmm64u[1]) & op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Bitwise Logical: ORPS/ORPD
    // Bochs: ORPS_VpsWps, ORPD_VpdWpd
    // ========================================================================

    /// ORPS — Bitwise OR of Packed Single-Precision (128-bit)
    pub(super) fn orps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[0] | op2.xmm64u[0];
            result.xmm64u[1] = op1.xmm64u[1] | op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// ORPD — Bitwise OR of Packed Double-Precision (128-bit)
    pub(super) fn orpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[0] | op2.xmm64u[0];
            result.xmm64u[1] = op1.xmm64u[1] | op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Bitwise Logical: XORPS/XORPD
    // Bochs: XORPS_VpsWps, XORPD_VpdWpd
    // ========================================================================

    /// XORPS — Bitwise XOR of Packed Single-Precision (128-bit)
    pub(super) fn xorps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[0] ^ op2.xmm64u[0];
            result.xmm64u[1] = op1.xmm64u[1] ^ op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// XORPD — Bitwise XOR of Packed Double-Precision (128-bit)
    pub(super) fn xorpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[0] ^ op2.xmm64u[0];
            result.xmm64u[1] = op1.xmm64u[1] ^ op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Compare: CMPPS/CMPPD/CMPSS/CMPSD (8 predicates via imm8)
    // Bochs: CMPPS_VpsWpsIb, CMPPD_VpdWpdIb, CMPSS_VssWssIb, CMPSD_VsdWsdIb
    // Result: all-ones mask if true, all-zeros if false
    // ========================================================================

    /// CMPPS — Compare Packed Single-Precision (4 x f32) with imm8 predicate
    pub(super) fn cmpps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let predicate = instr.ib();
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                result.xmm32u[i] = if sse_compare_f32(op1.xmm32f[i], op2.xmm32f[i], predicate) {
                    0xFFFF_FFFF
                } else {
                    0
                };
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CMPPD — Compare Packed Double-Precision (2 x f64) with imm8 predicate
    pub(super) fn cmppd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let predicate = instr.ib();
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                result.xmm64u[i] = if sse_compare_f64(op1.xmm64f[i], op2.xmm64f[i], predicate) {
                    0xFFFF_FFFF_FFFF_FFFF
                } else {
                    0
                };
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CMPSS — Compare Scalar Single-Precision (lowest f32) with imm8 predicate
    pub(super) fn cmpss_vss_wss_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        let op1 = unsafe { result.xmm32f[0] };
        let predicate = instr.ib();
        unsafe {
            result.xmm32u[0] = if sse_compare_f32(op1, op2, predicate) {
                0xFFFF_FFFF
            } else {
                0
            };
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CMPSD — Compare Scalar Double-Precision (lowest f64) with imm8 predicate
    pub(super) fn cmpsd_vsd_wsd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        let op1 = unsafe { result.xmm64f[0] };
        let predicate = instr.ib();
        unsafe {
            result.xmm64u[0] = if sse_compare_f64(op1, op2, predicate) {
                0xFFFF_FFFF_FFFF_FFFF
            } else {
                0
            };
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Compare: COMISS/COMISD — Ordered Compare Scalar to EFLAGS
    // Bochs: COMISS_VssWss, COMISD_VsdWsd
    // Sets ZF, PF, CF; clears OF, SF, AF
    // Raises #IA for any NaN (SNaN or QNaN)
    // ========================================================================

    /// COMISS — Ordered Compare Scalar Single-Precision to EFLAGS
    pub(super) fn comiss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = unsafe { self.read_xmm_reg(instr.dst()).xmm32f[0] };
        let op2 = self.sse_pfp_read_op2_ss(instr)?;

        let unordered = op1.is_nan() || op2.is_nan();
        let less = !unordered && op1 < op2;
        let equal = !unordered && op1 == op2;
        self.sse_set_eflags_compare(unordered, less, equal);
        Ok(())
    }

    /// COMISD — Ordered Compare Scalar Double-Precision to EFLAGS
    pub(super) fn comisd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = unsafe { self.read_xmm_reg(instr.dst()).xmm64f[0] };
        let op2 = self.sse_pfp_read_op2_sd(instr)?;

        let unordered = op1.is_nan() || op2.is_nan();
        let less = !unordered && op1 < op2;
        let equal = !unordered && op1 == op2;
        self.sse_set_eflags_compare(unordered, less, equal);
        Ok(())
    }

    // ========================================================================
    // Compare: UCOMISS/UCOMISD — Unordered Compare Scalar to EFLAGS
    // Bochs: UCOMISS_VssWss, UCOMISD_VsdWsd
    // Sets ZF, PF, CF; clears OF, SF, AF
    // Same behavior as COMISS/COMISD but does not raise #IA for QNaN
    // (For our emulator, we don't raise #IA exceptions anyway)
    // ========================================================================

    /// UCOMISS — Unordered Compare Scalar Single-Precision to EFLAGS
    pub(super) fn ucomiss_vss_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = unsafe { self.read_xmm_reg(instr.dst()).xmm32f[0] };
        let op2 = self.sse_pfp_read_op2_ss(instr)?;

        let unordered = op1.is_nan() || op2.is_nan();
        let less = !unordered && op1 < op2;
        let equal = !unordered && op1 == op2;
        self.sse_set_eflags_compare(unordered, less, equal);
        Ok(())
    }

    /// UCOMISD — Unordered Compare Scalar Double-Precision to EFLAGS
    pub(super) fn ucomisd_vsd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = unsafe { self.read_xmm_reg(instr.dst()).xmm64f[0] };
        let op2 = self.sse_pfp_read_op2_sd(instr)?;

        let unordered = op1.is_nan() || op2.is_nan();
        let less = !unordered && op1 < op2;
        let equal = !unordered && op1 == op2;
        self.sse_set_eflags_compare(unordered, less, equal);
        Ok(())
    }

    // ========================================================================
    // Conversions: Int32 to Float
    // Bochs: CVTSI2SS_VssEd, CVTSI2SD_VsdEd
    // ========================================================================

    /// CVTSI2SS — Convert Int32 to Scalar Single-Precision
    pub(super) fn cvtsi2ss_vss_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src1().into()) as i32
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_dword(seg, eaddr)? as i32
        };
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] = op2 as f32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSI2SD — Convert Int32 to Scalar Double-Precision
    pub(super) fn cvtsi2sd_vsd_ed(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr32(instr.src1().into()) as i32
        } else {
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_dword(seg, eaddr)? as i32
        };
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] = op2 as f64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Float to Int32
    // Bochs: CVTSS2SI_GdWss, CVTSD2SI_GdWsd, CVTTSS2SI_GdWss, CVTTSD2SI_GdWsd
    // Note: CVTSS2SI/CVTSD2SI use MXCSR rounding mode. We use native Rust
    // rounding (round-half-to-even) which matches the default MXCSR mode.
    // CVTTSS2SI/CVTTSD2SI always truncate toward zero.
    // ========================================================================

    /// CVTSS2SI — Convert Scalar Single-Precision to Int32 (MXCSR rounding)
    pub(super) fn cvtss2si_gd_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        // Use round-half-to-even (default MXCSR rounding mode)
        // Rust's f32::round_ties_even() matches IEEE 754 roundTiesToEven
        let result =
            if op.is_nan() || op.is_infinite() || op > i32::MAX as f32 || op < i32::MIN as f32 {
                // Integer indefinite value for out-of-range conversions
                0x8000_0000u32
            } else {
                #[cfg(not(feature = "no_std"))]
                {
                    (op.round_ties_even() as i32) as u32
                }
                #[cfg(feature = "no_std")]
                {
                    // Fallback: use truncation toward nearest (not exactly round-to-even
                    // but close enough for emulation)
                    let rounded = if op >= 0.0 {
                        (op + 0.5) as i32
                    } else {
                        (op - 0.5) as i32
                    };
                    rounded as u32
                }
            };
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    /// CVTSD2SI — Convert Scalar Double-Precision to Int32 (MXCSR rounding)
    pub(super) fn cvtsd2si_gd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let result =
            if op.is_nan() || op.is_infinite() || op > i32::MAX as f64 || op < i32::MIN as f64 {
                0x8000_0000u32
            } else {
                #[cfg(not(feature = "no_std"))]
                {
                    (op.round_ties_even() as i32) as u32
                }
                #[cfg(feature = "no_std")]
                {
                    let rounded = if op >= 0.0 {
                        (op + 0.5) as i32
                    } else {
                        (op - 0.5) as i32
                    };
                    rounded as u32
                }
            };
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    /// CVTTSS2SI — Convert Scalar Single-Precision to Int32 (truncate)
    pub(super) fn cvttss2si_gd_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let result =
            if op.is_nan() || op.is_infinite() || op > i32::MAX as f32 || op < i32::MIN as f32 {
                0x8000_0000u32
            } else {
                (op as i32) as u32
            };
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    /// CVTTSD2SI — Convert Scalar Double-Precision to Int32 (truncate)
    pub(super) fn cvttsd2si_gd_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let result =
            if op.is_nan() || op.is_infinite() || op > i32::MAX as f64 || op < i32::MIN as f64 {
                0x8000_0000u32
            } else {
                (op as i32) as u32
            };
        self.set_gpr32(instr.dst().into(), result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Int64 to Float (64-bit mode)
    // Bochs: CVTSI2SS_VssEq, CVTSI2SD_VsdEq
    // ========================================================================

    /// CVTSI2SS — Convert Int64 to Scalar Single-Precision (64-bit mode)
    pub(super) fn cvtsi2ss_vss_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src1() as usize) as i64
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)? as i64
        };
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] = op2 as f32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSI2SD — Convert Int64 to Scalar Double-Precision (64-bit mode)
    pub(super) fn cvtsi2sd_vsd_eq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.get_gpr64(instr.src1() as usize) as i64
        } else {
            let eaddr = self.resolve_addr64(instr);
            let seg = BxSegregs::from(instr.seg());
            self.read_virtual_qword_64(seg, eaddr)? as i64
        };
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] = op2 as f64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Float to Int64 (64-bit mode)
    // Bochs: CVTTSS2SI_GqWss, CVTTSD2SI_GqWsd, CVTSS2SI_GqWss, CVTSD2SI_GqWsd
    // ========================================================================

    /// CVTTSS2SI — Convert Scalar Single-Precision to Int64 (truncate, 64-bit mode)
    pub(super) fn cvttss2si_gq_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let result =
            if op.is_nan() || op.is_infinite() || op > i64::MAX as f32 || op < i64::MIN as f32 {
                0x8000_0000_0000_0000u64
            } else {
                (op as i64) as u64
            };
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// CVTTSD2SI — Convert Scalar Double-Precision to Int64 (truncate, 64-bit mode)
    pub(super) fn cvttsd2si_gq_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let result =
            if op.is_nan() || op.is_infinite() || op > i64::MAX as f64 || op < i64::MIN as f64 {
                0x8000_0000_0000_0000u64
            } else {
                (op as i64) as u64
            };
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// CVTSS2SI — Convert Scalar Single-Precision to Int64 (MXCSR rounding, 64-bit mode)
    pub(super) fn cvtss2si_gq_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_ss(instr)?;
        let result =
            if op.is_nan() || op.is_infinite() || op > i64::MAX as f32 || op < i64::MIN as f32 {
                0x8000_0000_0000_0000u64
            } else {
                #[cfg(not(feature = "no_std"))]
                {
                    (op.round_ties_even() as i64) as u64
                }
                #[cfg(feature = "no_std")]
                {
                    let rounded = if op >= 0.0 {
                        (op + 0.5) as i64
                    } else {
                        (op - 0.5) as i64
                    };
                    rounded as u64
                }
            };
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    /// CVTSD2SI — Convert Scalar Double-Precision to Int64 (MXCSR rounding, 64-bit mode)
    pub(super) fn cvtsd2si_gq_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op = self.sse_pfp_read_op2_sd(instr)?;
        let result =
            if op.is_nan() || op.is_infinite() || op > i64::MAX as f64 || op < i64::MIN as f64 {
                0x8000_0000_0000_0000u64
            } else {
                #[cfg(not(feature = "no_std"))]
                {
                    (op.round_ties_even() as i64) as u64
                }
                #[cfg(feature = "no_std")]
                {
                    let rounded = if op >= 0.0 {
                        (op + 0.5) as i64
                    } else {
                        (op - 0.5) as i64
                    };
                    rounded as u64
                }
            };
        self.set_gpr64(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Float precision conversions
    // Bochs: CVTPS2PD, CVTPD2PS, CVTSS2SD, CVTSD2SS
    // ========================================================================

    /// CVTPS2PD — Convert 2 Packed Singles to 2 Packed Doubles
    /// Reads low 2 floats from src, converts to 2 doubles in dst
    pub(super) fn cvtps2pd_vpd_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        // Only need low 64 bits (2 x f32) from source
        let op2 = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1())
        } else {
            // Read 64 bits from memory, zero-extend to 128
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            let lo = self.read_virtual_qword(seg, eaddr)?;
            let mut tmp = BxPackedXmmRegister::default();
            unsafe {
                tmp.xmm64u[0] = lo;
            }
            tmp
        };
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64f[0] = op2.xmm32f[0] as f64;
            result.xmm64f[1] = op2.xmm32f[1] as f64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTPD2PS — Convert 2 Packed Doubles to 2 Packed Singles
    /// Reads 2 doubles from src, converts to 2 singles in low part of dst
    pub(super) fn cvtpd2ps_vps_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32f[0] = op2.xmm64f[0] as f32;
            result.xmm32f[1] = op2.xmm64f[1] as f32;
            // High 64 bits zeroed (from default())
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSS2SD — Convert Scalar Single to Scalar Double
    pub(super) fn cvtss2sd_vsd_wss(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_ss(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm64f[0] = op2 as f64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTSD2SS — Convert Scalar Double to Scalar Single
    pub(super) fn cvtsd2ss_vss_wsd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_sd(instr)?;
        let mut result = self.read_xmm_reg(instr.dst());
        unsafe {
            result.xmm32f[0] = op2 as f32;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Conversions: Packed Int32 <-> Float
    // Bochs: CVTDQ2PS, CVTPS2DQ, CVTTPS2DQ, CVTDQ2PD, CVTPD2DQ, CVTTPD2DQ
    // ========================================================================

    /// CVTDQ2PS — Convert 4 Packed Int32 to 4 Packed Singles
    pub(super) fn cvtdq2ps_vps_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                result.xmm32f[i] = op2.xmm32s[i] as f32;
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTPS2DQ — Convert 4 Packed Singles to 4 Packed Int32 (MXCSR rounding)
    pub(super) fn cvtps2dq_vdq_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                let val = op2.xmm32f[i];
                result.xmm32s[i] = if val.is_nan()
                    || val.is_infinite()
                    || val > i32::MAX as f32
                    || val < i32::MIN as f32
                {
                    0x8000_0000u32 as i32
                } else {
                    #[cfg(not(feature = "no_std"))]
                    {
                        val.round_ties_even() as i32
                    }
                    #[cfg(feature = "no_std")]
                    {
                        if val >= 0.0 {
                            (val + 0.5) as i32
                        } else {
                            (val - 0.5) as i32
                        }
                    }
                };
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTTPS2DQ — Convert 4 Packed Singles to 4 Packed Int32 (truncate)
    pub(super) fn cvttps2dq_vdq_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..4 {
                let val = op2.xmm32f[i];
                result.xmm32s[i] = if val.is_nan()
                    || val.is_infinite()
                    || val > i32::MAX as f32
                    || val < i32::MIN as f32
                {
                    0x8000_0000u32 as i32
                } else {
                    val as i32
                };
            }
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTDQ2PD — Convert 2 Packed Int32 to 2 Packed Doubles
    /// Reads low 2 dwords (64 bits) from src, converts to 2 doubles in dst
    pub(super) fn cvtdq2pd_vpd_wq(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = if instr.mod_c0() {
            self.read_xmm_reg(instr.src1())
        } else {
            // Read 64 bits from memory
            let eaddr = self.resolve_addr32(instr);
            let seg = BxSegregs::from(instr.seg());
            let lo = self.read_virtual_qword(seg, eaddr)?;
            let mut tmp = BxPackedXmmRegister::default();
            unsafe {
                tmp.xmm64u[0] = lo;
            }
            tmp
        };
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64f[0] = op2.xmm32s[0] as f64;
            result.xmm64f[1] = op2.xmm32s[1] as f64;
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTPD2DQ — Convert 2 Packed Doubles to 2 Packed Int32 (MXCSR rounding)
    /// Result goes to low 64 bits of dst; high 64 bits zeroed
    pub(super) fn cvtpd2dq_vq_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                let val = op2.xmm64f[i];
                result.xmm32s[i] = if val.is_nan()
                    || val.is_infinite()
                    || val > i32::MAX as f64
                    || val < i32::MIN as f64
                {
                    0x8000_0000u32 as i32
                } else {
                    #[cfg(not(feature = "no_std"))]
                    {
                        val.round_ties_even() as i32
                    }
                    #[cfg(feature = "no_std")]
                    {
                        if val >= 0.0 {
                            (val + 0.5) as i32
                        } else {
                            (val - 0.5) as i32
                        }
                    }
                };
            }
            // High 64 bits zeroed (from default())
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// CVTTPD2DQ — Convert 2 Packed Doubles to 2 Packed Int32 (truncate)
    /// Result goes to low 64 bits of dst; high 64 bits zeroed
    pub(super) fn cvttpd2dq_vq_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            for i in 0..2 {
                let val = op2.xmm64f[i];
                result.xmm32s[i] = if val.is_nan()
                    || val.is_infinite()
                    || val > i32::MAX as f64
                    || val < i32::MIN as f64
                {
                    0x8000_0000u32 as i32
                } else {
                    val as i32
                };
            }
            // High 64 bits zeroed (from default())
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Shuffle: SHUFPS/SHUFPD
    // Bochs: SHUFPS_VpsWpsIb, SHUFPD_VpdWpdIb
    // ========================================================================

    /// SHUFPS — Shuffle Packed Single-Precision (imm8 selects lanes)
    /// Result[0] = op1[imm8[1:0]], Result[1] = op1[imm8[3:2]],
    /// Result[2] = op2[imm8[5:4]], Result[3] = op2[imm8[7:6]]
    pub(super) fn shufps_vps_wps_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let order = instr.ib();
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = op1.xmm32u[(order & 3) as usize];
            result.xmm32u[1] = op1.xmm32u[((order >> 2) & 3) as usize];
            result.xmm32u[2] = op2.xmm32u[((order >> 4) & 3) as usize];
            result.xmm32u[3] = op2.xmm32u[((order >> 6) & 3) as usize];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// SHUFPD — Shuffle Packed Double-Precision (imm8 selects lanes)
    /// Result[0] = op1[imm8[0]], Result[1] = op2[imm8[1]]
    pub(super) fn shufpd_vpd_wpd_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let order = instr.ib();
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[(order & 1) as usize];
            result.xmm64u[1] = op2.xmm64u[((order >> 1) & 1) as usize];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    // ========================================================================
    // Shuffle: UNPCKLPS/UNPCKHPS/UNPCKLPD/UNPCKHPD
    // Bochs: UNPCKLPS_VpsWps, UNPCKHPS_VpsWps, UNPCKLPD_VpdWpd, UNPCKHPD_VpdWpd
    // ========================================================================

    /// UNPCKLPS — Interleave Low Single-Precision
    /// Result = { op1[0], op2[0], op1[1], op2[1] }
    pub(super) fn unpcklps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = op1.xmm32u[0];
            result.xmm32u[1] = op2.xmm32u[0];
            result.xmm32u[2] = op1.xmm32u[1];
            result.xmm32u[3] = op2.xmm32u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// UNPCKHPS — Interleave High Single-Precision
    /// Result = { op1[2], op2[2], op1[3], op2[3] }
    pub(super) fn unpckhps_vps_wps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm32u[0] = op1.xmm32u[2];
            result.xmm32u[1] = op2.xmm32u[2];
            result.xmm32u[2] = op1.xmm32u[3];
            result.xmm32u[3] = op2.xmm32u[3];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// UNPCKLPD — Interleave Low Double-Precision
    /// Result = { op1[0], op2[0] }
    pub(super) fn unpcklpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[0];
            result.xmm64u[1] = op2.xmm64u[0];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }

    /// UNPCKHPD — Interleave High Double-Precision
    /// Result = { op1[1], op2[1] }
    pub(super) fn unpckhpd_vpd_wpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.prepare_sse()?;
        let op1 = self.read_xmm_reg(instr.dst());
        let op2 = self.sse_pfp_read_op2_xmm(instr)?;
        let mut result = BxPackedXmmRegister::default();
        unsafe {
            result.xmm64u[0] = op1.xmm64u[1];
            result.xmm64u[1] = op2.xmm64u[1];
        }
        self.write_xmm_reg_lo128(instr.dst(), result);
        Ok(())
    }
}
