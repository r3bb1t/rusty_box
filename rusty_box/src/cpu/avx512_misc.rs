#![allow(unused_unsafe)]

//! AVX-512F compress, expand, and miscellaneous instruction handlers
//!
//! Implements VPCOMPRESSD/Q, VPEXPANDD/Q, VPMOVDB, VPMOVDW, VPMOVQD
//! (register forms), VPCONFLICTD (AVX-512CD), VPLZCNTD/Q (AVX-512CD).
//!
//! Note: VPMOVD2M, VPMOVQ2M, VPMOVM2D, VPMOVM2Q live in avx512_cmp.rs.
//!
//! Mirrors Bochs `cpu/avx/avx512.cc`, `avx512_move.cc`, `avx512_conflict.cc`.

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::Instruction,
    xmm::BxPackedZmmRegister,
};

// ============================================================================
// Helper functions (duplicated from avx512.rs — module-private there)
// ============================================================================

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
        // SAFETY: opmask register union always valid for rrx (full 64-bit) access
        unsafe { cpu.opmask[k as usize].rrx() }
    }
}

/// Read ZMM register as a ZMM-width value
#[inline]
fn read_zmm<I: BxCpuIdTrait>(cpu: &BxCpuC<'_, I>, reg: u8) -> BxPackedZmmRegister {
    cpu.vmm[reg as usize]
}

/// Write ZMM register with dword-granularity masking, zeroing upper bits beyond VL
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
    // Zero upper elements beyond VL (EVEX always clears upper)
    for i in nelements..16 {
        dst.set_zmm32u(i, 0);
    }
}

/// Write ZMM register with qword-granularity masking
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

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
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
    // VPMOVDB — Truncate packed dwords to bytes (register form)
    // EVEX.F3.0F38.W0 31
    // ========================================================================

    /// VPMOVDB Wdq{k}, Vdq — register form
    /// Truncate each dword source element to a byte and pack into the lower
    /// portion of the destination register.
    pub fn evex_vpmovdb_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl); // number of dword src elements
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        // Destination is byte-granularity in the low `nelements` bytes
        let mut result = BxPackedZmmRegister::default();
        let dst_orig = &self.vmm[instr.dst() as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmmubyte(i, src.zmm32u(i) as u8);
            } else if zmask {
                result.set_zmmubyte(i, 0);
            } else {
                // merge: keep original destination byte
                result.set_zmmubyte(i, dst_orig.zmmubyte(i));
            }
        }
        // Bytes beyond nelements up to 16 are zeroed for VL < 512,
        // and all upper bytes beyond 16 are always zeroed.
        // For VL0 (4 elements): bytes 0-3 active, 4-63 zero
        // For VL1 (8 elements): bytes 0-7 active, 8-63 zero
        // For VL2 (16 elements): bytes 0-15 active, 16-63 zero

        self.vmm[instr.dst() as usize] = result;
        Ok(())
    }

    // ========================================================================
    // VPMOVDW — Truncate packed dwords to words (register form)
    // EVEX.F3.0F38.W0 33
    // ========================================================================

    /// VPMOVDW Wdq{k}, Vdq — register form
    /// Truncate each dword source element to a word and pack into the lower
    /// portion of the destination register.
    pub fn evex_vpmovdw_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = dword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = BxPackedZmmRegister::default();
        let dst_orig = &self.vmm[instr.dst() as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm16u(i, src.zmm32u(i) as u16);
            } else if zmask {
                result.set_zmm16u(i, 0);
            } else {
                result.set_zmm16u(i, dst_orig.zmm16u(i));
            }
        }
        // For VL0: words 0-3 active, rest zero
        // For VL1: words 0-7 active, rest zero
        // For VL2: words 0-15 active, rest zero

        self.vmm[instr.dst() as usize] = result;
        Ok(())
    }

    // ========================================================================
    // VPMOVQD — Truncate packed qwords to dwords (register form)
    // EVEX.F3.0F38.W0 35
    // ========================================================================

    /// VPMOVQD Wdq{k}, Vdq — register form
    /// Truncate each qword source element to a dword and pack into the lower
    /// portion of the destination register.
    pub fn evex_vpmovqd_r(&mut self, instr: &Instruction) -> super::Result<()> {
        let vl = instr.get_vl();
        let nelements = qword_elements(vl);
        let src = read_zmm(self, instr.src());
        let mask = read_opmask_for_write(self, instr);
        let zmask = instr.is_zero_masking() != 0;

        let mut result = BxPackedZmmRegister::default();
        let dst_orig = &self.vmm[instr.dst() as usize];
        for i in 0..nelements {
            if (mask >> i) & 1 != 0 {
                result.set_zmm32u(i, src.zmm64u(i) as u32);
            } else if zmask {
                result.set_zmm32u(i, 0);
            } else {
                result.set_zmm32u(i, dst_orig.zmm32u(i));
            }
        }
        // For VL0: dwords 0-1 active, rest zero
        // For VL1: dwords 0-3 active, rest zero
        // For VL2: dwords 0-7 active, rest zero

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
