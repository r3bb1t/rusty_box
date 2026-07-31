//! AVX-512F compress, expand, and miscellaneous instruction handlers
//!
//! Implements VPCOMPRESSD/Q, VPEXPANDD/Q, VPMOVDB, VPMOVDW, VPMOVQD
//! (register forms), VPCONFLICTD (AVX-512CD), VPLZCNTD/Q (AVX-512CD).
//!
//! Note: VPMOVD2M, VPMOVQ2M, VPMOVM2D, VPMOVM2Q live in avx512_cmp.rs.
//!
//! Mirrors Bochs `cpu/avx/avx512.cc`, `avx512_move.cc`, `avx512_conflict.cc`.

use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, decoder::Instruction, xmm::BxPackedZmmRegister};

// ============================================================================
// Helper functions (duplicated from avx512.rs — module-private there)
// ============================================================================

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

/// Number of 16-bit elements per vector length: VL0=8, VL1=16, VL2=32
#[inline]
fn word_elements(vl: u8) -> usize {
    match vl {
        0 => 8,
        1 => 16,
        _ => 32,
    }
}

/// Source element width of a VPMOV narrowing conversion. The element *count*
/// follows this, since each source element produces one destination element.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PmovSrc {
    Word,
    Dword,
    Qword,
}

/// Destination element width of a VPMOV narrowing conversion.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PmovDst {
    Byte,
    Word,
    Dword,
}

/// How a source element too wide for the destination is reduced.
/// Bochs xmm.h Saturate<Src><S|U>To<Dst><S|U>.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PmovSat {
    /// VPMOV{QB,QW,QD,DB,DW,WB} — discard the high bits.
    Truncate,
    /// VPMOVS* — read the source signed, clamp to the destination's signed range.
    Signed,
    /// VPMOVUS* — read the source unsigned, clamp to the destination's max.
    Unsigned,
}

