#![allow(unused_unsafe, dead_code)]

//! AVX-512F comparison and miscellaneous handlers
//!
//! Implements VCMPPS, VCMPPD (floating-point compare to opmask),
//! VPTESTMD/MQ/NMD/NMQ (packed test to opmask),
//! VPMOVM2D/Q (expand opmask to vector),
//! VPMOVD2M/Q2M (compress sign bits to opmask).
//!
//! Mirrors Bochs `cpu/avx/avx512_cmp.cc`, `avx512_pfp.cc`.

use super::softfloat3e::softfloat::softfloat_get_exception_flags;
use super::softfloat3e::softfloat_compare::{f32_compare_predicate, f64_compare_predicate};
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
        0 => 16,
        1 => 32,
        _ => 64,
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
fn read_opmask_for_write<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &BxCpuC<'_, I, T>,
    instr: &Instruction,
) -> u64 {
    let k = instr.opmask();
    if k == 0 {
        u64::MAX
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

/// Write ZMM register, zeroing upper bits beyond VL (dword masking granularity)
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
    // Zero upper elements beyond VL
    for i in nelements..16 {
        dst.set_zmm32u(i, 0);
    }
}

/// Write ZMM register for qword operations
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
    for i in nelements..8 {
        dst.set_zmm64u(i, 0);
    }
}

/// Read src2 dword elements from register or memory
fn read_src2_dwords<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    _nelements: usize,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        cpu.evex_load_broadcast_mask_vector_d(instr)
    }
}

/// Read src2 qword elements from register or memory
fn read_src2_qwords<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    instr: &Instruction,
    _nelements: usize,
) -> super::Result<BxPackedZmmRegister> {
    if instr.mod_c0() {
        Ok(read_zmm(cpu, instr.src2()))
    } else {
        cpu.evex_load_broadcast_mask_vector_q(instr)
    }
}

