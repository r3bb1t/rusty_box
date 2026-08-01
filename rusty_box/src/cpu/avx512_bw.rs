//! AVX-512BW byte/word operation handlers
//!
//! Implements EVEX-encoded packed byte and word operations with opmask support.
//! Handlers work for 128/256/512-bit via `get_vl()` (EVEX.L'L field).
//!
//! Mirrors Bochs `cpu/avx/avx512_bw.cc`.

use super::avx512_load::cut_opmask_to;
use super::sse::{saturate_word_s_to_byte_s, saturate_word_s_to_byte_u};
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    xmm::BxPackedZmmRegister,
};

/// Number of byte elements per vector length: VL0=16, VL1=32, VL2=64
#[inline]
fn byte_elements(vl: u8) -> usize {
    match vl {
        0 => 16, // 128-bit
        1 => 32, // 256-bit
        _ => 64, // 512-bit
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

/// Write ZMM register with per-byte masking, zeroing upper bytes beyond VL
pub(super) fn write_zmm_masked_b<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
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
pub(super) fn write_zmm_masked_w<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
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

// The memory form goes through the loader `ia_opcodes_evex.def` names for each
// caller, so embedded broadcast and masked fault suppression match Bochs. The
// UNPCK opcodes below use `LOAD_Vector` for both of their def entries rather
// than pairing it with a masked loader, so they call `evex_load_vector`
// directly instead of going through these helpers.

/// Read src2 as bytes — callers (VPADDB, VPSUBB, VPAVGB, VPMAXUB, VPMINUB)
/// pair `LOAD_Vector` with `LOAD_MASK_VectorB`.
fn read_rm_bytes<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    _vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src1()))
    } else {
        cpu.evex_load_vec_mask_b_pair(instr)
    }
}

/// Read src2 as words — callers (VPADDW, VPSUBW, VPMULLW, VPAVGW, VPMAXSW,
/// VPMINSW) pair `LOAD_Vector` with `LOAD_MASK_VectorW`.
fn read_rm_words<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    _vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src1()))
    } else {
        cpu.evex_load_vec_mask_w_pair(instr)
    }
}

/// Read src2 as dwords — callers (VPACKSSDW, VPACKUSDW) use
/// `LOAD_BROADCAST_VectorD` for both entries, so there is no masked variant.
fn read_rm_dwords<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    _vl: u8,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src1()))
    } else {
        cpu.evex_load_broadcast_vector_d(instr)
    }
}

