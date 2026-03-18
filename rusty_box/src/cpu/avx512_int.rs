//! AVX-512F additional integer operation handlers
//!
//! Implements packed integer multiply, multiply-add, min/max, and SAD operations
//! with EVEX opmask support. Handlers work for 128/256/512-bit via `get_vl()`.
//!
//! Mirrors Bochs `cpu/avx/avx512.cc` and `cpu/avx/avx512_bw.cc` integer ops.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedZmmRegister,
};

// ============================================================================
// Element count helpers (duplicated per-file to match crate pattern)
// ============================================================================

/// Number of 16-bit elements per vector length: VL0=8, VL1=16, VL2=32
#[inline]
fn word_elements(vl: u8) -> usize {
    match vl {
        0 => 8,
        1 => 16,
        _ => 32,
    }
}

/// Number of 32-bit elements per vector length: VL0=4, VL1=8, VL2=16
#[inline]
fn dword_elements(vl: u8) -> usize {
    match vl {
        0 => 4,
        1 => 8,
        _ => 16,
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

// ============================================================================
// Opmask / register helpers (duplicated per-file to match crate pattern)
// ============================================================================

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

/// Write ZMM register for dword operations with masking
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
        }
        for i in nelements..16 {
            dst.zmm32u[i] = 0;
        }
    }
}

/// Write ZMM register for qword operations with masking
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
        for i in nelements..8 {
            dst.zmm64u[i] = 0;
        }
    }
}

/// Write ZMM register for word operations with masking
fn write_zmm_masked_w<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nelements = word_elements(vl);
    unsafe {
        let dst = &mut cpu.vmm[reg as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                dst.zmm16u[i] = result.zmm16u[i];
            } else if zero_masking {
                dst.zmm16u[i] = 0;
            }
        }
        for i in nelements..32 {
            dst.zmm16u[i] = 0;
        }
    }
}

// ============================================================================
// Memory read helpers
// ============================================================================

/// Read src2 from register or memory as dwords
fn read_src2_dwords<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let ndwords = dword_elements(vl);
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
        for i in 0..ndwords {
            let val = cpu.v_read_dword(seg, laddr + (i * 4) as u64)?;
            unsafe { tmp.zmm32u[i] = val; }
        }
        Ok(tmp)
    }
}

/// Read src2 from register or memory as qwords
fn read_src2_qwords<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let nelements = qword_elements(vl);
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
        for i in 0..nelements {
            let lo = cpu.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
            let hi = cpu.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
            unsafe { tmp.zmm64u[i] = lo | (hi << 32); }
        }
        Ok(tmp)
    }
}

/// Read src2 from register or memory as words
fn read_src2_words<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let nwords = word_elements(vl);
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
        for i in 0..nwords {
            let val = cpu.v_read_word(seg, laddr + (i * 2) as u64)?;
            unsafe { tmp.zmm16u[i] = val; }
        }
        Ok(tmp)
    }
}

/// Read src2 from register or memory as raw bytes
fn read_src2_bytes<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let nbytes = vl_bytes(vl);
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister { zmm64u: [0; 8] };
        for i in 0..nbytes {
            let val = cpu.v_read_byte(seg, laddr + i as u64)?;
            unsafe { tmp.zmmubyte[i] = val; }
        }
        Ok(tmp)
    }
}

// ============================================================================
// Saturation helper
// ============================================================================

