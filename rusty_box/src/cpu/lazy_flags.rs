//! Bochs lazy flags evaluation.
//!
//! Mirrors `cpp_orig/bochs/cpu/lazy_flags.h`.
//!
//! Bochs encodes arithmetic flag results as two values:
//! - `result`: the sign-extended operation result (determines ZF, and SF via delta)
//! - `auxbits`: a packed bitfield encoding carry, overflow, adjust, parity, and sign deltas
//!
//! Individual flags (CF, OF, SF, ZF, AF, PF) are extracted lazily from these two values.
//! This avoids computing all 6 flags after every arithmetic operation — only the flags
//! actually read (e.g. by a conditional jump) are extracted.
//!
//! The `set_oszapc_*` methods mirror `SET_FLAGS_OSZAPC_*` macros from Bochs.
//! The `getB_*` / `set_*` methods mirror the inline methods on `bx_lazyflags_entry`.

use crate::config::BxAddress;

// Bit positions in auxbits (Bochs lazy_flags.h)
const LF_BIT_SD: u32 = 0;   // Sign Flag Delta
const LF_BIT_AF: u32 = 3;   // Adjust Flag
const LF_BIT_PDB: u32 = 8;  // Parity Delta Byte (8 bits wide)
const LF_BIT_PO: u32 = 30;  // Partial Overflow = CF ^ OF
const LF_BIT_CF: u32 = 31;  // Carry Flag

const LF_MASK_SD: u32 = 1 << LF_BIT_SD;
const LF_MASK_AF: u32 = 1 << LF_BIT_AF;
const LF_MASK_PDB: u32 = 0xFF << LF_BIT_PDB;
const LF_MASK_CF: u32 = 1 << LF_BIT_CF;
const LF_MASK_PO: u32 = 1 << LF_BIT_PO;

/// BX_LF_SIGN_BIT — 63 for x86_64 support
const BX_LF_SIGN_BIT: u32 = 63;

/// Bochs ADD carry-out vector: `(op1 & op2) | ((op1 | op2) & ~result)`
#[inline]
pub(super) fn add_cout_vec(op1: u64, op2: u64, result: u64) -> u64 {
    (op1 & op2) | ((op1 | op2) & !result)
}

/// Bochs SUB carry-out vector: `(~op1 & op2) | ((~op1 ^ op2) & result)`
#[inline]
pub(super) fn sub_cout_vec(op1: u64, op2: u64, result: u64) -> u64 {
    (!op1 & op2) | ((!op1 ^ op2) & result)
}

#[derive(Debug, Default)]
pub(crate) struct BxLazyflagsEntry {
    pub(super) result: BxAddress,
    pub(super) auxbits: BxAddress,
}

impl BxLazyflagsEntry {
    // ── SET_FLAGS macros ─────────────────────────────────────────────

    /// Bochs `SET_FLAGS_OSZAPC_SIZE(size, lf_carries, lf_result)`.
    ///
    /// Encodes the arithmetic result and carry vector into `result` + `auxbits`.
    /// `size` is the operand width in bits (8, 16, 32, 64).
    #[inline]
    pub(super) fn set_oszapc(&mut self, size: u32, carries: u64, lf_result: u64) {
        // Sign-extend result to BxAddress width
        let result_val: BxAddress = match size {
            8 => lf_result as i8 as i64 as u64,
            16 => lf_result as i16 as i64 as u64,
            32 => lf_result as i32 as i64 as u64,
            64 => lf_result,
            _ => lf_result,
        };

        let temp: u32 = match size {
            32 => (carries as u32) & !(LF_MASK_PDB | LF_MASK_SD),
            16 => ((carries as u32) & LF_MASK_AF) | ((carries as u32) << 16),
            8 => ((carries as u32) & LF_MASK_AF) | ((carries as u32) << 24),
            64 => {
                // For 64-bit: extract AF from carries, then shift top bits for PO
                let af = (carries as u32) & LF_MASK_AF;
                let po = ((carries >> (64 - 2)) as u32) << LF_BIT_PO;
                af | po
            }
            _ => {
                let af = (carries as u32) & LF_MASK_AF;
                let po = ((carries >> (size - 2)) as u32) << LF_BIT_PO;
                af | po
            }
        };

        self.result = result_val;
        self.auxbits = temp as BxAddress;
    }

    /// Bochs `SET_FLAGS_OSZAP_SIZE(size, lf_carries, lf_result)`.
    ///
    /// Like `set_oszapc` but preserves the existing CF (carry flag).
    #[inline]
    pub(super) fn set_oszap(&mut self, size: u32, carries: u64, lf_result: u64) {
        let result_val: BxAddress = match size {
            8 => lf_result as i8 as i64 as u64,
            16 => lf_result as i16 as i64 as u64,
            32 => lf_result as i32 as i64 as u64,
            64 => lf_result,
            _ => lf_result,
        };

        let temp: u32 = match size {
            32 => (carries as u32) & !(LF_MASK_PDB | LF_MASK_SD),
            16 => ((carries as u32) & LF_MASK_AF) | ((carries as u32) << 16),
            8 => ((carries as u32) & LF_MASK_AF) | ((carries as u32) << 24),
            _ => {
                let af = (carries as u32) & LF_MASK_AF;
                let po = ((carries >> (size - 2)) as u32) << LF_BIT_PO;
                af | po
            }
        };

        self.result = result_val;
        // Preserve CF: compute delta_c = (old_auxbits ^ temp) & CF, then XOR in
        let delta_c = ((self.auxbits as u32) ^ temp) & LF_MASK_CF;
        let delta_c = delta_c ^ (delta_c >> 1);
        self.auxbits = (temp ^ delta_c) as BxAddress;
    }

