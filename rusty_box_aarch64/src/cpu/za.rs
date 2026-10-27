//! The SME instructions on the ZA array (FEAT_SME): the outer products that
//! accumulate into its tiles, MOVA between a tile slice and a vector, ZERO,
//! ADDHA and ADDVA, the tile slice loads and stores, and LDR and STR of an
//! array vector.
//!
//! ZA is `SVL DIV 8` horizontal vectors of SVL bits (`Cpu::za_vector`). A
//! tile of `esize`-bit elements is every `esize DIV 8`th of them from its
//! number up (`ZAhslice`), so the array holds `esize DIV 8` such tiles,
//! each `SVL DIV esize` elements square; the vertical slice `s` of a tile
//! is element `s` of each of its horizontal ones (`ZAvslice`).
//!
//! ZERO, LDR and STR execute in or out of Streaming SVE mode
//! (`CheckSMEAndZAEnabled`), the others only in it
//! (`CheckStreamingSVEAndZAEnabled`), and all of them only while ZA
//! storage is enabled. The floating-point outer products compute as
//! `FPMulAdd_ZA` and `FPDotAdd_ZA` do, with the default NaN and no
//! exception, so they never write FPSR; BFMOPA and BFMOPS follow
//! BFloat16's own behaviours (`BFDotAdd`), as BFDOT does. The tile slice
//! loads and stores access memory as the SVE contiguous ones do (see
//! `sve::mem`): a load writes its slice once every access is done, a store
//! the elements before a faulting one.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09 (Arm ARM M.d), the
//! instruction pages each handler names and the shared pseudocode in
//! `aarch64/functions/sme` (`ZAvector`, `ZAhslice`, `ZAvslice`, `ZAslice`,
//! `ZAtile`) and `shared/functions/float` (`FPMulAdd_ZA`, `FPDotAdd_ZA`,
//! `FPDot`, `BFDotAdd`, `BFNeg`).

#[cfg(test)]
#[path = "../../tests/unit/za.rs"]
mod tests;

use super::exec::{bit, bits, ExecResult, Fault, Flow};
use super::features::VecLen;
use super::fp::{fp_mul_add, FpEnv, FpType};
use super::mmu::DataAccess;
use super::sve::{int, Esize, Pv, Zv};
use super::Cpu;
use crate::bus::PhysBus;
use crate::softfloat::{self as sf, FusedMulAdd};

/// The element size of a tile, `esize`: also how many tiles the array
/// holds (`esize DIV 8`), interleaved by vector.
///
/// Closed (R5): ZA's five element sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TileSize {
    B,
    H,
    S,
    D,
    Q,
}

impl TileSize {
    /// The element size `8 << UInt(size)` of a two-bit `size` field, or
    /// quadwords where the encoding's quadword select `q` is set.
    const fn of(size: u32, q: bool) -> Self {
        if q {
            return Self::Q;
        }
        match size & 3 {
            0 => Self::B,
            1 => Self::H,
            2 => Self::S,
            _ => Self::D,
        }
    }

    /// `log2(esize DIV 8)`.
    #[inline]
    const fn log2_bytes(self) -> u32 {
        self as u32
    }

    /// `esize DIV 8`: an element's bytes, and the number of tiles.
    #[inline]
    const fn bytes(self) -> usize {
        1 << self as u32
    }

    /// `SVL DIV esize`: a tile's slices, and the elements of each.
    #[inline]
    const fn dim(self, svl: VecLen) -> usize {
        svl.bytes() >> self as u32
    }
}

/// A tile slice, Arm `ZAslice`'s operands: horizontal or vertical slice
/// `index` of tile `tile` of `size` elements.
#[derive(Debug, Clone, Copy)]
struct Slice {
    size: TileSize,
    tile: usize,
    vertical: bool,
    index: usize,
}

/// What an outer product and ADDHA or ADDVA read once the enable check has
/// passed: the row predicate `Pn` (bits 12:10), the column predicate `Pm`
/// (bits 15:13), `Zn` (bits 9:5) and the 32-bit tile `ZAda` (bits 1:0).
struct TileOperands {
    svl: VecLen,
    rows: Pv,
    cols: Pv,
    zn: Zv,
    tile: usize,
}