/// Read src2 with `LOAD_Vector` regardless of masking — for the UNPCK opcodes,
/// whose base and `_Kmask` def entries both name `LOAD_Vector`.
fn read_rm_unmasked_vector<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src1()))
    } else {
        cpu.evex_load_vector(instr)
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_bytes(self, instr, vl)?;
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_words(self, instr, vl)?;
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_bytes(self, instr, vl)?;
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_words(self, instr, vl)?;
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_words(self, instr, vl)?;
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(
                i,
                ((src1.zmmubyte(i) as u16 + src2.zmmubyte(i) as u16 + 1) >> 1) as u8,
            );
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(
                i,
                ((src1.zmm16u(i) as u32 + src2.zmm16u(i) as u32 + 1) >> 1) as u16,
            );
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(
                i,
                if src1.zmmubyte(i) > src2.zmmubyte(i) {
                    src1.zmmubyte(i)
                } else {
                    src2.zmmubyte(i)
                },
            );
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(
                i,
                if src1.zmmubyte(i) < src2.zmmubyte(i) {
                    src1.zmmubyte(i)
                } else {
                    src2.zmmubyte(i)
                },
            );
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16s(
                i,
                if src1.zmm16s(i) > src2.zmm16s(i) {
                    src1.zmm16s(i)
                } else {
                    src2.zmm16s(i)
                },
            );
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16s(
                i,
                if src1.zmm16s(i) < src2.zmm16s(i) {
                    src1.zmm16s(i)
                } else {
                    src2.zmm16s(i)
                },
            );
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_dwords(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        for lane in 0..nlanes {
            let dword_base = lane * 4; // 4 dwords per 128-bit lane
            let word_base = lane * 8; // 8 words per 128-bit lane output
                                      // Pack 4 dwords from src1 into low 4 words of lane
            for j in 0..4 {
                result.set_zmm16s(
                    word_base + j,
                    saturate_i32_to_i16(src1.zmm32s(dword_base + j)),
                );
            }
            // Pack 4 dwords from src2 into high 4 words of lane
            for j in 0..4 {
                result.set_zmm16s(
                    word_base + 4 + j,
                    saturate_i32_to_i16(src2.zmm32s(dword_base + j)),
                );
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_dwords(self, instr, vl)?;
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
                result.set_zmm16u(
                    word_base + j,
                    saturate_i32_to_u16(src1.zmm32s(dword_base + j)),
                );
            }
            // Pack 4 dwords from src2 into high 4 words of lane
            for j in 0..4 {
                result.set_zmm16u(
                    word_base + 4 + j,
                    saturate_i32_to_u16(src2.zmm32s(dword_base + j)),
                );
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_unmasked_vector(self, instr)?;
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_unmasked_vector(self, instr)?;
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_unmasked_vector(self, instr)?;
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_unmasked_vector(self, instr)?;
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

    // ════════════════════════════════════════════════════════════════════
    // Saturating / min-max / pack / absolute byte and word arithmetic.
    //
    // Every opcode below pairs `LOAD_Vector` with `LOAD_MASK_VectorB` or
    // `LOAD_MASK_VectorW` in ia_opcodes_evex.def, except VPACKSSWB and
    // VPACKUSWB, whose base *and* `_Kmask` entries both name `LOAD_Vector` —
    // so those two must not go through the masked pair helper.
    // ════════════════════════════════════════════════════════════════════

    /// Element-wise two-operand byte op: src1 = Hdq (vvvv), src2 = rm.
    /// Bochs cpu_templates.h `HANDLE_AVX512_2OP_BYTE_EL_MASK`.
    fn evex_bw_2op_b(&mut self, instr: &Instruction, op: fn(u8, u8) -> u8) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_bytes(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(i, op(src1.zmmubyte(i), src2.zmmubyte(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// Element-wise two-operand word op.
    /// Bochs cpu_templates.h `HANDLE_AVX512_2OP_WORD_EL_MASK`.
    fn evex_bw_2op_w(&mut self, instr: &Instruction, op: fn(u16, u16) -> u16) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, op(src1.zmm16u(i), src2.zmm16u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// Single-operand byte op (VPABSB). Bochs `VPABSB_MASK_VdqWdqR`.
    fn evex_bw_1op_b(&mut self, instr: &Instruction, op: fn(u8) -> u8) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_vec_mask_b_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(i, op(src.zmmubyte(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// Single-operand word op (VPABSW). Bochs `VPABSW_MASK_VdqWdqR`.
    fn evex_bw_1op_w(&mut self, instr: &Instruction, op: fn(u16) -> u16) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_vec_mask_w_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, op(src.zmm16u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPADDSB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG EC
    pub fn evex_vpaddsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_b(instr, |a, b| (a as i8).saturating_add(b as i8) as u8)
    }

    /// VPADDSW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG ED
    pub fn evex_vpaddsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_w(instr, |a, b| (a as i16).saturating_add(b as i16) as u16)
    }

    /// VPADDUSB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG DC
    pub fn evex_vpaddusb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_b(instr, |a, b| a.saturating_add(b))
    }

    /// VPADDUSW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG DD
    pub fn evex_vpaddusw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_w(instr, |a, b| a.saturating_add(b))
    }

    /// VPSUBSB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG E8
    pub fn evex_vpsubsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_b(instr, |a, b| (a as i8).saturating_sub(b as i8) as u8)
    }

    /// VPSUBSW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG E9
    pub fn evex_vpsubsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_w(instr, |a, b| (a as i16).saturating_sub(b as i16) as u16)
    }

    /// VPSUBUSB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG D8
    pub fn evex_vpsubusb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_b(instr, |a, b| a.saturating_sub(b))
    }

    /// VPSUBUSW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG D9
    pub fn evex_vpsubusw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_w(instr, |a, b| a.saturating_sub(b))
    }

    /// VPMINSB Vdq{k}, Hdq, Wdq — EVEX.66.0F38.WIG 38
    pub fn evex_vpminsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_b(instr, |a, b| (a as i8).min(b as i8) as u8)
    }

    /// VPMAXSB Vdq{k}, Hdq, Wdq — EVEX.66.0F38.WIG 3C
    pub fn evex_vpmaxsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_b(instr, |a, b| (a as i8).max(b as i8) as u8)
    }

    /// VPMINUW Vdq{k}, Hdq, Wdq — EVEX.66.0F38.WIG 3A
    pub fn evex_vpminuw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_w(instr, |a, b| a.min(b))
    }

    /// VPMAXUW Vdq{k}, Hdq, Wdq — EVEX.66.0F38.WIG 3E
    pub fn evex_vpmaxuw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_w(instr, |a, b| a.max(b))
    }

    /// VPMULHRSW Vdq{k}, Hdq, Wdq — EVEX.66.0F38.WIG 0B
    /// Bochs simd_int.h `xmm_pmulhrsw`: ((a * b >> 14) + 1) >> 1.
    pub fn evex_vpmulhrsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_2op_w(instr, |a, b| {
            let t = ((a as i16 as i32 * b as i16 as i32) >> 14) + 1;
            (t >> 1) as u16
        })
    }

    /// VPABSB Vdq{k}, Wdq — EVEX.66.0F38.WIG 1C
    pub fn evex_vpabsb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_1op_b(instr, |a| (a as i8).unsigned_abs())
    }

    /// VPABSW Vdq{k}, Wdq — EVEX.66.0F38.WIG 1D
    pub fn evex_vpabsw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_bw_1op_w(instr, |a| (a as i16).unsigned_abs())
    }

    /// The shared body of VPACKSSWB and VPACKUSWB. Packing interleaves the
    /// two sources *within each 128-bit lane*: bytes 0..7 of a lane come from
    /// that lane of src1, bytes 8..15 from the same lane of src2. Both def
    /// entries name `LOAD_Vector`, so the memory operand is read unmasked
    /// even in the `_Kmask` form; only the write is byte-granular masked.
    fn evex_packwb(&mut self, instr: &Instruction, saturate: fn(i16) -> u8) -> super::Result<()> {
        let vl = instr.get_vl();
        let lanes = byte_elements(vl) / 16;
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_unmasked_vector(self, instr)?;
        let mut result = BxPackedZmmRegister::default();
        for lane in 0..lanes {
            for j in 0..8 {
                result.set_zmmubyte(lane * 16 + j, saturate(src1.zmm16u(lane * 8 + j) as i16));
                result.set_zmmubyte(
                    lane * 16 + 8 + j,
                    saturate(src2.zmm16u(lane * 8 + j) as i16),
                );
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPACKSSWB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG 63
    pub fn evex_vpacksswb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_packwb(instr, |v| saturate_word_s_to_byte_s(v) as u8)
    }

    /// VPACKUSWB Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG 67
    pub fn evex_vpackuswb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_packwb(instr, saturate_word_s_to_byte_u)
    }

    // ════════════════════════════════════════════════════════════════════
    // Word shifts. Three encodings, three different loaders — taken per
    // opcode from field 4 of ia_opcodes_evex.def, not assumed:
    //
    //   VPSxxW Vdq, Hdq, Wdq   count is the low qword of a 128-bit operand;
    //                          `LOADU_Wdq` for BOTH the base and _Kmask
    //                          entries, so the count is read unaligned and
    //                          unmasked even when a writemask is in play.
    //   VPSxxW Udq, Ib         count is the imm8; `LOAD_Vector` /
    //                          `LOAD_MASK_VectorW`.
    //   VPSxxVW Vdq, Hdq, Wdq  per-element counts; `LOAD_Vector` /
    //                          `LOAD_MASK_VectorW`.
    //
    // A count >= 16 produces 0 for the logical shifts; the arithmetic right
    // shift saturates to the sign bit instead, which is why it clamps the
    // count rather than zeroing (Bochs simd_int.h xmm_psraw).
    // ════════════════════════════════════════════════════════════════════

    /// Shift every word by one shared count taken from the low qword of the
    /// 128-bit rm operand.
    fn evex_wshift_by_xmm(
        &mut self,
        instr: &Instruction,
        shift: fn(u16, u32) -> u16,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src = read_zmm(self, instr.src2());
        let count_reg = if instr.mod_c0() {
            read_zmm(self, instr.src1())
        } else {
            self.evex_loadu_wdq(instr)?
        };
        let count = count_reg.zmm64u(0).min(u32::MAX as u64) as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, shift(src.zmm16u(i), count));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// Shift every word by the imm8 count.
    fn evex_wshift_by_imm(
        &mut self,
        instr: &Instruction,
        shift: fn(u16, u32) -> u16,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_vec_mask_w_pair(instr)?
        };
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, shift(src.zmm16u(i), count));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// Shift every word by its own count from the corresponding element of
    /// the rm operand.
    fn evex_wshift_variable(
        &mut self,
        instr: &Instruction,
        shift: fn(u16, u32) -> u16,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_rm_words(self, instr, vl)?;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, shift(src1.zmm16u(i), src2.zmm16u(i) as u32));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRLW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG D1
    pub fn evex_vpsrlw_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_wshift_by_xmm(instr, word_srl)
    }

    /// VPSRAW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG E1
    pub fn evex_vpsraw_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_wshift_by_xmm(instr, word_sra)
    }

    /// VPSLLW Vdq{k}, Hdq, Wdq — EVEX.66.0F.WIG F1
    pub fn evex_vpsllw_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_wshift_by_xmm(instr, word_sll)
    }

    /// VPSRLW Vdq{k}, Udq, Ib — EVEX.66.0F.WIG 71 /2
    pub fn evex_vpsrlw_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_wshift_by_imm(instr, word_srl)
    }

    /// VPSRAW Vdq{k}, Udq, Ib — EVEX.66.0F.WIG 71 /4
    pub fn evex_vpsraw_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_wshift_by_imm(instr, word_sra)
    }

    /// VPSLLW Vdq{k}, Udq, Ib — EVEX.66.0F.WIG 71 /6
    pub fn evex_vpsllw_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_wshift_by_imm(instr, word_sll)
    }

    /// VPSRLVW Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 10
    pub fn evex_vpsrlvw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_wshift_variable(instr, word_srl)
    }

    /// VPSRAVW Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 11
    pub fn evex_vpsravw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_wshift_variable(instr, word_sra)
    }

    /// VPSLLVW Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 12
    pub fn evex_vpsllvw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_wshift_variable(instr, word_sll)
    }

    // ========================================================================
    // VPSHUFLW / VPSHUFHW — shuffle one half of each 128-bit lane by imm8 and
    // copy the other half through. Bochs avx512.cc VPSHUFLW/HW_MASK_VdqWdqIbR.
    // ========================================================================

    /// The shared body; `high` selects the upper four words of each lane.
    fn evex_pshuf_half_words(&mut self, instr: &Instruction, high: bool) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_vector(instr)?
        };
        let order = instr.ib();
        let mut result = BxPackedZmmRegister::default();
        let lanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        for lane in 0..lanes {
            let base = lane * 8; // 8 words per 128-bit lane
            let (shuffled, copied) = if high { (base + 4, base) } else { (base, base + 4) };
            for n in 0..4 {
                let sel = ((order >> (2 * n)) & 3) as usize;
                result.set_zmm16u(shuffled + n, src.zmm16u(shuffled + sel));
            }
            for n in 0..4 {
                result.set_zmm16u(copied + n, src.zmm16u(copied + n));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSHUFLW Vdq{k}{z}, Wdq, Ib — EVEX.F2.0F.W0 70
    pub fn evex_vpshuflw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pshuf_half_words(instr, false)
    }

    /// VPSHUFHW Vdq{k}{z}, Wdq, Ib — EVEX.F3.0F.W0 70
    pub fn evex_vpshufhw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pshuf_half_words(instr, true)
    }


    // ========================================================================
    // Word-granular permutes (AVX512_BW). Bochs avx512.cc VPERMW /
    // VPERMT2W / VPERMI2W. All three pair LOAD_Vector with itself.
    // ========================================================================

    /// Read the r/m operand of a word-granular permute.
    fn perm_rm_w(&mut self, instr: &Instruction) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src1()))
        } else {
            self.evex_load_vector(instr)
        }
    }

    /// VPERMW Vdq{k}{z}, Hdq, Wdq — EVEX.66.0F38.W1 8D
    pub fn evex_vpermw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let elements = word_elements(vl);
        let control_mask = (elements - 1) as u16;
        let op1 = read_zmm(self, instr.src2()); // vvvv — the indices
        let op2 = self.perm_rm_w(instr)?; // rm — the table
        let mut result = BxPackedZmmRegister::default();
        for n in 0..elements {
            result.set_zmm16u(n, op2.zmm16u((op1.zmm16u(n) & control_mask) as usize));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// The shared two-table word body; `index_from_dst` selects VPERMI2W.
    fn evex_perm2_w(&mut self, instr: &Instruction, index_from_dst: bool) -> super::Result<()> {
        let vl = instr.get_vl();
        let elements = word_elements(vl);
        let control_mask = (elements - 1) as u16;
        let op1 = read_zmm(self, instr.src2());
        let op2 = self.perm_rm_w(instr)?;
        let dst = read_zmm(self, instr.dst());
        let mut result = BxPackedZmmRegister::default();
        for n in 0..elements {
            let control = if index_from_dst { dst.zmm16u(n) } else { op1.zmm16u(n) };
            let sel = (control & control_mask) as usize;
            let from_op2 = control & (elements as u16) != 0;
            let v = if from_op2 {
                op2.zmm16u(sel)
            } else if index_from_dst {
                op1.zmm16u(sel)
            } else {
                dst.zmm16u(sel)
            };
            result.set_zmm16u(n, v);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPERMT2W Vdq{k}{z}, Hdq, Wdq — EVEX.66.0F38.W1 7D
    pub fn evex_vpermt2w(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_perm2_w(instr, false)
    }

    /// VPERMI2W Vdq{k}{z}, Hdq, Wdq — EVEX.66.0F38.W1 75
    pub fn evex_vpermi2w(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_perm2_w(instr, true)
    }


    // ========================================================================
    // VMOVDQU8 / VMOVDQU16 — the byte- and word-granular unaligned moves.
    // Bochs avx512_move.cc VMOVDQU8/16_MASK_*. Same shape as the dword and
    // qword forms already here, but the opmask reaches 64 bits at byte
    // granularity, which is exactly the width where CUT_OPMASK would shift by
    // 64 — so, as elsewhere, the cut is skipped at VL512.
    // ========================================================================

    /// The writemask for a byte-granular move, cut to the element count.
    fn movdqu8_mask(&self, instr: &Instruction) -> u64 {
        let vl = instr.get_vl();
        let mask = read_opmask_for_write(self, instr);
        if vl == 2 {
            mask
        } else {
            mask & cut_opmask_to(byte_elements(vl))
        }
    }

    /// VMOVDQU8 Vdq{k}{z}, Wdq — register form.
    pub fn evex_vmovdqu8_load_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = self.movdqu8_mask(instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU8 Vdq{k}{z}, Mdq — masked load.
    pub fn evex_vmovdqu8_load_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let eaddr = self.resolve_addr(instr);
        let mask = self.movdqu8_mask(instr);
        let mut src = BxPackedZmmRegister::default();
        self.avx_masked_load8(instr, eaddr, &mut src, mask)?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU8 Wdq{k}, Vdq — register form of the store direction.
    pub fn evex_vmovdqu8_store_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = self.movdqu8_mask(instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_b(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU8 Mdq{k}, Vdq — masked store. Masked-off bytes are not written
    /// and cannot fault.
    pub fn evex_vmovdqu8_store_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let mask = self.movdqu8_mask(instr);
        if mask == 0 {
            return Ok(());
        }
        let src = read_zmm(self, instr.src());
        self.avx_masked_store8(instr, eaddr, &src, mask)
    }

    /// The writemask for a word-granular move.
    fn movdqu16_mask(&self, instr: &Instruction) -> u64 {
        let vl = instr.get_vl();
        read_opmask_for_write(self, instr) & cut_opmask_to(word_elements(vl))
    }

    /// VMOVDQU16 Vdq{k}{z}, Wdq — register form.
    pub fn evex_vmovdqu16_load_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = self.movdqu16_mask(instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU16 Vdq{k}{z}, Mdq — masked load.
    pub fn evex_vmovdqu16_load_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let eaddr = self.resolve_addr(instr);
        let mask = self.movdqu16_mask(instr);
        let mut src = BxPackedZmmRegister::default();
        self.avx_masked_load16(instr, eaddr, &mut src, mask)?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU16 Wdq{k}, Vdq — register form of the store direction.
    pub fn evex_vmovdqu16_store_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = self.movdqu16_mask(instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU16 Mdq{k}, Vdq — masked store.
    pub fn evex_vmovdqu16_store_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let mask = self.movdqu16_mask(instr);
        if mask == 0 {
            return Ok(());
        }
        let src = read_zmm(self, instr.src());
        self.avx_masked_store16(instr, eaddr, &src, mask)
    }


    /// VDBPSADBW Vdq{k}{z}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 42.
    ///
    /// Per 128-bit lane: the r/m operand is first dword-shuffled by imm8, then
    /// eight sums-of-absolute-differences are taken over sliding quadruples.
    /// Bochs avx512.cc VDBPSADBW_MASK_VdqHdqWdqIbR and simd_int.h
    /// `xmm_dbpsadbw` / `sad_quadruple`.
    pub fn evex_vdbpsadbw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let op1 = read_zmm(self, instr.src2()); // vvvv
        let op2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
        } else {
            self.evex_load_vector(instr)?
        };
        let order = instr.ib();
        let lanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();
        for lane in 0..lanes {
            let base_b = lane * 16; // bytes per 128-bit lane
            let base_w = lane * 8; // words per 128-bit lane

            // xmm_shufps(tmp, op2, op2, imm8) — both halves come from op2.
            let mut tmp = [0u8; 16];
            for (i, sel) in [order & 3, (order >> 2) & 3, (order >> 4) & 3, (order >> 6) & 3]
                .into_iter()
                .enumerate()
            {
                let from = base_b + (sel as usize) * 4;
                tmp[i * 4..i * 4 + 4]
                    .copy_from_slice(&[
                        op2.zmmubyte(from),
                        op2.zmmubyte(from + 1),
                        op2.zmmubyte(from + 2),
                        op2.zmmubyte(from + 3),
                    ]);
            }

            // The eight quadruple offsets are fixed by the instruction.
            const OFFSETS: [(usize, usize); 8] = [
                (0, 0), (0, 1), (4, 2), (4, 3), (8, 8), (8, 9), (12, 10), (12, 11),
            ];
            for (w, (o1, o2)) in OFFSETS.into_iter().enumerate() {
                let mut sum: u32 = 0;
                for n in 0..4 {
                    let a = op1.zmmubyte(base_b + n + o1) as i32;
                    let b = tmp[n + o2] as i32;
                    sum += (a - b).unsigned_abs();
                }
                result.set_zmm16u(base_w + w, sum as u16);
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


/// Logical right shift of a word; a count of 16 or more yields zero.
#[inline]
fn word_srl(v: u16, count: u32) -> u16 {
    if count >= 16 {
        0
    } else {
        v >> count
    }
}

/// Logical left shift of a word; a count of 16 or more yields zero.
#[inline]
fn word_sll(v: u16, count: u32) -> u16 {
    if count >= 16 {
        0
    } else {
        v << count
    }
}

/// Arithmetic right shift of a word. Unlike the logical shifts an
/// out-of-range count does not yield zero — every bit becomes the sign bit,
/// so the count is clamped to 15 (Bochs simd_int.h `xmm_psraw`).
#[inline]
fn word_sra(v: u16, count: u32) -> u16 {
    let n = if count >= 16 { 15 } else { count };
    ((v as i16) >> n) as u16
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    //! The saturating byte/word family is easy to get subtly wrong in two
    //! ways that a naive test would miss:
    //!
    //!   * saturation vs wrapping — a wrapping add produces a plausible
    //!     value everywhere except at the clamp edges, so the edges are the
    //!     only place the two differ;
    //!   * VPACKSSWB/VPACKUSWB interleave the two sources *per 128-bit lane*,
    //!     not across the whole register, so a whole-register implementation
    //!     is correct at VL128 and wrong at VL256/512.
    //!
    //! These drive `execute_instruction` rather than the handler directly,
    //! because the dispatcher arm is itself part of what is under test.

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::{BxSegregs, Instruction};
    use rusty_box_decoder::opcode::Opcode;

    /// Register-form EVEX instruction with no masking (k0): dst=0, src1=1,
    /// src2=2.
    fn evex_reg(opcode: Opcode, vl: u8) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0); // dst
        i.set_src_reg(1, 1); // src1 = Hdq (vvvv)
        i.set_src_reg(2, 2); // src2 = rm
        i.set_opmask(0);
        i.set_vex(true);
        i.set_vl(vl);
        i.set_seg(BxSegregs::Ds);
        // `init` assigns `flags` wholesale, so mod_c0 must be asserted after.
        i.init(0, 0, 1, 1);
        i.assert_mod_c0();
        i
    }

    #[test]
    fn saturating_byte_and_word_adds_clamp_instead_of_wrapping() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();

        // Signed byte: 100 + 100 saturates to +127, -100 + -100 to -128.
        cpu.vmm[2].set_zmmubyte(0, 100);
        cpu.vmm[1].set_zmmubyte(0, 100);
        cpu.vmm[2].set_zmmubyte(1, (-100i8) as u8);
        cpu.vmm[1].set_zmmubyte(1, (-100i8) as u8);
        // Unsigned byte: 200 + 100 saturates to 255.
        cpu.vmm[2].set_zmmubyte(2, 200);
        cpu.vmm[1].set_zmmubyte(2, 100);

        cpu.execute_instruction(&evex_reg(Opcode::EvexVpaddsbVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmmubyte(0), 127i8 as u8, "signed positive clamp");
        assert_eq!(cpu.vmm[0].zmmubyte(1), (-128i8) as u8, "signed negative clamp");

        cpu.execute_instruction(&evex_reg(Opcode::EvexVpaddusbVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmmubyte(2), 255, "unsigned clamp");

        // Signed word: 30000 + 30000 saturates to 32767, and unsigned
        // subtract clamps at 0 rather than wrapping to 65535.
        cpu.vmm[2].set_zmm16u(0, 30000);
        cpu.vmm[1].set_zmm16u(0, 30000);
        cpu.execute_instruction(&evex_reg(Opcode::EvexVpaddswVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm16u(0), 32767);

        cpu.vmm[2].set_zmm16u(0, 5);
        cpu.vmm[1].set_zmm16u(0, 9);
        cpu.execute_instruction(&evex_reg(Opcode::EvexVpsubuswVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm16u(0), 0, "unsigned subtract must not wrap");
    }

    #[test]
    fn vpabsb_maps_int_min_to_its_unsigned_magnitude() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        // abs(-128) is not representable as i8; x86 yields 0x80.
        cpu.vmm[2].set_zmmubyte(0, 0x80);
        cpu.vmm[2].set_zmmubyte(1, (-5i8) as u8);
        cpu.vmm[2].set_zmmubyte(2, 7);
        let mut i = evex_reg(Opcode::EvexVpabsbVdqWdq, 0);
        i.set_src_reg(1, 2); // single-operand form reads src()
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.vmm[0].zmmubyte(0), 0x80);
        assert_eq!(cpu.vmm[0].zmmubyte(1), 5);
        assert_eq!(cpu.vmm[0].zmmubyte(2), 7);
    }

    #[test]
    fn vpacksswb_interleaves_per_128_bit_lane_at_vl256() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();

        // Tag every source word with its lane so a whole-register pack is
        // distinguishable from a per-lane one. src1 words -> 0x0n, src2 -> 0x1n.
        for lane in 0..2usize {
            for j in 0..8usize {
                cpu.vmm[2].set_zmm16u(lane * 8 + j, (lane * 8 + j) as u16);
                cpu.vmm[1].set_zmm16u(lane * 8 + j, (0x10 + lane * 8 + j) as u16);
            }
        }

        cpu.execute_instruction(&evex_reg(Opcode::EvexVpacksswbVdqHdqWdq, 1))
            .unwrap();

        // Lane 0: bytes 0..7 from src1 lane 0, bytes 8..15 from src2 lane 0.
        // Lane 1: bytes 16..23 from src1 lane 1, bytes 24..31 from src2 lane 1.
        for lane in 0..2usize {
            for j in 0..8usize {
                assert_eq!(
                    cpu.vmm[0].zmmubyte(lane * 16 + j),
                    (lane * 8 + j) as u8,
                    "lane {lane} src1 byte {j}"
                );
                assert_eq!(
                    cpu.vmm[0].zmmubyte(lane * 16 + 8 + j),
                    (0x10 + lane * 8 + j) as u8,
                    "lane {lane} src2 byte {j}"
                );
            }
        }
    }

    #[test]
    fn word_shifts_treat_an_out_of_range_count_by_kind() {
        // A count of 16 or more zeroes the logical shifts, but the
        // arithmetic right shift fills with the sign bit instead — the one
        // case where "shift everything out" is not the same as "produce 0".
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.vmm[2].set_zmm16u(0, 0x8000); // negative
        cpu.vmm[2].set_zmm16u(1, 0x4000); // positive
        cpu.vmm[1].set_zmm64u(0, 20); // count, low qword of the 128-bit operand

        cpu.execute_instruction(&evex_reg(Opcode::EvexVpsrawVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm16u(0), 0xFFFF, "negative saturates to all ones");
        assert_eq!(cpu.vmm[0].zmm16u(1), 0x0000, "positive saturates to zero");

        cpu.execute_instruction(&evex_reg(Opcode::EvexVpsrlwVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm16u(0), 0, "logical right shift zeroes");

        cpu.execute_instruction(&evex_reg(Opcode::EvexVpsllwVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm16u(0), 0, "left shift zeroes");
    }

    #[test]
    fn word_shift_by_imm_and_by_element_use_their_own_counts() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();

        // imm8 form: the count is the immediate and there is only one source,
        // so it is read through src() = src1() — no vvvv operand is involved
        // and the 3-operand swap does not apply.
        cpu.vmm[2].set_zmm16u(0, 0x00F0);
        let mut i = evex_reg(Opcode::EvexVpsrlwUdqIb, 0);
        i.set_src_reg(1, 2); // single-source form reads src()
        i.set_iq(4);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.vmm[0].zmm16u(0), 0x000F);

        // Variable form: every element takes its own count.
        cpu.vmm[2].set_zmm16u(0, 0x0100);
        cpu.vmm[2].set_zmm16u(1, 0x0100);
        cpu.vmm[2].set_zmm16u(2, 0x0100);
        cpu.vmm[1].set_zmm16u(0, 0);
        cpu.vmm[1].set_zmm16u(1, 4);
        cpu.vmm[1].set_zmm16u(2, 99); // out of range for this element only
        cpu.execute_instruction(&evex_reg(Opcode::EvexVpsrlvwVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm16u(0), 0x0100);
        assert_eq!(cpu.vmm[0].zmm16u(1), 0x0010);
        assert_eq!(cpu.vmm[0].zmm16u(2), 0);
    }

    #[test]
    fn pack_saturation_differs_between_signed_and_unsigned_forms() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        // 0x0140 = 320: signed-byte saturation gives 127, unsigned gives 255.
        // -1 stays -1 signed but clamps to 0 unsigned.
        cpu.vmm[2].set_zmm16u(0, 320);
        cpu.vmm[2].set_zmm16u(1, (-1i16) as u16);

        cpu.execute_instruction(&evex_reg(Opcode::EvexVpacksswbVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmmubyte(0), 127);
        assert_eq!(cpu.vmm[0].zmmubyte(1), 0xFF, "signed -1 stays -1");

        cpu.execute_instruction(&evex_reg(Opcode::EvexVpackuswbVdqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmmubyte(0), 255);
        assert_eq!(cpu.vmm[0].zmmubyte(1), 0, "negative clamps to zero");
    }

    #[test]
    fn pshuflw_and_pshufhw_each_touch_only_their_own_half_of_every_lane() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        // Two 128-bit lanes, words numbered 0..15 so any misplacement shows.
        for n in 0..16 {
            cpu.vmm[1].set_zmm16u(n, n as u16);
        }
        let mut i = evex_reg(Opcode::EvexVpshuflwVdqWdqIb, 1);
        i.set_iq(0x1B); // 00_01_10_11 — reverse the four selected words
        i.set_vl(1);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(
            (0..8).map(|n| cpu.vmm[0].zmm16u(n)).collect::<alloc::vec::Vec<_>>(),
            [3, 2, 1, 0, 4, 5, 6, 7],
            "low four words reversed, high four untouched"
        );
        assert_eq!(
            (8..16).map(|n| cpu.vmm[0].zmm16u(n)).collect::<alloc::vec::Vec<_>>(),
            [11, 10, 9, 8, 12, 13, 14, 15],
            "the second 128-bit lane is shuffled independently"
        );

        let mut i = evex_reg(Opcode::EvexVpshufhwVdqWdqIb, 1);
        i.set_iq(0x1B);
        i.set_vl(1);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(
            (0..8).map(|n| cpu.vmm[0].zmm16u(n)).collect::<alloc::vec::Vec<_>>(),
            [0, 1, 2, 3, 7, 6, 5, 4],
            "high four words reversed, low four untouched"
        );
    }


    #[test]
    fn vmovdqu8_masks_at_byte_granularity_across_all_64_lanes() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for n in 0..64 {
            cpu.vmm[1].set_zmmubyte(n, 0x11);
            cpu.vmm[0].set_zmmubyte(n, 0x22);
        }
        // At VL512 a byte-granular opmask fills all 64 bits — the width at
        // which the element-count cut would shift by 64, so it is skipped.
        cpu.bx_write_opmask(1, 0x0F0F_0F0F_0F0F_0F0F);
        let mut i = evex_reg(Opcode::EvexVmovdqu8VdqWdqKmask, 2);
        i.set_opmask(1);
        i.set_vl(2);
        cpu.execute_instruction(&i).unwrap();
        for n in 0..64 {
            let want = if (n % 8) < 4 { 0x11 } else { 0x22 };
            assert_eq!(cpu.vmm[0].zmmubyte(n), want, "byte {n} merge-masked");
        }

        // Zero masking clears the unselected bytes instead of merging them.
        for n in 0..64 {
            cpu.vmm[0].set_zmmubyte(n, 0x22);
        }
        let mut i = evex_reg(Opcode::EvexVmovdqu8VdqWdqKmask, 2);
        i.set_opmask(1);
        i.set_vl(2);
        i.set_zero_masking(1);
        cpu.execute_instruction(&i).unwrap();
        for n in 0..64 {
            let want = if (n % 8) < 4 { 0x11 } else { 0x00 };
            assert_eq!(cpu.vmm[0].zmmubyte(n), want, "byte {n} zero-masked");
        }
    }

    #[test]
    fn vmovdqu16_masks_at_word_granularity() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for n in 0..8 {
            cpu.vmm[1].set_zmm16u(n, 0x1111);
            cpu.vmm[0].set_zmm16u(n, 0x2222);
        }
        cpu.bx_write_opmask(1, 0b0101_0101);
        let mut i = evex_reg(Opcode::EvexVmovdqu16VdqWdqKmask, 1);
        i.set_opmask(1);
        i.set_vl(1);
        cpu.execute_instruction(&i).unwrap();
        for n in 0..8 {
            let want = if n % 2 == 0 { 0x1111 } else { 0x2222 };
            assert_eq!(cpu.vmm[0].zmm16u(n), want, "word {n}");
        }
    }


    #[test]
    fn vdbpsadbw_sums_absolute_differences_over_sliding_quadruples() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        // Bytes 0..15 in both operands, and imm8 0xE4 = the identity dword
        // shuffle, so the r/m operand passes through unchanged.
        for n in 0..16 {
            cpu.vmm[2].set_zmmubyte(n, n as u8); // vvvv
            cpu.vmm[1].set_zmmubyte(n, n as u8); // rm
        }
        let mut i = evex_reg(Opcode::EvexVdbpsadbwVdqHdqWdqIbKmask, 0);
        i.set_iq(0xE4);
        cpu.execute_instruction(&i).unwrap();
        // The eight quadruple offsets are fixed: (0,0) (0,1) (4,2) (4,3)
        // (8,8) (8,9) (12,10) (12,11).
        assert_eq!(
            (0..8).map(|n| cpu.vmm[0].zmm16u(n)).collect::<alloc::vec::Vec<_>>(),
            [0, 4, 8, 4, 0, 4, 8, 4]
        );

        // A shuffle of 0x00 replaces every dword of the r/m operand with its
        // dword 0, i.e. bytes 0,1,2,3 repeated.
        let mut i = evex_reg(Opcode::EvexVdbpsadbwVdqHdqWdqIbKmask, 0);
        i.set_iq(0x00);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.vmm[0].zmm16u(0), 0, "quadruple 0 still matches exactly");
        assert_eq!(
            cpu.vmm[0].zmm16u(4),
            32,
            "op1 bytes 8..11 against the repeated 0..3 differ by 8 each"
        );
    }

}
