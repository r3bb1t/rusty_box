//! AVX-512 Foundation (AVX-512F) instruction handlers
//!
//! Implements core 512-bit vector operations with opmask support.
//! Handlers work for 128/256/512-bit via `get_vl()` (EVEX.L'L field).
//!
//! Mirrors Bochs `cpu/avx/avx512.cc`, `avx512_move.cc`, `avx512_pfp.cc`.

use super::avx512_load::cut_opmask_to;
use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedZmmRegister,
};
// Load-bearing in pure no-std builds (core f32/f64 lack these inherent
// methods there); redundant in unit graphs where std is linked, so the
// unused-import lint is allowed rather than losing the no-std resolution.
#[cfg(not(feature = "std"))]
#[allow(unused_imports)]
use crate::cpu::float::FloatExt;

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

/// Write ZMM register, zeroing upper bits beyond VL
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
        // else: merge masking — keep original value
    }
    // Zero upper elements beyond VL (EVEX always clears upper)
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
    // Zero upper elements beyond VL
    for i in nelements..8 {
        dst.set_zmm64u(i, 0);
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // VMOVDQU32/64 — Unaligned move (EVEX-encoded)
    // ========================================================================

    /// VMOVDQU32 Vdq{k}, Wdq — EVEX.0F.W0 6F (load, register form)
    pub fn evex_vmovdqu32_load_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU32 Vdq{k}, Mdq — EVEX.0F.W0 6F (load, memory form).
    /// Bochs avx512_move.cc VMOVUPS_MASK_VpsWpsM: only the elements the opmask
    /// selects are read, so a masked-off element on an unmapped page must not
    /// fault.
    pub fn evex_vmovdqu32_load_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let eaddr = self.resolve_addr(instr);
        let mask = read_opmask_for_write(self, instr);
        let mut src = BxPackedZmmRegister::default();
        self.avx_masked_load32(instr, eaddr, &mut src, mask)?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU32 Wdq{k}, Vdq — EVEX.0F.W0 7F (store, register form)
    pub fn evex_vmovdqu32_store_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDQU32 Mdq{k}, Vdq — EVEX.0F.W0 7F (store, memory form).
    /// Bochs avx512_move.cc VMOVUPS_MASK_WpsVpsM -> avx_masked_store32, which
    /// probes every active element before committing any, so a fault part way
    /// through cannot leave a partial store behind.
    pub fn evex_vmovdqu32_store_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        self.avx_masked_store32(instr, eaddr, &src, mask)
    }

    /// VMOVDQU64 — same as VMOVDQU32 but with qword masking granularity
    /// EVEX.0F.W1 6F (load), 7F (store)
    pub fn evex_vmovdqu64_load_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    /// Bochs avx512_move.cc VMOVUPD_MASK_VpdWpdM — masked qword load with
    /// fault suppression on the inactive lanes.
    pub fn evex_vmovdqu64_load_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let eaddr = self.resolve_addr(instr);
        let mask = read_opmask_for_write(self, instr);
        let mut src = BxPackedZmmRegister::default();
        self.avx_masked_load64(instr, eaddr, &mut src, mask)?;
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &src, mask, zmask, vl);
        Ok(())
    }

    pub fn evex_vmovdqu64_store_r(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_vmovdqu32_store_r(instr) // register form is identical
    }

    /// Bochs avx512_move.cc VMOVUPD_MASK_WpdVpdM -> avx_masked_store64.
    pub fn evex_vmovdqu64_store_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        self.avx_masked_store64(instr, eaddr, &src, mask)
    }

    // ========================================================================
    // VPADDD/Q — Packed integer add (EVEX-encoded)
    // ========================================================================

    /// VPADDD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 FE
    pub fn evex_vpaddd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, src1.zmm32u(i).wrapping_add(src2.zmm32u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPADDQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 D4
    pub fn evex_vpaddq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, src1.zmm64u(i).wrapping_add(src2.zmm64u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSUBD/Q — Packed integer subtract
    // ========================================================================

    /// VPSUBD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 FA
    pub fn evex_vpsubd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, src1.zmm32u(i).wrapping_sub(src2.zmm32u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSUBQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 FB
    pub fn evex_vpsubq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, src1.zmm64u(i).wrapping_sub(src2.zmm64u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPXORD/Q, VPORD/Q, VPANDD/Q, VPANDND/Q — Packed bitwise logical
    // ========================================================================

    /// VPXORD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 EF
    pub fn evex_vpxord(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, src1.zmm32u(i) ^ src2.zmm32u(i));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPXORQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 EF
    pub fn evex_vpxorq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, src1.zmm64u(i) ^ src2.zmm64u(i));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPORD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 EB
    pub fn evex_vpord(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, src1.zmm32u(i) | src2.zmm32u(i));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPANDD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 DB
    pub fn evex_vpandd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, src1.zmm32u(i) & src2.zmm32u(i));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPANDND Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 DF
    pub fn evex_vpandnd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, (!src1.zmm32u(i)) & src2.zmm32u(i));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // Qword-granularity bitwise ops.
    //
    // The bit pattern these produce is identical to their dword twins, but the
    // element width is what opmask bits and embedded broadcast are counted in:
    // VPORQ applies mask bit i to qword i and broadcasts a qword, where VPORD
    // applies it to dword i and broadcasts a dword. They therefore need their
    // own handlers rather than sharing the dword ones.
    // ========================================================================

    /// VPORQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 EB (also serves VORPD).
    pub(super) fn evex_vporq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, src1.zmm64u(i) | src2.zmm64u(i));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPANDQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 DB (also serves VANDPD).
    pub(super) fn evex_vpandq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, src1.zmm64u(i) & src2.zmm64u(i));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPANDNQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 DF (also serves VANDNPD).
    pub(super) fn evex_vpandnq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, (!src1.zmm64u(i)) & src2.zmm64u(i));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPBROADCASTD/Q — Broadcast scalar to all elements
    // ========================================================================

    /// VPBROADCASTD Vdq{k}, Wd — EVEX.66.0F38.W0 58
    pub fn evex_vpbroadcastd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let mask = read_opmask_for_write(self, instr);
        let scalar = if instr.mod_c0() {
            read_zmm(self, instr.src()).zmm32u(0)
        } else if (mask & cut_opmask_to(nelements)) != 0 {
            let laddr = self.resolve_addr(instr);
            self.v_read_dword(BxSegregs::from(instr.seg()), laddr)?
        } else {
            // Bochs avx512_broadcast.cc VPBROADCASTD_MASK_VdqWdM guards the
            // read on a non-empty opmask, so a fully masked-off broadcast
            // performs no memory access and cannot fault.
            0
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPBROADCASTQ Vdq{k}, Wq — EVEX.66.0F38.W1 59
    pub fn evex_vpbroadcastq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let mask = read_opmask_for_write(self, instr);
        let scalar = if instr.mod_c0() {
            read_zmm(self, instr.src()).zmm64u(0)
        } else if (mask & cut_opmask_to(nelements)) != 0 {
            let laddr = self.resolve_addr(instr);
            self.v_read_qword(BxSegregs::from(instr.seg()), laddr)?
        } else {
            // Bochs avx512_broadcast.cc VPBROADCASTQ_MASK_VdqWqM: no read at
            // all when the opmask selects nothing.
            0
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPBROADCASTD Vdq{k}, Gd — EVEX.66.0F38.W0 7C (broadcast from GPR)
    pub fn evex_vpbroadcastd_gpr(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let scalar = self.get_gpr32(instr.src() as usize);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPBROADCASTQ Vdq{k}, Gq — EVEX.66.0F38.W1 7C (broadcast from GPR)
    pub fn evex_vpbroadcastq_gpr(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let scalar = self.get_gpr64(instr.src() as usize);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPTERNLOGD/Q — Bitwise ternary logic (3-input truth table)
    // Most commonly used AVX-512F instruction — replaces AND/OR/XOR combos
    // ========================================================================

    /// VPTERNLOGD Vdq{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 25
    pub fn evex_vpternlogd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let dst_reg = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let imm8 = instr.ib() as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let a = dst_reg.zmm32u(i);
            let b = src1.zmm32u(i);
            let c = src2.zmm32u(i);
            // For each bit position, compute truth table lookup
            // bit_index = (a_bit << 2) | (b_bit << 1) | c_bit
            // result_bit = (imm8 >> bit_index) & 1
            let mut r = 0u32;
            for bit in 0..32 {
                let a_bit = (a >> bit) & 1;
                let b_bit = (b >> bit) & 1;
                let c_bit = (c >> bit) & 1;
                let idx = (a_bit << 2) | (b_bit << 1) | c_bit;
                r |= ((imm8 >> idx) & 1) << bit;
            }
            result.set_zmm32u(i, r);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPTERNLOGQ Vdq{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W1 25
    pub fn evex_vpternlogq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let dst_reg = read_zmm(self, instr.dst());
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let imm8 = instr.ib() as u64;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let a = dst_reg.zmm64u(i);
            let b = src1.zmm64u(i);
            let c = src2.zmm64u(i);
            let mut r = 0u64;
            for bit in 0..64 {
                let a_bit = (a >> bit) & 1;
                let b_bit = (b >> bit) & 1;
                let c_bit = (c >> bit) & 1;
                let idx = (a_bit << 2) | (b_bit << 1) | c_bit;
                r |= ((imm8 >> idx) & 1) << bit;
            }
            result.set_zmm64u(i, r);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSLLD/Q, VPSRLD/Q — Packed shift by immediate
    // ========================================================================

    /// VPSLLD Vdq{k}, Hdq, Ib — EVEX.66.0F.W0 72 /6
    pub fn evex_vpslld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(
                i,
                if count >= 32 {
                    0
                } else {
                    src.zmm32u(i) << count
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRLD Vdq{k}, Hdq, Ib — EVEX.66.0F.W0 72 /2
    pub fn evex_vpsrld_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(
                i,
                if count >= 32 {
                    0
                } else {
                    src.zmm32u(i) >> count
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRAD Vdq{k}, Hdq, Ib — EVEX.66.0F.W0 72 /4 (arithmetic right shift)
    pub fn evex_vpsrad_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(
                i,
                if count >= 32 {
                    ((src.zmm32u(i) as i32) >> 31) as u32
                } else {
                    ((src.zmm32u(i) as i32) >> count) as u32
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSLLQ Vdq{k}, Hdq, Ib — EVEX.66.0F.W1 73 /6
    pub fn evex_vpsllq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(
                i,
                if count >= 64 {
                    0
                } else {
                    src.zmm64u(i) << count
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRLQ Vdq{k}, Hdq, Ib — EVEX.66.0F.W1 73 /2
    pub fn evex_vpsrlq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(
                i,
                if count >= 64 {
                    0
                } else {
                    src.zmm64u(i) >> count
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRAQ Vdq{k}, Hdq, Ib — EVEX.66.0F.W1 72 /4 (arithmetic right shift qword)
    pub fn evex_vpsraq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = instr.ib() as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(
                i,
                if count >= 64 {
                    ((src.zmm64u(i) as i64) >> 63) as u64
                } else {
                    ((src.zmm64u(i) as i64) >> count) as u64
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VEXTRACTI32x4 / VINSERTI32x4 — Extract/Insert 128-bit lane
    // ========================================================================

    /// VEXTRACTI32x4 Wdq{k}, Vdq, Ib — EVEX.66.0F3A.W0 39
    pub fn evex_vextracti32x4(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = read_zmm(self, instr.src());
        let num_lanes = vl_bytes(vl) / 16; // 1/2/4 lanes
        let imm = (instr.ib() as usize) & (num_lanes - 1); // Bochs: imm & (len-1)
        let mut result = BxPackedZmmRegister::default();
        // Copy 128-bit lane
        result.set_zmm32u(0, src.zmm32u(imm * 4));
        result.set_zmm32u(1, src.zmm32u(imm * 4 + 1));
        result.set_zmm32u(2, src.zmm32u(imm * 4 + 2));
        result.set_zmm32u(3, src.zmm32u(imm * 4 + 3));
        if instr.mod_c0() {
            // Register form — write 128 bits, zero upper
            let mask = read_opmask_for_write(self, instr);
            let zmask = instr.is_zero_masking() != 0;
            write_zmm_masked(self, instr.dst(), &result, mask, zmask, 0); // VL=0 (128-bit)
        } else {
            // Memory form — write 16 bytes
            let laddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            let mask = read_opmask_for_write(self, instr);
            for i in 0..4u64 {
                if (mask >> i) & 1 != 0 {
                    let val = result.zmm32u(i as usize);
                    self.v_write_dword(seg, laddr + i * 4, val)?;
                }
            }
        }
        Ok(())
    }

    /// VINSERTI32x4 Vdq{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 38
    pub fn evex_vinserti32x4(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let num_lanes = vl_bytes(vl) / 16;
        let imm = (instr.ib() as usize) & (num_lanes - 1);
        // Start with src1 (the full vector)
        let mut result = read_zmm(self, instr.src1());
        // Read 128-bit insert value
        let insert = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Both def entries use LOADU_Wdq.
            self.evex_loadu_wdq(instr)?
        };
        // Insert 128-bit lane
        result.set_zmm32u(imm * 4, insert.zmm32u(0));
        result.set_zmm32u(imm * 4 + 1, insert.zmm32u(1));
        result.set_zmm32u(imm * 4 + 2, insert.zmm32u(2));
        result.set_zmm32u(imm * 4 + 3, insert.zmm32u(3));
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSHUFB — Packed shuffle bytes (EVEX)
    // ========================================================================

    /// VPSHUFB Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 00
    pub fn evex_vpshufb(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let bytes = vl_bytes(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Both def entries use LOAD_Vector.
            self.evex_load_vector(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        // Per-lane shuffle: each 128-bit lane independently
        let lanes = bytes / 16;
        for lane in 0..lanes {
            let base = lane * 16;
            for i in 0..16 {
                let ctrl = src2.zmmubyte(base + i);
                if ctrl & 0x80 != 0 {
                    result.set_zmmubyte(base + i, 0);
                } else {
                    let idx = (ctrl & 0x0F) as usize;
                    result.set_zmmubyte(base + i, src1.zmmubyte(base + idx));
                }
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        // Byte-granularity masking
        let nelements = bytes;
        let dst = &mut self.vmm[instr.dst() as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                dst.set_zmmubyte(i, result.zmmubyte(i));
            } else if zmask {
                dst.set_zmmubyte(i, 0);
            }
        }
        for i in nelements..64 {
            dst.set_zmmubyte(i, 0);
        }
        Ok(())
    }

    // ========================================================================
    // VPSHUFD — Shuffle packed dwords by immediate
    // ========================================================================

    /// VPSHUFD Vdq{k}, Wdq, Ib — EVEX.66.0F.W0 70
    pub fn evex_vpshufd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            // Both def entries use LOAD_BROADCAST_VectorD.
            self.evex_load_broadcast_vector_d(instr)?
        };
        let imm8 = instr.ib();
        let mut result = BxPackedZmmRegister::default();
        let lanes = nelements / 4;
        for lane in 0..lanes {
            let base = lane * 4;
            for j in 0..4 {
                let sel = ((imm8 >> (j * 2)) & 0x03) as usize;
                result.set_zmm32u(base + j, src.zmm32u(base + sel));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSLLD/Q, VPSRLD/Q by XMM count (shift by register)
    // ========================================================================

    /// VPSLLD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 F2 (shift left by XMM[63:0])
    pub fn evex_vpslld_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count64 = count_reg.zmm64u(0);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(
                i,
                if count64 >= 32 {
                    0
                } else {
                    src.zmm32u(i) << (count64 as u32)
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRLD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 D2 (shift right by XMM[63:0])
    pub fn evex_vpsrld_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count64 = count_reg.zmm64u(0);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(
                i,
                if count64 >= 32 {
                    0
                } else {
                    src.zmm32u(i) >> (count64 as u32)
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRAD Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 E2 (arithmetic shift right by XMM[63:0])
    pub fn evex_vpsrad_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count64 = count_reg.zmm64u(0);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(
                i,
                if count64 >= 32 {
                    ((src.zmm32u(i) as i32) >> 31) as u32
                } else {
                    ((src.zmm32u(i) as i32) >> (count64 as u32)) as u32
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSLLQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 F3
    pub fn evex_vpsllq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count64 = count_reg.zmm64u(0);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(
                i,
                if count64 >= 64 {
                    0
                } else {
                    src.zmm64u(i) << (count64 as u32)
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRLQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 D3
    pub fn evex_vpsrlq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count64 = count_reg.zmm64u(0);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(
                i,
                if count64 >= 64 {
                    0
                } else {
                    src.zmm64u(i) >> (count64 as u32)
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPSRAQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 E2 (arithmetic shift right qword)
    pub fn evex_vpsraq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src1());
        let count_reg = read_zmm(self, instr.src2());
        let count64 = count_reg.zmm64u(0);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(
                i,
                if count64 >= 64 {
                    ((src.zmm64u(i) as i64) >> 63) as u64
                } else {
                    ((src.zmm64u(i) as i64) >> (count64 as u32)) as u64
                },
            );
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPCMPD/Q — Packed compare producing opmask result
    // ========================================================================

    /// VPCMPD Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 1F
    pub fn evex_vpcmpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Single def entry: LOAD_BROADCAST_MASK_VectorD.
            self.evex_load_broadcast_mask_vector_d(instr)?
        };
        let imm3 = instr.ib() & 0x07;
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            let a = src1.zmm32u(i) as i32;
            let b = src2.zmm32u(i) as i32;
            let cmp = match imm3 {
                0 => a == b, // EQ
                1 => a < b,  // LT
                2 => a <= b, // LE
                3 => false,  // FALSE
                4 => a != b, // NEQ
                5 => a >= b, // NLT (GE)
                6 => a > b,  // NLE (GT)
                _ => true,   // TRUE
            };
            if cmp && ((write_mask >> i) & 1 != 0) {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPCMPUD Kk{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 1E (unsigned compare)
    pub fn evex_vpcmpud(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Single def entry: LOAD_BROADCAST_MASK_VectorD.
            self.evex_load_broadcast_mask_vector_d(instr)?
        };
        let imm3 = instr.ib() & 0x07;
        let write_mask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..nelements {
            let a = src1.zmm32u(i);
            let b = src2.zmm32u(i);
            let cmp = match imm3 {
                0 => a == b,
                1 => a < b,
                2 => a <= b,
                3 => false,
                4 => a != b,
                5 => a >= b,
                6 => a > b,
                _ => true,
            };
            if cmp && ((write_mask >> i) & 1 != 0) {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // VPMULLD — Packed multiply low dword
    // ========================================================================

    /// VPMULLD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 40
    pub fn evex_vpmulld(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, src1.zmm32u(i).wrapping_mul(src2.zmm32u(i)));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPMINSD/VPMAXSD — Packed min/max signed dword
    // ========================================================================

    /// VPMINSD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 39
    pub fn evex_vpminsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let a = src1.zmm32u(i) as i32;
            let b = src2.zmm32u(i) as i32;
            result.set_zmm32u(i, if a < b { a } else { b } as u32);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPMAXSD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 3D
    pub fn evex_vpmaxsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let a = src1.zmm32u(i) as i32;
            let b = src2.zmm32u(i) as i32;
            result.set_zmm32u(i, if a > b { a } else { b } as u32);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPROLD/VPRORD — Rotate left/right packed dwords by immediate
    // AVX-512F specific — no VEX equivalent (Bochs avx512.cc)
    // ========================================================================

    /// VPROLD Vdq{k}, Hdq, Ib — EVEX.66.0F.W0 72 /1
    pub fn evex_vprold_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = (instr.ib() & 0x1F) as u32; // modulo 32
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, src.zmm32u(i).rotate_left(count));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPRORD Vdq{k}, Hdq, Ib — EVEX.66.0F.W0 72 /0
    pub fn evex_vprord_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = (instr.ib() & 0x1F) as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, src.zmm32u(i).rotate_right(count));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPROLQ Vdq{k}, Hdq, Ib — EVEX.66.0F.W1 72 /1
    pub fn evex_vprolq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = (instr.ib() & 0x3F) as u32; // modulo 64
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, src.zmm64u(i).rotate_left(count));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPRORQ Vdq{k}, Hdq, Ib — EVEX.66.0F.W1 72 /0
    pub fn evex_vprorq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let count = (instr.ib() & 0x3F) as u32;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, src.zmm64u(i).rotate_right(count));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPERMD — Permute packed dwords (EVEX)
    // ========================================================================

    /// VPERMD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 36
    pub fn evex_vpermd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let idx = read_zmm(self, instr.src1());
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Both def entries use LOAD_BROADCAST_VectorD.
            self.evex_load_broadcast_vector_d(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let sel = (idx.zmm32u(i) & (nelements as u32 - 1)) as usize;
            result.set_zmm32u(i, src.zmm32u(sel));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPERMQ Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 36
    pub fn evex_vpermq_reg(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let idx = read_zmm(self, instr.src1());
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Both def entries use LOAD_BROADCAST_VectorQ — the masked form is
            // not fault-suppressing for this opcode.
            self.evex_load_broadcast_vector_q(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let sel = (idx.zmm64u(i) & (nelements as u64 - 1)) as usize;
            result.set_zmm64u(i, src.zmm64u(sel));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPERMQ Vdq{k}, Wdq, Ib — EVEX.66.0F3A.W1 00 (immediate form)
    pub fn evex_vpermq_imm(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            // Both def entries use LOAD_BROADCAST_VectorQ.
            self.evex_load_broadcast_vector_q(instr)?
        };
        let imm8 = instr.ib();
        let mut result = BxPackedZmmRegister::default();
        // Per 256-bit lane, select from 4 qwords using imm8
        let lanes = nelements / 4;
        for lane in 0..lanes.max(1) {
            let base = lane * 4;
            for j in 0..4.min(nelements) {
                let sel = ((imm8 >> (j * 2)) & 0x03) as usize;
                result.set_zmm64u(base + j, src.zmm64u(base + sel));
            }
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPUNPCKLDQ/VPUNPCKHDQ — Unpack and interleave dwords
    // ========================================================================

    /// VPUNPCKLDQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 62
    pub fn evex_vpunpckldq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Both def entries use LOAD_BROADCAST_VectorD.
            self.evex_load_broadcast_vector_d(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        let lanes = vl_bytes(vl) / 16;
        for lane in 0..lanes {
            let base = lane * 4;
            // Interleave low halves of each 128-bit lane
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

    /// VPUNPCKLQDQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 6C
    pub fn evex_vpunpcklqdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Both def entries use LOAD_BROADCAST_VectorQ.
            self.evex_load_broadcast_vector_q(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        let lanes = vl_bytes(vl) / 16;
        for lane in 0..lanes {
            let base = lane * 2;
            result.set_zmm64u(base, src1.zmm64u(base));
            result.set_zmm64u(base + 1, src2.zmm64u(base));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPUNPCKHQDQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 6D
    pub fn evex_vpunpckhqdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Both def entries use LOAD_BROADCAST_VectorQ.
            self.evex_load_broadcast_vector_q(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        let lanes = vl_bytes(vl) / 16;
        for lane in 0..lanes {
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
    // VPBLENDMD/Q — Blend packed dwords/qwords using opmask
    // ========================================================================

    /// VPBLENDMD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 64
    pub fn evex_vpblendmd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Single def entry: LOAD_BROADCAST_MASK_VectorD.
            self.evex_load_broadcast_mask_vector_d(instr)?
        };
        let mask = read_opmask_for_write(self, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(
                i,
                if (mask >> i) & 1 != 0 {
                    src2.zmm32u(i)
                } else {
                    src1.zmm32u(i)
                },
            );
        }
        for i in nelements..16 {
            result.set_zmm32u(i, 0);
        }
        self.vmm[instr.dst() as usize] = result;
        Ok(())
    }

    /// VPBLENDMQ Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 64
    pub fn evex_vpblendmq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // VPBLENDMQ has a single def entry, LOAD_BROADCAST_MASK_VectorQ.
            self.evex_load_broadcast_mask_vector_q(instr)?
        };
        let mask = read_opmask_for_write(self, instr);
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(
                i,
                if (mask >> i) & 1 != 0 {
                    src2.zmm64u(i)
                } else {
                    src1.zmm64u(i)
                },
            );
        }
        for i in nelements..8 {
            result.set_zmm64u(i, 0);
        }
        self.vmm[instr.dst() as usize] = result;
        Ok(())
    }

    // ========================================================================
    // VPABSD — Packed absolute value dword
    // ========================================================================

    /// VPABSD Vdq{k}, Wdq — EVEX.66.0F38.W0 1E
    pub fn evex_vpabsd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, (src.zmm32u(i) as i32).unsigned_abs());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPABSQ Vdq{k}, Wdq — EVEX.66.0F38.W1 1F
    pub fn evex_vpabsq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, (src.zmm64u(i) as i64).unsigned_abs());
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPSLLDQ/VPSRLDQ — Shift double quadword by immediate (byte shift)
    // ========================================================================

    /// VPSLLDQ Vdq, Hdq, Ib — EVEX.66.0F.W0 73 /7
    pub fn evex_vpslldq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let bytes = vl_bytes(vl);
        let src = read_zmm(self, instr.src());
        let shift = (instr.ib() as usize).min(16);
        let mut result = BxPackedZmmRegister::default();
        let lanes = bytes / 16;
        for lane in 0..lanes {
            let base = lane * 16;
            for i in 0..16 {
                if i >= shift {
                    result.set_zmmubyte(base + i, src.zmmubyte(base + i - shift));
                }
                // else: result stays 0 (shifted in zeros)
            }
        }
        // No opmask for VPSLLDQ/VPSRLDQ (Bochs: always unmasked)
        let dst = &mut self.vmm[instr.dst() as usize];
        for i in 0..bytes {
            dst.set_zmmubyte(i, result.zmmubyte(i));
        }
        for i in bytes..64 {
            dst.set_zmmubyte(i, 0);
        }
        Ok(())
    }

    /// VPSRLDQ Vdq, Hdq, Ib — EVEX.66.0F.W0 73 /3
    pub fn evex_vpsrldq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let bytes = vl_bytes(vl);
        let src = read_zmm(self, instr.src());
        let shift = (instr.ib() as usize).min(16);
        let mut result = BxPackedZmmRegister::default();
        let lanes = bytes / 16;
        for lane in 0..lanes {
            let base = lane * 16;
            for i in 0..16 {
                if i + shift < 16 {
                    result.set_zmmubyte(base + i, src.zmmubyte(base + i + shift));
                }
            }
        }
        let dst = &mut self.vmm[instr.dst() as usize];
        for i in 0..bytes {
            dst.set_zmmubyte(i, result.zmmubyte(i));
        }
        for i in bytes..64 {
            dst.set_zmmubyte(i, 0);
        }
        Ok(())
    }

    /// VPUNPCKHDQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W0 6A
    pub fn evex_vpunpckhdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let src1 = read_zmm(self, instr.src1());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Both def entries use LOAD_BROADCAST_VectorD.
            self.evex_load_broadcast_vector_d(instr)?
        };
        let mut result = BxPackedZmmRegister::default();
        let lanes = vl_bytes(vl) / 16;
        for lane in 0..lanes {
            let base = lane * 4;
            // Interleave high halves of each 128-bit lane
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
    // Variable shifts — VPSLLVD/Q, VPSRLVD/Q, VPSRAVD/Q
    // Per-element shift counts from src2 (Bochs avx512.cc)
    // ========================================================================

    /// VPSLLVD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 47
    pub fn evex_vpsllvd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            let c = s2.zmm32u(i);
            r.set_zmm32u(i, if c >= 32 { 0 } else { s1.zmm32u(i) << c });
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// VPSLLVQ Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 47
    pub fn evex_vpsllvq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            let c = s2.zmm64u(i);
            r.set_zmm64u(i, if c >= 64 { 0 } else { s1.zmm64u(i) << c });
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// VPSRLVD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 45
    pub fn evex_vpsrlvd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            let c = s2.zmm32u(i);
            r.set_zmm32u(i, if c >= 32 { 0 } else { s1.zmm32u(i) >> c });
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// VPSRLVQ Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 45
    pub fn evex_vpsrlvq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            let c = s2.zmm64u(i);
            r.set_zmm64u(i, if c >= 64 { 0 } else { s1.zmm64u(i) >> c });
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// VPSRAVD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 46
    pub fn evex_vpsravd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            let c = s2.zmm32u(i);
            r.set_zmm32u(
                i,
                if c >= 32 {
                    ((s1.zmm32u(i) as i32) >> 31) as u32
                } else {
                    ((s1.zmm32u(i) as i32) >> c) as u32
                },
            );
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// VPSRAVQ Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 46
    pub fn evex_vpsravq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            let c = s2.zmm64u(i);
            r.set_zmm64u(
                i,
                if c >= 64 {
                    ((s1.zmm64u(i) as i64) >> 63) as u64
                } else {
                    ((s1.zmm64u(i) as i64) >> c) as u64
                },
            );
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    // ========================================================================
    // Variable rotates — VPROLVD/Q, VPRORVD/Q
    // ========================================================================

    /// VPROLVD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 15
    pub fn evex_vprolvd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm32u(i, s1.zmm32u(i).rotate_left(s2.zmm32u(i) & 31));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// VPROLVQ Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 15
    pub fn evex_vprolvq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64u(i, s1.zmm64u(i).rotate_left((s2.zmm64u(i) & 63) as u32));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// VPRORVD Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W0 14
    pub fn evex_vprorvd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm32u(i, s1.zmm32u(i).rotate_right(s2.zmm32u(i) & 31));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// VPRORVQ Vdq{k}, Hdq, Wdq — EVEX.66.0F38.W1 14
    pub fn evex_vprorvq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64u(i, s1.zmm64u(i).rotate_right((s2.zmm64u(i) & 63) as u32));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    // ========================================================================
    // VPMULUDQ — Unsigned multiply packed dwords → qword results
    // ========================================================================

    /// VPMULUDQ Vdq{k}, Hdq, Wdq — EVEX.66.0F.W1 F4
    pub fn evex_vpmuludq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            // Multiply low 32 bits of each qword element
            let a = s1.zmm64u(i) & 0xFFFFFFFF;
            let b = s2.zmm64u(i) & 0xFFFFFFFF;
            r.set_zmm64u(i, a.wrapping_mul(b));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    // ========================================================================
    // VPALIGNR — Align right (EVEX, per 128-bit lane)
    // ========================================================================

    /// VPALIGNR Vdq{k}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 0F
    pub fn evex_vpalignr(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let bytes = vl_bytes(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Both def entries use LOAD_Vector.
            self.evex_load_vector(instr)?
        };
        let shift = instr.ib() as usize;
        let mut r = BxPackedZmmRegister::default();
        let lanes = bytes / 16;
        for lane in 0..lanes {
            let base = lane * 16;
            // Concatenate [src1:src2] as 32 bytes, shift right by imm8 bytes
            let mut concat = [0u8; 32];
            for (j, elem) in concat[..16].iter_mut().enumerate() {
                *elem = s2.zmmubyte(base + j);
            }
            for (j, elem) in concat[16..32].iter_mut().enumerate() {
                *elem = s1.zmmubyte(base + j);
            }
            for j in 0..16 {
                let idx = j + shift;
                r.set_zmmubyte(base + j, if idx < 32 { concat[idx] } else { 0 });
            }
        }
        // Byte-granularity masking
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        let dst = &mut self.vmm[instr.dst() as usize];
        for i in 0..bytes {
            if (mask >> i) & 1 != 0 {
                dst.set_zmmubyte(i, r.zmmubyte(i));
            } else if zmask {
                dst.set_zmmubyte(i, 0);
            }
        }
        for i in bytes..64 {
            dst.set_zmmubyte(i, 0);
        }
        Ok(())
    }

    // ========================================================================
    // VPMOVZXDQ/VPMOVSXDQ — Zero/Sign extend dwords to qwords
    // ========================================================================

    /// VPMOVZXDQ Vdq{k}, Wdq — EVEX.66.0F38.W0 35
    pub fn evex_vpmovzxdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl); // output qword count
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_half_vec_mask_d_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64u(i, src.zmm32u(i) as u64);
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// VPMOVSXDQ Vdq{k}, Wdq — EVEX.66.0F38.W0 25
    pub fn evex_vpmovsxdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_half_vec_mask_d_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64u(i, (src.zmm32u(i) as i32) as i64 as u64);
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    // ========================================================================
    // VPCMPEQD/VPCMPGTD — Compare equal/greater producing opmask
    // ========================================================================

    /// VPCMPEQD Kk{k}, Hdq, Wdq — EVEX.66.0F.W0 76
    pub fn evex_vpcmpeqd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Single def entry: LOAD_BROADCAST_MASK_VectorD.
            self.evex_load_broadcast_mask_vector_d(instr)?
        };
        let wmask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..ne {
            if s1.zmm32u(i) == s2.zmm32u(i) && ((wmask >> i) & 1 != 0) {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPCMPGTD Kk{k}, Hdq, Wdq — EVEX.66.0F.W0 66
    pub fn evex_vpcmpgtd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Single def entry: LOAD_BROADCAST_MASK_VectorD.
            self.evex_load_broadcast_mask_vector_d(instr)?
        };
        let wmask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..ne {
            if (s1.zmm32u(i) as i32) > (s2.zmm32u(i) as i32) && ((wmask >> i) & 1 != 0) {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPCMPEQQ Kk{k}, Hdq, Wdq — EVEX.66.0F.W1 29 (0F38 29 actually)
    pub fn evex_vpcmpeqq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Single def entry: LOAD_BROADCAST_MASK_VectorQ.
            self.evex_load_broadcast_mask_vector_q(instr)?
        };
        let wmask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..ne {
            if s1.zmm64u(i) == s2.zmm64u(i) && ((wmask >> i) & 1 != 0) {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    /// VPCMPGTQ Kk{k}, Hdq, Wdq — EVEX.66.0F38.W1 37
    pub fn evex_vpcmpgtq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src1());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src2())
        } else {
            // Single def entry: LOAD_BROADCAST_MASK_VectorQ.
            self.evex_load_broadcast_mask_vector_q(instr)?
        };
        let wmask = read_opmask_for_write(self, instr);
        let mut result: u64 = 0;
        for i in 0..ne {
            if (s1.zmm64u(i) as i64) > (s2.zmm64u(i) as i64) && ((wmask >> i) & 1 != 0) {
                result |= 1 << i;
            }
        }
        self.bx_write_opmask(instr.dst() as usize, result);
        Ok(())
    }

    // ========================================================================
    // Packed FP arithmetic (EVEX) — VADD/SUB/MUL/DIV/MAX/MIN/SQRT PS/PD
    // ========================================================================

    /// Helper: read rm operand (src1 in our convention = Intel's src2) as packed f32
    fn read_evex_rm_ps(
        &mut self,
        instr: &Instruction,
        _ne: usize,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src1()))
        } else {
            self.evex_load_bcst_d_pair(instr)
        }
    }
    fn read_evex_rm_pd(
        &mut self,
        instr: &Instruction,
        _ne: usize,
    ) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src1()))
        } else {
            self.evex_load_bcst_q_pair(instr)
        }
    }

    pub fn evex_vaddps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_ps(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm32f(i, s1.zmm32f(i) + s2.zmm32f(i));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vaddpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_pd(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64f(i, s1.zmm64f(i) + s2.zmm64f(i));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vsubps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_ps(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm32f(i, s1.zmm32f(i) - s2.zmm32f(i));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vsubpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_pd(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64f(i, s1.zmm64f(i) - s2.zmm64f(i));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vmulps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_ps(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm32f(i, s1.zmm32f(i) * s2.zmm32f(i));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vmulpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_pd(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64f(i, s1.zmm64f(i) * s2.zmm64f(i));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vdivps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_ps(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm32f(i, s1.zmm32f(i) / s2.zmm32f(i));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vdivpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_pd(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64f(i, s1.zmm64f(i) / s2.zmm64f(i));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    // x86 MAX: if either NaN, return src2
    fn x86_maxf32(a: f32, b: f32) -> f32 {
        if a.is_nan() || b.is_nan() {
            b
        } else if a > b {
            a
        } else {
            b
        }
    }
    fn x86_maxf64(a: f64, b: f64) -> f64 {
        if a.is_nan() || b.is_nan() {
            b
        } else if a > b {
            a
        } else {
            b
        }
    }
    fn x86_minf32(a: f32, b: f32) -> f32 {
        if a.is_nan() || b.is_nan() {
            b
        } else if a < b {
            a
        } else {
            b
        }
    }
    fn x86_minf64(a: f64, b: f64) -> f64 {
        if a.is_nan() || b.is_nan() {
            b
        } else if a < b {
            a
        } else {
            b
        }
    }

    pub fn evex_vmaxps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_ps(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm32f(i, Self::x86_maxf32(s1.zmm32f(i), s2.zmm32f(i)));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vmaxpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_pd(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64f(i, Self::x86_maxf64(s1.zmm64f(i), s2.zmm64f(i)));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vminps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_ps(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm32f(i, Self::x86_minf32(s1.zmm32f(i), s2.zmm32f(i)));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vminpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = self.read_evex_rm_pd(instr, ne)?; // s1=vvvv, s2=rm
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64f(i, Self::x86_minf64(s1.zmm64f(i), s2.zmm64f(i)));
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vsqrtps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm32f(i, src.zmm32f(i).sqrt());
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    pub fn evex_vsqrtpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            r.set_zmm64f(i, src.zmm64f(i).sqrt());
        }
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    //! The bitwise EVEX ops come in dword and qword flavours that produce the
    //! same bit pattern, which makes it tempting to serve both from one
    //! handler. The element width is still observable: opmask bits and
    //! embedded broadcast are counted in elements, so VPORQ must apply mask
    //! bit i to qword i where VPORD applies it to dword i. These tests pin
    //! that, because a wrong-width handler produces a correct-looking result
    //! everywhere except the masked lanes.

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::{BxSegregs, Instruction};
    use rusty_box_decoder::opcode::Opcode;

    /// Register-form EVEX.128 instruction: dst=0, src1=1, src2=2, mask k1,
    /// zero-masking on.
    fn evex_reg_form(opcode: Opcode) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0); // dst
        i.set_src_reg(1, 1); // src1
        i.set_src_reg(2, 2); // src2
        i.set_opmask(1);
        i.set_zero_masking(1);
        i.set_vex(true);
        i.set_vl(0); // 128-bit: 2 qwords / 4 dwords
        i.set_seg(BxSegregs::Ds);
        // `init` assigns `flags` wholesale, so the register-form bit has to be
        // asserted after it or it is silently wiped and the handler takes the
        // memory path instead.
        i.init(0, 0, 1, 1);
        i.assert_mod_c0();
        i
    }

    #[test]
    fn qword_bitwise_ops_mask_per_qword_not_per_dword() {
        // k1 = 0b01 leaves only element 0 active. Under qword granularity that
        // is the whole low 64 bits; under dword granularity it would be only
        // the low 32, zeroing the second dword and truncating the result.
        // The VxxxPD packed-FP aliases are qword-granular for the same reason
        // and were misrouted to the dword handlers alongside the integer forms.
        for (opcode, expect_lo) in [
            (Opcode::EvexVporqVdqHdqWdq, 0x1111_1111_2222_22FFu64),
            (Opcode::EvexVpandqVdqHdqWdq, 0x0000_0000_0000_0022u64),
            (Opcode::EvexVpxorqVdqHdqWdq, 0x1111_1111_2222_22DDu64),
            (Opcode::EvexVorpdVpdHpdWpd, 0x1111_1111_2222_22FFu64),
            (Opcode::EvexVandpdVpdHpdWpd, 0x0000_0000_0000_0022u64),
            (Opcode::EvexVxorpdVpdHpdWpd, 0x1111_1111_2222_22DDu64),
        ] {
            let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
            cpu.vmm[1].set_zmm64u(0, 0x1111_1111_2222_2222);
            cpu.vmm[1].set_zmm64u(1, 0x3333_3333_4444_4444);
            cpu.vmm[2].set_zmm64u(0, 0x0000_0000_0000_00FF);
            cpu.vmm[2].set_zmm64u(1, 0xFF00_0000_0000_0000);
            cpu.vmm[0].set_zmm64u(0, 0xDEAD_BEEF_DEAD_BEEF);
            cpu.vmm[0].set_zmm64u(1, 0xDEAD_BEEF_DEAD_BEEF);
            cpu.opmask[1].set_rrx(0b01);

            // Driven through `execute_instruction` rather than by calling the
            // handler directly: the defect this pins was a dispatcher arm
            // sending the qword opcode to the dword handler, which a direct
            // call would not have caught.
            let i = evex_reg_form(opcode);
            cpu.execute_instruction(&i).unwrap();

            assert_eq!(
                cpu.vmm[0].zmm64u(0),
                expect_lo,
                "{opcode:?}: active qword must be written whole; a dword-granular \
                 handler would zero the upper half"
            );
            assert_eq!(
                cpu.vmm[0].zmm64u(1),
                0,
                "{opcode:?}: masked-off qword must be zeroed under zero-masking"
            );
        }
    }

    #[test]
    fn vpandnq_negates_src1_at_qword_granularity() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.vmm[1].set_zmm64u(0, 0x0000_0000_0000_00F0);
        cpu.vmm[2].set_zmm64u(0, 0xFFFF_FFFF_FFFF_FFFF);
        cpu.vmm[0].set_zmm64u(0, 0xDEAD_BEEF_DEAD_BEEF);
        cpu.vmm[0].set_zmm64u(1, 0xDEAD_BEEF_DEAD_BEEF);
        cpu.opmask[1].set_rrx(0b01);

        let i = evex_reg_form(Opcode::EvexVpandnqVdqHdqWdq);
        cpu.execute_instruction(&i).unwrap();

        assert_eq!(
            cpu.vmm[0].zmm64u(0),
            0xFFFF_FFFF_FFFF_FF0F,
            "VPANDNQ is (!src1) & src2 across the full qword"
        );
        assert_eq!(cpu.vmm[0].zmm64u(1), 0, "masked-off qword zeroed");
    }
}
