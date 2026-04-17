

//! AVX-512BW byte/word operation handlers
//!
//! Implements EVEX-encoded packed byte and word operations with opmask support.
//! Handlers work for 128/256/512-bit via `get_vl()` (EVEX.L'L field).
//!
//! Mirrors Bochs `cpu/avx/avx512_bw.cc`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedZmmRegister,
};

/// Number of byte elements per vector length: VL0=16, VL1=32, VL2=64
#[inline]
fn byte_elements(vl: u8) -> usize {
    match vl {
        0 => 16,  // 128-bit
        1 => 32,  // 256-bit
        _ => 64,  // 512-bit
    }
}

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
fn read_opmask_for_write<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(cpu: &BxCpuC<'_, I, T>, instr: &Instruction) -> u64 {
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
fn read_zmm<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(cpu: &BxCpuC<'_, I, T>, reg: u8) -> BxPackedZmmRegister {
    cpu.vmm[reg as usize]
}

/// Write ZMM register with per-byte masking, zeroing upper bytes beyond VL
fn write_zmm_masked_b<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nbytes = byte_elements(vl);
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..nbytes {
        if (mask >> i) & 1 != 0 {
            dst.set_zmmubyte(i, result.zmmubyte(i));
        } else if zero_masking {
            dst.set_zmmubyte(i, 0);
        }
        // else: merge masking — keep original value
    }
    // Zero upper bytes beyond VL (EVEX always clears upper)
    for i in nbytes..64 {
        dst.set_zmmubyte(i, 0);
    }
}

/// Write ZMM register with per-word masking, zeroing upper words beyond VL
fn write_zmm_masked_w<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    reg: u8,
    result: &BxPackedZmmRegister,
    mask: u64,
    zero_masking: bool,
    vl: u8,
) {
    let nwords = word_elements(vl);
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..nwords {
        if (mask >> i) & 1 != 0 {
            dst.set_zmm16u(i, result.zmm16u(i));
        } else if zero_masking {
            dst.set_zmm16u(i, 0);
        }
    }
    // Zero upper words beyond VL
    for i in nwords..32 {
        dst.set_zmm16u(i, 0);
    }
}

/// Read src2 from register or memory as bytes
fn read_src2_bytes<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let nbytes = vl_bytes(vl);
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        for i in 0..nbytes {
            let val = cpu.v_read_byte(seg, laddr + i as u64)?;
            tmp.set_zmmubyte(i, val);
        }
        Ok(tmp)
    }
}

/// Read src2 from register or memory as words
fn read_src2_words<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let nwords = word_elements(vl);
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        for i in 0..nwords {
            let val = cpu.v_read_word(seg, laddr + (i * 2) as u64)?;
            tmp.set_zmm16u(i, val);
        }
        Ok(tmp)
    }
}

