//! AVX-512F compress, expand, and miscellaneous instruction handlers
//!
//! Implements VPCOMPRESSD/Q, VPEXPANDD/Q, VPMOVDB, VPMOVDW, VPMOVQD
//! (register forms), VPCONFLICTD (AVX-512CD), VPLZCNTD/Q (AVX-512CD).
//!
//! Note: VPMOVD2M, VPMOVQ2M, VPMOVM2D, VPMOVM2Q live in avx512_cmp.rs.
//!
//! Mirrors Bochs `cpu/avx/avx512.cc`, `avx512_move.cc`, `avx512_conflict.cc`.

use super::avx512_load::cut_opmask_to;
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

    /// VPCONFLICTQ Vdq{k}{z}, Wdq — EVEX.66.0F38.W1 C4. Qword counterpart of
    /// [`Self::evex_vpconflictd`]: each element gets a bitmask of the earlier
    /// elements it equals.
    pub fn evex_vpconflictq(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = if instr.mod_c0() {
            read_zmm(self, instr.src())
        } else {
            self.evex_load_broadcast_vector_q(instr)?
        };
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = BxPackedZmmRegister::default();
        for i in 0..nelements {
            let mut conflict_bits: u64 = 0;
            for j in 0..i {
                if src.zmm64u(j) == src.zmm64u(i) {
                    conflict_bits |= 1u64 << j;
                }
            }
            result.set_zmm64u(i, conflict_bits);
        }

        write_zmm_masked_q(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    /// VPBROADCASTMB2Q Vdq, k — EVEX.F3.0F38.W1 2A. Broadcasts the opmask
    /// itself, zero-extended, into every qword. Bochs avx512_bitalg.cc; the
    /// opmask here is the *source*, not a writemask, so the write is unmasked.
    pub fn evex_vpbroadcastmb2q(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let value = self.opmask_rrx(instr.src() as usize) & 0xFF;
        let mut result = BxPackedZmmRegister::default();
        for n in 0..qword_elements(vl) {
            result.set_zmm64u(n, value);
        }
        write_zmm_masked_q(self, instr.dst(), &result, u64::MAX, true, vl);
        Ok(())
    }

    /// VPBROADCASTMW2D Vdq, k — EVEX.F3.0F38.W0 3A.
    pub fn evex_vpbroadcastmw2d(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let value = (self.opmask_rrx(instr.src() as usize) & 0xFFFF) as u32;
        let mut result = BxPackedZmmRegister::default();
        for n in 0..dword_elements(vl) {
            result.set_zmm32u(n, value);
        }
        write_zmm_masked(self, instr.dst(), &result, u64::MAX, true, vl);
        Ok(())
    }

    /// VBROADCASTF32x2 Vps{k}{z}, Wq — EVEX.66.0F38.W0 19. Broadcasts a
    /// *qword* — a pair of singles — but writemasks at dword granularity.
    pub fn evex_vbroadcastf32x2(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let value = if instr.mod_c0() {
            read_zmm(self, instr.src()).zmm64u(0)
        } else {
            self.evex_load_wsd_pair(instr)?.zmm64u(0)
        };
        let mut result = BxPackedZmmRegister::default();
        for n in 0..qword_elements(vl) {
            result.set_zmm64u(n, value);
        }
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;
        write_zmm_masked(self, instr.dst(), &result, mask, zmask, vl);
        Ok(())
    }

    // ========================================================================
    // VEXPAND / VCOMPRESS — gather the set opmask bits to or from a
    // contiguous run. Bochs avx512.cc VEXPANDPS/PD and VCOMPRESSPS/PD.
    //
    // The memory forms touch only as many elements as the opmask has bits
    // set, contiguously from the effective address, so they go through the
    // masked load and store with a *density* mask rather than the writemask.
    // ========================================================================

    /// VEXPANDPS Vps{k}{z}, Wps — EVEX.66.0F38.W0 88
    pub fn evex_vexpandps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let zmask = instr.is_zero_masking() != 0;
        let opmask = read_opmask_for_write(self, instr) & cut_opmask_to(nelements);

        let mut result = if zmask {
            BxPackedZmmRegister::default()
        } else {
            read_zmm(self, instr.dst())
        };

        if opmask != 0 {
            let op = if instr.mod_c0() {
                read_zmm(self, instr.src())
            } else {
                // Only popcount(opmask) elements are read, from the bottom up.
                let load_mask = (1u64 << opmask.count_ones()) - 1;
                let eaddr = self.resolve_addr(instr);
                let mut tmp = BxPackedZmmRegister::default();
                self.avx_masked_load32(instr, eaddr, &mut tmp, load_mask)?;
                tmp
            };
            let mut k = 0;
            for n in 0..nelements {
                if (opmask >> n) == 0 {
                    break;
                }
                if (opmask >> n) & 1 != 0 {
                    result.set_zmm32u(n, op.zmm32u(k));
                    k += 1;
                }
            }
        }

        write_zmm_masked(self, instr.dst(), &result, u64::MAX, true, vl);
        Ok(())
    }

    /// VEXPANDPD Vpd{k}{z}, Wpd — EVEX.66.0F38.W1 88
    pub fn evex_vexpandpd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let zmask = instr.is_zero_masking() != 0;
        let opmask = read_opmask_for_write(self, instr) & cut_opmask_to(nelements);

        let mut result = if zmask {
            BxPackedZmmRegister::default()
        } else {
            read_zmm(self, instr.dst())
        };

        if opmask != 0 {
            let op = if instr.mod_c0() {
                read_zmm(self, instr.src())
            } else {
                let load_mask = (1u64 << opmask.count_ones()) - 1;
                let eaddr = self.resolve_addr(instr);
                let mut tmp = BxPackedZmmRegister::default();
                self.avx_masked_load64(instr, eaddr, &mut tmp, load_mask)?;
                tmp
            };
            let mut k = 0;
            for n in 0..nelements {
                if (opmask >> n) == 0 {
                    break;
                }
                if (opmask >> n) & 1 != 0 {
                    result.set_zmm64u(n, op.zmm64u(k));
                    k += 1;
                }
            }
        }

        write_zmm_masked_q(self, instr.dst(), &result, u64::MAX, true, vl);
        Ok(())
    }

    /// VCOMPRESSPS Wps{k}, Vps — EVEX.66.0F38.W0 8A. The inverse: the
    /// selected elements are packed down to a contiguous run.
    pub fn evex_vcompressps(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let opmask = read_opmask_for_write(self, instr) & cut_opmask_to(nelements);
        let op = read_zmm(self, instr.src());

        let mut result = BxPackedZmmRegister::default();
        let mut k = 0usize;
        for n in 0..nelements {
            if (opmask >> n) == 0 {
                break;
            }
            if (opmask >> n) & 1 != 0 {
                result.set_zmm32u(k, op.zmm32u(n));
                k += 1;
            }
        }
        // The destination run is as long as the number of selected elements.
        let writemask = if k >= 64 { u64::MAX } else { (1u64 << k) - 1 };

        if instr.mod_c0() {
            let zmask = instr.is_zero_masking() != 0;
            write_zmm_masked(self, instr.dst(), &result, writemask, zmask, vl);
            Ok(())
        } else {
            let eaddr = self.resolve_addr(instr);
            self.avx_masked_store32(instr, eaddr, &result, writemask)
        }
    }

    /// VCOMPRESSPD Wpd{k}, Vpd — EVEX.66.0F38.W1 8A
    pub fn evex_vcompresspd(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let opmask = read_opmask_for_write(self, instr) & cut_opmask_to(nelements);
        let op = read_zmm(self, instr.src());

        let mut result = BxPackedZmmRegister::default();
        let mut k = 0usize;
        for n in 0..nelements {
            if (opmask >> n) == 0 {
                break;
            }
            if (opmask >> n) & 1 != 0 {
                result.set_zmm64u(k, op.zmm64u(n));
                k += 1;
            }
        }
        let writemask = (1u64 << k) - 1;

        if instr.mod_c0() {
            let zmask = instr.is_zero_masking() != 0;
            write_zmm_masked_q(self, instr.dst(), &result, writemask, zmask, vl);
            Ok(())
        } else {
            let eaddr = self.resolve_addr(instr);
            self.avx_masked_store64(instr, eaddr, &result, writemask)
        }
    }


}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    //! VPBROADCASTMB2Q/MW2D are the one place an opmask register is read as
    //! *data* rather than as a writemask, and VPCONFLICT looks only backwards
    //! — element n never sees elements above it.

    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
    use crate::cpu::decoder::BxSegregs;
    use rusty_box_decoder::opcode::Opcode;

    use super::*;

    fn evex_misc(opcode: Opcode, vl: u8) -> Instruction {
        let mut i = Instruction::default();
        i.set_ia_opcode(opcode);
        i.set_src_reg(0, 0);
        i.set_src_reg(1, 1);
        i.set_src_reg(2, 2);
        i.set_opmask(0);
        i.set_vl(vl);
        i.set_vex(true);
        i.set_seg(BxSegregs::Ds);
        i.init(0, 0, 1, 1);
        i.assert_mod_c0();
        i
    }

    #[test]
    fn opmask_broadcasts_read_the_mask_as_data() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.bx_write_opmask(1, 0xFFFF_00AB);

        c.execute_instruction(&evex_misc(Opcode::EvexVpbroadcastmb2qVdqKeb, 1))
            .unwrap();
        for n in 0..4 {
            assert_eq!(c.vmm[0].zmm64u(n), 0xAB, "qword {n} takes the low 8 bits");
        }

        c.execute_instruction(&evex_misc(Opcode::EvexVpbroadcastmw2dVdqKew, 1))
            .unwrap();
        for n in 0..8 {
            assert_eq!(c.vmm[0].zmm32u(n), 0x00AB, "dword {n} takes the low 16 bits");
        }
    }

    #[test]
    fn vpconflictq_marks_only_earlier_matching_elements() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        c.vmm[1].set_zmm64u(0, 7);
        c.vmm[1].set_zmm64u(1, 9);
        c.vmm[1].set_zmm64u(2, 7);
        c.vmm[1].set_zmm64u(3, 7);
        c.execute_instruction(&evex_misc(Opcode::EvexVpconflictqVdqWdqKmask, 1))
            .unwrap();
        assert_eq!(c.vmm[0].zmm64u(0), 0b0000, "nothing precedes element 0");
        assert_eq!(c.vmm[0].zmm64u(1), 0b0000, "9 matches nothing earlier");
        assert_eq!(c.vmm[0].zmm64u(2), 0b0001, "element 2 equals element 0");
        assert_eq!(
            c.vmm[0].zmm64u(3),
            0b0101,
            "element 3 equals elements 0 and 2"
        );
    }

    #[test]
    fn expand_and_compress_are_inverses_over_the_opmask() {
        let mut c = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
        // Source holds 10,11,12,13 contiguously.
        for (n, v) in [10u32, 11, 12, 13].into_iter().enumerate() {
            c.vmm[1].set_zmm32u(n, v);
        }
        for n in 0..4 {
            c.vmm[0].set_zmm32u(n, 0xDEAD_BEEF);
        }
        // EXPAND scatters them to the set mask positions, taking the source
        // elements in order from the bottom.
        c.bx_write_opmask(1, 0b1010);
        let mut i = evex_misc(Opcode::EvexVexpandpsVpsWpsKmask, 0);
        i.set_opmask(1);
        c.execute_instruction(&i).unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 0xDEAD_BEEF, "unselected element merges");
        assert_eq!(c.vmm[0].zmm32u(1), 10, "first set bit takes source[0]");
        assert_eq!(c.vmm[0].zmm32u(2), 0xDEAD_BEEF);
        assert_eq!(c.vmm[0].zmm32u(3), 11, "second set bit takes source[1]");

        // COMPRESS packs the selected elements back down to a contiguous run
        // and leaves the rest of the destination alone.
        for (n, v) in [20u32, 21, 22, 23].into_iter().enumerate() {
            c.vmm[1].set_zmm32u(n, v);
        }
        for n in 0..4 {
            c.vmm[0].set_zmm32u(n, 0xDEAD_BEEF);
        }
        let mut i = evex_misc(Opcode::EvexVcompresspsWpsVpsKmask, 0);
        i.set_opmask(1);
        c.execute_instruction(&i).unwrap();
        assert_eq!(c.vmm[0].zmm32u(0), 21, "element 1 was selected first");
        assert_eq!(c.vmm[0].zmm32u(1), 23, "element 3 second");
        assert_eq!(
            c.vmm[0].zmm32u(2),
            0xDEAD_BEEF,
            "the run is only as long as the popcount"
        );
    }

}
