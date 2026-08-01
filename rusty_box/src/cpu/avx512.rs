//! AVX-512 Foundation (AVX-512F) instruction handlers
//!
//! Implements core 512-bit vector operations with opmask support.
//! Handlers work for 128/256/512-bit via `get_vl()` (EVEX.L'L field).
//!
//! Mirrors Bochs `cpu/avx/avx512.cc`, `avx512_move.cc`, `avx512_pfp.cc`.

use super::avx512_load::cut_opmask_to;
use super::avx512_bw::write_zmm_masked_w;
use super::softfloat3e::f32_addsub::{f32_add, f32_sub};
use super::softfloat3e::f32_compare::{f32_max, f32_min};
use super::softfloat3e::f32_div::f32_div;
use super::softfloat3e::f32_mul::f32_mul;
use super::softfloat3e::f32_sqrt::f32_sqrt;
use super::softfloat3e::f64_addsub::{f64_add, f64_sub};
use super::softfloat3e::f64_compare::{f64_max, f64_min};
use super::softfloat3e::f64_div::f64_div;
use super::softfloat3e::f64_mul::f64_mul;
use super::softfloat3e::f64_sqrt::f64_sqrt;
use super::softfloat3e::softfloat::{softfloat_get_exception_flags, SoftFloatStatus};
use super::softfloat3e::softfloat_types::{Float32, Float64};
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

/// Width pairing of a VPMOV widening conversion, named after the mnemonic
/// suffix: `Bw` is byte-to-word, `Dq` dword-to-qword, and so on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PmovWiden {
    Bw,
    Bd,
    Bq,
    Wd,
    Wq,
    Dq,
}

/// Number of byte elements per vector length: VL0=16, VL1=32, VL2=64
#[inline]
fn byte_elements_bcast(vl: u8) -> usize {
    match vl {
        0 => 16,
        1 => 32,
        _ => 64,
    }
}