// ============================================================================
// Floating-point comparison predicates (32 predicates, imm8[4:0])
// ============================================================================

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // VCMPPS / VCMPPD — Compare packed FP, producing an opmask
    // EVEX.NDS.W0.0F C2 /r ib and EVEX.NDS.W1.0F C2 /r ib
    //
    // Bochs avx512_pfp.cc VCMPPS_MASK_KGwHpsWpsIbR: an element masked off by
    // the writemask is not compared at all, so it raises no exception and
    // its result bit stays clear. The accumulated flags then go through
    // check_exceptionsSSE before the opmask is written.
    // ========================================================================

    /// The shared body of the four VCMPPS/VCMPPD forms.
    fn evex_cmp_pfp(
        &mut self,
        instr: &Instruction,
        src2: BxPackedZmmRegister,
        qword: bool,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = if qword {
            qword_elements(vl)
        } else {
            dword_elements(vl)
        };
        let src1 = read_zmm(self, instr.src1());
        let predicate = instr.ib() & 0x1F;
        let write_mask = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (write_mask >> i) & 1 == 0 {
                continue;
            }
            let hit = if qword {
                f64_compare_predicate(predicate, src1.zmm64u(i), src2.zmm64u(i), &mut status)
            } else {
                f32_compare_predicate(predicate, src1.zmm32u(i), src2.zmm32u(i), &mut status)
            };
            if hit {
                result |= 1 << i;
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VCMPPS Kk{k}, Hps, Wps, Ib — register form
    pub fn evex_vcmpps_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src2 = read_zmm(self, instr.src2());
        self.evex_cmp_pfp(instr, src2, false)
    }

    /// VCMPPS Kk{k}, Hps, Mps, Ib — memory form
    pub fn evex_vcmpps_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let src2 = self.evex_load_broadcast_mask_vector_d(instr)?;
        self.evex_cmp_pfp(instr, src2, false)
    }

    /// VCMPPD Kk{k}, Hpd, Wpd, Ib — register form
    pub fn evex_vcmppd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let src2 = read_zmm(self, instr.src2());
        self.evex_cmp_pfp(instr, src2, true)
    }

    /// VCMPPD Kk{k}, Hpd, Mpd, Ib — memory form
    pub fn evex_vcmppd_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let src2 = self.evex_load_broadcast_mask_vector_q(instr)?;
        self.evex_cmp_pfp(instr, src2, true)
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
            if (src1.zmm32u(i) & src2.zmm32u(i)) != 0 && ((write_mask >> i) & 1 != 0) {
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
            if (src1.zmm32u(i) & src2.zmm32u(i)) != 0 && ((write_mask >> i) & 1 != 0) {
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
            if (src1.zmm64u(i) & src2.zmm64u(i)) != 0 && ((write_mask >> i) & 1 != 0) {
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
            if (src1.zmm64u(i) & src2.zmm64u(i)) != 0 && ((write_mask >> i) & 1 != 0) {
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
            if (src1.zmm32u(i) & src2.zmm32u(i)) == 0 && ((write_mask >> i) & 1 != 0) {
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
            if (src1.zmm32u(i) & src2.zmm32u(i)) == 0 && ((write_mask >> i) & 1 != 0) {
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
            if (src1.zmm64u(i) & src2.zmm64u(i)) == 0 && ((write_mask >> i) & 1 != 0) {
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
            if (src1.zmm64u(i) & src2.zmm64u(i)) == 0 && ((write_mask >> i) & 1 != 0) {
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
        let mask = self.opmask_rrx(instr.src() as usize);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, if (mask >> i) & 1 != 0 { 0xFFFF_FFFF } else { 0 });
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
        let mask = self.opmask_rrx(instr.src() as usize);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, if (mask >> i) & 1 != 0 { u64::MAX } else { 0 });
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

    // ════════════════════════════════════════════════════════════════════
    // Byte and word compares that produce an opmask (AVX512_BW), plus the
    // qword VPCMP forms. Each has a single def entry — the destination is
    // the opmask, so there is no `_Kmask` twin — and every one names a
    // masked loader, so a masked-off element performs no memory access.
    //
    // At VL512 a byte compare fills all 64 bits of the result, which is why
    // nothing here applies Bochs's CUT_OPMASK: the cut would shift by 64.
    // ════════════════════════════════════════════════════════════════════

    /// The shared body of the byte-granular compares.
    /// `LOAD_MASK_VectorB` per ia_opcodes_evex.def.
    fn evex_cmp_to_opmask_b(
        &mut self,
        instr: &Instruction,
        pred: impl Fn(u8, u8) -> bool,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_mask_vector_b(instr)?
        };
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (write_mask >> i) & 1 != 0 && pred(src1.zmmubyte(i), src2.zmmubyte(i)) {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// The shared body of the word-granular compares.
    /// `LOAD_MASK_VectorW` per ia_opcodes_evex.def.
    fn evex_cmp_to_opmask_w(
        &mut self,
        instr: &Instruction,
        pred: impl Fn(u16, u16) -> bool,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_mask_vector_w(instr)?
        };
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (write_mask >> i) & 1 != 0 && pred(src1.zmm16u(i), src2.zmm16u(i)) {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPCMPEQB Kk{k}, Hdq, Wdq — EVEX.66.0F.WIG 74
    pub fn evex_vpcmpeqb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cmp_to_opmask_b(instr, |a, b| a == b)
    }

    /// VPCMPGTB Kk{k}, Hdq, Wdq — EVEX.66.0F.WIG 64 (signed)
    pub fn evex_vpcmpgtb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cmp_to_opmask_b(instr, |a, b| (a as i8) > (b as i8))
    }

    /// VPTESTMB Kk{k}, Hdq, Wdq — EVEX.66.0F38.W0 26
    pub fn evex_vptestmb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cmp_to_opmask_b(instr, |a, b| (a & b) != 0)
    }

    /// VPTESTNMB Kk{k}, Hdq, Wdq — EVEX.F3.0F38.W0 26
    pub fn evex_vptestnmb(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cmp_to_opmask_b(instr, |a, b| (a & b) == 0)
    }

    /// VPCMPEQW Kk{k}, Hdq, Wdq — EVEX.66.0F.WIG 75
    pub fn evex_vpcmpeqw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cmp_to_opmask_w(instr, |a, b| a == b)
    }

    /// VPCMPGTW Kk{k}, Hdq, Wdq — EVEX.66.0F.WIG 65 (signed)
    pub fn evex_vpcmpgtw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cmp_to_opmask_w(instr, |a, b| (a as i16) > (b as i16))
    }

    /// VPTESTMW Kk{k}, Hdq, Wdq — EVEX.66.0F38.W1 26
    pub fn evex_vptestmw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cmp_to_opmask_w(instr, |a, b| (a & b) != 0)
    }

    /// VPTESTNMW Kk{k}, Hdq, Wdq — EVEX.F3.0F38.W1 26
    pub fn evex_vptestnmw(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_cmp_to_opmask_w(instr, |a, b| (a & b) == 0)
    }

    /// VPCMPB Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 3F (signed)
    pub fn evex_vpcmpb(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm3 = instr.ib() & 0x07;
        self.evex_cmp_to_opmask_b(instr, move |a, b| {
            cmp_predicate(imm3, (a as i8).cmp(&(b as i8)))
        })
    }

    /// VPCMPUB Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 3E (unsigned)
    pub fn evex_vpcmpub(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm3 = instr.ib() & 0x07;
        self.evex_cmp_to_opmask_b(instr, move |a, b| cmp_predicate(imm3, a.cmp(&b)))
    }

    /// VPCMPW Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W1 3F (signed)
    pub fn evex_vpcmpw(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm3 = instr.ib() & 0x07;
        self.evex_cmp_to_opmask_w(instr, move |a, b| {
            cmp_predicate(imm3, (a as i16).cmp(&(b as i16)))
        })
    }

    /// VPCMPUW Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W1 3E (unsigned)
    pub fn evex_vpcmpuw(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm3 = instr.ib() & 0x07;
        self.evex_cmp_to_opmask_w(instr, move |a, b| cmp_predicate(imm3, a.cmp(&b)))
    }

    /// The shared body of VPCMPQ and VPCMPUQ. Unlike the byte and word
    /// forms these support embedded broadcast, so they name
    /// `LOAD_BROADCAST_MASK_VectorQ`.
    fn evex_cmp_to_opmask_q(
        &mut self,
        instr: &Instruction,
        pred: impl Fn(u64, u64) -> bool,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_broadcast_mask_vector_q(instr)?
        };
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (write_mask >> i) & 1 != 0 && pred(src1.zmm64u(i), src2.zmm64u(i)) {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPCMPQ Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W1 1F (signed)
    pub fn evex_vpcmpq(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm3 = instr.ib() & 0x07;
        self.evex_cmp_to_opmask_q(instr, move |a, b| {
            cmp_predicate(imm3, (a as i64).cmp(&(b as i64)))
        })
    }

    /// VPCMPUQ Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W1 1E (unsigned)
    pub fn evex_vpcmpuq(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm3 = instr.ib() & 0x07;
        self.evex_cmp_to_opmask_q(instr, move |a, b| cmp_predicate(imm3, a.cmp(&b)))
    }


    // ════════════════════════════════════════════════════════════════════
    // Byte and word opmask <-> vector conversions (AVX512_BW). All four are
    // register-only — their def entries name BxError as the load function —
    // and none of them takes a writemask.
    // ════════════════════════════════════════════════════════════════════

    /// VPMOVM2B Vdq, Kk — EVEX.F3.0F38.W0 28
    pub fn evex_vpmovm2b(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let mask = self.opmask_rrx(instr.src() as usize);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(i, if (mask >> i) & 1 != 0 { 0xFF } else { 0 });
        }
        write_zmm_masked_b_all(self, instr.dst(), &result, vl);
        Ok(())
    }

    /// VPMOVM2W Vdq, Kk — EVEX.F3.0F38.W1 28
    pub fn evex_vpmovm2w(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let mask = self.opmask_rrx(instr.src() as usize);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, if (mask >> i) & 1 != 0 { 0xFFFF } else { 0 });
        }
        write_zmm_masked_w_all(self, instr.dst(), &result, vl);
        Ok(())
    }

    /// VPMOVB2M Kk, Vdq — EVEX.F3.0F38.W0 29
    pub fn evex_vpmovb2m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src = read_zmm(self, instr.src());
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src.zmmubyte(i) >> 7) != 0 {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPMOVW2M Kk, Vdq — EVEX.F3.0F38.W1 29
    pub fn evex_vpmovw2m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src = read_zmm(self, instr.src());
        let mut result: u64 = 0;
        for i in 0..nelements {
            if (src.zmm16u(i) >> 15) != 0 {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════
    // VPBLENDMB/W — select per element between the two sources under the
    // opmask. Unlike ordinary writemasking, an unselected element takes
    // src1 rather than being merged or zeroed, so the write is unmasked.
    // ════════════════════════════════════════════════════════════════════

    /// VPBLENDMB Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 66
    pub fn evex_vpblendmb(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_mask_vector_b(instr)?
        };
        let mask = read_opmask_for_write(self, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(
                i,
                if (mask >> i) & 1 != 0 {
                    src2.zmmubyte(i)
                } else {
                    src1.zmmubyte(i)
                },
            );
        }
        write_zmm_masked_b_all(self, instr.dst(), &result, vl);
        Ok(())
    }

    /// VPBLENDMW Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 66
    pub fn evex_vpblendmw(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_mask_vector_w(instr)?
        };
        let mask = read_opmask_for_write(self, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(
                i,
                if (mask >> i) & 1 != 0 {
                    src2.zmm16u(i)
                } else {
                    src1.zmm16u(i)
                },
            );
        }
        write_zmm_masked_w_all(self, instr.dst(), &result, vl);
        Ok(())
    }

}


