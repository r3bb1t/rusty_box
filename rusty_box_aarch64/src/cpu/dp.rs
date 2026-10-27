//! Integer data processing and branches (Arm ARM C4.1.2–C4.1.5, C6.2).
//!
//! Each handler covers one encoding class and decodes its own fields; the
//! comment on each names the encodings it accepts and the pseudocode helper
//! it follows.

use super::exec::{bit, bits, sext, ExecResult, Flow, Nzcv};
use super::Cpu;

/// `AddWithCarry` result: the value and the flags it would set.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AddResult {
    pub(crate) value: u64,
    pub(crate) n: bool,
    pub(crate) z: bool,
    pub(crate) c: bool,
    pub(crate) v: bool,
}

impl AddResult {
    /// The flags the result would set.
    #[inline(always)]
    pub(crate) const fn nzcv(&self) -> Nzcv {
        Nzcv {
            n: self.n,
            z: self.z,
            c: self.c,
            v: self.v,
        }
    }
}

/// Arm ARM `AddWithCarry(x, y, carry_in)` at 64 or 32 bits.
#[inline]
pub(crate) fn add_with_carry(x: u64, y: u64, carry: bool, sf: bool) -> AddResult {
    if sf {
        let unsigned = u128::from(x) + u128::from(y) + u128::from(carry);
        let value = unsigned as u64;
        let signed = i128::from(x as i64) + i128::from(y as i64) + i128::from(carry);
        AddResult {
            value,
            n: value >> 63 != 0,
            z: value == 0,
            c: unsigned >> 64 != 0,
            v: signed != i128::from(value as i64),
        }
    } else {
        let (x, y) = (x as u32, y as u32);
        let unsigned = u64::from(x) + u64::from(y) + u64::from(carry);
        let value = unsigned as u32;
        let signed = i64::from(x as i32) + i64::from(y as i32) + i64::from(carry);
        AddResult {
            value: u64::from(value),
            n: value >> 31 != 0,
            z: value == 0,
            c: unsigned >> 32 != 0,
            v: signed != i64::from(value as i32),
        }
    }
}

#[inline(always)]
pub(crate) const fn datasize(sf: bool) -> u32 {
    if sf {
        64
    } else {
        32
    }
}

/// All ones in the low `n` bits (`n` up to 64).
#[inline(always)]
pub(crate) const fn ones(n: u32) -> u64 {
    if n >= 64 {
        u64::MAX
    } else {
        (1u64 << n) - 1
    }
}

/// Rotate the low `size` bits of `value` right by `amount` (`amount < size`).
#[inline]
pub(crate) const fn ror(value: u64, amount: u32, size: u32) -> u64 {
    let value = value & ones(size);
    if amount == 0 {
        value
    } else {
        ((value >> amount) | (value << (size - amount))) & ones(size)
    }
}

/// Repeat the low `esize` bits of `element` across `size` bits.
#[inline]
pub(crate) const fn replicate(element: u64, esize: u32, size: u32) -> u64 {
    let element = element & ones(esize);
    let mut out = 0u64;
    let mut pos = 0;
    while pos < size {
        out |= element << pos;
        pos += esize;
    }
    out
}

/// Arm ARM `ShiftReg`: shift type 0 LSL, 1 LSR, 2 ASR, 3 ROR, by `amount`
/// (`amount < size`).
#[inline]
pub(crate) const fn shift_reg(value: u64, shift: u32, amount: u32, size: u32) -> u64 {
    let value = value & ones(size);
    match shift {
        0 => {
            if amount >= 64 {
                0
            } else {
                (value << amount) & ones(size)
            }
        }
        1 => {
            if amount >= 64 {
                0
            } else {
                value >> amount
            }
        }
        2 => {
            let signed = sext(value, size) as i64;
            let shifted = if amount >= 64 {
                signed >> 63
            } else {
                signed >> amount
            };
            (shifted as u64) & ones(size)
        }
        _ => ror(value, amount % size, size),
    }
}

