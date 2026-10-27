//! Advanced SIMD data movement and immediates (Arm ARM C3.5, C7.2): MOVI and
//! friends, DUP/INS/UMOV/SMOV, EXT, the permutes and TBL/TBX. The arithmetic
//! classes are in `simd_arith`.
//!
//! Vectors are held as `u128` with element 0 in the low bits; a 64-bit
//! (`Q = 0`) form writes its result zero-extended to 128 bits, as the
//! pseudocode's `V[d] = result` at datasize 64 does.

use super::dp::{ones, replicate};
use super::exec::{bit, bits, sext, ExecResult, Flow};
use super::fp::FpType;
use super::{Cpu, Levels};

/// Element `index` of `esize` bits.
#[inline]
pub(crate) fn elem(v: u128, index: u32, esize: u32) -> u64 {
    ((v >> (index * esize)) as u64) & ones(esize)
}

#[inline]
pub(crate) fn set_elem(v: &mut u128, index: u32, esize: u32, value: u64) {
    let shift = index * esize;
    let mask = u128::from(ones(esize)) << shift;
    *v = (*v & !mask) | ((u128::from(value & ones(esize))) << shift);
}

/// Truncate a result to the operation's datasize.
#[inline]
pub(crate) fn at_datasize(v: u128, q: bool) -> u128 {
    if q {
        v
    } else {
        v & u128::from(u64::MAX)
    }
}

#[inline]
const fn datasize(q: bool) -> u32 {
    if q {
        128
    } else {
        64
    }
}

/// Arm ARM `VFPExpandImm(imm8, N)` at `ty`'s width: sign `imm8<7>`, an
/// `E`-bit exponent `NOT(imm8<6>):Replicate(imm8<6>, E-3):imm8<5:4>`, and
/// the fraction `imm8<3:0>` followed by zeros.
pub(crate) fn vfp_expand_imm(imm8: u64, ty: FpType) -> u64 {
    let n = ty.esize();
    let e = match ty {
        FpType::Half => 5,
        FpType::Single => 8,
        FpType::Double => 11,
    };
    let f = n - e - 1;
    let sign = (imm8 >> 7) & 1;
    let b6 = (imm8 >> 6) & 1;
    let replicated = if b6 == 1 { ones(e - 3) } else { 0 };
    let exp = ((b6 ^ 1) << (e - 1)) | (replicated << 2) | ((imm8 >> 4) & 3);
    (sign << (n - 1)) | (exp << f) | ((imm8 & 0xF) << (f - 4))
}

/// Arm ARM `AdvSIMDExpandImm(op, cmode, imm8)`.
fn adv_simd_expand_imm(op: bool, cmode: u32, imm8: u64) -> u64 {
    match cmode >> 1 {
        0 => replicate(imm8, 32, 64),
        1 => replicate(imm8 << 8, 32, 64),
        2 => replicate(imm8 << 16, 32, 64),
        3 => replicate(imm8 << 24, 32, 64),
        4 => replicate(imm8, 16, 64),
        5 => replicate(imm8 << 8, 16, 64),
        6 => {
            if cmode & 1 == 0 {
                replicate((imm8 << 8) | 0xFF, 32, 64)
            } else {
                replicate((imm8 << 16) | 0xFFFF, 32, 64)
            }
        }
        _ => match (cmode & 1, op) {
            (0, false) => replicate(imm8, 8, 64),
            (0, true) => {
                let mut v = 0u64;
                for i in 0..8 {
                    if (imm8 >> i) & 1 == 1 {
                        v |= 0xFF << (8 * i);
                    }
                }
                v
            }
            (_, false) => replicate(vfp_expand_imm(imm8, FpType::Single), 32, 64),
            (_, true) => vfp_expand_imm(imm8, FpType::Double),
        },
    }
}

