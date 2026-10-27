//! The instructions on FEAT_SME2's ZT0, the 512-bit lookup table register:
//! ZERO {ZT0}, and LDR and STR of ZT0. MOVT between ZT0 and a general
//! register is defined only in Debug state, which this processor never
//! enters, so it decodes UNDEFINED (`Encoding::class`).
//!
//! Each of them executes in or out of Streaming SVE mode (`CheckSMEEnabled`)
//! and only while ZA storage is enabled and `SMCR_EL1.EZT0` gives access to
//! ZT0 (`CheckSMEZT0Enabled`). LDR and STR transfer the register's 64
//! bytes as LDR and STR of a vector do (see `sve::mem`), each byte an
//! access of its own, aligned when the address is to 16 bytes.
//!
//! Provenance (R7): the Arm A64 ISA XML, release 2026-09 (Arm ARM M.d), the
//! instruction pages each handler names and the shared pseudocode in
//! `aarch64/functions/sve` (`CheckSMEZT0Enabled`).

#[cfg(test)]
#[path = "../../tests/unit/zt0.rs"]
mod tests;

use super::exec::{bit, bits, ExecResult, Flow};
use super::sme::ZT0_BYTES;
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
}