/// Arm ARM `ExtendReg`: extend option (UXTB..SXTX), then shift left.
#[inline]
pub(crate) const fn extend_reg(value: u64, option: u32, shift: u32, size: u32) -> u64 {
    let len = 8 << (option & 3);
    let extended = if option & 4 != 0 {
        sext(value & ones(len), len)
    } else {
        value & ones(len)
    };
    (extended << shift) & ones(size)
}

/// The masks of Arm ARM `DecodeBitMasks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BitMasks {
    pub(crate) wmask: u64,
    pub(crate) tmask: u64,
}

/// Arm ARM `DecodeBitMasks(immN, imms, immr, immediate)` for a `size`-bit
/// operation. `None` where the pseudocode says UNDEFINED.
pub(crate) fn decode_bit_masks(
    n: bool,
    imms: u32,
    immr: u32,
    immediate: bool,
    size: u32,
) -> Option<BitMasks> {
    let combined = (u32::from(n) << 6) | (!imms & 0x3F);
    if combined == 0 {
        return None;
    }
    let len = 31 - combined.leading_zeros();
    if len < 1 {
        return None;
    }
    let esize = 1u32 << len;
    if esize > size {
        return None;
    }
    let levels = (1u32 << len) - 1;
    if immediate && (imms & levels) == levels {
        return None;
    }
    let s = imms & levels;
    let r = immr & levels;
    let d = s.wrapping_sub(r) & levels;
    let welem = ones(s + 1);
    let telem = ones(d + 1);
    Some(BitMasks {
        wmask: replicate(ror(welem, r, esize), esize, size),
        tmask: replicate(telem, esize, size),
    })
}

impl Cpu {
    /// Set NZCV to the flags of an `AddWithCarry` result.
    #[inline]
    pub(super) fn set_flags(&mut self, r: AddResult) {
        self.pstate.set_flags(r.nzcv());
    }

    /// Flags of a logical result: N and Z from the value, C and V cleared.
    #[inline]
    pub(super) fn set_logic_flags(&mut self, value: u64, sf: bool) {
        let top = datasize(sf) - 1;
        self.pstate.n = (value >> top) & 1 != 0;
        self.pstate.z = value & ones(datasize(sf)) == 0;
        self.pstate.c = false;
        self.pstate.v = false;
    }

    /// ADD/ADDS/SUB/SUBS (immediate): `sf op S 100010 sh imm12 Rn Rd`. Bit
    /// 23 is 0 in every word that reaches here (Arm's `ADD_32_addsub_imm`
    /// and its siblings fix it; `shift = 1x` was a ReservedValue in the
    /// ARMv8.0 decode).
    pub(crate) fn exec_addsub_imm(&mut self, _pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let sub = bit(insn, 30);
        let set_flags = bit(insn, 29);
        let imm = u64::from(bits(insn, 21, 10)) << if bit(insn, 22) { 12 } else { 0 };
        let rn = bits(insn, 9, 5);
        let rd = bits(insn, 4, 0);
        let op1 = self.x_or_sp(rn);
        let r = if sub {
            add_with_carry(op1, !imm, true, sf)
        } else {
            add_with_carry(op1, imm, false, sf)
        };
        if set_flags {
            self.set_flags(r);
            self.set_x_or_zr_sized(rd, r.value, sf);
        } else {
            self.set_x_or_sp_sized(rd, r.value, sf);
        }
        Ok(Flow::Next)
    }

    /// ADD/ADDS/SUB/SUBS (shifted register): `sf op S 01011 shift 0 Rm imm6 Rn Rd`.
    pub(crate) fn exec_addsub_shift(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let shift = bits(insn, 23, 22);
        let amount = bits(insn, 15, 10);
        if shift == 3 || (!sf && amount >= 32) {
            return self.undefined(pc, insn);
        }
        let op2 = shift_reg(
            self.x_or_zr(bits(insn, 20, 16)),
            shift,
            amount,
            datasize(sf),
        );
        let op1 = self.x_or_zr(bits(insn, 9, 5));
        let r = if bit(insn, 30) {
            add_with_carry(op1, !op2, true, sf)
        } else {
            add_with_carry(op1, op2, false, sf)
        };
        if bit(insn, 29) {
            self.set_flags(r);
        }
        self.set_x_or_zr_sized(bits(insn, 4, 0), r.value, sf);
        Ok(Flow::Next)
    }

