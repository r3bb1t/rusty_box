

//! AVX-512F scalar floating-point instruction handlers
//!
//! Implements EVEX-encoded scalar FP operations (VADDSS/SD, VSUBSS/SD,
//! VMULSS/SD, VDIVSS/SD, VSQRTSS/SD, VMAXSS/SD, VMINSS/SD, VMOVSS/SD).
//!
//! Scalar instructions operate on element [0] only. Upper elements come from
//! src1 (the VEX.vvvv operand). Opmask bit 0 controls merging/zeroing of
//! the scalar result element.
//!
//! Mirrors Bochs `cpu/avx/avx512_pfp.cc`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedZmmRegister,
};

/// Read opmask value for masking. k0 returns all-ones (no masking).
#[inline]
fn read_opmask_for_write<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, instr: &Instruction) -> u64 {
    let k = instr.opmask();
    if k == 0 {
        u64::MAX
    } else {
        // SAFETY: opmask register union always valid for rrx (full 64-bit) access
        cpu.opmask_rrx(k as usize)
    }
}

/// Read ZMM register as a ZMM-width value.
#[inline]
fn read_zmm<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, reg: u8) -> BxPackedZmmRegister {
    cpu.vmm[reg as usize]
}

/// Write scalar f32 result to dst ZMM register.
///
/// Element [0] is the result, subject to opmask bit 0 merge/zero masking.
/// Elements [1..3] come from src1. Elements [4..15] are zeroed (EVEX clears
/// upper bits).
fn write_scalar_ss<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    dst_reg: u8,
    src1: &BxPackedZmmRegister,
    result_elem0: f32,
    mask: u64,
    zero_masking: bool,
) {
    let dst = &mut cpu.vmm[dst_reg as usize];
    // Element [0]: apply opmask bit 0
    if (mask & 1) != 0 {
        dst.set_zmm32f(0, result_elem0);
    } else if zero_masking {
        dst.set_zmm32u(0, 0);
    }
    // else: merge masking — keep original dst[0]

    // Elements [1..3] from src1
    dst.set_zmm32u(1, src1.zmm32u(1));
    dst.set_zmm32u(2, src1.zmm32u(2));
    dst.set_zmm32u(3, src1.zmm32u(3));

    // Zero upper elements [4..15] (EVEX always clears upper)
    for i in 4..16 {
        dst.set_zmm32u(i, 0);
    }
}