/// The floating-point outer products into 32-bit tiles.
///
/// Closed (R5): Arm's three `mortlach_32bit_fp_prod` classes FEAT_SME has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FpProduct {
    /// FMOPA, FMOPS: of single-precision elements (`FPMulAdd_ZA`).
    Single,
    /// The sums of the products of pairs of 16-bit elements.
    Pairs(PairFormat),
}

/// The 16-bit elements a widening outer product, or FEAT_SME2's FDOT,
/// BFDOT, FMLAL and BFMLAL into ZA array vectors, multiplies.
///
/// Closed (R5): the two formats of FEAT_SME's widening outer products.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PairFormat {
    /// BFMOPA, BFMOPS (`BFDotAdd`).
    BFloat16,
    /// FMOPA, FMOPS (`FPDotAdd_ZA`).
    Half,
}

impl PairFormat {
    /// The negation FMOPS, BFMOPS, FMLSL and BFMLSL apply to an element:
    /// BFNeg without alternate handling, or FPNeg, under which a NaN keeps
    /// its sign while FPCR.AH = 1.
    pub(super) fn negate(self, x: u16, env: &FpEnv) -> u16 {
        match self {
            Self::BFloat16 => x ^ 0x8000,
            Self::Half => env.neg(FpType::Half, u64::from(x)) as u16,
        }
    }

    /// `sum` plus `row[0] * col[0] + row[1] * col[1]`: `BFDotAdd`, or
    /// `FPDotAdd_ZA` under `env`, which has FPCR.DN set.
    pub(super) fn dot(self, sum: u32, row: [u16; 2], col: [u16; 2], env: &mut FpEnv) -> u32 {
        let ([r0, r1], [c0, c1]) = (row, col);
        match self {
            Self::BFloat16 => sf::bf16_dot_add(sum, r0, r1, c0, c1, env.handling()),
            Self::Half => {
                let dot = sf::f16_dot(r0, r1, c0, c1, &mut env.status);
                sf::f32_add(sum, dot, &mut env.status)
            }
        }
    }

    /// `sum` plus `n * m`, each widened to single precision, in one
    /// rounding: `BFMulAddH_ZA`, or `FPMulAddH_ZA`, under `env`, which has
    /// FPCR.DN set.
    pub(super) fn mul_add(self, sum: u32, n: u16, m: u16, env: &mut FpEnv) -> u32 {
        match self {
            Self::BFloat16 => {
                let (n, m) = (u64::from(n) << 16, u64::from(m) << 16);
                fp_mul_add(FusedMulAdd::Fmadd, FpType::Single, u64::from(sum), n, m, env) as u32
            }
            Self::Half => sf::f16_mul_add_long(FusedMulAdd::Fmadd, sum, n, m, &mut env.status),
        }
    }
}

/// Pair `i` of a widening outer product's 16-bit elements, elements `2i`
/// and `2i + 1` of a vector: which of them the predicate makes active, and
/// their values, an inactive one +0 (Arm's `erow_0`, `erow_1`, `ecol_0`,
/// `ecol_1`).
#[derive(Debug, Clone, Copy)]
struct Pair {
    active: [bool; 2],
    elements: [u16; 2],
}

impl Pair {
    fn of(z: &Zv, mask: &Pv, i: usize) -> Self {
        let active = [mask.active(2 * i, Esize::H), mask.active(2 * i + 1, Esize::H)];
        let element = |k: usize| if active[k] { z.get(2 * i + k, Esize::H) as u16 } else { 0 };
        Self {
            active,
            elements: [element(0), element(1)],
        }
    }

    /// Whether a row pair and a column pair have an element of the same
    /// place both active: whether the tile element accumulates their
    /// products.
    fn meets(&self, col: &Pair) -> bool {
        (self.active[0] && col.active[0]) || (self.active[1] && col.active[1])
    }
}

/// The integer element of an outer product, or of FEAT_SME2's products
/// into ZA array vectors, as `UInt` or `SInt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Signedness {
    Signed,
    Unsigned,
}

impl Signedness {
    pub(super) const fn of(unsigned: bool) -> Self {
        if unsigned {
            Self::Unsigned
        } else {
            Self::Signed
        }
    }

    /// The low `es` bits of `value` as an integer.
    pub(super) fn int(self, value: u64, es: Esize) -> i128 {
        int(value, es, self == Self::Unsigned)
    }
}

