//! AVX-512F shuffle, permute, and miscellaneous handlers
//!
//! Implements VSHUFF32x4/64x2, VPERMILPS/PD, VPERMPD, VPERMPS,
//! VSHUFPS/PD, VUNPCKLPS/PD, VUNPCKHPS/PD with opmask support.
//!
//! Mirrors Bochs `cpu/avx/avx512.cc` shuffle/permute section.

use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, decoder::Instruction, xmm::BxPackedZmmRegister};

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

/// Write ZMM register with dword masking, zeroing upper bits beyond VL
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
    }
    // Zero upper elements beyond VL (EVEX always clears upper)
    for i in nelements..16 {
        dst.set_zmm32u(i, 0);
    }
}

/// Write ZMM register with qword masking
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

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // VSHUFF32x4 — Shuffle 128-bit lanes of two Float32 sources (EVEX)
    // Bochs: VSHUFF32x4_MASK_VpsHpsWpsIbR
    //
    // VL256: result.lane[0] = src1.lane[order[0:0]]
    //        result.lane[1] = src2.lane[order[1:1]]
    // VL512: result.lane[0] = src1.lane[order[1:0]]
    //        result.lane[1] = src1.lane[order[3:2]]
    //        result.lane[2] = src2.lane[order[5:4]]
    //        result.lane[3] = src2.lane[order[7:6]]
    // ========================================================================

    pub fn evex_vshuff32x4(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_zmm(self, instr.src1());
        let order = instr.ib();
        let mut result = BxPackedZmmRegister::default();

        if vl == 1 {
            // VL256: 2 output lanes
            let lane0 = (order & 0x1) as usize;
            let lane1 = ((order >> 1) & 0x1) as usize;
            // lane 0 from src1
            result.set_zmm128(0, src1.zmm128(lane0));
            // lane 1 from src2
            result.set_zmm128(1, src2.zmm128(lane1));
        } else {
            // VL512: 4 output lanes
            let lane0 = (order & 0x3) as usize;
            let lane1 = ((order >> 2) & 0x3) as usize;
            let lane2 = ((order >> 4) & 0x3) as usize;
            let lane3 = ((order >> 6) & 0x3) as usize;
            // lanes 0-1 from src1
            result.set_zmm128(0, src1.zmm128(lane0));
            result.set_zmm128(1, src1.zmm128(lane1));
            // lanes 2-3 from src2
            result.set_zmm128(2, src2.zmm128(lane2));
            result.set_zmm128(3, src2.zmm128(lane3));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VSHUFF64x2 — Shuffle 128-bit lanes of two Float64 sources (EVEX)
    // Bochs: VSHUFF64x2_MASK_VpdHpdWpdIbR
    // Same lane selection as VSHUFF32x4, but qword masking granularity.
    // ========================================================================

    pub fn evex_vshuff64x2(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_zmm(self, instr.src1());
        let order = instr.ib();
        let mut result = BxPackedZmmRegister::default();

        if vl == 1 {
            // VL256
            let lane0 = (order & 0x1) as usize;
            let lane1 = ((order >> 1) & 0x1) as usize;
            result.set_zmm128(0, src1.zmm128(lane0));
            result.set_zmm128(1, src2.zmm128(lane1));
        } else {
            // VL512
            let lane0 = (order & 0x3) as usize;
            let lane1 = ((order >> 2) & 0x3) as usize;
            let lane2 = ((order >> 4) & 0x3) as usize;
            let lane3 = ((order >> 6) & 0x3) as usize;
            result.set_zmm128(0, src1.zmm128(lane0));
            result.set_zmm128(1, src1.zmm128(lane1));
            result.set_zmm128(2, src2.zmm128(lane2));
            result.set_zmm128(3, src2.zmm128(lane3));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPERMILPS imm — Per-lane shuffle SP FP by immediate
    // Bochs: VPERMILPS_MASK_VpsWpsIbR (uses xmm_shufps per lane)
    //
    // Each 128-bit lane: 4 floats shuffled by imm8[1:0], [3:2], [5:4], [7:6]
    // result.lane[n][0] = src.lane[n][imm[1:0]]
    // result.lane[n][1] = src.lane[n][imm[3:2]]
    // result.lane[n][2] = src.lane[n][imm[5:4]]
    // result.lane[n][3] = src.lane[n][imm[7:6]]
    // ========================================================================

    pub fn evex_vpermilps_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let order = instr.ib();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();

        for lane in 0..nlanes {
            let base = lane * 4;
            result.set_zmm32u(base, src.zmm32u(base + ((order as usize) & 0x3)));
            result.set_zmm32u(base + 1, src.zmm32u(base + ((order as usize >> 2) & 0x3)));
            result.set_zmm32u(base + 2, src.zmm32u(base + ((order as usize >> 4) & 0x3)));
            result.set_zmm32u(base + 3, src.zmm32u(base + ((order as usize >> 6) & 0x3)));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPERMILPD imm — Per-lane permute DP FP by immediate
    // Bochs: VPERMILPD_MASK_VpdWpdIbR (uses xmm_shufpd per lane)
    //
    // Each 128-bit lane has 2 qwords. Per lane, the control bits shift right
    // by 2 bits per lane:
    //   lane 0: result[0] = src[order[0]], result[1] = src[order[1]]
    //   lane 1: order >>= 2, same pattern
    //   ...
    // ========================================================================

    pub fn evex_vpermilpd_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mut order = instr.ib();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();

        for lane in 0..nlanes {
            let base = lane * 2;
            // xmm_shufpd: result[0] = src[(order>>0) & 1], result[1] = src[(order>>1) & 1]
            result.set_zmm64u(base, src.zmm64u(base + ((order as usize) & 0x1)));
            result.set_zmm64u(base + 1, src.zmm64u(base + ((order as usize >> 1) & 0x1)));
            order >>= 2;
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPERMILPS reg — Per-element permute SP FP using register indices
    // Bochs: HANDLE_AVX512_3OP_DWORD_EL_MASK<xmm_permilps>
    //
    // Per 128-bit lane:
    //   result.lane[n][i] = src1.lane[n][ ctrl.lane[n][i] & 3 ]
    // (src1=Hps, ctrl=Wps in Bochs terminology)
    // ========================================================================

    pub fn evex_vpermilps_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src2()); // vvvv — the data
        let ctrl = self.perm_rm_d(instr)?; // rm — the control
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();

        for lane in 0..nlanes {
            let base = lane * 4;
            for i in 0..4 {
                let sel = (ctrl.zmm32u(base + i) & 0x3) as usize;
                result.set_zmm32u(base + i, src.zmm32u(base + sel));
            }
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPERMPD imm — Permute DP FP by immediate (per 256-bit lane)
    // Bochs: VPERMQ_MASK_VdqWdqIbR (uses ymm_vpermq per 256-bit lane)
    //
    // Per 256-bit lane: 4 qwords shuffled by:
    //   result[0] = src[(control) & 3]
    //   result[1] = src[(control>>2) & 3]
    //   result[2] = src[(control>>4) & 3]
    //   result[3] = src[(control>>6) & 3]
    // VL256: 1 ymm lane. VL512: 2 ymm lanes.
    // ========================================================================

    pub fn evex_vpermpd_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let control = instr.ib();
        let mut result = BxPackedZmmRegister::default();

        // Process per 256-bit lane (each has 4 qwords)
        let nymm_lanes = match vl {
            0 => 1,
            1 => 1,
            _ => 2,
        };
        for ymm in 0..nymm_lanes {
            let base = ymm * 4;
            result.set_zmm64u(base, src.zmm64u(base + ((control as usize) & 0x3)));
            result.set_zmm64u(base + 1, src.zmm64u(base + ((control as usize >> 2) & 0x3)));
            result.set_zmm64u(base + 2, src.zmm64u(base + ((control as usize >> 4) & 0x3)));
            result.set_zmm64u(base + 3, src.zmm64u(base + ((control as usize >> 6) & 0x3)));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPERMPS — Permute SP FP by register indices (full-width)
    // Bochs: VPERMPS_MASK_VpsHpsWpsR
    //
    // result.dword[n] = src2.dword[ src1.dword[n] & (elements-1) ]
    // ========================================================================

    pub fn evex_vpermps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let idx = read_zmm(self, instr.src2());
        let src = read_zmm(self, instr.src1());
        let shuffle_mask = (nelements - 1) as u32;
        let mut result = BxPackedZmmRegister::default();

        for n in 0..nelements {
            let sel = (idx.zmm32u(n) & shuffle_mask) as usize;
            result.set_zmm32u(n, src.zmm32u(sel));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VSHUFPS — Shuffle packed SP FP (per 128-bit lane)
    // Bochs: VSHUFPS_MASK_VpsHpsWpsIbR (uses xmm_shufps per lane)
    //
    // Per 128-bit lane:
    //   result[0] = src1[imm[1:0]]
    //   result[1] = src1[imm[3:2]]
    //   result[2] = src2[imm[5:4]]
    //   result[3] = src2[imm[7:6]]
    // ========================================================================

    pub fn evex_vshufps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_zmm(self, instr.src1());
        let order = instr.ib();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();

        for lane in 0..nlanes {
            let base = lane * 4;
            result.set_zmm32u(base, src1.zmm32u(base + ((order as usize) & 0x3)));
            result.set_zmm32u(base + 1, src1.zmm32u(base + ((order as usize >> 2) & 0x3)));
            result.set_zmm32u(base + 2, src2.zmm32u(base + ((order as usize >> 4) & 0x3)));
            result.set_zmm32u(base + 3, src2.zmm32u(base + ((order as usize >> 6) & 0x3)));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VSHUFPD — Shuffle packed DP FP (per 128-bit lane)
    // Bochs: VSHUFPD_MASK_VpdHpdWpdIbR (uses xmm_shufpd per lane)
    //
    // Per 128-bit lane:
    //   result[0] = src1[(order>>0) & 1]
    //   result[1] = src2[(order>>1) & 1]
    // order >>= 2 for each subsequent lane.
    // ========================================================================

    pub fn evex_vshufpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_zmm(self, instr.src1());
        let mut order = instr.ib();
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();

        for lane in 0..nlanes {
            let base = lane * 2;
            result.set_zmm64u(base, src1.zmm64u(base + ((order as usize) & 0x1)));
            result.set_zmm64u(base + 1, src2.zmm64u(base + ((order as usize >> 1) & 0x1)));
            order >>= 2;
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VUNPCKLPS — Interleave low SP FP from two sources (per 128-bit lane)
    // Bochs: HANDLE_AVX512_2OP_DWORD_EL_MASK<xmm_unpcklps>
    //
    // Per 128-bit lane:
    //   result[0] = src1[0], result[1] = src2[0]
    //   result[2] = src1[1], result[3] = src2[1]
    // ========================================================================

    pub fn evex_vunpcklps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_zmm(self, instr.src1());
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();

        for lane in 0..nlanes {
            let base = lane * 4;
            result.set_zmm32u(base, src1.zmm32u(base));
            result.set_zmm32u(base + 1, src2.zmm32u(base));
            result.set_zmm32u(base + 2, src1.zmm32u(base + 1));
            result.set_zmm32u(base + 3, src2.zmm32u(base + 1));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VUNPCKHPS — Interleave high SP FP from two sources (per 128-bit lane)
    // Bochs: HANDLE_AVX512_2OP_DWORD_EL_MASK<xmm_unpckhps>
    //
    // Per 128-bit lane:
    //   result[0] = src1[2], result[1] = src2[2]
    //   result[2] = src1[3], result[3] = src2[3]
    // ========================================================================

    pub fn evex_vunpckhps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_zmm(self, instr.src1());
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();

        for lane in 0..nlanes {
            let base = lane * 4;
            result.set_zmm32u(base, src1.zmm32u(base + 2));
            result.set_zmm32u(base + 1, src2.zmm32u(base + 2));
            result.set_zmm32u(base + 2, src1.zmm32u(base + 3));
            result.set_zmm32u(base + 3, src2.zmm32u(base + 3));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VUNPCKLPD — Interleave low DP FP from two sources (per 128-bit lane)
    // Bochs: HANDLE_AVX512_2OP_QWORD_EL_MASK<xmm_unpcklpd>
    //
    // Per 128-bit lane:
    //   result[0] = src1[0], result[1] = src2[0]
    // ========================================================================

    pub fn evex_vunpcklpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_zmm(self, instr.src1());
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();

        for lane in 0..nlanes {
            let base = lane * 2;
            result.set_zmm64u(base, src1.zmm64u(base));
            result.set_zmm64u(base + 1, src2.zmm64u(base));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VUNPCKHPD — Interleave high DP FP from two sources (per 128-bit lane)
    // Bochs: HANDLE_AVX512_2OP_QWORD_EL_MASK<xmm_unpckhpd>
    //
    // Per 128-bit lane:
    //   result[0] = src1[1], result[1] = src2[1]
    // ========================================================================

    pub fn evex_vunpckhpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src2());
        let src2 = read_zmm(self, instr.src1());
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();

        for lane in 0..nlanes {
            let base = lane * 2;
            result.set_zmm64u(base, src1.zmm64u(base + 1));
            result.set_zmm64u(base + 1, src2.zmm64u(base + 1));
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // Two-table permutes and the remaining single-table ones.
    // Bochs avx512.cc VPERMT2*/VPERMI2*, VPERMW, VPERMPD, and the AVX512_DQ
    // VPMULLQ.
    //
    // VPERMT2 takes its indices from vvvv and selects between the destination
    // and r/m; VPERMI2 takes them from the destination and selects between
    // vvvv and r/m. Either way the index's `elements` bit picks the table and
    // the low bits pick the element, so the pair addresses a 2n-element pool.
    // ========================================================================

    /// Read the r/m operand of a dword-granular permute.
    fn perm_rm_d(&mut self, instr: &Instruction) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src1()))
        } else {
            self.evex_load_broadcast_vector_d(instr)
        }
    }

    /// Qword counterpart of [`Self::perm_rm_d`].
    fn perm_rm_q(&mut self, instr: &Instruction) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src1()))
        } else {
            self.evex_load_broadcast_vector_q(instr)
        }
    }

    /// The shared dword body. `index_from_dst` selects VPERMI2 over VPERMT2.
    fn evex_perm2_d(&mut self, instr: &Instruction, index_from_dst: bool) -> super::Result<()> {
        let vl = instr.get_vl();
        let elements = dword_elements(vl);
        let control_mask = (elements - 1) as u32;
        let op1 = read_zmm(self, instr.src2()); // vvvv
        let op2 = self.perm_rm_d(instr)?;
        let dst = read_zmm(self, instr.dst());
        let mut result = BxPackedZmmRegister::default();
        for n in 0..elements {
            let control = if index_from_dst { dst.zmm32u(n) } else { op1.zmm32u(n) };
            let sel = (control & control_mask) as usize;
            let from_op2 = control & (elements as u32) != 0;
            let v = if from_op2 {
                op2.zmm32u(sel)
            } else if index_from_dst {
                op1.zmm32u(sel)
            } else {
                dst.zmm32u(sel)
            };
            result.set_zmm32u(n, v);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// Qword counterpart of [`Self::evex_perm2_d`].
    fn evex_perm2_q(&mut self, instr: &Instruction, index_from_dst: bool) -> super::Result<()> {
        let vl = instr.get_vl();
        let elements = qword_elements(vl);
        let control_mask = (elements - 1) as u64;
        let op1 = read_zmm(self, instr.src2());
        let op2 = self.perm_rm_q(instr)?;
        let dst = read_zmm(self, instr.dst());
        let mut result = BxPackedZmmRegister::default();
        for n in 0..elements {
            let control = if index_from_dst { dst.zmm64u(n) } else { op1.zmm64u(n) };
            let sel = (control & control_mask) as usize;
            let from_op2 = control & (elements as u64) != 0;
            let v = if from_op2 {
                op2.zmm64u(sel)
            } else if index_from_dst {
                op1.zmm64u(sel)
            } else {
                dst.zmm64u(sel)
            };
            result.set_zmm64u(n, v);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPERMT2D / VPERMT2PS — EVEX.66.0F38.W0 7E / 7F
    pub fn evex_vpermt2d(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_perm2_d(instr, false)
    }
    /// VPERMI2D / VPERMI2PS — EVEX.66.0F38.W0 76 / 77
    pub fn evex_vpermi2d(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_perm2_d(instr, true)
    }
    /// VPERMT2Q / VPERMT2PD — EVEX.66.0F38.W1 7E / 7F
    pub fn evex_vpermt2q(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_perm2_q(instr, false)
    }
    /// VPERMI2Q / VPERMI2PD — EVEX.66.0F38.W1 76 / 77
    pub fn evex_vpermi2q(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_perm2_q(instr, true)
    }

    /// VPERMPD Vpd{k}{z}, Hpd, Wpd — EVEX.66.0F38.W1 16, the variable form:
    /// vvvv holds one index per element, rm is the table.
    pub fn evex_vpermpd_var(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let elements = qword_elements(vl);
        let control_mask = (elements - 1) as u64;
        let op1 = read_zmm(self, instr.src2());
        let op2 = self.perm_rm_q(instr)?;
        let mut result = BxPackedZmmRegister::default();
        for n in 0..elements {
            result.set_zmm64u(n, op2.zmm64u((op1.zmm64u(n) & control_mask) as usize));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPERMILPD Vpd{k}{z}, Hpd, Wpd — EVEX.66.0F38.W1 0D. Per 128-bit lane,
    /// and the selector is bit 1 of each qword rather than bit 0.
    pub fn evex_vpermilpd_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src2()); // vvvv — the data
        let ctrl = self.perm_rm_q(instr)?; // rm — the control
        let nlanes = match vl {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut result = BxPackedZmmRegister::default();
        for lane in 0..nlanes {
            let base = lane * 2;
            for i in 0..2 {
                let sel = ((ctrl.zmm64u(base + i) >> 1) & 1) as usize;
                result.set_zmm64u(base + i, src.zmm64u(base + sel));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPMULLQ Vdq{k}{z}, Hdq, Wdq — EVEX.66.0F38.W1 40 (AVX512_DQ). The low
    /// 64 bits of each 64x64 product.
    pub fn evex_vpmullq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let elements = qword_elements(vl);
        let op1 = read_zmm(self, instr.src2());
        let op2 = self.perm_rm_q(instr)?;
        let mut result = BxPackedZmmRegister::default();
        for n in 0..elements {
            result.set_zmm64u(n, op1.zmm64u(n).wrapping_mul(op2.zmm64u(n)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    //! The two-table permutes address a pool twice the vector's width: the
    //! index's `elements` bit picks which table and the low bits pick the
    //! element. VPERMT2 reads its indices from vvvv and selects between the
    //! destination and r/m; VPERMI2 reads them from the destination and
    //! selects between vvvv and r/m — so the same index vector gives
    //! different answers for the two, which is what these tests pin.

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::BxSegregs;
    use rusty_box_decoder::opcode::Opcode;

    use super::*;

    fn evex(opcode: Opcode) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0); // dst
        i.set_src_reg(1, 1); // ModRM.rm
        i.set_src_reg(2, 2); // EVEX.vvvv
        i.set_opmask(0);
        i.set_vex(true);
        i.set_vl(0); // 128-bit: four dwords, so an eight-element pool
        i.set_seg(BxSegregs::Ds);
        i.init(0, 0, 1, 1);
        i.assert_mod_c0();
        i
    }

    #[test]
    fn vpermt2d_indexes_from_vvvv_across_the_destination_and_rm() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for n in 0..4 {
            c.vmm[0].set_zmm32u(n, 100 + n as u32); // dst  = table 0
            c.vmm[1].set_zmm32u(n, 200 + n as u32); // rm   = table 1
        }
        // Indices 0..3 with bit 2 set on the middle two, selecting table 1.
        for (n, ix) in [0u32, 5, 6, 3].into_iter().enumerate() {
            c.vmm[2].set_zmm32u(n, ix);
        }
        c.execute_instruction(&evex(Opcode::EvexVpermt2dVdqHdqWdqKmask))
            .unwrap();
        assert_eq!(
            [0, 1, 2, 3].map(|n| c.vmm[0].zmm32u(n)),
            [100, 201, 202, 103]
        );
    }

    #[test]
    fn vpermi2d_indexes_from_the_destination_across_vvvv_and_rm() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for n in 0..4 {
            c.vmm[2].set_zmm32u(n, 100 + n as u32); // vvvv = table 0
            c.vmm[1].set_zmm32u(n, 200 + n as u32); // rm   = table 1
        }
        for (n, ix) in [0u32, 5, 6, 3].into_iter().enumerate() {
            c.vmm[0].set_zmm32u(n, ix); // the destination carries the indices
        }
        c.execute_instruction(&evex(Opcode::EvexVpermi2dVdqHdqWdqKmask))
            .unwrap();
        assert_eq!(
            [0, 1, 2, 3].map(|n| c.vmm[0].zmm32u(n)),
            [100, 201, 202, 103]
        );
    }

    #[test]
    fn vpermilpd_selects_on_bit_one_within_each_128_bit_lane() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.vmm[2].set_zmm64u(0, 0xAAAA); // vvvv — the data
        c.vmm[2].set_zmm64u(1, 0xBBBB);
        // Bit 0 is ignored; only bit 1 selects.
        c.vmm[1].set_zmm64u(0, 0b10);
        c.vmm[1].set_zmm64u(1, 0b01);
        c.execute_instruction(&evex(Opcode::EvexVpermilpdVpdHpdWpd))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 0xBBBB, "bit 1 set selects element 1");
        assert_eq!(c.vmm[0].zmm64u(1), 0xAAAA, "bit 0 is not part of the control");
    }

    #[test]
    fn vpmullq_keeps_the_low_64_bits_of_the_product() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.vmm[2].set_zmm64u(0, 0x1_0000_0001);
        c.vmm[1].set_zmm64u(0, 0x1_0000_0001);
        c.vmm[2].set_zmm64u(1, 7);
        c.vmm[1].set_zmm64u(1, 6);
        c.execute_instruction(&evex(Opcode::EvexVpmullqVdqHdqWdq))
            .unwrap();
        // (2^32+1)^2 = 2^64 + 2^33 + 1, and the 2^64 term is discarded.
        assert_eq!(c.vmm[0].zmm64u(0), 0x2_0000_0001);
        assert_eq!(c.vmm[0].zmm64u(1), 42);
    }
}