/// The eight VPCMP integer predicates selected by imm8[2:0], applied to an
/// already-computed ordering. Bochs avx512_cmp.cc uses the same table for
/// every width and signedness; only the comparison feeding it changes.
#[inline]
fn cmp_predicate(imm3: u8, ord: core::cmp::Ordering) -> bool {
    use core::cmp::Ordering::*;
    match imm3 {
        0 => ord == Equal,               // EQ
        1 => ord == Less,                // LT
        2 => ord != Greater,             // LE
        3 => false,                      // FALSE
        4 => ord != Equal,               // NEQ
        5 => ord != Less,                // NLT (GE)
        6 => ord == Greater,             // NLE (GT)
        _ => true,                       // TRUE
    }
}


/// Write every byte element up to VL and zero the rest. Used by the
/// instructions that produce a full vector regardless of the opmask
/// (VPMOVM2B, VPBLENDMB) — the mask has already been consumed as data.
fn write_zmm_masked_b_all<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    reg: u8,
    result: &BxPackedZmmRegister,
    vl: u8,
) {
    let nbytes = byte_elements(vl);
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..nbytes {
        dst.set_zmmubyte(i, result.zmmubyte(i));
    }
    for i in nbytes..64 {
        dst.set_zmmubyte(i, 0);
    }
}

