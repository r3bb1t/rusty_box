//! The instructions on FEAT_SME2's ZT0, the 512-bit lookup table register:
//! ZERO {ZT0}, LDR and STR of ZT0, and LUTI2 and LUTI4, which look up
//! vectors of 2-bit or 4-bit indexes in its sixteen 32-bit entries. MOVT
//! between ZT0 and a general register is defined only in Debug state,
//! which this processor never enters, so it decodes UNDEFINED
//! (`Encoding::class`).
//!
//! ZERO, LDR and STR execute in or out of Streaming SVE mode
//! (`CheckSMEEnabled`), LUTI2 and LUTI4 only in it
//! (`CheckStreamingSVEEnabled`), and all of them only while ZA storage is
//! enabled and `SMCR_EL1.EZT0` gives access to ZT0
//! (`CheckSMEZT0Enabled`). LDR and STR transfer the register's 64 bytes as
//! LDR and STR of a vector do (see `sve::mem`), each byte an access of its
//! own, aligned when the address is to 16 bytes.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09 (Arm ARM M.d), the
//! instruction pages each handler names and the shared pseudocode in
//! `aarch64/functions/sve` (`CheckSMEZT0Enabled`).

#[cfg(test)]
#[path = "../../tests/unit/zt0.rs"]
mod tests;

use super::exec::{bit, bits, ExecResult, Flow};
use super::sme::ZT0_BYTES;
use super::sve::{group, Esize, Zv};
use super::Cpu;
use crate::bus::PhysBus;

impl Cpu {
    /// ZERO {ZT0} (`zero_zt_i_`: `11000000 01001000 00000000 00000001`):
    /// every bit of ZT0 zero.
    pub(crate) fn exec_sme_zero_zt0(&mut self) -> ExecResult {
        self.check_sme_enabled()?;
        self.check_sme_zt0_enabled()?;
        self.set_zt0(&[0; ZT0_BYTES]);
        Ok(Flow::Next)
    }

    /// LDR ZT0 and STR ZT0 (`ldr_zt_br_`, `str_zt_br_`: `11100001 00 x11111
    /// 100000 Rn 00000`, a store by bit 21): ZT0's 64 bytes at `X[n]` or
    /// SP.
    pub(crate) fn exec_sme_fill_spill_zt0<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let store = bit(insn, 21);
        self.check_sme_enabled()?;
        self.check_sme_zt0_enabled()?;
        let addr = self.base_register(bits(insn, 9, 5))?;
        let aligned = addr % 16 == 0;
        if store {
            self.spill_bytes(bus, addr, aligned, &self.zt0())?;
        } else {
            let loaded = self.fill_bytes(bus, addr, aligned, ZT0_BYTES)?;
            let mut table = [0; ZT0_BYTES];
            table.copy_from_slice(&loaded.as_bytes()[..ZT0_BYTES]);
            self.set_zt0(&table);
        }
        Ok(Flow::Next)
    }

    /// LUTI2 and LUTI4 of ZT0 to one, two or four registers (`luti2_z_ztz_`
    /// to `luti4_mz4_ztz_1`: `11000000 10 n1 001 opc size 00 Zn Zd`): one
    /// register `Zd` by bit 22, else the group `Zd:'0'` by bit 14 or
    /// `Zd:'00'`; LUTI2 by bit 18, its indexes `isize` = 2 bits, else LUTI4,
    /// 4 bits. Element `e` of register `r`, of `esize` bits by `size` (bits
    /// 13:12), is the low `esize` bits of ZT0's entry that index
    /// `(segment * nreg + r) * (SVL DIV esize) + e` of `Zn` (bits 9:5)
    /// selects; `segment` is the immediate from bit 17 (LUTI2) or 16
    /// (LUTI4) down to bit `14 + log2(nreg)`, modulo `esize DIV (isize *
    /// nreg)`. `Zn` is read before any register is written. `size = 11`,
    /// and LUTI4 of bytes to four registers, are UNDEFINED.
    pub(crate) fn exec_sme_lookup_table(&mut self, pc: u64, insn: u32) -> ExecResult {
        let index_bits: usize = if bit(insn, 18) { 2 } else { 4 };
        let nreg: usize = match (bit(insn, 22), bit(insn, 14)) {
            (true, _) => 1,
            (false, true) => 2,
            (false, false) => 4,
        };
        let es = match (bits(insn, 13, 12), index_bits, nreg) {
            (0b11, _, _) | (0b00, 4, 4) => return self.undefined(pc, insn),
            (size, _, _) => Esize::from_size(size),
        };
        let svl = self.check_streaming_sve_enabled()?;
        self.check_sme_zt0_enabled()?;
        let elements = es.elements(svl);
        let segments = es.bits() as usize / (index_bits * nreg);
        let imm = bits(insn, if index_bits == 2 { 17 } else { 16 }, 14 + nreg.trailing_zeros()) as usize;
        let segment = imm % segments;
        let indexes = self.z(bits(insn, 9, 5), svl);
        let table = self.zt0();
        let d = if nreg == 1 { bits(insn, 4, 0) } else { group(insn, 4, nreg) };
        for r in 0..nreg {
            let base = (segment * nreg + r) * elements;
            let mut result = Zv::ZERO;
            for e in 0..elements {
                let at = (base + e) * index_bits;
                let index = usize::from(indexes.byte(at / 8) >> (at % 8)) & ((1 << index_bits) - 1);
                for b in 0..es.bytes() {
                    result.set_byte(e * es.bytes() + b, table[4 * index + b]);
                }
            }
            self.set_z(d + r as u32, svl, &result);
        }
        Ok(Flow::Next)
    }
}