impl Cpu {
    /// The slice an instruction names: its tile and offset in the four bits
    /// from `lo` up (`ZAd:off`, the tile in the high `log2(esize DIV 8)`),
    /// vertical by `V` (bit 15), its index `(UInt(Ws) + offset) MOD dim`
    /// with `Ws` one of W12 to W15 by `Rs` (bits 14:13).
    fn tile_slice(&self, insn: u32, lo: u32, size: TileSize, svl: VecLen) -> Slice {
        let field = bits(insn, lo + 3, lo);
        let offset_bits = 4 - size.log2_bytes();
        let offset = u64::from(field & ((1 << offset_bits) - 1));
        let ws = self.x_or_zr(12 + bits(insn, 14, 13)) as u32;
        Slice {
            size,
            tile: (field >> offset_bits) as usize,
            vertical: bit(insn, 15),
            index: ((u64::from(ws) + offset) % size.dim(svl) as u64) as usize,
        }
    }

    /// Arm `ZAslice{SVL}(tile, esize, vertical, slice)`.
    fn za_slice(&self, s: Slice, svl: VecLen) -> Zv {
        let b = s.size.bytes();
        if !s.vertical {
            return self.za_vector(s.tile + s.index * b, svl);
        }
        let mut result = Zv::ZERO;
        for e in 0..s.size.dim(svl) {
            let row = self.za_row(s.tile + e * b, svl);
            for i in 0..b {
                result.set_byte(e * b + i, row[s.index * b + i]);
            }
        }
        result
    }

    /// Arm `ZAslice{SVL}(tile, esize, vertical, slice) = value`.
    fn set_za_slice(&mut self, s: Slice, svl: VecLen, value: &Zv) {
        let b = s.size.bytes();
        if !s.vertical {
            self.set_za_vector(s.tile + s.index * b, svl, value);
            return;
        }
        for e in 0..s.size.dim(svl) {
            self.set_za_bytes(s.tile + e * b, svl, s.index * b, &value.as_bytes()[e * b..(e + 1) * b]);
        }
    }

    /// The [`TileOperands`] of `insn`, after `CheckStreamingSVEAndZAEnabled`.
    fn tile_operands(&self, insn: u32) -> Result<TileOperands, Fault> {
        let svl = self.check_streaming_sve_and_za_enabled()?;
        Ok(TileOperands {
            svl,
            rows: self.p(bits(insn, 12, 10), svl),
            cols: self.p(bits(insn, 15, 13), svl),
            zn: self.z(bits(insn, 9, 5), svl),
            tile: bits(insn, 1, 0) as usize,
        })
    }

    /// FMOPA and FMOPS of single-precision values (`fmopa_za_pp_zz_32`,
    /// `fmops_za_pp_zz_32`), BFMOPA and BFMOPS (`bfmopa_za32_pp_zz_`,
    /// `bfmops_za32_pp_zz_`) and FMOPA and FMOPS of half-precision values
    /// (`fmopa_za32_pp_zz_16`, `fmops_za32_pp_zz_16`): `1000000 op0 10 op1
    /// Zm Pm Pn Zn S 00 ZAda`, the product by `op0` and `op1` and
    /// subtracting by `S`. Element `[row, col]` of the 32-bit tile `ZAda`
    /// accumulates the product of row element `row` of `Zn` and column
    /// element `col` of `Zm`, or the sum of the products of their pairs,
    /// where `Pn` and `Pm` make them active; FMOPS, BFMOPS negate the active
    /// row elements. Every vector of the tile is written, whatever is
    /// active (`ZAtile`).
    pub(crate) fn exec_sme_fp_outer_product(&mut self, insn: u32) -> ExecResult {
        let product = match (bit(insn, 24), bit(insn, 21)) {
            (false, _) => FpProduct::Single,
            (true, false) => FpProduct::Pairs(PairFormat::BFloat16),
            (true, true) => FpProduct::Pairs(PairFormat::Half),
        };
        let subtract = bit(insn, 4);
        let op = self.tile_operands(insn)?;
        let zm = self.z(bits(insn, 20, 16), op.svl);
        let mut env = self.fp_env().with_default_nan();
        let dim = TileSize::S.dim(op.svl);
        let form = if subtract { FusedMulAdd::Fmsub } else { FusedMulAdd::Fmadd };
        for row in 0..dim {
            let index = op.tile + row * TileSize::S.bytes();
            let mut acc = self.za_vector(index, op.svl);
            match product {
                FpProduct::Single if op.rows.active(row, Esize::S) => {
                    let element = op.zn.get(row, Esize::S);
                    for col in (0..dim).filter(|&col| op.cols.active(col, Esize::S)) {
                        let sum = acc.get(col, Esize::S);
                        let result = fp_mul_add(form, FpType::Single, sum, element, zm.get(col, Esize::S), &mut env);
                        acc.set(col, Esize::S, result);
                    }
                }
                FpProduct::Single => {}
                FpProduct::Pairs(format) => {
                    let mut pair = Pair::of(&op.zn, &op.rows, row);
                    if subtract {
                        for k in (0..2).filter(|&k| pair.active[k]) {
                            pair.elements[k] = format.negate(pair.elements[k], &env);
                        }
                    }
                    for col in 0..dim {
                        let col_pair = Pair::of(&zm, &op.cols, col);
                        if pair.meets(&col_pair) {
                            let sum = acc.get(col, Esize::S) as u32;
                            let dot = format.dot(sum, pair.elements, col_pair.elements, &mut env);
                            acc.set(col, Esize::S, u64::from(dot));
                        }
                    }
                }
            }
            self.set_za_vector(index, op.svl, &acc);
        }
        Ok(Flow::Next)
    }

