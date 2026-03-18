//! AVX-512F shuffle, permute, and miscellaneous handlers
//!
//! Implements VSHUFF32x4/64x2, VPERMILPS/PD, VPERMPD, VPERMPS,
//! VSHUFPS/PD, VUNPCKLPS/PD, VUNPCKHPS/PD with opmask support.
//!
//! Mirrors Bochs `cpu/avx/avx512.cc` shuffle/permute section.

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
        0 => 4,   // 128-bit
        1 => 8,   // 256-bit
        _ => 16,  // 512-bit
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

/// Write ZMM register with dword masking, zeroing upper bits beyond VL
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
        // Zero upper elements beyond VL (EVEX always clears upper)
        for i in nelements..16 {
            dst.zmm32u[i] = 0;
        }
    }
}

/// Write ZMM register with qword masking
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
        // Zero upper elements beyond VL
        for i in nelements..8 {
            dst.zmm64u[i] = 0;
        }
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // VSHUFF32x4 — Shuffle 128-bit lanes of two float32 sources (EVEX)
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
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let order = instr.ib();
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            if vl == 1 {
                // VL256: 2 output lanes
                let lane0 = (order & 0x1) as usize;
                let lane1 = ((order >> 1) & 0x1) as usize;
                // lane 0 from src1
                result.zmm128[0] = src1.zmm128[lane0];
                // lane 1 from src2
                result.zmm128[1] = src2.zmm128[lane1];
            } else {
                // VL512: 4 output lanes
                let lane0 = (order & 0x3) as usize;
                let lane1 = ((order >> 2) & 0x3) as usize;
                let lane2 = ((order >> 4) & 0x3) as usize;
                let lane3 = ((order >> 6) & 0x3) as usize;
                // lanes 0-1 from src1
                result.zmm128[0] = src1.zmm128[lane0];
                result.zmm128[1] = src1.zmm128[lane1];
                // lanes 2-3 from src2
                result.zmm128[2] = src2.zmm128[lane2];
                result.zmm128[3] = src2.zmm128[lane3];
            }
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VSHUFF64x2 — Shuffle 128-bit lanes of two float64 sources (EVEX)
    // Bochs: VSHUFF64x2_MASK_VpdHpdWpdIbR
    // Same lane selection as VSHUFF32x4, but qword masking granularity.
    // ========================================================================

    pub fn evex_vshuff64x2(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let order = instr.ib();
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            if vl == 1 {
                // VL256
                let lane0 = (order & 0x1) as usize;
                let lane1 = ((order >> 1) & 0x1) as usize;
                result.zmm128[0] = src1.zmm128[lane0];
                result.zmm128[1] = src2.zmm128[lane1];
            } else {
                // VL512
                let lane0 = (order & 0x3) as usize;
                let lane1 = ((order >> 2) & 0x3) as usize;
                let lane2 = ((order >> 4) & 0x3) as usize;
                let lane3 = ((order >> 6) & 0x3) as usize;
                result.zmm128[0] = src1.zmm128[lane0];
                result.zmm128[1] = src1.zmm128[lane1];
                result.zmm128[2] = src2.zmm128[lane2];
                result.zmm128[3] = src2.zmm128[lane3];
            }
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
        let nlanes = match vl { 0 => 1, 1 => 2, _ => 4 };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for lane in 0..nlanes {
                let base = lane * 4;
                result.zmm32u[base]     = src.zmm32u[base + ((order as usize) & 0x3)];
                result.zmm32u[base + 1] = src.zmm32u[base + ((order as usize >> 2) & 0x3)];
                result.zmm32u[base + 2] = src.zmm32u[base + ((order as usize >> 4) & 0x3)];
                result.zmm32u[base + 3] = src.zmm32u[base + ((order as usize >> 6) & 0x3)];
            }
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
        let nlanes = match vl { 0 => 1, 1 => 2, _ => 4 };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for lane in 0..nlanes {
                let base = lane * 2;
                // xmm_shufpd: result[0] = src[(order>>0) & 1], result[1] = src[(order>>1) & 1]
                result.zmm64u[base]     = src.zmm64u[base + ((order as usize) & 0x1)];
                result.zmm64u[base + 1] = src.zmm64u[base + ((order as usize >> 1) & 0x1)];
                order >>= 2;
            }
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
        let src = read_zmm(self, instr.src1());
        let ctrl = read_zmm(self, instr.src2());
        let nlanes = match vl { 0 => 1, 1 => 2, _ => 4 };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for lane in 0..nlanes {
                let base = lane * 4;
                for i in 0..4 {
                    let sel = (ctrl.zmm32u[base + i] & 0x3) as usize;
                    result.zmm32u[base + i] = src.zmm32u[base + sel];
                }
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
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        // Process per 256-bit lane (each has 4 qwords)
        let nymm_lanes = match vl { 0 => 1, 1 => 1, _ => 2 };
        unsafe {
            for ymm in 0..nymm_lanes {
                let base = ymm * 4;
                result.zmm64u[base]     = src.zmm64u[base + ((control as usize) & 0x3)];
                result.zmm64u[base + 1] = src.zmm64u[base + ((control as usize >> 2) & 0x3)];
                result.zmm64u[base + 2] = src.zmm64u[base + ((control as usize >> 4) & 0x3)];
                result.zmm64u[base + 3] = src.zmm64u[base + ((control as usize >> 6) & 0x3)];
            }
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
        let idx = read_zmm(self, instr.src1());
        let src = read_zmm(self, instr.src2());
        let shuffle_mask = (nelements - 1) as u32;
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for n in 0..nelements {
                let sel = (idx.zmm32u[n] & shuffle_mask) as usize;
                result.zmm32u[n] = src.zmm32u[sel];
            }
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
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let order = instr.ib();
        let nlanes = match vl { 0 => 1, 1 => 2, _ => 4 };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for lane in 0..nlanes {
                let base = lane * 4;
                result.zmm32u[base]     = src1.zmm32u[base + ((order as usize) & 0x3)];
                result.zmm32u[base + 1] = src1.zmm32u[base + ((order as usize >> 2) & 0x3)];
                result.zmm32u[base + 2] = src2.zmm32u[base + ((order as usize >> 4) & 0x3)];
                result.zmm32u[base + 3] = src2.zmm32u[base + ((order as usize >> 6) & 0x3)];
            }
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
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let mut order = instr.ib();
        let nlanes = match vl { 0 => 1, 1 => 2, _ => 4 };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for lane in 0..nlanes {
                let base = lane * 2;
                result.zmm64u[base]     = src1.zmm64u[base + ((order as usize) & 0x1)];
                result.zmm64u[base + 1] = src2.zmm64u[base + ((order as usize >> 1) & 0x1)];
                order >>= 2;
            }
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
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let nlanes = match vl { 0 => 1, 1 => 2, _ => 4 };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for lane in 0..nlanes {
                let base = lane * 4;
                result.zmm32u[base]     = src1.zmm32u[base];
                result.zmm32u[base + 1] = src2.zmm32u[base];
                result.zmm32u[base + 2] = src1.zmm32u[base + 1];
                result.zmm32u[base + 3] = src2.zmm32u[base + 1];
            }
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
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let nlanes = match vl { 0 => 1, 1 => 2, _ => 4 };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for lane in 0..nlanes {
                let base = lane * 4;
                result.zmm32u[base]     = src1.zmm32u[base + 2];
                result.zmm32u[base + 1] = src2.zmm32u[base + 2];
                result.zmm32u[base + 2] = src1.zmm32u[base + 3];
                result.zmm32u[base + 3] = src2.zmm32u[base + 3];
            }
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
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let nlanes = match vl { 0 => 1, 1 => 2, _ => 4 };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for lane in 0..nlanes {
                let base = lane * 2;
                result.zmm64u[base]     = src1.zmm64u[base];
                result.zmm64u[base + 1] = src2.zmm64u[base];
            }
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
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let nlanes = match vl { 0 => 1, 1 => 2, _ => 4 };
        let mut result = BxPackedZmmRegister { zmm64u: [0; 8] };

        unsafe {
            for lane in 0..nlanes {
                let base = lane * 2;
                result.zmm64u[base]     = src1.zmm64u[base + 1];
                result.zmm64u[base + 1] = src2.zmm64u[base + 1];
            }
        }

        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}