/// Number of 16-bit elements per vector length: VL0=8, VL1=16, VL2=32
#[inline]
fn word_elements_bcast(vl: u8) -> usize {
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
    /// The aligned vector moves (VMOVAPS/APD, VMOVDQA32/64, VMOVNT*) require
    /// the effective address to be aligned to the full vector width and raise
    /// #GP(0) otherwise. The unaligned forms (VMOVUPS/UPD, VMOVDQU*) do not.
    /// Bochs avx512_move.cc VMOVAPS_MASK_VpsWpsM.
    fn evex_check_vector_alignment(&mut self, instr: &Instruction, eaddr: u64) -> super::Result<()> {
        let len_in_bytes = match instr.get_vl() {
            0 => 16u64,
            1 => 32,
            _ => 64,
        };
        let seg = BxSegregs::from(instr.seg());
        let laddr = self.get_laddr64(seg as usize, eaddr);
        if laddr & (len_in_bytes - 1) != 0 {
            return self.exception(super::cpu::Exception::Gp, 0);
        }
        Ok(())
    }

    /// VMOVAPS/APD, VMOVDQA32/64 — memory load. Identical to the unaligned
    /// form except for the alignment requirement.
    pub fn evex_vmovaps_load_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        self.evex_check_vector_alignment(instr, eaddr)?;
        self.evex_vmovdqu32_load_m(instr)
    }

    /// VMOVAPS/APD, VMOVDQA32/64 — memory store.
    pub fn evex_vmovaps_store_m(&mut self, instr: &Instruction) -> super::Result<()> {
        let eaddr = self.resolve_addr(instr);
        self.evex_check_vector_alignment(instr, eaddr)?;
        self.evex_vmovdqu32_store_m(instr)
    }

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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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

    /// VPBROADCASTB Vdq{k}, Gb — EVEX.66.0F38.W0 7A (broadcast from GPR).
    /// Register-only: the def entry names BxError as its load function.
    pub fn evex_vpbroadcastb_gpr(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = byte_elements_bcast(vl);
        let scalar = self.get_gpr32(instr.src() as usize) as u8;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmmubyte(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        super::avx512_bw::write_zmm_masked_b(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPBROADCASTW Vdq{k}, Gw — EVEX.66.0F38.W0 7B (broadcast from GPR).
    pub fn evex_vpbroadcastw_gpr(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = word_elements_bcast(vl);
        let scalar = self.get_gpr32(instr.src() as usize) as u16;
        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm16u(i, scalar);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_w(self, instr.dst(), &result, mask, zmask, vl);
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
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
        let mut result = read_zmm(self, instr.src2());
        // Read 128-bit insert value
        let insert = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src = read_zmm(self, instr.src2());
        let count_reg = if instr.mod_c0() {
            read_zmm(self, instr.src1())
        } else {
            self.evex_loadu_wdq(instr)?
        };
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
        let src = read_zmm(self, instr.src2());
        let count_reg = if instr.mod_c0() {
            read_zmm(self, instr.src1())
        } else {
            self.evex_loadu_wdq(instr)?
        };
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
        let src = read_zmm(self, instr.src2());
        let count_reg = if instr.mod_c0() {
            read_zmm(self, instr.src1())
        } else {
            self.evex_loadu_wdq(instr)?
        };
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
        let src = read_zmm(self, instr.src2());
        let count_reg = if instr.mod_c0() {
            read_zmm(self, instr.src1())
        } else {
            self.evex_loadu_wdq(instr)?
        };
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
        let src = read_zmm(self, instr.src2());
        let count_reg = if instr.mod_c0() {
            read_zmm(self, instr.src1())
        } else {
            self.evex_loadu_wdq(instr)?
        };
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
        let src = read_zmm(self, instr.src2());
        let count_reg = if instr.mod_c0() {
            read_zmm(self, instr.src1())
        } else {
            self.evex_loadu_wdq(instr)?
        };
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
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
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_q_pair(instr)?
        };
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
        let idx = read_zmm(self, instr.src2());
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let idx = read_zmm(self, instr.src2());
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let src1 = read_zmm(self, instr.src2());
        let src2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
    // VPMOVSX / VPMOVZX — sign- or zero-extend to a wider element
    //
    // Bochs avx512_move.cc VPMOV{S,Z}X{BW,BD,BQ,WD,WQ,DQ}_MASK_VdqWdqR. All
    // twelve are the same loop differing only in the two widths and whether
    // the source element is sign- or zero-extended, so they share one
    // implementation. The source occupies a fraction of the destination width,
    // which is why each pair names a Half / Quarter / Eighth loader.
    // ========================================================================

    /// Source-to-destination width pairing of a VPMOV widening conversion,
    /// named after the mnemonic suffix.
    pub(super) fn evex_vpmov_widen(
        &mut self,
        instr: &Instruction,
        widen: PmovWiden,
        signed: bool,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            // Loader per the def entry for this width pairing.
            match widen {
                PmovWiden::Bw => self.evex_load_half_vec_mask_b_pair(instr)?,
                PmovWiden::Bd => self.evex_load_quarter_vec_mask_b_pair(instr)?,
                PmovWiden::Bq => self.evex_load_eighth_vec_mask_b_pair(instr)?,
                PmovWiden::Wd => self.evex_load_half_vec_mask_w_pair(instr)?,
                PmovWiden::Wq => self.evex_load_quarter_vec_mask_w_pair(instr)?,
                PmovWiden::Dq => self.evex_load_half_vec_mask_d_pair(instr)?,
            }
        };

        let mut r = BxPackedZmmRegister::default();
        let m = read_opmask_for_write(self, instr);
        let z = instr.is_zero_masking() != 0;

        // Read the narrow element, widen it, and write at the destination
        // width. The element count follows the destination.
        match widen {
            PmovWiden::Bw => {
                let ne = vl_bytes(vl) / 2; // word elements
                for i in 0..ne {
                    let v = src.zmmubyte(i);
                    r.set_zmm16u(i, if signed { v as i8 as i16 as u16 } else { v as u16 });
                }
                write_zmm_masked_w(self, instr.dst(), &r, m, z, vl);
            }
            PmovWiden::Bd => {
                let ne = dword_elements(vl);
                for i in 0..ne {
                    let v = src.zmmubyte(i);
                    r.set_zmm32u(i, if signed { v as i8 as i32 as u32 } else { v as u32 });
                }
                write_zmm_masked(self, instr.dst(), &r, m, z, vl);
            }
            PmovWiden::Bq => {
                let ne = qword_elements(vl);
                for i in 0..ne {
                    let v = src.zmmubyte(i);
                    r.set_zmm64u(i, if signed { v as i8 as i64 as u64 } else { v as u64 });
                }
                write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
            }
            PmovWiden::Wd => {
                let ne = dword_elements(vl);
                for i in 0..ne {
                    let v = src.zmm16u(i);
                    r.set_zmm32u(i, if signed { v as i16 as i32 as u32 } else { v as u32 });
                }
                write_zmm_masked(self, instr.dst(), &r, m, z, vl);
            }
            PmovWiden::Wq => {
                let ne = qword_elements(vl);
                for i in 0..ne {
                    let v = src.zmm16u(i);
                    r.set_zmm64u(i, if signed { v as i16 as i64 as u64 } else { v as u64 });
                }
                write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
            }
            PmovWiden::Dq => {
                let ne = qword_elements(vl);
                for i in 0..ne {
                    let v = src.zmm32u(i);
                    r.set_zmm64u(i, if signed { v as i32 as i64 as u64 } else { v as u64 });
                }
                write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
            }
        }
        Ok(())
    }

    // ========================================================================
    // VPCMPEQD/VPCMPGTD — Compare equal/greater producing opmask
    // ========================================================================

    /// VPCMPEQD Kk{k}, Hdq, Wdq — EVEX.66.0F.W0 76
    pub fn evex_vpcmpeqd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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
        let s1 = read_zmm(self, instr.src2());
        let s2 = if instr.mod_c0() {
            read_zmm(self, instr.src1())
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

    /// Two-operand packed EVEX FP over VL, single-precision.
    /// Bochs cpu_templates_pfp.h `HANDLE_AVX512_PFP_2OP`: the `_mask`
    /// primitives skip masked-off elements entirely, so those raise no
    /// exception; `check_exceptionsSSE` then runs before the destination
    /// write, and the embedded rounding control (EVEX.b on a register
    /// operand at VL512) overrides MXCSR.RC.
    fn evex_pfp_2op_ps(
        &mut self,
        instr: &Instruction,
        func: fn(Float32, Float32, &mut SoftFloatStatus) -> Float32,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let s1 = read_zmm(self, instr.src2()); // vvvv
        let s2 = self.read_evex_rm_ps(instr, ne)?; // rm
        let m = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            if (m >> i) & 1 != 0 {
                r.set_zmm32u(i, func(s1.zmm32u(i), s2.zmm32u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    /// Two-operand packed EVEX FP over VL, double-precision.
    fn evex_pfp_2op_pd(
        &mut self,
        instr: &Instruction,
        func: fn(Float64, Float64, &mut SoftFloatStatus) -> Float64,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = qword_elements(vl);
        let s1 = read_zmm(self, instr.src2()); // vvvv
        let s2 = self.read_evex_rm_pd(instr, ne)?; // rm
        let m = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            if (m >> i) & 1 != 0 {
                r.set_zmm64u(i, func(s1.zmm64u(i), s2.zmm64u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }

    pub fn evex_vaddps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_ps(instr, f32_add)
    }
    pub fn evex_vaddpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_pd(instr, f64_add)
    }
    pub fn evex_vsubps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_ps(instr, f32_sub)
    }
    pub fn evex_vsubpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_pd(instr, f64_sub)
    }
    pub fn evex_vmulps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_ps(instr, f32_mul)
    }
    pub fn evex_vmulpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_pd(instr, f64_mul)
    }
    pub fn evex_vdivps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_ps(instr, f32_div)
    }
    pub fn evex_vdivpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_pd(instr, f64_div)
    }
    pub fn evex_vmaxps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_ps(instr, f32_max)
    }
    pub fn evex_vmaxpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_pd(instr, f64_max)
    }
    pub fn evex_vminps(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_ps(instr, f32_min)
    }
    pub fn evex_vminpd(&mut self, instr: &Instruction) -> super::Result<()> {
        self.evex_pfp_2op_pd(instr, f64_min)
    }

    pub fn evex_vsqrtps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let ne = dword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_bcst_d_pair(instr)?
        };
        let m = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            if (m >> i) & 1 != 0 {
                r.set_zmm32u(i, f32_sqrt(src.zmm32u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
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
        let m = read_opmask_for_write(self, instr);
        let mut status = self.sse_status();
        self.softfloat_rc_override(&mut status, instr);
        let mut r = BxPackedZmmRegister::default();
        for i in 0..ne {
            if (m >> i) & 1 != 0 {
                r.set_zmm64u(i, f64_sqrt(src.zmm64u(i), &mut status));
            }
        }
        self.check_exceptions_sse(softfloat_get_exception_flags(&status))?;
        let z = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &r, m, z, vl);
        Ok(())
    }
    // ========================================================================
    // Element duplication, dword/qword rotate-align, and the FP blends.
    // Bochs avx512.cc VMOVSLDUP/VMOVSHDUP/VMOVDDUP, VALIGND/Q, VBLENDMPS/PD.
    // ========================================================================

    /// Read a whole vector from the r/m operand. All the callers below pair
    /// `LOAD_Vector` with itself, so there is no masked variant to choose.
    fn read_rm_vector(&mut self, instr: &Instruction) -> super::Result<BxPackedZmmRegister> {
        if instr.mod_c0() {
            Ok(read_zmm(self, instr.src()))
        } else {
            self.evex_load_vector(instr)
        }
    }

    /// VMOVSLDUP Vps{k}{z}, Wps — EVEX.F3.0F.W0 12. Each even dword is copied
    /// over the odd one above it.
    pub fn evex_vmovsldup(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let mut op = self.read_rm_vector(instr)?;
        for n in (0..dword_elements(vl)).step_by(2) {
            op.set_zmm32u(n + 1, op.zmm32u(n));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &op, mask, zmask, vl);
        Ok(())
    }

    /// VMOVSHDUP Vps{k}{z}, Wps — EVEX.F3.0F.W0 16. The odd dword is copied
    /// down over the even one below it.
    pub fn evex_vmovshdup(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let mut op = self.read_rm_vector(instr)?;
        for n in (0..dword_elements(vl)).step_by(2) {
            op.set_zmm32u(n, op.zmm32u(n + 1));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &op, mask, zmask, vl);
        Ok(())
    }

    /// VMOVDDUP Vpd{k}{z}, Wpd — EVEX.F2.0F.W1 12. Qword granularity.
    pub fn evex_vmovddup(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let mut op = self.read_rm_vector(instr)?;
        for n in (0..qword_elements(vl)).step_by(2) {
            op.set_zmm64u(n + 1, op.zmm64u(n));
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &op, mask, zmask, vl);
        Ok(())
    }

    /// VALIGND Vdq{k}{z}, Hdq, Wdq, Ib — EVEX.66.0F3A.W0 03. Concatenates
    /// vvvv:rm and extracts a dword-granular window starting at imm8.
    pub fn evex_valignd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let elements_mask = dword_elements(vl) - 1;
        let op1 = read_zmm(self, instr.src2()); // vvvv — the high half
        let op2 = self.read_evex_rm_ps(instr, dword_elements(vl))?; // rm — the low half
        let shift = (instr.ib() as usize) & elements_mask;
        let mut result = BxPackedZmmRegister::default();
        for n in 0..=elements_mask {
            let index = (shift + n) & elements_mask;
            let v = if (n + shift) <= elements_mask {
                op2.zmm32u(index)
            } else {
                op1.zmm32u(index)
            };
            result.set_zmm32u(n, v);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VALIGNQ Vdq{k}{z}, Hdq, Wdq, Ib — EVEX.66.0F3A.W1 03.
    pub fn evex_valignq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let elements_mask = qword_elements(vl) - 1;
        let op1 = read_zmm(self, instr.src2());
        let op2 = self.read_evex_rm_pd(instr, qword_elements(vl))?;
        let shift = (instr.ib() as usize) & elements_mask;
        let mut result = BxPackedZmmRegister::default();
        for n in 0..=elements_mask {
            let index = (shift + n) & elements_mask;
            let v = if (n + shift) <= elements_mask {
                op2.zmm64u(index)
            } else {
                op1.zmm64u(index)
            };
            result.set_zmm64u(n, v);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
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
        cpu.vmm[2].set_zmm64u(0, 0x0000_0000_0000_00F0);
        cpu.vmm[1].set_zmm64u(0, 0xFFFF_FFFF_FFFF_FFFF);
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

    // ---- operand order -------------------------------------------------
    //
    // Bochs numbers a 3-operand vector op as i->src1() = EVEX.vvvv and
    // i->src2() = ModRM.rm. This decoder is the other way round, so a handler
    // translated positionally from upstream reads its sources reversed. Every
    // case below is asymmetric, so it fails if the two are exchanged.
    //
    // vmm[2] is EVEX.vvvv (Bochs op1); vmm[1] is ModRM.rm (Bochs op2).

    /// Same shape as `evex_reg_form` but unmasked, so the assertions are about
    /// operand order alone rather than about masking.
    fn evex_unmasked(opcode: Opcode) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0);
        i.set_src_reg(1, 1); // ModRM.rm
        i.set_src_reg(2, 2); // EVEX.vvvv
        i.set_opmask(0);
        i.set_vex(true);
        i.set_vl(0);
        i.set_seg(BxSegregs::Ds);
        i.init(0, 0, 1, 1);
        i.assert_mod_c0();
        i
    }

    #[test]
    fn operand_order_vpsub_subtracts_rm_from_vvvv() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.vmm[2].set_zmm32u(0, 10); // vvvv
        cpu.vmm[1].set_zmm32u(0, 3); // rm
        cpu.execute_instruction(&evex_unmasked(Opcode::EvexVpsubdVdqHdqWdq))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 7, "vvvv - rm, not rm - vvvv");

        cpu.vmm[2].set_zmm64u(0, 10);
        cpu.vmm[1].set_zmm64u(0, 3);
        cpu.execute_instruction(&evex_unmasked(Opcode::EvexVpsubqVdqHdqWdq))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm64u(0), 7);
    }

    #[test]
    fn operand_order_vpandn_negates_vvvv() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.vmm[2].set_zmm32u(0, 0x0000_00F0); // vvvv — the negated side
        cpu.vmm[1].set_zmm32u(0, 0xFFFF_FFFF); // rm
        cpu.execute_instruction(&evex_unmasked(Opcode::EvexVpandndVdqHdqWdq))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 0xFFFF_FF0F, "(!vvvv) & rm");
    }

    #[test]
    fn operand_order_vpcmpgtd_compares_vvvv_against_rm() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.vmm[2].set_zmm32u(0, 5); // vvvv
        cpu.vmm[1].set_zmm32u(0, 3); // rm
        cpu.vmm[2].set_zmm32u(1, 3);
        cpu.vmm[1].set_zmm32u(1, 5);
        cpu.execute_instruction(&evex_unmasked(Opcode::EvexVpcmpgtdKgwHdqWdq))
            .unwrap();
        assert_eq!(
            cpu.opmask[0].rrx() & 0b11,
            0b01,
            "element 0 has vvvv > rm, element 1 does not"
        );
    }

    #[test]
    fn operand_order_vpshufb_takes_its_control_bytes_from_rm() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        // vvvv is the data, rm is the shuffle control.
        for i in 0..16 {
            cpu.vmm[2].set_zmmubyte(i, (0xA0 + i) as u8);
            cpu.vmm[1].set_zmmubyte(i, 0); // every lane selects data byte 0
        }
        cpu.execute_instruction(&evex_unmasked(Opcode::EvexVpshufbVdqHdqWdq))
            .unwrap();
        for i in 0..16 {
            assert_eq!(
                cpu.vmm[0].zmmubyte(i),
                0xA0,
                "control from rm selecting data[0] from vvvv"
            );
        }
    }

    #[test]
    fn operand_order_vpblendm_takes_vvvv_where_the_mask_is_clear() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for i in 0..4 {
            cpu.vmm[2].set_zmm32u(i, 0x1111_1111); // vvvv
            cpu.vmm[1].set_zmm32u(i, 0x2222_2222); // rm
        }
        cpu.bx_write_opmask(1, 0b0101);
        let mut i = evex_unmasked(Opcode::EvexVpblendmdVdqHdqWdq);
        i.set_opmask(1);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 0x2222_2222, "set -> rm");
        assert_eq!(cpu.vmm[0].zmm32u(1), 0x1111_1111, "clear -> vvvv");
        assert_eq!(cpu.vmm[0].zmm32u(2), 0x2222_2222);
        assert_eq!(cpu.vmm[0].zmm32u(3), 0x1111_1111);
    }

    #[test]
    fn operand_order_variable_shift_counts_come_from_rm() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.vmm[2].set_zmm32u(0, 0x0000_0100); // vvvv: the value
        cpu.vmm[2].set_zmm32u(1, 0x0000_0100);
        cpu.vmm[1].set_zmm32u(0, 4); // rm: per-element counts
        cpu.vmm[1].set_zmm32u(1, 8);
        cpu.execute_instruction(&evex_unmasked(Opcode::EvexVpsrlvdVdqHdqWdq))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 0x10);
        assert_eq!(cpu.vmm[0].zmm32u(1), 0x1);
    }

    #[test]
    fn operand_order_vinserti32x4_inserts_rm_into_vvvv() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for i in 0..8 {
            cpu.vmm[2].set_zmm32u(i, 0x1111_1111); // vvvv: the base vector
        }
        for i in 0..4 {
            cpu.vmm[1].set_zmm32u(i, 0x2222_2222); // rm: the inserted lane
        }
        let mut i = evex_unmasked(Opcode::EvexVinserti32x4VdqHdqWdqIb);
        // The immediate, VL and the opmask index share one u32 — imm8 is byte
        // 0, VL is byte 1, the opmask is byte 3 — so `set_iq` wipes the other
        // two and has to come first.
        i.set_iq(1); // insert into the upper lane
        i.set_vl(1); // 256-bit: two 128-bit lanes
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.vmm[0].zmm32u(0), 0x1111_1111, "lane 0 keeps vvvv");
        assert_eq!(cpu.vmm[0].zmm32u(4), 0x2222_2222, "lane 1 takes rm");
    }

    #[test]
    fn operand_order_kandnw_negates_vvvv() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        cpu.bx_write_opmask(2, 0x00F0); // vvvv
        cpu.bx_write_opmask(1, 0xFFFF); // rm
        let mut i = evex_unmasked(Opcode::KandnwKgwKhwKew);
        i.set_opmask(0);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(cpu.opmask[0].rrx(), 0xFF0F, "(!vvvv) & rm");
    }


    // ---- duplication and align ------------------------------------------

    #[test]
    fn duplication_moves_copy_in_the_direction_the_opcode_names() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for (n, v) in [10u32, 11, 12, 13].into_iter().enumerate() {
            cpu.vmm[1].set_zmm32u(n, v); // one-operand form reads src() = src1()
        }
        cpu.execute_instruction(&evex_unmasked(Opcode::EvexVmovsldupVpsWps))
            .unwrap();
        assert_eq!(
            [0, 1, 2, 3].map(|n| cpu.vmm[0].zmm32u(n)),
            [10, 10, 12, 12],
            "SLDUP copies each even dword up"
        );

        cpu.execute_instruction(&evex_unmasked(Opcode::EvexVmovshdupVpsWps))
            .unwrap();
        assert_eq!(
            [0, 1, 2, 3].map(|n| cpu.vmm[0].zmm32u(n)),
            [11, 11, 13, 13],
            "SHDUP copies each odd dword down"
        );

        cpu.vmm[1].set_zmm64u(0, 0xAAAA);
        cpu.vmm[1].set_zmm64u(1, 0xBBBB);
        cpu.execute_instruction(&evex_unmasked(Opcode::EvexVmovddupVpdWpd))
            .unwrap();
        assert_eq!(cpu.vmm[0].zmm64u(0), 0xAAAA);
        assert_eq!(cpu.vmm[0].zmm64u(1), 0xAAAA, "DDUP duplicates the even qword");
    }

    #[test]
    fn valignd_concatenates_vvvv_above_rm_and_windows_from_the_bottom() {
        let mut cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        for (n, v) in [20u32, 21, 22, 23].into_iter().enumerate() {
            cpu.vmm[2].set_zmm32u(n, v); // vvvv — the high half
        }
        for (n, v) in [10u32, 11, 12, 13].into_iter().enumerate() {
            cpu.vmm[1].set_zmm32u(n, v); // rm — the low half
        }
        let mut i = evex_unmasked(Opcode::EvexValigndVdqHdqWdqIbKmask);
        i.set_iq(1); // must precede set_vl/set_opmask: they share one u32
        cpu.execute_instruction(&i).unwrap();
        assert_eq!(
            [0, 1, 2, 3].map(|n| cpu.vmm[0].zmm32u(n)),
            [11, 12, 13, 20],
            "a shift of one dword pulls the lowest element of vvvv in at the top"
        );

        // A shift of zero is the r/m operand unchanged.
        let mut i = evex_unmasked(Opcode::EvexValigndVdqHdqWdqIbKmask);
        i.set_iq(0);
        cpu.execute_instruction(&i).unwrap();
        assert_eq!([0, 1, 2, 3].map(|n| cpu.vmm[0].zmm32u(n)), [10, 11, 12, 13]);
    }

}