    /// SMOPA, SUMOPA, USMOPA, UMOPA, SMOPS, SUMOPS, USMOPS, UMOPS
    /// (`smopa_za_pp_zz_32` to `umops_za_pp_zz_32`: `1010000 u0 10 u1 Zm
    /// Pm Pn Zn S 00 ZAda`): element `[row, col]` of the 32-bit tile `ZAda`
    /// gains, or by `S` loses, the products of the four bytes of row
    /// element `row` of `Zn` (unsigned by `u0`) and column element `col` of
    /// `Zm` (unsigned by `u1`) whose bytes `Pn` and `Pm` both make active,
    /// modulo 2^32.
    pub(crate) fn exec_sme_int_outer_product(&mut self, insn: u32) -> ExecResult {
        let (n_sign, m_sign) = (Signedness::of(bit(insn, 24)), Signedness::of(bit(insn, 21)));
        let subtract = bit(insn, 4);
        let op = self.tile_operands(insn)?;
        let zm = self.z(bits(insn, 20, 16), op.svl);
        let dim = TileSize::S.dim(op.svl);
        for row in 0..dim {
            let index = op.tile + row * TileSize::S.bytes();
            let mut acc = self.za_vector(index, op.svl);
            // The row's four bytes, `None` where `Pn` makes one inactive.
            let row_bytes: [Option<i128>; 4] = core::array::from_fn(|k| {
                let a = 4 * row + k;
                op.rows.active(a, Esize::B).then(|| n_sign.int(op.zn.get(a, Esize::B), Esize::B))
            });
            for col in 0..dim {
                let mut sum = acc.get(col, Esize::S) as u32;
                for (k, element) in row_bytes.iter().enumerate() {
                    let b = 4 * col + k;
                    if let (Some(element), true) = (element, op.cols.active(b, Esize::B)) {
                        let product = (element * m_sign.int(zm.get(b, Esize::B), Esize::B)) as u32;
                        sum = if subtract {
                            sum.wrapping_sub(product)
                        } else {
                            sum.wrapping_add(product)
                        };
                    }
                }
                acc.set(col, Esize::S, u64::from(sum));
            }
            self.set_za_vector(index, op.svl, &acc);
        }
        Ok(Flow::Next)
    }

    /// MOVA (vector to tile slice) (`mova_za_p_rz_b` to `mova_za_p_rz_q`:
    /// `11000000 size 000 0 0 Q V Rs Pg Zn 0 ZAd:off`): the elements of
    /// `Zn` that `Pg` makes active, into the tile slice.
    pub(crate) fn exec_sme_move_to_slice(&mut self, insn: u32) -> ExecResult {
        let size = TileSize::of(bits(insn, 23, 22), bit(insn, 16));
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let mask = self.p(bits(insn, 12, 10), svl);
        let operand = self.z(bits(insn, 9, 5), svl);
        let slice = self.tile_slice(insn, 0, size, svl);
        let mut result = self.za_slice(slice, svl);
        merge_active(&mut result, &operand, &mask, size, svl);
        self.set_za_slice(slice, svl, &result);
        Ok(Flow::Next)
    }