/// Write scalar f64 result to dst ZMM register.
///
/// Element [0] is the result, subject to opmask bit 0 merge/zero masking.
/// Element [1] comes from src1. Elements [2..7] are zeroed.
fn write_scalar_sd<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    dst_reg: u8,
    src1: &BxPackedZmmRegister,
    result_elem0: f64,
    mask: u64,
    zero_masking: bool,
) {
    let dst = &mut cpu.vmm[dst_reg as usize];
    // Element [0]: apply opmask bit 0
    if (mask & 1) != 0 {
        dst.set_zmm64f(0, result_elem0);
    } else if zero_masking {
        dst.set_zmm64u(0, 0);
    }
    // else: merge masking — keep original dst[0]

    // Element [1] from src1
    dst.set_zmm64u(1, src1.zmm64u(1));

    // Zero upper elements [2..7]
    for i in 2..8 {
        dst.set_zmm64u(i, 0);
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // Helper: read scalar f32 source operand (register or memory)
    // ========================================================================

    /// Read scalar f32 from src2 (register) or memory.
    /// Register form: returns zmm32f[0] of src2.
    /// Memory form: reads 4 bytes from memory.
    #[inline]
    /// Read scalar f32 from rm operand (src1 in our convention).
    /// Register form: XMM element [0] of src1 (rm).
    /// Memory form: reads 4 bytes from memory.
    fn evex_read_rm_ss(&mut self, instr: &Instruction) -> super::Result<f32> {
        if instr.mod_c0() {
            Ok(self.vmm[instr.src1() as usize].zmm32f(0))
        } else {
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let bits = self.v_read_dword(seg, laddr)?;
            Ok(f32::from_bits(bits))
        }
    }

    /// Read scalar f64 from src2 (register) or memory.
    /// Register form: returns zmm64f[0] of src2.
    /// Memory form: reads 8 bytes from memory.
    #[inline]
    /// Read scalar f64 from rm operand (src1 in our convention).
    fn evex_read_rm_sd(&mut self, instr: &Instruction) -> super::Result<f64> {
        if instr.mod_c0() {
            Ok(self.vmm[instr.src1() as usize].zmm64f(0))
        } else {
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let lo = self.v_read_dword(seg, laddr)? as u64;
            let hi = self.v_read_dword(seg, laddr + 4)? as u64;
            Ok(f64::from_bits(lo | (hi << 32)))
        }
    }

    // ========================================================================
    // VADDSS / VADDSD — Add Scalar Single/Double-Precision
    // EVEX.LIG.F3.0F.W0 58 / EVEX.LIG.F2.0F.W1 58
    // ========================================================================

    /// VADDSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vaddss(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_ss(instr)?;
        let result = src1.zmm32f(0) + src2_val ;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VADDSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vaddsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_sd(instr)?;
        let result = src1.zmm64f(0) + src2_val ;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    // ========================================================================
    // VSUBSS / VSUBSD — Subtract Scalar Single/Double-Precision
    // EVEX.LIG.F3.0F.W0 5C / EVEX.LIG.F2.0F.W1 5C
    // ========================================================================

    /// VSUBSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vsubss(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_ss(instr)?;
        let result = src1.zmm32f(0) - src2_val ;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VSUBSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vsubsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_sd(instr)?;
        let result = src1.zmm64f(0) - src2_val ;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    // ========================================================================
    // VMULSS / VMULSD — Multiply Scalar Single/Double-Precision
    // EVEX.LIG.F3.0F.W0 59 / EVEX.LIG.F2.0F.W1 59
    // ========================================================================

    /// VMULSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vmulss(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_ss(instr)?;
        let result = src1.zmm32f(0) * src2_val ;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VMULSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vmulsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_sd(instr)?;
        let result = src1.zmm64f(0) * src2_val ;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    // ========================================================================
    // VDIVSS / VDIVSD — Divide Scalar Single/Double-Precision
    // EVEX.LIG.F3.0F.W0 5E / EVEX.LIG.F2.0F.W1 5E
    // ========================================================================

    /// VDIVSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vdivss(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_ss(instr)?;
        let result = src1.zmm32f(0) / src2_val ;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VDIVSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vdivsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_sd(instr)?;
        let result = src1.zmm64f(0) / src2_val ;
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    // ========================================================================
    // VSQRTSS / VSQRTSD — Square Root of Scalar Single/Double-Precision
    // EVEX.LIG.F3.0F.W0 51 / EVEX.LIG.F2.0F.W1 51
    // ========================================================================

    /// VSQRTSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vsqrtss(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_ss(instr)?;
        let result = src2_val.sqrt();
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VSQRTSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vsqrtsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_sd(instr)?;
        let result = src2_val.sqrt();
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    // ========================================================================
    // VMAXSS / VMAXSD — Maximum of Scalar Single/Double-Precision
    // EVEX.LIG.F3.0F.W0 5F / EVEX.LIG.F2.0F.W1 5F
    //
    // SSE MAX semantics: if either operand is NaN, return src2 (source).
    // If src2 > src1, return src2; else return src1.
    // ========================================================================

    /// VMAXSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vmaxss(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_ss(instr)?;
        let src1_val = src1.zmm32f(0);
        let result = if src1_val.is_nan() || src2_val.is_nan() || src2_val > src1_val {
            src2_val
        } else {
            src1_val
        };
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VMAXSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vmaxsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_sd(instr)?;
        let src1_val = src1.zmm64f(0);
        let result = if src1_val.is_nan() || src2_val.is_nan() || src2_val > src1_val {
            src2_val
        } else {
            src1_val
        };
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    // ========================================================================
    // VMINSS / VMINSD — Minimum of Scalar Single/Double-Precision
    // EVEX.LIG.F3.0F.W0 5D / EVEX.LIG.F2.0F.W1 5D
    //
    // SSE MIN semantics: if either operand is NaN, return src2 (source).
    // If src2 < src1, return src2; else return src1.
    // ========================================================================

    /// VMINSS xmm1{k1}{z}, xmm2, xmm3/m32
    pub fn evex_vminss(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_ss(instr)?;
        let src1_val = src1.zmm32f(0);
        let result = if src1_val.is_nan() || src2_val.is_nan() || src2_val < src1_val {
            src2_val
        } else {
            src1_val
        };
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_ss(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    /// VMINSD xmm1{k1}{z}, xmm2, xmm3/m64
    pub fn evex_vminsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
        let src2_val = self.evex_read_rm_sd(instr)?;
        let src1_val = src1.zmm64f(0);
        let result = if src1_val.is_nan() || src2_val.is_nan() || src2_val < src1_val {
            src2_val
        } else {
            src1_val
        };
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_scalar_sd(self, instr.dst(), &src1, result, mask, zmask);
        Ok(())
    }

    // ========================================================================
    // VMOVSS — Move Scalar Single-Precision
    // EVEX.LIG.F3.0F.W0 10 (load) / EVEX.LIG.F3.0F.W0 11 (store)
    //
    // Memory form load: dst[0] = mem32, dst[1..15] = 0
    // Register form load: dst[0] = src2[0], dst[1..3] = src1[1..3], dst[4..15] = 0
    // Memory form store: mem32 = src[0]
    // Register form store: dst[0] = src[0], dst[1..3] = src1[1..3], dst[4..15] = 0
    // ========================================================================

    /// VMOVSS xmm1{k1}{z}, xmm2, xmm3 (register form load)
    /// VMOVSS xmm1{k1}{z}, m32 (memory form load)
    pub fn evex_vmovss_load(&mut self, instr: &Instruction) -> super::Result<()> {
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        if instr.mod_c0() {
            // Register form: dst[0] = src2[0], dst[1..3] = src1[1..3], zero upper
            let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
            let src2 = read_zmm(self, instr.src2());
            let val = src2.zmm32f(0);
            write_scalar_ss(self, instr.dst(), &src1, val, mask, zmask);
        } else {
            // Memory form: dst[0] = mem32, rest zeroed
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let bits = self.v_read_dword(seg, laddr)?;
            let val = f32::from_bits(bits);

            let dst = &mut self.vmm[instr.dst() as usize];
            // Element [0]: apply opmask bit 0
            if (mask & 1) != 0 {
                dst.set_zmm32f(0, val);
            } else if zmask {
                dst.set_zmm32u(0, 0);
            }
            // Memory form: all other elements zeroed
            for i in 1..16 {
                dst.set_zmm32u(i, 0);
            }
        }
        Ok(())
    }

    /// VMOVSD xmm1{k1}{z}, xmm2, xmm3 (register form load)
    /// VMOVSD xmm1{k1}{z}, m64 (memory form load)
    pub fn evex_vmovsd_load(&mut self, instr: &Instruction) -> super::Result<()> {
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        if instr.mod_c0() {
            // Register form: dst[0] = src2[0], dst[1] = src1[1], zero upper
            let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
            let src2 = read_zmm(self, instr.src2());
            let val = src2.zmm64f(0);
            write_scalar_sd(self, instr.dst(), &src1, val, mask, zmask);
        } else {
            // Memory form: dst[0] = mem64, rest zeroed
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let lo = self.v_read_dword(seg, laddr)? as u64;
            let hi = self.v_read_dword(seg, laddr + 4)? as u64;
            let val = f64::from_bits(lo | (hi << 32));

            let dst = &mut self.vmm[instr.dst() as usize];
            // Element [0]: apply opmask bit 0
            if (mask & 1) != 0 {
                dst.set_zmm64f(0, val);
            } else if zmask {
                dst.set_zmm64u(0, 0);
            }
            // Memory form: all other elements zeroed
            for i in 1..8 {
                dst.set_zmm64u(i, 0);
            }
        }
        Ok(())
    }

    /// VMOVSS xmm1/m32{k1}, xmm2 (register form store)
    /// VMOVSS m32{k1}, xmm1 (memory form store)
    pub fn evex_vmovss_store(&mut self, instr: &Instruction) -> super::Result<()> {
        let mask = read_opmask_for_write(self, instr);

        if instr.mod_c0() {
            // Register form store: dst[0] = src[0], dst[1..3] = src1[1..3], zero upper
            let src = read_zmm(self, instr.src());
            let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
            let zmask = instr.is_zero_masking() != 0;
            let val = src.zmm32f(0);
            write_scalar_ss(self, instr.dst(), &src1, val, mask, zmask);
        } else {
            // Memory form store: write element [0] to memory
            if (mask & 1) != 0 {
                let src = read_zmm(self, instr.src());
                let laddr = self.resolve_addr(instr);
                let seg = BxSegregs::from(instr.seg());
                let bits = src.zmm32u(0);
                self.v_write_dword(seg, laddr, bits)?;
            }
        }
        Ok(())
    }

    /// VMOVSD xmm1/m64{k1}, xmm2 (register form store)
    /// VMOVSD m64{k1}, xmm1 (memory form store)
    pub fn evex_vmovsd_store(&mut self, instr: &Instruction) -> super::Result<()> {
        let mask = read_opmask_for_write(self, instr);

        if instr.mod_c0() {
            // Register form store: dst[0] = src[0], dst[1] = src1[1], zero upper
            let src = read_zmm(self, instr.src());
            let src1 = read_zmm(self, instr.src2()); // vvvv — provides upper elements
            let zmask = instr.is_zero_masking() != 0;
            let val = src.zmm64f(0);
            write_scalar_sd(self, instr.dst(), &src1, val, mask, zmask);
        } else {
            // Memory form store: write element [0] to memory
            if (mask & 1) != 0 {
                let src = read_zmm(self, instr.src());
                let laddr = self.resolve_addr(instr);
                let seg = BxSegregs::from(instr.seg());
                let val = src.zmm64u(0);
                let lo = val as u32;
                let hi = (val >> 32) as u32;
                self.v_write_dword(seg, laddr, lo)?;
                self.v_write_dword(seg, laddr + 4, hi)?;
            }
        }
        Ok(())
    }
}
