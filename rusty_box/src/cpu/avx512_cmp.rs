//! AVX-512F comparison and miscellaneous handlers
//!
//! Implements VCMPPS, VCMPPD (floating-point compare to opmask),
//! VPTESTMD/MQ/NMD/NMQ (packed test to opmask),
//! VPMOVM2D/Q (expand opmask to vector),
//! VPMOVD2M/Q2M (compress sign bits to opmask).
//!
//! Mirrors Bochs `cpu/avx/avx512_cmp.cc`, `avx512_pfp.cc`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
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
fn read_opmask_for_write<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, instr: &Instruction) -> u64 {
    let k = instr.opmask();
    if k == 0 {
        u64::MAX
    } else {
        // SAFETY: opmask register union always valid for rrx (full 64-bit) access
        unsafe { cpu.opmask[k as usize].rrx() }
    }
}

/// Read ZMM register as a ZMM-width value
#[inline]
fn read_zmm<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, reg: u8) -> BxPackedZmmRegister {
    cpu.vmm[reg as usize]
}

/// Write ZMM register, zeroing upper bits beyond VL (dword masking granularity)
fn write_zmm_masked<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
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
    // Zero upper elements beyond VL
    for i in nelements..16 {
        dst.set_zmm32u(i, 0);
    }
}

/// Write ZMM register for qword operations
fn write_zmm_masked_q<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
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
    for i in nelements..8 {
        dst.set_zmm64u(i, 0);
    }
}

/// Read src2 dword elements from register or memory
fn read_src2_dwords<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    nelements: usize,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let mut tmp = BxPackedZmmRegister::default();
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        for i in 0..nelements {
            tmp.set_zmm32u(i, cpu.v_read_dword(seg, laddr + (i * 4) as u64)?);
        }
        Ok(tmp)
    }
}

/// Read src2 qword elements from register or memory
fn read_src2_qwords<I: BxCpuIdTrait>(
    cpu: &mut BxCpuC<'_, I>,
    instr: &Instruction,
    nelements: usize,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        let mut tmp = BxPackedZmmRegister::default();
        let laddr = cpu.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        for i in 0..nelements {
            let lo = cpu.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
            let hi = cpu.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
            tmp.set_zmm64u(i, lo | (hi << 32));
        }
        Ok(tmp)
    }
}

// ============================================================================
// Floating-point comparison predicates (32 predicates, imm8[4:0])
// ============================================================================

/// Compare two f32 values per the VCMPPS/VCMPSS predicate encoding.
///
/// The 32 predicates (imm8 & 0x1F) group into 8 base operations that repeat
/// with signaling/quiet variants (same logic for emulation purposes):
///   0=EQ, 1=LT, 2=LE, 3=UNORD, 4=NEQ, 5=NLT, 6=NLE, 7=ORD
#[inline]
/// Intel VCMPPS/VCMPSD predicate encoding (32 predicates, two groups).
/// Group A (0-7, 16-23): ordered base comparisons.
/// Group B (8-15, 24-31): swapped ordered/unordered sense.
/// Signaling vs quiet (0-7 vs 16-23, 8-15 vs 24-31) only affects exception
/// flags which we don't implement — same logical result within each pair.
fn fp_compare_f32(a: f32, b: f32, imm: u8) -> bool {
    let unordered = a.is_nan() || b.is_nan();
    match imm & 0x1F {
        // Group A: ordered (0-7, 16-23)
        0 | 16 => !unordered && a == b,         // EQ_OQ / EQ_OS
        1 | 17 => !unordered && a < b,          // LT_OS / LT_OQ
        2 | 18 => !unordered && a <= b,         // LE_OS / LE_OQ
        3 | 19 => unordered,                     // UNORD_Q / UNORD_S
        4 | 20 => unordered || a != b,           // NEQ_UQ / NEQ_US
        5 | 21 => unordered || a >= b,           // NLT_US / NLT_UQ
        6 | 22 => unordered || a > b,            // NLE_US / NLE_UQ
        7 | 23 => !unordered,                    // ORD_Q / ORD_S
        // Group B: swapped (8-15, 24-31)
        8 | 24 => unordered || a == b,           // EQ_UQ / EQ_US
        9 | 25 => unordered || a < b,            // NGE_US / NGE_UQ
        10 | 26 => unordered || a <= b,          // NGT_US / NGT_UQ
        11 | 27 => false,                         // FALSE_OQ / FALSE_OS
        12 | 28 => !unordered && a != b,          // NEQ_OQ / NEQ_OS
        13 | 29 => !unordered && a >= b,          // GE_OS / GE_OQ
        14 | 30 => !unordered && a > b,           // GT_OS / GT_OQ
        15 | 31 => true,                          // TRUE_UQ / TRUE_US
        _ => unreachable!(),
    }
}