/// Word-granular counterpart of [`write_zmm_masked_b_all`].
fn write_zmm_masked_w_all<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>(
    cpu: &mut BxCpuC<'_, I, T>,
    reg: u8,
    result: &BxPackedZmmRegister,
    vl: u8,
) {
    let nwords = word_elements(vl);
    let dst = &mut cpu.vmm[reg as usize];
    for i in 0..nwords {
        dst.set_zmm16u(i, result.zmm16u(i));
    }
    for i in nwords..32 {
        dst.set_zmm16u(i, 0);
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    //! Three things about the byte/word opmask family are worth pinning:
    //!
    //!   * at VL512 a byte compare fills all 64 bits of the destination
    //!     opmask, which is exactly the width where Bochs's CUT_OPMASK would
    //!     shift by 64 and which it therefore skips;
    //!   * the writemask gates which elements may set a result bit, so a
    //!     compare that is true everywhere still yields only the masked bits;
    //!   * VPBLENDM is *not* ordinary writemasking — an unselected element
    //!     takes src1 rather than being merged or zeroed, so it must write
    //!     the full vector.

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::{BxSegregs, Instruction};
    use rusty_box_decoder::opcode::Opcode;

    fn evex_reg(opcode: Opcode, vl: u8) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0); // dst (opmask or vector)
        i.set_src_reg(1, 1); // src1
        i.set_src_reg(2, 2); // src2
        i.set_opmask(0);
        i.set_vex(true);
        i.set_vl(vl);
        i.set_seg(BxSegregs::Ds);
        i.init(0, 0, 1, 1);
        i.assert_mod_c0();
        i
    }

    #[test]
    fn byte_compare_fills_all_64_opmask_bits_at_vl512() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        // Equal everywhere: every one of the 64 byte lanes must set its bit.
        for i in 0..64 {
            cpu.vmm[1].set_zmmubyte(i, 0x5A);
            cpu.vmm[2].set_zmmubyte(i, 0x5A);
        }
        cpu.execute_instruction(&evex_reg(Opcode::EvexVpcmpeqbKgqHdqWdq, 2))
            .unwrap();
        assert_eq!(cpu.opmask_rrx(0), u64::MAX, "all 64 lanes equal");

        // One lane differs -> exactly that bit clears.
        cpu.vmm[2].set_zmmubyte(63, 0x00);
        cpu.execute_instruction(&evex_reg(Opcode::EvexVpcmpeqbKgqHdqWdq, 2))
            .unwrap();
        assert_eq!(cpu.opmask_rrx(0), u64::MAX >> 1);
    }

    #[test]
    fn the_writemask_gates_which_bits_a_compare_may_set() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for i in 0..16 {
            cpu.vmm[1].set_zmmubyte(i, 1);
            cpu.vmm[2].set_zmmubyte(i, 1);
        }
        cpu.bx_write_opmask(1, 0b1010);
        let mut i = evex_reg(Opcode::EvexVpcmpeqbKgqHdqWdq, 0);
        i.set_opmask(1);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(
            cpu.opmask_rrx(0),
            0b1010,
            "equal everywhere, but only the writemasked lanes may report it"
        );
    }

    #[test]
    fn vptestm_and_vptestnm_are_complementary() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.vmm[1].set_zmmubyte(0, 0b1100);
        cpu.vmm[2].set_zmmubyte(0, 0b0011); // AND == 0
        cpu.vmm[1].set_zmmubyte(1, 0b1100);
        cpu.vmm[2].set_zmmubyte(1, 0b0100); // AND != 0

        cpu.execute_instruction(&evex_reg(Opcode::EvexVptestmbKgqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.opmask_rrx(0) & 0b11, 0b10);

        cpu.execute_instruction(&evex_reg(Opcode::EvexVptestnmbKgqHdqWdq, 0))
            .unwrap();
        assert_eq!(cpu.opmask_rrx(0) & 0b11, 0b01);
    }

    #[test]
    fn vpcmpb_predicates_cover_signed_and_unsigned_orderings() {
        // 0xFF is -1 signed but 255 unsigned, so the signed and unsigned
        // forms of the same predicate must disagree on it.
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.vmm[1].set_zmmubyte(0, 0xFF);
        cpu.vmm[2].set_zmmubyte(0, 0x01);

        let mut i = evex_reg(Opcode::EvexVpcmpbKgqHdqWdqIb, 0);
        i.set_iq(1); // LT
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.opmask_rrx(0) & 1, 1, "signed: -1 < 1");

        let mut i = evex_reg(Opcode::EvexVpcmpubKgqHdqWdqIb, 0);
        i.set_iq(1); // LT
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.opmask_rrx(0) & 1, 0, "unsigned: 255 is not < 1");

        // FALSE and TRUE ignore the operands entirely.
        let mut i = evex_reg(Opcode::EvexVpcmpbKgqHdqWdqIb, 0);
        i.set_iq(3);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.opmask_rrx(0), 0);
        let mut i = evex_reg(Opcode::EvexVpcmpbKgqHdqWdqIb, 0);
        i.set_iq(7);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.opmask_rrx(0), 0xFFFF, "VL128 has 16 byte lanes");
    }

    #[test]
    fn vpmovb2m_and_vpmovm2b_round_trip_through_the_sign_bits() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.vmm[1].set_zmmubyte(0, 0x80); // sign set
        cpu.vmm[1].set_zmmubyte(1, 0x7F); // sign clear
        cpu.vmm[1].set_zmmubyte(2, 0xFF); // sign set

        let mut i = evex_reg(Opcode::EvexVpmovb2mKgqWdq, 0);
        i.set_src_reg(1, 1); // single-source form reads src()
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.opmask_rrx(0) & 0b111, 0b101);

        cpu.bx_write_opmask(2, 0b101);
        let mut i = evex_reg(Opcode::EvexVpmovm2bVdqKeq, 0);
        i.set_src_reg(1, 2);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.vmm[0].zmmubyte(0), 0xFF);
        assert_eq!(cpu.vmm[0].zmmubyte(1), 0x00);
        assert_eq!(cpu.vmm[0].zmmubyte(2), 0xFF);
    }

    #[test]
    fn vpblendmb_takes_src1_where_the_mask_is_clear_rather_than_merging() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for i in 0..16 {
            cpu.vmm[0].set_zmmubyte(i, 0xAA); // poison: must not survive
            cpu.vmm[1].set_zmmubyte(i, 0x11); // src1
            cpu.vmm[2].set_zmmubyte(i, 0x22); // src2
        }
        cpu.bx_write_opmask(1, 0b0101);
        let mut i = evex_reg(Opcode::EvexVpblendmbVdqHdqWdq, 0);
        i.set_opmask(1);
        cpu.execute_instruction(&i).unwrap();

        assert_eq!(cpu.vmm[0].zmmubyte(0), 0x22, "mask set -> src2");
        assert_eq!(cpu.vmm[0].zmmubyte(1), 0x11, "mask clear -> src1, not merge");
        assert_eq!(cpu.vmm[0].zmmubyte(2), 0x22);
        assert_eq!(cpu.vmm[0].zmmubyte(3), 0x11);
        assert_eq!(cpu.vmm[0].zmmubyte(15), 0x11);
    }
}