    /// MOVA (tile slice to vector) (`mova_z_p_rza_b` to `mova_z_p_rza_q`:
    /// `11000000 size 000 0 1 Q V Rs Pg 0 ZAn:off Zd`): the elements of the
    /// tile slice that `Pg` makes active, into `Zd`.
    pub(crate) fn exec_sme_move_from_slice(&mut self, insn: u32) -> ExecResult {
        let size = TileSize::of(bits(insn, 23, 22), bit(insn, 16));
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let mask = self.p(bits(insn, 12, 10), svl);
        let slice = self.tile_slice(insn, 5, size, svl);
        let operand = self.za_slice(slice, svl);
        let d = bits(insn, 4, 0);
        let mut result = self.z(d, svl);
        merge_active(&mut result, &operand, &mask, size, svl);
        self.set_z(d, svl, &result);
        Ok(Flow::Next)
    }

    /// ZERO (`zero_za_i_`: `11000000 00001000 00000000 imm8`): each 64-bit
    /// tile whose bit of `imm8` is set, all zero.
    pub(crate) fn exec_sme_zero(&mut self, insn: u32) -> ExecResult {
        let svl = self.check_sme_and_za_enabled()?;
        let mask = bits(insn, 7, 0);
        let tiles = TileSize::D.bytes();
        for tile in (0..tiles).filter(|&t| mask & (1 << t) != 0) {
            for slice in 0..TileSize::D.dim(svl) {
                self.set_za_vector(tile + slice * tiles, svl, &Zv::ZERO);
            }
        }
        Ok(Flow::Next)
    }

    /// ADDHA and ADDVA of words (`addha_za_pp_z_32`, `addva_za_pp_z_32`:
    /// `11000000 10 010 00 V Pm Pn Zn 000 ZAda`): element `[row, col]` of the
    /// 32-bit tile `ZAda` gains element `col` of `Zn` (ADDHA, each
    /// horizontal slice the vector) or element `row` (ADDVA, by `V`), where
    /// `Pn` makes the row active and `Pm` the column, modulo 2^32.
    pub(crate) fn exec_sme_add_vector(&mut self, insn: u32) -> ExecResult {
        let vertical = bit(insn, 16);
        let op = self.tile_operands(insn)?;
        let dim = TileSize::S.dim(op.svl);
        for row in 0..dim {
            let index = op.tile + row * TileSize::S.bytes();
            let mut acc = self.za_vector(index, op.svl);
            for col in 0..dim {
                if op.rows.active(row, Esize::S) && op.cols.active(col, Esize::S) {
                    let element = op.zn.get(if vertical { row } else { col }, Esize::S);
                    acc.set(col, Esize::S, acc.get(col, Esize::S).wrapping_add(element));
                }
            }
            self.set_za_vector(index, op.svl, &acc);
        }
        Ok(Flow::Next)
    }