/// Reduce one element to `dst` width under `sat`. Returns the raw destination
/// bits, so a signed clamp lands as the two's-complement pattern of that width.
#[inline]
fn pmov_convert(raw: u64, src: PmovSrc, dst: PmovDst, sat: PmovSat) -> u64 {
    match sat {
        PmovSat::Truncate => raw,
        PmovSat::Unsigned => {
            let max = match dst {
                PmovDst::Byte => u8::MAX as u64,
                PmovDst::Word => u16::MAX as u64,
                PmovDst::Dword => u32::MAX as u64,
            };
            raw.min(max)
        }
        PmovSat::Signed => {
            // Sign-extend from the source width before comparing.
            let signed = match src {
                PmovSrc::Word => raw as u16 as i16 as i64,
                PmovSrc::Dword => raw as u32 as i32 as i64,
                PmovSrc::Qword => raw as i64,
            };
            let (lo, hi) = match dst {
                PmovDst::Byte => (i8::MIN as i64, i8::MAX as i64),
                PmovDst::Word => (i16::MIN as i64, i16::MAX as i64),
                PmovDst::Dword => (i32::MIN as i64, i32::MAX as i64),
            };
            signed.clamp(lo, hi) as u64
        }
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

/// Write ZMM register with dword-granularity masking, zeroing upper bits beyond VL
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

/// Write ZMM register with qword-granularity masking
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

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    // ========================================================================
    // VPCOMPRESSD — Compress packed dwords (EVEX.66.0F38.W0 8B)
    // ========================================================================

    /// VPCOMPRESSD Vdq{k}, Wdq — register form
    /// For each bit set in opmask, store the corresponding source dword
    /// contiguously in the destination. Mask-0 elements are skipped.
    pub fn evex_vpcompressd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = BxPackedZmmRegister::default();
        let mut k = 0usize; // output index

        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(k, src.zmm32u(i));
                k += 1;
            }
        }
        // Remaining elements: zero if zero-masking, else merge from dest
        let dst = &mut self.vmm[instr.dst() as usize];
        for i in 0..k {
            dst.set_zmm32u(i, result.zmm32u(i));
        }
        for i in k..nelements {
            if zmask {
                dst.set_zmm32u(i, 0);
            }
            // else: merge — keep original value
        }
        // Zero upper elements beyond VL
        for i in nelements..16 {
            dst.set_zmm32u(i, 0);
        }
        Ok(())
    }

    // ========================================================================
    // VPCOMPRESSQ — Compress packed qwords (EVEX.66.0F38.W1 8B)
    // ========================================================================

    /// VPCOMPRESSQ Vdq{k}, Wdq — register form
    pub fn evex_vpcompressq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = BxPackedZmmRegister::default();
        let mut k = 0usize;

        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm64u(k, src.zmm64u(i));
                k += 1;
            }
        }
        let dst = &mut self.vmm[instr.dst() as usize];
        for i in 0..k {
            dst.set_zmm64u(i, result.zmm64u(i));
        }
        for i in k..nelements {
            if zmask {
                dst.set_zmm64u(i, 0);
            }
        }
        for i in nelements..8 {
            dst.set_zmm64u(i, 0);
        }
        Ok(())
    }

    // ========================================================================
    // VPEXPANDD — Expand packed dwords (EVEX.66.0F38.W0 89)
    // ========================================================================

    /// VPEXPANDD Vdq{k}, Wdq — register form
    /// Read contiguous source dwords and scatter them to positions where
    /// opmask bits are set. Where mask is 0: merge or zero based on EVEX.z.
    pub fn evex_vpexpandd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = if zmask {
            BxPackedZmmRegister::default()
        } else {
            read_zmm(self, instr.dst())
        };
        let mut k = 0usize; // source index (contiguous)

        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, src.zmm32u(k));
                k += 1;
            } else if zmask {
                result.set_zmm32u(i, 0);
            }
            // else: merge — keep dest value already in result
        }
        // Zero upper elements beyond VL
        for i in nelements..16 {
            result.set_zmm32u(i, 0);
        }

        self.vmm[instr.dst() as usize] = result;
        Ok(())
    }

    // ========================================================================
    // VPEXPANDQ — Expand packed qwords (EVEX.66.0F38.W1 89)
    // ========================================================================

    /// VPEXPANDQ Vdq{k}, Wdq — register form
    pub fn evex_vpexpandq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = if zmask {
            BxPackedZmmRegister::default()
        } else {
            read_zmm(self, instr.dst())
        };
        let mut k = 0usize;

        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm64u(i, src.zmm64u(k));
                k += 1;
            } else if zmask {
                result.set_zmm64u(i, 0);
            }
        }
        for i in nelements..8 {
            result.set_zmm64u(i, 0);
        }

        self.vmm[instr.dst() as usize] = result;
        Ok(())
    }

    // ========================================================================
    // VPMOV narrowing conversions (register forms)
    //
    // Bochs avx512_move.cc VPMOV{,S,US}{QB,QW,QD,DB,DW,WB}_MASK_WdqVdqR. Every
    // one of the eighteen is the same loop over source elements differing only
    // in the two widths and how an out-of-range value is reduced, so they share
    // an implementation rather than being written out one by one.
    //
    // The destination is at most 128 bits wide (one element per source element,
    // each narrower), so building the result from zero reproduces Bochs's
    // explicit `dst.xmm32u(1) = 0` / `xmm64u(1) = 0` tail clears plus the
    // BX_WRITE_XMM_REG_CLEAR_HIGH above 128 bits.
    // ========================================================================

    /// One VPMOV narrowing conversion, selected by source width, destination
    /// width and saturation mode.
    pub(super) fn evex_vpmov_narrow(
        &mut self,
        instr: &Instruction,
        src_w: PmovSrc,
        dst_w: PmovDst,
        sat: PmovSat,
    ) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = match src_w {
            PmovSrc::Word => word_elements(vl),
            PmovSrc::Dword => dword_elements(vl),
            PmovSrc::Qword => qword_elements(vl),
        };
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = BxPackedZmmRegister::default();
        let dst_orig = self.vmm[instr.dst() as usize];
        for i in 0..nelements {
            let active = (mask >> i) & 1 != 0;
            let value = if active {
                let raw = match src_w {
                    PmovSrc::Word => src.zmm16u(i) as u64,
                    PmovSrc::Dword => src.zmm32u(i) as u64,
                    PmovSrc::Qword => src.zmm64u(i),
                };
                pmov_convert(raw, src_w, dst_w, sat)
            } else if zmask {
                0
            } else {
                // Merge: keep the destination element that is already there.
                match dst_w {
                    PmovDst::Byte => dst_orig.zmmubyte(i) as u64,
                    PmovDst::Word => dst_orig.zmm16u(i) as u64,
                    PmovDst::Dword => dst_orig.zmm32u(i) as u64,
                }
            };
            match dst_w {
                PmovDst::Byte => result.set_zmmubyte(i, value as u8),
                PmovDst::Word => result.set_zmm16u(i, value as u16),
                PmovDst::Dword => result.set_zmm32u(i, value as u32),
            }
        }

        self.vmm[instr.dst() as usize] = result;
        Ok(())
    }

    // ========================================================================
    // VPCONFLICTD — Detect conflicts within a vector of dwords (AVX-512CD)
    // EVEX.66.0F38.W0 C4
    // ========================================================================

    /// VPCONFLICTD Vdq{k}, Wdq
    /// For each dword element, set bits in the result for all earlier elements
    /// that have the same value: result[i] = bitmask of j < i where src[j] == src[i]
    pub fn evex_vpconflictd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let mut conflict_bits: u32 = 0;
            for j in 0..i {
                if src.zmm32u(j) == src.zmm32u(i) {
                    conflict_bits |= 1u32 << j;
                }
            }
            result.set_zmm32u(i, conflict_bits);
        }

        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPLZCNTD — Count leading zeros of packed dwords (AVX-512CD)
    // EVEX.66.0F38.W0 44
    // ========================================================================

    /// VPLZCNTD Vdq{k}, Wdq
    /// Count leading zeros of each packed dword element.
    pub fn evex_vplzcntd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm32u(i, src.zmm32u(i).leading_zeros());
        }

        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VPLZCNTQ — Count leading zeros of packed qwords (AVX-512CD)
    // EVEX.66.0F38.W1 44
    // ========================================================================

    /// VPLZCNTQ Vdq{k}, Wdq
    /// Count leading zeros of each packed qword element.
    pub fn evex_vplzcntq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            result.set_zmm64u(i, src.zmm64u(i).leading_zeros() as u64);
        }

        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }
}