    // ── Convenience setters matching Bochs macros ────────────────────

    /// `SET_FLAGS_OSZAPC_LOGIC_8(result)` — logic ops (AND/OR/XOR/TEST)
    #[inline]
    pub(super) fn set_oszapc_logic_8(&mut self, result: u8) {
        self.set_oszapc(8, 0, result as u64);
    }
    #[inline]
    pub(super) fn set_oszapc_logic_16(&mut self, result: u16) {
        self.set_oszapc(16, 0, result as u64);
    }
    #[inline]
    pub(super) fn set_oszapc_logic_32(&mut self, result: u32) {
        self.set_oszapc(32, 0, result as u64);
    }
    #[inline]
    pub(super) fn set_oszapc_logic_64(&mut self, result: u64) {
        self.set_oszapc(64, 0, result);
    }

    /// `SET_FLAGS_OSZAPC_ADD_8(op1, op2, sum)`
    #[inline]
    pub(super) fn set_oszapc_add_8(&mut self, op1: u8, op2: u8, sum: u8) {
        self.set_oszapc(8, add_cout_vec(op1 as u64, op2 as u64, sum as u64), sum as u64);
    }
    #[inline]
    pub(super) fn set_oszapc_add_16(&mut self, op1: u16, op2: u16, sum: u16) {
        self.set_oszapc(16, add_cout_vec(op1 as u64, op2 as u64, sum as u64), sum as u64);
    }
    #[inline]
    pub(super) fn set_oszapc_add_32(&mut self, op1: u32, op2: u32, sum: u32) {
        self.set_oszapc(32, add_cout_vec(op1 as u64, op2 as u64, sum as u64), sum as u64);
    }
    #[inline]
    pub(super) fn set_oszapc_add_64(&mut self, op1: u64, op2: u64, sum: u64) {
        self.set_oszapc(64, add_cout_vec(op1, op2, sum), sum);
    }

    /// `SET_FLAGS_OSZAPC_SUB_8(op1, op2, diff)`
    #[inline]
    pub(super) fn set_oszapc_sub_8(&mut self, op1: u8, op2: u8, diff: u8) {
        self.set_oszapc(8, sub_cout_vec(op1 as u64, op2 as u64, diff as u64), diff as u64);
    }
    #[inline]
    pub(super) fn set_oszapc_sub_16(&mut self, op1: u16, op2: u16, diff: u16) {
        self.set_oszapc(16, sub_cout_vec(op1 as u64, op2 as u64, diff as u64), diff as u64);
    }
    #[inline]
    pub(super) fn set_oszapc_sub_32(&mut self, op1: u32, op2: u32, diff: u32) {
        self.set_oszapc(32, sub_cout_vec(op1 as u64, op2 as u64, diff as u64), diff as u64);
    }
    #[inline]
    pub(super) fn set_oszapc_sub_64(&mut self, op1: u64, op2: u64, diff: u64) {
        self.set_oszapc(64, sub_cout_vec(op1, op2, diff), diff);
    }

    /// `SET_FLAGS_OSZAP_ADD_*` — INC (preserves CF)
    #[inline]
    pub(super) fn set_oszap_add_16(&mut self, op1: u16, op2: u16, sum: u16) {
        self.set_oszap(16, add_cout_vec(op1 as u64, op2 as u64, sum as u64), sum as u64);
    }
    #[inline]
    pub(super) fn set_oszap_add_8(&mut self, op1: u8, op2: u8, sum: u8) {
        self.set_oszap(8, add_cout_vec(op1 as u64, op2 as u64, sum as u64), sum as u64);
    }
    #[inline]
    pub(super) fn set_oszap_sub_8(&mut self, op1: u8, op2: u8, diff: u8) {
        self.set_oszap(8, sub_cout_vec(op1 as u64, op2 as u64, diff as u64), diff as u64);
    }
    #[inline]
    pub(super) fn set_oszap_add_32(&mut self, op1: u32, op2: u32, sum: u32) {
        self.set_oszap(32, add_cout_vec(op1 as u64, op2 as u64, sum as u64), sum as u64);
    }
    #[inline]
    pub(super) fn set_oszap_sub_32(&mut self, op1: u32, op2: u32, diff: u32) {
        self.set_oszap(32, sub_cout_vec(op1 as u64, op2 as u64, diff as u64), diff as u64);
    }
    #[inline]
    pub(super) fn set_oszap_sub_16(&mut self, op1: u16, op2: u16, diff: u16) {
        self.set_oszap(16, sub_cout_vec(op1 as u64, op2 as u64, diff as u64), diff as u64);
    }
    #[inline]
    pub(super) fn set_oszap_add_64(&mut self, op1: u64, op2: u64, sum: u64) {
        self.set_oszap(64, add_cout_vec(op1, op2, sum), sum);
    }
    #[inline]
    pub(super) fn set_oszap_sub_64(&mut self, op1: u64, op2: u64, diff: u64) {
        self.set_oszap(64, sub_cout_vec(op1, op2, diff), diff);
    }

