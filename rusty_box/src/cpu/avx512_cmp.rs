#![allow(unused_unsafe, dead_code)]

//! AVX-512F comparison and miscellaneous handlers
//!
//! Implements VCMPPS, VCMPPD (floating-point compare to opmask),
//! VPTESTMD/MQ/NMD/NMQ (packed test to opmask),
//! VPMOVM2D/Q (expand opmask to vector),
//! VPMOVD2M/Q2M (compress sign bits to opmask).
//!
//! Mirrors Bochs `cpu/avx/avx512_cmp.cc`, `avx512_pfp.cc`.

use super::softfloat3e::softfloat::softfloat_getExceptionFlags;
use super::softfloat3e::softfloat_compare::{f32_compare_predicate, f64_compare_predicate};
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
        self.check_exceptions_sse(softfloat_getExceptionFlags(&status))?;
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
}
