//! The atomic memory operations, SWP, CAS and CASP (FEAT_LSE), and the RCpc
//! loads and stores: LDAPR (FEAT_LRCPC), LDAPUR and STLUR (FEAT_LRCPC2).
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09: the LDADD, LDCLR,
//! LDEOR, LDSET, LDSMAX, LDSMIN, LDUMAX, LDUMIN, SWP, CAS, CASP, LDAPR,
//! LDAPUR* and STLUR* pages and the shared `MemAtomic` and `MemAtomicInt`.
//! With one processor executing one instruction at a time every access is
//! single-copy atomic and in order, so the acquire and release semantics
//! (`A`, `R`) order nothing further here.

use super::exec::{bit, bits, sext, ExecResult, Flow};
use super::mmu::{DataAccess, DataDirection};
use super::Cpu;
use crate::bus::PhysBus;

/// Arm `MemAtomicOp`: what an atomic read-modify-write computes.
///
/// Closed (R5): the integer operations of FEAT_LSE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomicOp {
    Add,
    /// LDCLR: `old AND NOT operand`.
    Bic,
    Eor,
    /// LDSET: `old OR operand`.
    Orr,
    Smax,
    Smin,
    Umax,
    Umin,
    Swp,
    /// CAS and CASP, against this comparison value.
    Cas(u128),
}

/// Arm `MemAtomicInt` on `bits`-wide values: the value to write, or `None`
/// when a compare-and-swap's comparison fails and nothing is written (the
/// choice `Unpredictable_WRITEFAILEDCAS` leaves open). The operand is the
/// low `bits` of the register it came from (`X[s, datasize]`).
fn atomic_result(op: AtomicOp, old: u128, operand: u128, bits: u32) -> Option<u128> {
    let mask = if bits == 128 { u128::MAX } else { (1u128 << bits) - 1 };
    let (old, operand) = (old & mask, operand & mask);
    let signed = |v: u128| ((v << (128 - bits)) as i128) >> (128 - bits);
    let result = match op {
        AtomicOp::Add => old.wrapping_add(operand),
        AtomicOp::Bic => old & !operand,
        AtomicOp::Eor => old ^ operand,
        AtomicOp::Orr => old | operand,
        AtomicOp::Smax => {
            if signed(old) >= signed(operand) {
                old
            } else {
                operand
            }
        }
        AtomicOp::Smin => {
            if signed(old) <= signed(operand) {
                old
            } else {
                operand
            }
        }
        AtomicOp::Umax => old.max(operand),
        AtomicOp::Umin => old.min(operand),
        AtomicOp::Swp => operand,
        AtomicOp::Cas(compare) => {
            if old != compare & mask {
                return None;
            }
            operand
        }
    };
    Some(result & mask)
}

impl Cpu {
    /// Arm `MemAtomic`: an atomic read-modify-write of `1 << log2` bytes at
    /// `va` (`log2` in 0..=4), returning the value read. The alignment rule
    /// is the atomic one, and the access reads and writes
    /// (`CreateAccDescAtomicOp`): a fault reports WnR set only when it is
    /// the write that the page refuses.
    fn atomic_rmw<B: PhysBus>(&mut self, bus: &mut B, va: u64, log2: u32, op: AtomicOp, operand: u128) -> Result<u128, super::exec::Fault> {
        let pa = self.translate_for(bus, va, 1 << log2, DataDirection::ReadWrite, DataAccess::ATOMIC)?;
        let old = self.read_translated(bus, va, pa, log2)?;
        if let Some(new) = atomic_result(op, old, operand, 8 << log2) {
            self.write_translated(bus, va, pa, log2, new)?;
        }
        Ok(old)
    }