    /// ADD/ADDS/SUB/SUBS (extended register): `sf op S 01011 00 1 Rm option imm3 Rn Rd`.
    pub(crate) fn exec_addsub_ext(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let amount = bits(insn, 12, 10);
        if amount > 4 || bits(insn, 23, 22) != 0 {
            return self.undefined(pc, insn);
        }
        let op2 = extend_reg(
            self.x_or_zr(bits(insn, 20, 16)),
            bits(insn, 15, 13),
            amount,
            datasize(sf),
        );
        let op1 = self.x_or_sp(bits(insn, 9, 5));
        let r = if bit(insn, 30) {
            add_with_carry(op1, !op2, true, sf)
        } else {
            add_with_carry(op1, op2, false, sf)
        };
        let rd = bits(insn, 4, 0);
        if bit(insn, 29) {
            self.set_flags(r);
            self.set_x_or_zr_sized(rd, r.value, sf);
        } else {
            self.set_x_or_sp_sized(rd, r.value, sf);
        }
        Ok(Flow::Next)
    }

    /// ADC/ADCS/SBC/SBCS: `sf op S 11010000 Rm 000000 Rn Rd`.
    pub(crate) fn exec_addsub_carry(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bits(insn, 15, 10) != 0 {
            return self.undefined(pc, insn);
        }
        let sf = bit(insn, 31);
        let op1 = self.x_or_zr(bits(insn, 9, 5));
        let rm = self.x_or_zr(bits(insn, 20, 16));
        let op2 = if bit(insn, 30) { !rm } else { rm };
        let r = add_with_carry(op1, op2, self.pstate.c, sf);
        if bit(insn, 29) {
            self.set_flags(r);
        }
        self.set_x_or_zr_sized(bits(insn, 4, 0), r.value, sf);
        Ok(Flow::Next)
    }

    /// AND/ORR/EOR/ANDS (immediate): `sf opc 100100 N immr imms Rn Rd`.
    pub(crate) fn exec_logic_imm(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let n = bit(insn, 22);
        if !sf && n {
            return self.undefined(pc, insn);
        }
        let Some(masks) = decode_bit_masks(
            n,
            bits(insn, 15, 10),
            bits(insn, 21, 16),
            true,
            datasize(sf),
        ) else {
            return self.undefined(pc, insn);
        };
        let op1 = self.x_or_zr(bits(insn, 9, 5));
        let imm = masks.wmask;
        let opc = bits(insn, 30, 29);
        let value = match opc {
            0 | 3 => op1 & imm,
            1 => op1 | imm,
            _ => op1 ^ imm,
        } & ones(datasize(sf));
        let rd = bits(insn, 4, 0);
        if opc == 3 {
            self.set_logic_flags(value, sf);
            self.set_x_or_zr(rd, value);
        } else {
            self.set_x_or_sp(rd, value);
        }
        Ok(Flow::Next)
    }

    /// AND/BIC/ORR/ORN/EOR/EON/ANDS/BICS (shifted register):
    /// `sf opc 01010 shift N Rm imm6 Rn Rd`.
    pub(crate) fn exec_logic_shift(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let amount = bits(insn, 15, 10);
        if !sf && amount >= 32 {
            return self.undefined(pc, insn);
        }
        let size = datasize(sf);
        let mut op2 = shift_reg(
            self.x_or_zr(bits(insn, 20, 16)),
            bits(insn, 23, 22),
            amount,
            size,
        );
        if bit(insn, 21) {
            op2 = !op2 & ones(size);
        }
        let op1 = self.x_or_zr(bits(insn, 9, 5)) & ones(size);
        let opc = bits(insn, 30, 29);
        let value = match opc {
            0 | 3 => op1 & op2,
            1 => op1 | op2,
            _ => op1 ^ op2,
        };
        if opc == 3 {
            self.set_logic_flags(value, sf);
        }
        self.set_x_or_zr(bits(insn, 4, 0), value);
        Ok(Flow::Next)
    }