impl<L: Levels> Cpu<L> {
    /// MOVI/MVNI/ORR/BIC (vector, immediate) and FMOV (vector, immediate):
    /// `0 Q op 0111100000 abc cmode o2 1 defgh Rd`.
    pub(crate) fn exec_simd_imm(&mut self, pc: u64, insn: u32) -> ExecResult {
        let q = bit(insn, 30);
        let op = bit(insn, 29);
        let cmode = bits(insn, 15, 12);
        let imm8 = (u64::from(bits(insn, 18, 16)) << 5) | u64::from(bits(insn, 9, 5));
        if bit(insn, 11) {
            // o2 = 1: FMOV (vector, immediate) of half precision (FEAT_FP16,
            // which the decoder checks), `op = 0` and `cmode = 1111`.
            if op || cmode != 0b1111 {
                return self.undefined(pc, insn);
            }
            self.simd_check()?;
            let imm64 = replicate(vfp_expand_imm(imm8, FpType::Half), 16, 64);
            let imm = at_datasize(u128::from(imm64) | (u128::from(imm64) << 64), q);
            self.write_v(bits(insn, 4, 0) as usize, imm);
            return Ok(Flow::Next);
        }
        if cmode == 0b1111 && op && !q {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let imm64 = adv_simd_expand_imm(op, cmode, imm8);
        let imm = at_datasize(u128::from(imm64) | (u128::from(imm64) << 64), q);
        let rd = bits(insn, 4, 0) as usize;

        #[derive(PartialEq)]
        enum Kind {
            Movi,
            Mvni,
            Orr,
            Bic,
        }
        let kind = if cmode >= 0b1110 {
            Kind::Movi
        } else {
            let orr_bic = if cmode & 0b1000 == 0 || cmode & 0b1100 == 0b1000 {
                cmode & 1 == 1
            } else {
                false
            };
            match (orr_bic, op) {
                (true, false) => Kind::Orr,
                (true, true) => Kind::Bic,
                (false, false) => Kind::Movi,
                (false, true) => Kind::Mvni,
            }
        };
        let current = self.v[rd];
        let result = match kind {
            Kind::Movi => imm,
            Kind::Mvni => !imm,
            Kind::Orr => current | imm,
            Kind::Bic => current & !imm,
        };
        self.write_v(rd, at_datasize(result, q));
        Ok(Flow::Next)
    }

    /// DUP (element, general), SMOV, UMOV, INS (general, element):
    /// `0 Q op 01110000 imm5 0 imm4 1 Rn Rd`.
    pub(crate) fn exec_simd_ins(&mut self, pc: u64, insn: u32) -> ExecResult {
        let q = bit(insn, 30);
        let op = bit(insn, 29);
        let imm5 = bits(insn, 20, 16);
        let imm4 = bits(insn, 14, 11);
        let rn = bits(insn, 9, 5);
        let rd = bits(insn, 4, 0);
        let size = imm5.trailing_zeros();
        if size > 3 {
            return self.undefined(pc, insn);
        }
        let esize = 8 << size;
        let index = imm5 >> (size + 1);

        #[derive(Clone, Copy)]
        enum Kind {
            DupElement,
            DupGeneral,
            Smov,
            Umov,
            InsGeneral,
            InsElement,
        }
        let kind = match (op, imm4) {
            (false, 0b0000) if !(size == 3 && !q) => Kind::DupElement,
            (false, 0b0001) if !(size == 3 && !q) => Kind::DupGeneral,
            // SMOV Wd from B/H; SMOV Xd from B/H/S.
            (false, 0b0101) if (q && size <= 2) || (!q && size <= 1) => Kind::Smov,
            // UMOV Wd from B/H/S; UMOV Xd from D.
            (false, 0b0111) if (q && size == 3) || (!q && size <= 2) => Kind::Umov,
            (false, 0b0011) if q => Kind::InsGeneral,
            (true, _) if q => Kind::InsElement,
            _ => return self.undefined(pc, insn),
        };
        // SMOV and UMOV of element 0 are legal in Streaming SVE mode (Arm
        // ARM SMOV, UMOV: `if index == 0 then AArch64_CheckFPEnabled()`).
        if matches!(kind, Kind::Smov | Kind::Umov) && index == 0 {
            self.fp_check()?;
        } else {
            self.simd_check()?;
        }
        let duplicated = |element: u64| {
            let r = replicate(element, esize, 64);
            at_datasize(u128::from(r) | (u128::from(r) << 64), q)
        };
        let d = rd as usize;
        match kind {
            Kind::DupElement => {
                self.write_v(d, duplicated(elem(self.v[rn as usize], index, esize)));
            }
            Kind::DupGeneral => {
                self.write_v(d, duplicated(self.x_or_zr(rn) & ones(esize)));
            }
            Kind::Smov => {
                let v = sext(elem(self.v[rn as usize], index, esize), esize);
                self.set_x_or_zr(rd, if q { v } else { v & 0xFFFF_FFFF });
            }
            Kind::Umov => {
                let v = elem(self.v[rn as usize], index, esize);
                self.set_x_or_zr(rd, v);
            }
            Kind::InsGeneral => {
                let mut r = self.v[d];
                set_elem(&mut r, index, esize, self.x_or_zr(rn));
                self.write_v(d, r);
            }
            Kind::InsElement => {
                let src = imm4 >> size;
                let mut r = self.v[d];
                set_elem(&mut r, index, esize, elem(self.v[rn as usize], src, esize));
                self.write_v(d, r);
            }
        }
        Ok(Flow::Next)
    }

    /// DUP (element, scalar), the `MOV` scalar alias:
    /// `01 0 11110000 imm5 0 0000 1 Rn Rd`.
    pub(crate) fn exec_simd_scalar_dup(&mut self, pc: u64, insn: u32) -> ExecResult {
        let imm5 = bits(insn, 20, 16);
        let size = imm5.trailing_zeros();
        if size > 3 || bit(insn, 29) || bits(insn, 14, 11) != 0 {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let esize = 8 << size;
        let value = elem(self.v[bits(insn, 9, 5) as usize], imm5 >> (size + 1), esize);
        self.write_v(bits(insn, 4, 0) as usize, u128::from(value));
        Ok(Flow::Next)
    }

    /// EXT: `0 Q 101110 00 0 Rm 0 imm4 0 Rn Rd`.
    pub(crate) fn exec_simd_ext(&mut self, pc: u64, insn: u32) -> ExecResult {
        let q = bit(insn, 30);
        let imm4 = bits(insn, 14, 11);
        if bits(insn, 23, 22) != 0 || (!q && imm4 >= 8) {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let n = at_datasize(self.v[bits(insn, 9, 5) as usize], q);
        let m = at_datasize(self.v[bits(insn, 20, 16) as usize], q);
        let pos = imm4 * 8;
        let width = datasize(q);
        let r = if pos == 0 {
            n
        } else {
            (n >> pos) | (m << (width - pos))
        };
        self.write_v(bits(insn, 4, 0) as usize, at_datasize(r, q));
        Ok(Flow::Next)
    }

    /// UZP1/UZP2/TRN1/TRN2/ZIP1/ZIP2: `0 Q 001110 size 0 Rm 0 opcode 10 Rn Rd`.
    pub(crate) fn exec_simd_perm(&mut self, pc: u64, insn: u32) -> ExecResult {
        let q = bit(insn, 30);
        let size = bits(insn, 23, 22);
        let opcode = bits(insn, 14, 12);
        if size == 3 && !q {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[bits(insn, 20, 16) as usize];
        let esize = 8u32 << size;
        let elements = datasize(q) / esize;
        let part = opcode >> 2;
        let mut r = 0u128;
        match opcode & 3 {
            1 => {
                for e in 0..elements {
                    let i = 2 * e + part;
                    let v = if i < elements { elem(n, i, esize) } else { elem(m, i - elements, esize) };
                    set_elem(&mut r, e, esize, v);
                }
            }
            2 => {
                for p in 0..elements / 2 {
                    set_elem(&mut r, 2 * p, esize, elem(n, 2 * p + part, esize));
                    set_elem(&mut r, 2 * p + 1, esize, elem(m, 2 * p + part, esize));
                }
            }
            3 => {
                let base = part * elements / 2;
                for p in 0..elements / 2 {
                    set_elem(&mut r, 2 * p, esize, elem(n, base + p, esize));
                    set_elem(&mut r, 2 * p + 1, esize, elem(m, base + p, esize));
                }
            }
            _ => return self.undefined(pc, insn),
        }
        self.write_v(bits(insn, 4, 0) as usize, at_datasize(r, q));
        Ok(Flow::Next)
    }

    /// TBL/TBX: `0 Q 001110 00 0 Rm 0 len op 00 Rn Rd`.
    pub(crate) fn exec_simd_tbl(&mut self, pc: u64, insn: u32) -> ExecResult {
        let q = bit(insn, 30);
        if bits(insn, 23, 22) != 0 {
            return self.undefined(pc, insn);
        }
        self.simd_check()?;
        let regs = bits(insn, 14, 13) + 1;
        let tbx = bit(insn, 12);
        let rn = bits(insn, 9, 5);
        let indices = self.v[bits(insn, 20, 16) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let mut r = if tbx { self.v[rd] } else { 0 };
        for i in 0..datasize(q) / 8 {
            let index = elem(indices, i, 8) as u32;
            if index < 16 * regs {
                let table = self.v[((rn + index / 16) % 32) as usize];
                set_elem(&mut r, i, 8, elem(table, index % 16, 8));
            } else if !tbx {
                set_elem(&mut r, i, 8, 0);
            }
        }
        self.write_v(rd, at_datasize(r, q));
        Ok(Flow::Next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmov_immediates_expand_to_their_float_values() {
        // fmov s0, #1.0 -> imm8 0x70; fmov d0, #-2.0 -> imm8 0x80
        assert_eq!(f32::from_bits(vfp_expand_imm(0x70, FpType::Single) as u32), 1.0);
        assert_eq!(f64::from_bits(vfp_expand_imm(0x80, FpType::Double)), -2.0);
        assert_eq!(f64::from_bits(vfp_expand_imm(0x70, FpType::Double)), 1.0);
        // fmov h0, #1.0 and #-31.0 (imm8 0x70 and 0xBF), GNU as.
        assert_eq!(vfp_expand_imm(0x70, FpType::Half), 0x3C00);
        assert_eq!(vfp_expand_imm(0xBF, FpType::Half), 0xCFC0);
        // The largest and smallest single-precision magnitudes, 31.0 and 0.125.
        assert_eq!(f32::from_bits(vfp_expand_imm(0x3F, FpType::Single) as u32), 31.0);
        assert_eq!(f32::from_bits(vfp_expand_imm(0x40, FpType::Single) as u32), 0.125);
    }

    #[test]
    fn movi_byte_mask_form_expands_each_bit_to_a_byte() {
        assert_eq!(adv_simd_expand_imm(true, 0b1110, 0b1000_0001), 0xFF00_0000_0000_00FF);
    }
}
