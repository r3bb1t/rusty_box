//! The Advanced SIMD BFloat16 instructions (FEAT_BF16): BFDOT, BFMLALB,
//! BFMLALT and BFMMLA. BFCVT and BFCVTN, the conversions to BFloat16, are
//! with the other conversions (`fp.rs`, `simd_fp.rs`).
//!
//! Provenance (R7): the Arm A64 ISA XML (release 2026-09), each
//! instruction's page and the shared pseudocode it calls (`BFDotAdd`,
//! `BFMulAddH`, `BFMatMulAddH`); the arithmetic is in `softfloat/bfloat.rs`.

use super::exec::{bit, bits, ExecResult, Flow};
use super::simd::{elem, set_elem};
use super::Cpu;
use crate::softfloat::{self as sf, FusedMulAdd};

impl Cpu {
    /// BFDOT, vector (`0 Q 1 01110 01 0 Rm 1 1111 1 Rn Rd`) and by element
    /// (`0 Q 0 01111 01 L M Rm 1111 H 0 Rn Rd`). Each 32-bit lane of Vd
    /// accumulates the dot product of the matching pair of BFloat16
    /// elements of Vn with Vm's pair, by element the pair `H:L` of all of
    /// `V[M:Rm]` (Arm BFDotAdd, which follows BFloat16's own behaviours,
    /// reading of FPCR only AH, the default NaN's sign).
    pub(crate) fn exec_simd_bfdot(&mut self, insn: u32, by_element: bool) -> ExecResult {
        let q = bit(insn, 30);
        self.simd_check()?;
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[bits(insn, 20, 16) as usize];
        let index = (bits(insn, 11, 11) << 1) | bits(insn, 21, 21);
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let handling = self.fp_handling();
        let mut r = 0u128;
        for e in 0..if q { 4 } else { 2 } {
            let pair = if by_element { index } else { e };
            let sum = sf::bf16_dot_add(
                elem(d, e, 32) as u32,
                elem(n, 2 * e, 16) as u16,
                elem(n, 2 * e + 1, 16) as u16,
                elem(m, 2 * pair, 16) as u16,
                elem(m, 2 * pair + 1, 16) as u16,
                handling,
            );
            set_elem(&mut r, e, 32, u64::from(sum));
        }
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// BFMLALB and BFMLALT (`Q` = 0 and 1), vector (`0 Q 1 01110 11 0 Rm 1
    /// 1111 1 Rn Rd`) and by element (`0 Q 0 01111 11 L M Rm 1111 H 0 Rn
    /// Rd`). Each 32-bit lane of Vd accumulates the product of the even
    /// (`B`) or odd (`T`) BFloat16 element of Vn's matching pair with Vm's,
    /// by element with the element `H:L:M` of V0-V15: a single-precision
    /// fused multiply-add under FPCR (Arm BFMulAddH) — under FPCR.AH = 1
    /// with every denormal flushed, rounding to nearest and no exception.
    pub(crate) fn exec_simd_bfmlal(&mut self, insn: u32, by_element: bool) -> ExecResult {
        let sel = u32::from(bit(insn, 30));
        self.simd_check()?;
        let n = self.v[bits(insn, 9, 5) as usize];
        let (m, index) = if by_element {
            let index = (bits(insn, 11, 11) << 2) | (bits(insn, 21, 21) << 1) | bits(insn, 20, 20);
            (self.v[bits(insn, 19, 16) as usize], Some(index))
        } else {
            (self.v[bits(insn, 20, 16) as usize], None)
        };
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let mut env = self.fp_env();
        let mut r = 0u128;
        for e in 0..4 {
            let element1 = (elem(n, 2 * e + sel, 16) as u32) << 16;
            let element2 = (elem(m, index.unwrap_or(2 * e + sel), 16) as u32) << 16;
            let addend = elem(d, e, 32) as u32;
            let sum = env
                .status
                .with_altfp_overrides(|fp| sf::f32_mul_add(FusedMulAdd::Fmadd, addend, element1, element2, fp));
            set_elem(&mut r, e, 32, u64::from(sum));
        }
        self.fp_commit(&env);
        self.write_v(rd, r);
        Ok(Flow::Next)
    }

    /// BFMMLA: `0 1 1 01110 01 0 Rm 1 1101 1 Rn Rd`. Vd, a 2×2 matrix of
    /// singles, accumulates the product of Vn, a 2×4 matrix of BFloat16
    /// values by rows, and the transpose of Vm, another (Arm BFMatMulAddH:
    /// each element two chained BFDotAdds).
    pub(crate) fn exec_simd_bfmmla(&mut self, insn: u32) -> ExecResult {
        self.simd_check()?;
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[bits(insn, 20, 16) as usize];
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let handling = self.fp_handling();
        let mut r = 0u128;
        for i in 0..2 {
            for j in 0..2 {
                let mut sum = elem(d, 2 * i + j, 32) as u32;
                for k in 0..2 {
                    sum = sf::bf16_dot_add(
                        sum,
                        elem(n, 4 * i + 2 * k, 16) as u16,
                        elem(n, 4 * i + 2 * k + 1, 16) as u16,
                        elem(m, 4 * j + 2 * k, 16) as u16,
                        elem(m, 4 * j + 2 * k + 1, 16) as u16,
                        handling,
                    );
                }
                set_elem(&mut r, 2 * i + j, 32, u64::from(sum));
            }
        }
        self.write_v(rd, r);
        Ok(Flow::Next)
    }
}