    /// MOVN/MOVZ/MOVK: `sf opc 100101 hw imm16 Rd`.
    pub(crate) fn exec_movewide(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let opc = bits(insn, 30, 29);
        let hw = bits(insn, 22, 21);
        if opc == 1 || (!sf && hw >= 2) {
            return self.undefined(pc, insn);
        }
        let pos = hw * 16;
        let imm = u64::from(bits(insn, 20, 5)) << pos;
        let rd = bits(insn, 4, 0);
        let value = match opc {
            0 => !imm,
            2 => imm,
            _ => (self.x_or_zr(rd) & !(0xFFFFu64 << pos)) | imm,
        };
        self.set_x_or_zr_sized(rd, value, sf);
        Ok(Flow::Next)
    }

    /// ADR/ADRP: `op immlo 10000 immhi Rd`.
    pub(crate) fn exec_pcrel(&mut self, pc: u64, insn: u32) -> ExecResult {
        let imm = (u64::from(bits(insn, 23, 5)) << 2) | u64::from(bits(insn, 30, 29));
        let value = if bit(insn, 31) {
            (pc & !0xFFF).wrapping_add(sext(imm << 12, 33))
        } else {
            pc.wrapping_add(sext(imm, 21))
        };
        self.set_x_or_zr(bits(insn, 4, 0), value);
        Ok(Flow::Next)
    }

    /// SBFM/BFM/UBFM: `sf opc 100110 N immr imms Rn Rd`.
    pub(crate) fn exec_bitfield(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let opc = bits(insn, 30, 29);
        let n = bit(insn, 22);
        let immr = bits(insn, 21, 16);
        let imms = bits(insn, 15, 10);
        if opc == 3 || sf != n || (!sf && (immr >= 32 || imms >= 32)) {
            return self.undefined(pc, insn);
        }
        let size = datasize(sf);
        let Some(masks) = decode_bit_masks(n, imms, immr, false, size) else {
            return self.undefined(pc, insn);
        };
        let rd = bits(insn, 4, 0);
        let src = self.x_or_zr(bits(insn, 9, 5)) & ones(size);
        let dst = if opc == 1 {
            self.x_or_zr(rd) & ones(size)
        } else {
            0
        };
        let bot = (dst & !masks.wmask) | (ror(src, immr, size) & masks.wmask);
        let top = if opc == 0 {
            if (src >> imms) & 1 != 0 {
                ones(size)
            } else {
                0
            }
        } else {
            dst
        };
        let value = ((top & !masks.tmask) | (bot & masks.tmask)) & ones(size);
        self.set_x_or_zr(rd, value);
        Ok(Flow::Next)
    }

    /// EXTR: `sf 00 100111 N 0 Rm imms Rn Rd`.
    pub(crate) fn exec_extract(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let lsb = bits(insn, 15, 10);
        if sf != bit(insn, 22) || (!sf && lsb >= 32) {
            return self.undefined(pc, insn);
        }
        let size = datasize(sf);
        let hi = u128::from(self.x_or_zr(bits(insn, 9, 5)) & ones(size));
        let lo = u128::from(self.x_or_zr(bits(insn, 20, 16)) & ones(size));
        let concat = (hi << size) | lo;
        let value = ((concat >> lsb) as u64) & ones(size);
        self.set_x_or_zr(bits(insn, 4, 0), value);
        Ok(Flow::Next)
    }