#[inline]
fn fp_compare_f64(a: f64, b: f64, imm: u8) -> bool {
    let unordered = a.is_nan() || b.is_nan();
    match imm & 0x1F {
        0 | 16 => !unordered && a == b,
        1 | 17 => !unordered && a < b,
        2 | 18 => !unordered && a <= b,
        3 | 19 => unordered,
        4 | 20 => unordered || a != b,
        5 | 21 => unordered || a >= b,
        6 | 22 => unordered || a > b,
        7 | 23 => !unordered,
        8 | 24 => unordered || a == b,
        9 | 25 => unordered || a < b,
        10 | 26 => unordered || a <= b,
        11 | 27 => false,
        12 | 28 => !unordered && a != b,
        13 | 29 => !unordered && a >= b,
        14 | 30 => !unordered && a > b,
        15 | 31 => true,
        _ => unreachable!(),
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ========================================================================
    // VCMPPS — Compare packed single-precision FP, producing opmask
    // EVEX.NDS.W0.0F C2 /r ib
    // ========================================================================

    /// VCMPPS Kk{k}, Hps, Wps, Ib — register form
    pub fn evex_vcmpps_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let imm = instr.ib();
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if fp_compare_f32(src1.zmm32f(i), src2.zmm32f(i), imm)
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VCMPPS Kk{k}, Hps, Mps, Ib — memory form
    pub fn evex_vcmpps_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let mut src2 = BxPackedZmmRegister::default();
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        for i in 0..nelements {
            src2.set_zmm32u(i, self.v_read_dword(seg, laddr + (i * 4) as u64)?);
        }
        let imm = instr.ib();
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if fp_compare_f32(src1.zmm32f(i), src2.zmm32f(i), imm)
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // VCMPPD — Compare packed double-precision FP, producing opmask
    // EVEX.NDS.W1.0F C2 /r ib
    // ========================================================================

    /// VCMPPD Kk{k}, Hpd, Wpd, Ib — register form
    pub fn evex_vcmppd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let imm = instr.ib();
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if fp_compare_f64(src1.zmm64f(i), src2.zmm64f(i), imm)
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VCMPPD Kk{k}, Hpd, Mpd, Ib — memory form
    pub fn evex_vcmppd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let mut src2 = BxPackedZmmRegister::default();
        let laddr = self.resolve_addr(instr);
        let seg = BxSegregs::from(instr.seg());
        for i in 0..nelements {
            let lo = self.v_read_dword(seg, laddr + (i * 8) as u64)? as u64;
            let hi = self.v_read_dword(seg, laddr + (i * 8 + 4) as u64)? as u64;
            src2.set_zmm64u(i, lo | (hi << 32));
        }
        let imm = instr.ib();
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if fp_compare_f64(src1.zmm64f(i), src2.zmm64f(i), imm)
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // VPTESTMD — Test packed dwords, set opmask where (src1 AND src2) != 0
    // EVEX.NDS.66.0F38.W0 27
    // ========================================================================

    /// VPTESTMD Kk{k}, Hdq, Wdq — register form
    pub fn evex_vptestmd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src1.zmm32u(i) & src2.zmm32u(i)) != 0
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPTESTMD Kk{k}, Hdq, Mdq — memory form
    pub fn evex_vptestmd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_dwords(self, instr, nelements)?;
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src1.zmm32u(i) & src2.zmm32u(i)) != 0
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // VPTESTMQ — Test packed qwords, set opmask where (src1 AND src2) != 0
    // EVEX.NDS.66.0F38.W1 27
    // ========================================================================

    /// VPTESTMQ Kk{k}, Hdq, Wdq — register form
    pub fn evex_vptestmq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src1.zmm64u(i) & src2.zmm64u(i)) != 0
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPTESTMQ Kk{k}, Hdq, Mdq — memory form
    pub fn evex_vptestmq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_qwords(self, instr, nelements)?;
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src1.zmm64u(i) & src2.zmm64u(i)) != 0
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // VPTESTNMD — Test packed dwords, set opmask where (src1 AND src2) == 0
    // EVEX.NDS.F3.0F38.W0 27
    // ========================================================================

    /// VPTESTNMD Kk{k}, Hdq, Wdq — register form
    pub fn evex_vptestnmd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src1.zmm32u(i) & src2.zmm32u(i)) == 0
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPTESTNMD Kk{k}, Hdq, Mdq — memory form
    pub fn evex_vptestnmd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_dwords(self, instr, nelements)?;
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src1.zmm32u(i) & src2.zmm32u(i)) == 0
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // VPTESTNMQ — Test packed qwords, set opmask where (src1 AND src2) == 0
    // EVEX.NDS.F3.0F38.W1 27
    // ========================================================================

    /// VPTESTNMQ Kk{k}, Hdq, Wdq — register form
    pub fn evex_vptestnmq_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_zmm(self, instr.src2());
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src1.zmm64u(i) & src2.zmm64u(i)) == 0
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPTESTNMQ Kk{k}, Hdq, Mdq — memory form
    pub fn evex_vptestnmq_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = read_src2_qwords(self, instr, nelements)?;
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src1.zmm64u(i) & src2.zmm64u(i)) == 0
                && ((write_mask >> i) & 1 != 0)
            {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // VPMOVM2D — Expand opmask to packed dword vector
    // EVEX.F3.0F38.W0 38
    // Set each dword to 0xFFFFFFFF where mask bit is 1, 0 where 0.
    // ========================================================================

    /// VPMOVM2D Vdq, Kk
    pub fn evex_vpmovm2d(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        // SAFETY: opmask register union always valid for rrx (full 64-bit) access
        let mask = unsafe { self.opmask[instr.src() as usize].rrx() };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, if (mask >> i) & 1 != 0 {
                0xFFFF_FFFF
            } else {
                0
            });
        }
        // No write masking for this instruction; always full write, zero upper
        write_zmm_masked(self, instr.dst(), &result, u64::MAX, true, vl);
        Ok(())
    }

    // ========================================================================
    // VPMOVM2Q — Expand opmask to packed qword vector
    // EVEX.F3.0F38.W1 38
    // Set each qword to all-ones where mask bit is 1, all-zeros where 0.
    // ========================================================================

    /// VPMOVM2Q Vdq, Kk
    pub fn evex_vpmovm2q(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        // SAFETY: opmask register union always valid for rrx (full 64-bit) access
        let mask = unsafe { self.opmask[instr.src() as usize].rrx() };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, if (mask >> i) & 1 != 0 {
                u64::MAX
            } else {
                0
            });
        }
        write_zmm_masked_q(self, instr.dst(), &result, u64::MAX, true, vl);
        Ok(())
    }

    // ========================================================================
    // VPMOVD2M — Compress sign bits of packed dwords to opmask
    // EVEX.F3.0F38.W0 39
    // result_bit[i] = src.zmm32u[i] >> 31
    // ========================================================================

    /// VPMOVD2M Kk, Vdq
    pub fn evex_vpmovd2m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src.zmm32u(i) >> 31) != 0 {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // VPMOVQ2M — Compress sign bits of packed qwords to opmask
    // EVEX.F3.0F38.W1 39
    // result_bit[i] = src.zmm64u[i] >> 63
    // ========================================================================

    /// VPMOVQ2M Kk, Vdq
    pub fn evex_vpmovq2m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src.zmm64u(i) >> 63) != 0 {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }
}