/// Read src2 from register or memory as dwords
fn read_src2_dwords<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let ndwords = dword_elements(vl);
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        let mut tmp = BxPackedZmmRegister::default();
        for i in 0..ndwords {
            let val = cpu.v_read_dword(seg, laddr + (i * 4) as u64)?;
            tmp.set_zmm32u(i, val);
        }
        Ok(tmp)
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // VPADDB/W — Packed byte/word add (EVEX-encoded)
    // ========================================================================

    /// VPADDB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG FC
    pub fn evex_vpaddb(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(i, src1.zmmubyte(i).wrapping_add(src2.zmmubyte(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPADDW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG FD
    pub fn evex_vpaddw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, src1.zmm16u(i).wrapping_add(src2.zmm16u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSUBB/W — Packed byte/word subtract
    // ========================================================================

    /// VPSUBB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG F8
    pub fn evex_vpsubb(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(i, src1.zmmubyte(i).wrapping_sub(src2.zmmubyte(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSUBW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG F9
    pub fn evex_vpsubw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, src1.zmm16u(i).wrapping_sub(src2.zmm16u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMULLW — Packed multiply low words
    // ========================================================================

    /// VPMULLW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG D5
    pub fn evex_vpmullw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let product = (src1.zmm16u(i) as u32).wrapping_mul(src2.zmm16u(i) as u32);
            result.set_zmm16u(i, product as u16);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPAVGB/W — Packed average bytes/words
    // ========================================================================

    /// VPAVGB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG E0
    pub fn evex_vpavgb(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(i, ((src1.zmmubyte(i) as u16 + src2.zmmubyte(i) as u16 + 1) >> 1) as u8);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPAVGW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG E3
    pub fn evex_vpavgw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, ((src1.zmm16u(i) as u32 + src2.zmm16u(i) as u32 + 1) >> 1) as u16);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMAXUB / VPMINUB — Packed max/min unsigned bytes
    // ========================================================================

    /// VPMAXUB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG DE
    pub fn evex_vpmaxub(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(i, if src1.zmmubyte(i) > src2.zmmubyte(i) {
                src1.zmmubyte(i)
            } else {
                src2.zmmubyte(i)
            });
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPMINUB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG DA
    pub fn evex_vpminub(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(i, if src1.zmmubyte(i) < src2.zmmubyte(i) {
                src1.zmmubyte(i)
            } else {
                src2.zmmubyte(i)
            });
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMAXSW / VPMINSW — Packed max/min signed words
    // ========================================================================

    /// VPMAXSW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG EE
    pub fn evex_vpmaxsw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16s(i, if src1.zmm16s(i) > src2.zmm16s(i) {
                src1.zmm16s(i)
            } else {
                src2.zmm16s(i)
            });
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPMINSW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG EA
    pub fn evex_vpminsw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16s(i, if src1.zmm16s(i) < src2.zmm16s(i) {
                src1.zmm16s(i)
            } else {
                src2.zmm16s(i)
            });
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPACKSSDW — Pack dwords to signed saturated words (per 128-bit lane)
    // ========================================================================

    /// VPACKSSDW Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 6B
    pub fn evex_vpackssdw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_dwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        for lane in 0..nlanes {
            let dword_base = lane * 4; // 4 dwords per 128-bit lane
            let word_base = lane * 8;  // 8 words per 128-bit lane output
            // Pack 4 dwords from src1 into low 4 words of lane
            for j in 0..4 {
                result.set_zmm16s(word_base + j, saturate_i32_to_i16(src1.zmm32s(dword_base + j)));
            }
            // Pack 4 dwords from src2 into high 4 words of lane
            for j in 0..4 {
                result.set_zmm16s(word_base + 4 + j, saturate_i32_to_i16(src2.zmm32s(dword_base + j)));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPACKUSDW — Pack dwords to unsigned saturated words (per 128-bit lane)
    // ========================================================================

    /// VPACKUSDW Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 2B
    pub fn evex_vpackusdw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_dwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        for lane in 0..nlanes {
            let dword_base = lane * 4;
            let word_base = lane * 8;
            // Pack 4 dwords from src1 into low 4 words of lane
            for j in 0..4 {
                result.set_zmm16u(word_base + j, saturate_i32_to_u16(src1.zmm32s(dword_base + j)));
            }
            // Pack 4 dwords from src2 into high 4 words of lane
            for j in 0..4 {
                result.set_zmm16u(word_base + 4 + j, saturate_i32_to_u16(src2.zmm32s(dword_base + j)));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPUNPCKLBW / VPUNPCKHBW — Interleave low/high bytes (per 128-bit lane)
    // ========================================================================

    /// VPUNPCKLBW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG 60
    pub fn evex_vpunpcklbw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        for lane in 0..nlanes {
            let byte_base = lane * 16; // 16 bytes per 128-bit lane
            // Interleave low 8 bytes from src1 and src2
            for j in 0..8 {
                result.set_zmmubyte(byte_base + j * 2, src1.zmmubyte(byte_base + j));
                result.set_zmmubyte(byte_base + j * 2 + 1, src2.zmmubyte(byte_base + j));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPUNPCKHBW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG 68
    pub fn evex_vpunpckhbw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        for lane in 0..nlanes {
            let byte_base = lane * 16;
            // Interleave high 8 bytes from src1 and src2
            for j in 0..8 {
                result.set_zmmubyte(byte_base + j * 2, src1.zmmubyte(byte_base + 8 + j));
                result.set_zmmubyte(byte_base + j * 2 + 1, src2.zmmubyte(byte_base + 8 + j));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPUNPCKLWD / VPUNPCKHWD — Interleave low/high words (per 128-bit lane)
    // ========================================================================

    /// VPUNPCKLWD Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG 61
    pub fn evex_vpunpcklwd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        for lane in 0..nlanes {
            let word_base = lane * 8; // 8 words per 128-bit lane
            // Interleave low 4 words from src1 and src2
            for j in 0..4 {
                result.set_zmm16u(word_base + j * 2, src1.zmm16u(word_base + j));
                result.set_zmm16u(word_base + j * 2 + 1, src2.zmm16u(word_base + j));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPUNPCKHWD Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG 69
    pub fn evex_vpunpckhwd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        for lane in 0..nlanes {
            let word_base = lane * 8;
            // Interleave high 4 words from src1 and src2
            for j in 0..4 {
                result.set_zmm16u(word_base + j * 2, src1.zmm16u(word_base + 4 + j));
                result.set_zmm16u(word_base + j * 2 + 1, src2.zmm16u(word_base + 4 + j));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}

// ============================================================================
// Saturation helpers
// ============================================================================

/// Saturate i32 to i16 range [-32768, 32767]
#[inline]
fn saturate_i32_to_i16(val: i32) -> i16 {
    if val > i16::MAX as i32 {
        i16::MAX
    } else if val < i16::MIN as i32 {
        i16::MIN
    } else {
        val as i16
    }
}

/// Saturate i32 to u16 range [0, 65535]
#[inline]
fn saturate_i32_to_u16(val: i32) -> u16 {
    if val > u16::MAX as i32 {
        u16::MAX
    } else if val < 0 {
        0
    } else {
        val as u16
    }
}