    /// B.cond: `0101010 0 imm19 0 cond`.
    pub(crate) fn exec_condbranch(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 4) {
            return self.undefined(pc, insn);
        }
        if self.condition_holds(bits(insn, 3, 0)) {
            Ok(Flow::Jump(
                pc.wrapping_add(sext(u64::from(bits(insn, 23, 5)) << 2, 21)),
            ))
        } else {
            Ok(Flow::Next)
        }
    }

    /// CBZ/CBNZ: `sf 011010 op imm19 Rt`.
    pub(crate) fn exec_compbranch(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let value = self.x_or_zr(bits(insn, 4, 0)) & ones(datasize(sf));
        if (value == 0) != bit(insn, 24) {
            Ok(Flow::Jump(
                pc.wrapping_add(sext(u64::from(bits(insn, 23, 5)) << 2, 21)),
            ))
        } else {
            Ok(Flow::Next)
        }
    }

    /// TBZ/TBNZ: `b5 011011 op b40 imm14 Rt`.
    pub(crate) fn exec_testbranch(&mut self, pc: u64, insn: u32) -> ExecResult {
        let pos = (u32::from(bit(insn, 31)) << 5) | bits(insn, 23, 19);
        let set = (self.x_or_zr(bits(insn, 4, 0)) >> pos) & 1 != 0;
        if set == bit(insn, 24) {
            Ok(Flow::Jump(
                pc.wrapping_add(sext(u64::from(bits(insn, 18, 5)) << 2, 16)),
            ))
        } else {
            Ok(Flow::Next)
        }
    }

    /// B/BL: `op 00101 imm26`.
    pub(crate) fn exec_branch_imm(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 31) {
            self.x[30] = pc.wrapping_add(4);
        }
        Ok(Flow::Jump(
            pc.wrapping_add(sext(u64::from(bits(insn, 25, 0)) << 2, 28)),
        ))
    }

    /// BR/BLR/RET/ERET/DRPS: `1101011 opc op2 op3 Rn op4`. The pointer
    /// authentication forms are a separate feature set and never reach here.
    pub(crate) fn exec_branch_reg(&mut self, pc: u64, insn: u32) -> ExecResult {
        let opc = bits(insn, 24, 21);
        let rn = bits(insn, 9, 5);
        if bits(insn, 20, 16) != 0x1F || bits(insn, 15, 10) != 0 || bits(insn, 4, 0) != 0 {
            return self.undefined(pc, insn);
        }
        match opc {
            0 | 2 => Ok(Flow::Jump(self.x_or_zr(rn))),
            1 => {
                let target = self.x_or_zr(rn);
                self.x[30] = pc.wrapping_add(4);
                Ok(Flow::Jump(target))
            }
            4 if rn == 0x1F => {
                if self.pstate.el == super::El::El0 {
                    return self.undefined(pc, insn);
                }
                Ok(Flow::Jump(self.exception_return()))
            }
            // DRPS is UNDEFINED outside Debug state; nothing else is defined.
            _ => self.undefined(pc, insn),
        }
    }

    /// CCMN/CCMP (register and immediate):
    /// `sf op 1 11010010 Rm|imm5 cond o2 0 Rn o3 nzcv`.
    pub(crate) fn exec_condcmp(&mut self, pc: u64, insn: u32) -> ExecResult {
        if !bit(insn, 29) || bit(insn, 10) || bit(insn, 4) {
            return self.undefined(pc, insn);
        }
        let sf = bit(insn, 31);
        if self.condition_holds(bits(insn, 15, 12)) {
            let op1 = self.x_or_zr(bits(insn, 9, 5));
            let op2 = if bit(insn, 11) {
                u64::from(bits(insn, 20, 16))
            } else {
                self.x_or_zr(bits(insn, 20, 16))
            };
            let r = if bit(insn, 30) {
                add_with_carry(op1, !op2, true, sf)
            } else {
                add_with_carry(op1, op2, false, sf)
            };
            self.set_flags(r);
        } else {
            self.pstate.set_nzcv(u64::from(bits(insn, 3, 0)) << 28);
        }
        Ok(Flow::Next)
    }

    /// CSEL/CSINC/CSINV/CSNEG: `sf op 0 11010100 Rm cond op2 Rn Rd`.
    pub(crate) fn exec_condsel(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 29) || bit(insn, 11) {
            return self.undefined(pc, insn);
        }
        let sf = bit(insn, 31);
        let value = if self.condition_holds(bits(insn, 15, 12)) {
            self.x_or_zr(bits(insn, 9, 5))
        } else {
            let m = self.x_or_zr(bits(insn, 20, 16));
            match (bit(insn, 30), bit(insn, 10)) {
                (false, false) => m,
                (false, true) => m.wrapping_add(1),
                (true, false) => !m,
                (true, true) => (!m).wrapping_add(1),
            }
        };
        self.set_x_or_zr_sized(bits(insn, 4, 0), value, sf);
        Ok(Flow::Next)
    }

    /// RBIT/REV16/REV32/REV/CLZ/CLS: `sf 1 0 11010110 00000 opcode Rn Rd`.
    pub(crate) fn exec_dp1(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        if bit(insn, 29) || bits(insn, 20, 16) != 0 {
            return self.undefined(pc, insn);
        }
        let x = self.x_or_zr(bits(insn, 9, 5));
        let value = match (bits(insn, 15, 10), sf) {
            (0, true) => x.reverse_bits(),
            (0, false) => u64::from((x as u32).reverse_bits()),
            (1, _) => {
                let swapped =
                    ((x & 0x00FF_00FF_00FF_00FF) << 8) | ((x >> 8) & 0x00FF_00FF_00FF_00FF);
                swapped & ones(datasize(sf))
            }
            (2, true) => {
                u64::from((x as u32).swap_bytes())
                    | (u64::from(((x >> 32) as u32).swap_bytes()) << 32)
            }
            (2, false) => u64::from((x as u32).swap_bytes()),
            (3, true) => x.swap_bytes(),
            (4, true) => u64::from(x.leading_zeros()),
            (4, false) => u64::from((x as u32).leading_zeros()),
            (5, true) => {
                let sign = ((x as i64) >> 63) as u64;
                u64::from((x ^ sign).leading_zeros() - 1)
            }
            (5, false) => {
                let w = x as u32;
                let sign = ((w as i32) >> 31) as u32;
                u64::from((w ^ sign).leading_zeros() - 1)
            }
            _ => return self.undefined(pc, insn),
        };
        self.set_x_or_zr_sized(bits(insn, 4, 0), value, sf);
        Ok(Flow::Next)
    }

    /// UDIV/SDIV/LSLV/LSRV/ASRV/RORV: `sf 0 0 11010110 Rm opcode Rn Rd`.
    pub(crate) fn exec_dp2(&mut self, pc: u64, insn: u32) -> ExecResult {
        if bit(insn, 29) {
            return self.undefined(pc, insn);
        }
        let sf = bit(insn, 31);
        let size = datasize(sf);
        let n = self.x_or_zr(bits(insn, 9, 5)) & ones(size);
        let m = self.x_or_zr(bits(insn, 20, 16)) & ones(size);
        let value = match bits(insn, 15, 10) {
            // Division by zero gives zero (Arm ARM UDIV/SDIV pseudocode).
            2 => n.checked_div(m).unwrap_or(0),
            3 => {
                if m == 0 {
                    0
                } else if sf {
                    (n as i64).wrapping_div(m as i64) as u64
                } else {
                    u64::from((n as i32).wrapping_div(m as i32) as u32)
                }
            }
            op @ 8..=11 => shift_reg(n, op - 8, (m % u64::from(size)) as u32, size),
            _ => return self.undefined(pc, insn),
        };
        self.set_x_or_zr_sized(bits(insn, 4, 0), value, sf);
        Ok(Flow::Next)
    }

    /// MADD/MSUB/SMADDL/SMSUBL/SMULH/UMADDL/UMSUBL/UMULH:
    /// `sf op54 11011 op31 Rm o0 Ra Rn Rd`.
    pub(crate) fn exec_dp3(&mut self, pc: u64, insn: u32) -> ExecResult {
        let sf = bit(insn, 31);
        let op31 = bits(insn, 23, 21);
        let o0 = bit(insn, 15);
        if bits(insn, 30, 29) != 0 || (!sf && op31 != 0) {
            return self.undefined(pc, insn);
        }
        let n = self.x_or_zr(bits(insn, 9, 5));
        let m = self.x_or_zr(bits(insn, 20, 16));
        let a = self.x_or_zr(bits(insn, 14, 10));
        let value = match (op31, o0) {
            (0, _) => {
                let product = n.wrapping_mul(m);
                if o0 {
                    a.wrapping_sub(product)
                } else {
                    a.wrapping_add(product)
                }
            }
            (1, _) | (5, _) => {
                let product = if op31 == 1 {
                    ((n as i32 as i64).wrapping_mul(m as i32 as i64)) as u64
                } else {
                    u64::from(n as u32).wrapping_mul(u64::from(m as u32))
                };
                if o0 {
                    a.wrapping_sub(product)
                } else {
                    a.wrapping_add(product)
                }
            }
            (2, false) => ((i128::from(n as i64) * i128::from(m as i64)) >> 64) as u64,
            (6, false) => ((u128::from(n) * u128::from(m)) >> 64) as u64,
            _ => return self.undefined(pc, insn),
        };
        self.set_x_or_zr_sized(bits(insn, 4, 0), value, sf);
        Ok(Flow::Next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `DecodeBitMasks` against encodings whose immediates are known from
    /// the Arm ARM examples and from assembler output.
    #[test]
    fn logical_immediates_decode_to_their_assembler_values() {
        // and x0, x1, #0xff  -> N=1 immr=0 imms=7
        assert_eq!(
            decode_bit_masks(true, 7, 0, true, 64).map(|m| m.wmask),
            Some(0xFF)
        );
        // orr w0, w1, #0x80000000 -> N=0 immr=1 imms=0
        assert_eq!(
            decode_bit_masks(false, 0, 1, true, 32).map(|m| m.wmask),
            Some(0x8000_0000)
        );
        // and x0, x0, #0x5555555555555555 -> N=0 immr=0 imms=0b111100
        assert_eq!(
            decode_bit_masks(false, 0b111100, 0, true, 64).map(|m| m.wmask),
            Some(0x5555_5555_5555_5555)
        );
        // and x0, x0, #0xfffffffffffffff0 -> N=1 immr=60 imms=59
        assert_eq!(
            decode_bit_masks(true, 59, 60, true, 64).map(|m| m.wmask),
            Some(0xFFFF_FFFF_FFFF_FFF0)
        );
        // all-ones is not encodable as an immediate
        assert_eq!(decode_bit_masks(true, 63, 0, true, 64), None);
    }

    /// LSL #1 is UBFM immr=63 imms=62; LSR #4 is UBFM immr=4 imms=63. The
    /// masks are the pseudocode's, worked by hand: UBFM's result is
    /// `ROR(src, R) & wmask & tmask`.
    #[test]
    fn bitfield_masks_express_the_shift_aliases() {
        let lsl = decode_bit_masks(true, 62, 63, false, 64).map(|m| (m.wmask, m.tmask));
        assert_eq!(lsl, Some((0xFFFF_FFFF_FFFF_FFFE, u64::MAX)));
        let lsr = decode_bit_masks(true, 63, 4, false, 64).map(|m| (m.wmask, m.tmask));
        assert_eq!(lsr, Some((u64::MAX, 0x0FFF_FFFF_FFFF_FFFF)));
        let src = 0x8000_0000_0000_0011u64;
        let ubfm = |m: BitMasks, r: u32| ror(src, r, 64) & m.wmask & m.tmask;
        assert_eq!(
            decode_bit_masks(true, 62, 63, false, 64).map(|m| ubfm(m, 63)),
            Some(src << 1)
        );
        assert_eq!(
            decode_bit_masks(true, 63, 4, false, 64).map(|m| ubfm(m, 4)),
            Some(src >> 4)
        );
    }

    #[test]
    fn add_with_carry_sets_flags_as_the_pseudocode() {
        let r = add_with_carry(0x7FFF_FFFF, 1, false, false);
        assert!(r.v && r.n && !r.c && !r.z);
        let r = add_with_carry(0, !0u64, true, true); // 0 - 0 via SUBS
        assert!(r.z && r.c && !r.n && !r.v);
        let r = add_with_carry(1, !2u64, true, true); // 1 - 2
        assert!(r.n && !r.c && !r.z);
    }
}