    /// LD1B, LD1H, LD1W, LD1D to a tile slice (`ld1b_za_p_rrr_` to
    /// `ld1d_za_p_rrr_`: `11100000 msz 0 Rm V Rs Pg Rn 0 ZAt:off`) and
    /// LD1Q (`ld1q_za_p_rrr_`: `11100001 110 Rm V Rs Pg Rn 0 ZAt`): element
    /// `e` from `X[n] + (X[m] + e) * mbytes` where `Pg` (P0 to P7) makes it
    /// active, zero where not.
    pub(crate) fn exec_sme_load_slice<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let size = TileSize::of(bits(insn, 23, 22), bit(insn, 24));
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let (mask, slice) = (self.p(bits(insn, 12, 10), svl), self.tile_slice(insn, 0, size, svl));
        let base = self.base_register(bits(insn, 9, 5))?;
        let mut addr = base.wrapping_add(self.x_or_zr(bits(insn, 20, 16)).wrapping_mul(size.bytes() as u64));
        let b = size.bytes();
        let mut result = Zv::ZERO;
        for e in 0..size.dim(svl) {
            if mask.bit(e * b) {
                let data = self.read_element(bus, addr, size)?;
                for (i, &byte) in data.to_le_bytes()[..b].iter().enumerate() {
                    result.set_byte(e * b + i, byte);
                }
            }
            addr = addr.wrapping_add(b as u64);
        }
        self.set_za_slice(slice, svl, &result);
        Ok(Flow::Next)
    }

    /// ST1B, ST1H, ST1W, ST1D from a tile slice (`st1b_za_p_rrr_` to
    /// `st1d_za_p_rrr_`: `11100000 msz 1 Rm V Rs Pg Rn 0 ZAt:off`) and
    /// ST1Q (`st1q_za_p_rrr_`: `11100001 111 Rm V Rs Pg Rn 0 ZAt`): each
    /// element `Pg` makes active, to where [`Cpu::exec_sme_load_slice`]
    /// loads it from.
    pub(crate) fn exec_sme_store_slice<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let size = TileSize::of(bits(insn, 23, 22), bit(insn, 24));
        let svl = self.check_streaming_sve_and_za_enabled()?;
        let (mask, slice) = (self.p(bits(insn, 12, 10), svl), self.tile_slice(insn, 0, size, svl));
        let base = self.base_register(bits(insn, 9, 5))?;
        let mut addr = base.wrapping_add(self.x_or_zr(bits(insn, 20, 16)).wrapping_mul(size.bytes() as u64));
        let source = self.za_slice(slice, svl);
        let b = size.bytes();
        for e in 0..size.dim(svl) {
            if mask.bit(e * b) {
                let mut raw = [0u8; 16];
                raw[..b].copy_from_slice(&source.as_bytes()[e * b..(e + 1) * b]);
                self.write_element(bus, addr, size, u128::from_le_bytes(raw))?;
            }
            addr = addr.wrapping_add(b as u64);
        }
        Ok(Flow::Next)
    }

    /// `Mem[addr, mbytes]` of one tile element: 16 bytes as a quadword
    /// access.
    fn read_element<B: PhysBus>(&mut self, bus: &mut B, addr: u64, size: TileSize) -> Result<u128, Fault> {
        match size {
            TileSize::Q => self.read_mem_u128(bus, addr, DataAccess::NORMAL),
            _ => Ok(u128::from(self.read_mem(bus, addr, size.log2_bytes(), DataAccess::NORMAL)?)),
        }
    }

    /// `Mem[addr, mbytes] = value` of one tile element.
    fn write_element<B: PhysBus>(&mut self, bus: &mut B, addr: u64, size: TileSize, value: u128) -> Result<(), Fault> {
        match size {
            TileSize::Q => self.write_mem_u128(bus, addr, value, DataAccess::NORMAL),
            _ => self.write_mem(bus, addr, size.log2_bytes(), value as u64, DataAccess::NORMAL),
        }
    }

    /// LDR and STR of a ZA array vector (`ldr_za_ri_`, `str_za_ri_`:
    /// `11100001 00 op 00000 0 Rv 000 Rn 0 off4`, a store by `op`): array
    /// vector `(UInt(Wv) + off4) MOD dim`, `Wv` one of W12 to W15 by `Rv`,
    /// at `X[n] + off4 * dim`, `dim` being the vector's `SVL DIV 8` bytes;
    /// each byte an access of its own, aligned when the base is to 16
    /// bytes, as LDR and STR of a vector register are.
    pub(crate) fn exec_sme_fill_spill<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let store = bit(insn, 21);
        let offset = u64::from(bits(insn, 3, 0));
        let svl = self.check_sme_and_za_enabled()?;
        let dim = svl.bytes() as u64;
        let wv = self.x_or_zr(12 + bits(insn, 14, 13)) as u32;
        let vector = ((u64::from(wv) + offset) % dim) as usize;
        let base = self.base_register(bits(insn, 9, 5))?;
        let addr = base.wrapping_add(offset * dim);
        let aligned = base % 16 == 0;
        if store {
            let source = self.za_vector(vector, svl);
            self.spill_bytes(bus, addr, aligned, &source.as_bytes()[..svl.bytes()])?;
        } else {
            let loaded = self.fill_bytes(bus, addr, aligned, svl.bytes())?;
            self.set_za_vector(vector, svl, &loaded);
        }
        Ok(Flow::Next)
    }
}

/// `result[e*:esize] = operand[e*:esize]` for each element `e` of `size`
/// that `mask` makes active (`ActivePredicateElement`, which reads the
/// predicate bit of the element's first byte).
fn merge_active(result: &mut Zv, operand: &Zv, mask: &Pv, size: TileSize, svl: VecLen) {
    let b = size.bytes();
    for e in (0..size.dim(svl)).filter(|&e| mask.bit(e * b)) {
        for i in e * b..(e + 1) * b {
            result.set_byte(i, operand.byte(i));
        }
    }
}