/// Saturate an i32 to i16 range [-32768, 32767]
#[inline]
fn saturate_i16(val: i32) -> i16 {
    if val > 32767 {
        32767
    } else if val < -32768 {
        -32768
    } else {
        val as i16
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // VPMULDQ — Signed multiply packed dwords, return qword results
    // EVEX.66.0F38.W1 28
    // ========================================================================

    /// VPMULDQ Vdq{k}, Hdq, Wdq
    ///
    /// For each qword element i: multiply the low dwords of each qword pair
    /// (signed) and store the full 64-bit result.
    /// result.zmm64s[i] = (src1.zmm32s[i*2] as i64) * (src2.zmm32s[i*2] as i64)
    pub fn evex_vpmuldq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_qwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                // Multiply the low (even-indexed) dwords of each qword pair
                let a = src1.zmm32s[i * 2] as i64;
                let b = src2.zmm32s[i * 2] as i64;
                result.zmm64s[i] = a.wrapping_mul(b);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMULHUW — Unsigned multiply packed words, high result
    // EVEX.66.0F.WIG E4
    // ========================================================================

    /// VPMULHUW Vdq{k}, Hdq, Wdq
    ///
    /// For each word element: unsigned multiply, return high 16 bits.
    /// result.zmm16u[i] = ((src1.zmm16u[i] as u32 * src2.zmm16u[i] as u32) >> 16) as u16
    pub fn evex_vpmulhuw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let product = (src1.zmm16u[i] as u32) * (src2.zmm16u[i] as u32);
                result.zmm16u[i] = (product >> 16) as u16;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMULHW — Signed multiply packed words, high result
    // EVEX.66.0F.WIG E5
    // ========================================================================

    /// VPMULHW Vdq{k}, Hdq, Wdq
    ///
    /// For each word element: signed multiply, return high 16 bits.
    /// result.zmm16s[i] = ((src1.zmm16s[i] as i32 * src2.zmm16s[i] as i32) >> 16) as i16
    pub fn evex_vpmulhw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                let product = (src1.zmm16s[i] as i32) * (src2.zmm16s[i] as i32);
                result.zmm16s[i] = (product >> 16) as i16;
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMADDWD — Multiply and add packed words to dwords
    // EVEX.66.0F.WIG F5
    // ========================================================================

    /// VPMADDWD Vdq{k}, Hdq, Wdq
    ///
    /// For each dword element i: multiply adjacent signed word pairs and add.
    /// result.zmm32s[i] = src1.zmm16s[i*2] * src2.zmm16s[i*2]
    ///                   + src1.zmm16s[i*2+1] * src2.zmm16s[i*2+1]
    pub fn evex_vpmaddwd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ndwords = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..ndwords {
                let lo = (src1.zmm16s[i * 2] as i32) * (src2.zmm16s[i * 2] as i32);
                let hi = (src1.zmm16s[i * 2 + 1] as i32) * (src2.zmm16s[i * 2 + 1] as i32);
                result.zmm32s[i] = lo.wrapping_add(hi);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMADDUBSW — Multiply unsigned bytes by signed bytes, add pairs to words
    // EVEX.66.0F38.WIG 04
    // ========================================================================

    /// VPMADDUBSW Vdq{k}, Hdq, Wdq
    ///
    /// For each word element i: multiply unsigned/signed byte pairs and add
    /// with saturation to i16.
    /// temp = src1.zmmubyte[i*2] * src2.zmm_sbyte[i*2]
    ///      + src1.zmmubyte[i*2+1] * src2.zmm_sbyte[i*2+1]
    /// result.zmm16s[i] = saturate_i16(temp)
    pub fn evex_vpmaddubsw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nwords = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nwords {
                let a0 = src1.zmmubyte[i * 2] as i32;
                let b0 = src2.zmm_sbyte[i * 2] as i32;
                let a1 = src1.zmmubyte[i * 2 + 1] as i32;
                let b1 = src2.zmm_sbyte[i * 2 + 1] as i32;
                let sum = a0 * b0 + a1 * b1;
                result.zmm16s[i] = saturate_i16(sum);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSADBW — Sum of absolute differences of bytes to qwords
    // EVEX.66.0F.WIG F6
    // ========================================================================

    /// VPSADBW Vdq, Hdq, Wdq
    ///
    /// For each qword element: sum the absolute differences of 8 consecutive
    /// byte pairs. No opmask support (VPSADBW ignores mask per Intel SDM).
    /// result.zmm64u[i] = sum(|src1.zmmubyte[j] - src2.zmmubyte[j]|) for j in group of 8
    pub fn evex_vpsadbw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nqwords = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nqwords {
                let base = i * 8;
                let mut sum = 0u64;
                for j in 0..8 {
                    let a = src1.zmmubyte[base + j] as i16;
                    let b = src2.zmmubyte[base + j] as i16;
                    sum += (a - b).unsigned_abs() as u64;
                }
                result.zmm64u[i] = sum;
            }
        }
        // VPSADBW does not support opmask — write all, zero upper
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMINUD / VPMAXUD — Packed min/max unsigned dwords
    // EVEX.66.0F38.W0 3B / EVEX.66.0F38.W0 3F
    // ========================================================================

    /// VPMINUD Vdq{k}, Hdq, Wdq
    pub fn evex_vpminud(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_dwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = core::cmp::min(src1.zmm32u[i], src2.zmm32u[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPMAXUD Vdq{k}, Hdq, Wdq
    pub fn evex_vpmaxud(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_dwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm32u[i] = core::cmp::max(src1.zmm32u[i], src2.zmm32u[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMINUQ / VPMAXUQ — Packed min/max unsigned qwords
    // EVEX.66.0F38.W1 3B / EVEX.66.0F38.W1 3F
    // ========================================================================

    /// VPMINUQ Vdq{k}, Hdq, Wdq
    pub fn evex_vpminuq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_qwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = core::cmp::min(src1.zmm64u[i], src2.zmm64u[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPMAXUQ Vdq{k}, Hdq, Wdq
    pub fn evex_vpmaxuq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_qwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64u[i] = core::cmp::max(src1.zmm64u[i], src2.zmm64u[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMINSQ / VPMAXSQ — Packed min/max signed qwords
    // EVEX.66.0F38.W1 39 / EVEX.66.0F38.W1 3D
    // ========================================================================

    /// VPMINSQ Vdq{k}, Hdq, Wdq
    pub fn evex_vpminsq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_qwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64s[i] = core::cmp::min(src1.zmm64s[i], src2.zmm64s[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPMAXSQ Vdq{k}, Hdq, Wdq
    pub fn evex_vpmaxsq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_qwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };
        unsafe {
            for i in 0..nelements {
                result.zmm64s[i] = core::cmp::max(src1.zmm64s[i], src2.zmm64s[i]);
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}