    /// Arm group `memop` as this processor has it: the atomic memory
    /// operations and SWP (FEAT_LSE), and LDAPR (FEAT_LRCPC):
    /// `size 111 0 00 A R 1 Rs o3 opc 00 Rn Rt`. `Rs` is the operand, and
    /// `Rt` receives the old value, zero-extended to 32 bits below 64 (XZR
    /// discards it: the STADD-style aliases).
    pub(crate) fn exec_memop<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32) -> ExecResult {
        let size = bits(insn, 31, 30);
        let rs = bits(insn, 20, 16);
        let rn = bits(insn, 9, 5);
        let rt = bits(insn, 4, 0);
        let op = match (bit(insn, 15), bits(insn, 14, 12)) {
            // LDAPR, LDAPRB, LDAPRH: a load-acquire RCpc.
            (true, 0b100) => {
                let address = self.base_register(rn)?;
                let value = self.read_mem(bus, address, size, DataAccess::ORDERED)?;
                self.set_x_or_zr(rt, value);
                return Ok(Flow::Next);
            }
            (true, 0b000) => AtomicOp::Swp,
            // The decoder hands this class no other `o3 = 1` encoding.
            (true, _) => return self.undefined(pc, insn),
            (false, 0b000) => AtomicOp::Add,
            (false, 0b001) => AtomicOp::Bic,
            (false, 0b010) => AtomicOp::Eor,
            (false, 0b011) => AtomicOp::Orr,
            (false, 0b100) => AtomicOp::Smax,
            (false, 0b101) => AtomicOp::Smin,
            (false, 0b110) => AtomicOp::Umax,
            (false, _) => AtomicOp::Umin,
        };
        let address = self.base_register(rn)?;
        let operand = u128::from(self.x_or_zr(rs));
        let old = self.atomic_rmw(bus, address, size, op, operand)?;
        self.set_x_or_zr(rt, old as u64);
        Ok(Flow::Next)
    }

    /// CAS, CASA, CASL, CASAL of every size (Arm group `comswap`):
    /// `size 001000 1 A 1 Rs R 11111 Rn Rt`. `Rs` is the comparison value
    /// and receives the value read, zero-extended to 32 bits below 64; `Rt`
    /// is the value written when they are equal.
    pub(crate) fn exec_cas<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let size = bits(insn, 31, 30);
        let rs = bits(insn, 20, 16);
        let rn = bits(insn, 9, 5);
        let rt = bits(insn, 4, 0);
        let address = self.base_register(rn)?;
        let compare = u128::from(self.x_or_zr(rs));
        let new = u128::from(self.x_or_zr(rt));
        let old = self.atomic_rmw(bus, address, size, AtomicOp::Cas(compare), new)?;
        self.set_x_or_zr(rs, old as u64);
        Ok(Flow::Next)
    }

    /// CASP, CASPA, CASPL, CASPAL (Arm group `comswappr`):
    /// `0 sz 001000 0 A 1 Rs R 11111 Rn Rt`. The pairs `Rs:Rs+1` and
    /// `Rt:Rt+1`, the first register of each the low half, compare and swap
    /// 64 or 128 bits; an odd `Rs` or `Rt` is UNDEFINED. `Rs+1` and `Rt+1`
    /// are XZR when `Rs` or `Rt` is 30.
    pub(crate) fn exec_casp<B: PhysBus>(&mut self, bus: &mut B, pc: u64, insn: u32) -> ExecResult {
        // Each register of a pair is 2^half_log2 bytes: 4 or 8.
        let half_log2 = 2 + bits(insn, 30, 30);
        let rs = bits(insn, 20, 16);
        let rn = bits(insn, 9, 5);
        let rt = bits(insn, 4, 0);
        if rs & 1 == 1 || rt & 1 == 1 {
            return self.undefined(pc, insn);
        }
        let half_bits = 8 << half_log2;
        let half = (1u128 << half_bits) - 1;
        let pair = |cpu: &Cpu, r: u32| (u128::from(cpu.x_or_zr(r)) & half) | ((u128::from(cpu.x_or_zr(r + 1)) & half) << half_bits);
        let compare = pair(self, rs);
        let new = pair(self, rt);
        let address = self.base_register(rn)?;
        let old = self.atomic_rmw(bus, address, half_log2 + 1, AtomicOp::Cas(compare), new)?;
        self.set_x_or_zr(rs, (old & half) as u64);
        self.set_x_or_zr(rs + 1, ((old >> half_bits) & half) as u64);
        Ok(Flow::Next)
    }

    /// LDAPUR*, STLUR* (Arm group `ldapstl_unscaled`, FEAT_LRCPC2):
    /// `size 011001 opc 0 imm9 00 Rn Rt`, at `Rn|SP` plus the signed
    /// unscaled `imm9`. `opc` = 00 stores, 01 loads zero-extended, 10 loads
    /// sign-extended to 64 bits and 11 to 32.
    pub(crate) fn exec_ldapstl_unscaled<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let size = bits(insn, 31, 30);
        let opc = bits(insn, 23, 22);
        let rn = bits(insn, 9, 5);
        let rt = bits(insn, 4, 0);
        let offset = sext(u64::from(bits(insn, 20, 12)), 9);
        let address = self.base_register(rn)?.wrapping_add(offset);
        if opc == 0b00 {
            let value = self.x_or_zr(rt);
            self.write_mem(bus, address, size, value, DataAccess::ORDERED)?;
            return Ok(Flow::Next);
        }
        let value = self.read_mem(bus, address, size, DataAccess::ORDERED)?;
        let width = 8 << size;
        let result = match opc {
            0b01 => value,
            0b10 => sext(value, width),
            _ => sext(value, width) & 0xFFFF_FFFF,
        };
        self.set_x_or_zr(rt, result);
        Ok(Flow::Next)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/atomic.rs"]
mod tests;