    // ── Flag getters (Bochs getB_* / get_*) ─────────────────────────

    /// OF: `(auxbits + (1 << PO)) >> CF_BIT & 1`
    #[inline]
    pub(super) fn getb_of(&self) -> u32 {
        ((self.auxbits as u32).wrapping_add(1u32 << LF_BIT_PO) >> LF_BIT_CF) & 1
    }

    /// SF: `(result >> SIGN_BIT) ^ (auxbits >> SD_BIT) & 1`
    #[inline]
    pub(super) fn getb_sf(&self) -> u32 {
        ((self.result >> BX_LF_SIGN_BIT) as u32 ^ (self.auxbits as u32 >> LF_BIT_SD)) & 1
    }

    /// ZF: `result == 0`
    #[inline]
    pub(super) fn getb_zf(&self) -> u32 {
        (self.result == 0) as u32
    }

    /// AF: `(auxbits >> AF_BIT) & 1`
    #[inline]
    pub(super) fn getb_af(&self) -> u32 {
        (self.auxbits as u32 >> LF_BIT_AF) & 1
    }

    /// PF: parity of `(result ^ (auxbits >> PDB)) & 0xFF`
    #[inline]
    pub(super) fn getb_pf(&self) -> u32 {
        let temp = (self.result as u32 & 0xFF) ^ ((self.auxbits as u32 >> LF_BIT_PDB) & 0xFF);
        let temp = (temp ^ (temp >> 4)) & 0x0F;
        (0x9669u32 >> temp) & 1
    }

    /// CF: `(auxbits >> CF_BIT) & 1`
    #[inline]
    pub(super) fn getb_cf(&self) -> u32 {
        (self.auxbits as u32 >> LF_BIT_CF) & 1
    }

    // ── Flag setters ────────────────────────────────────────────────

    /// Set OF+CF together (Bochs `set_flags_OxxxxC`)
    #[inline]
    pub(super) fn set_flags_oxxxxc(&mut self, new_of: u32, new_cf: u32) {
        let temp_po = new_of ^ new_cf;
        self.auxbits = (self.auxbits as u32 & !(LF_MASK_PO | LF_MASK_CF)
            | (temp_po << LF_BIT_PO)
            | (new_cf << LF_BIT_CF)) as BxAddress;
    }

    /// Assert both OF and CF (Bochs `assert_flags_OxxxxC` = `set_flags_OxxxxC(1, 1)`).
    #[inline]
    pub(super) fn assert_flags_oxxxxc(&mut self) {
        self.set_flags_oxxxxc(1, 1);
    }

    #[inline]
    pub(super) fn set_cf(&mut self, val: bool) {
        let temp_of = self.getb_of();
        self.set_flags_oxxxxc(temp_of, val as u32);
    }

    #[inline]
    pub(super) fn set_of(&mut self, val: bool) {
        let temp_cf = self.getb_cf();
        self.set_flags_oxxxxc(val as u32, temp_cf);
    }

    #[inline]
    pub(super) fn set_sf(&mut self, val: bool) {
        let temp_sf = self.getb_sf();
        self.auxbits ^= ((temp_sf ^ val as u32) << LF_BIT_SD) as BxAddress;
    }

    #[inline]
    pub(super) fn set_zf(&mut self, val: bool) {
        if val {
            // assert ZF: merge sign into SD, merge parity into PDB, then zero result
            self.auxbits ^= (((self.result >> BX_LF_SIGN_BIT) as u32 & 1) << LF_BIT_SD) as BxAddress;
            let temp_pdb = self.result as u32 & 0xFF;
            self.auxbits ^= ((temp_pdb) << LF_BIT_PDB) as BxAddress;
            self.result = 0;
        } else {
            // clear ZF: set bit 8 of result so result != 0
            self.result |= 1 << 8;
        }
    }

    #[inline]
    pub(super) fn set_af(&mut self, val: bool) {
        self.auxbits = ((self.auxbits as u32 & !LF_MASK_AF) | ((val as u32) << LF_BIT_AF)) as BxAddress;
    }

    #[inline]
    pub(super) fn set_pf(&mut self, val: bool) {
        let temp_pdb = (self.result as u32 & 0xFF) ^ (!val as u32);
        self.auxbits = ((self.auxbits as u32 & !LF_MASK_PDB) | (temp_pdb << LF_BIT_PDB)) as BxAddress;
    }
}
