//! Pointer authentication: FEAT_PAuth, FEAT_PAuth2, FEAT_FPAC and
//! FEAT_FPACCOMBINE, with the QARMA5 algorithm (FEAT_PACQARMA5).
//!
//! Provenance (R7): the Arm A64 ISA XML (release 2026-09), its shared
//! pseudocode under `aarch64/functions/pac` (`ComputePACQARMA` and its
//! cell, tweak and substitution functions, `InsertPAC`, `Authenticate`,
//! `Strip`, `AddPACGA`, `IsAPIAKeyEnabled` and its siblings,
//! `AArch64_PACFailException`), `EffectiveTBI` and
//! `CalculateBottomPACBit`. This core has neither EL2 nor EL3, so no key
//! use traps, and only the EL1&0 translation regime, whose pointers have
//! upper and lower address ranges; it has neither FEAT_MTE nor FEAT_LVA3,
//! nor FEAT_CONSTPACFIELD or FEAT_PAuth_LR.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use super::exception::{Exception, ExceptionClass};
use super::exec::{bit, bits, sext, ExecResult, Flow};
use super::features::Feature;
use super::mmu::DataAccess;
use super::{Cpu, El};
use crate::bus::PhysBus;

/// One 128-bit pointer authentication key, `APxxKeyHi_EL1:APxxKeyLo_EL1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Key {
    pub(crate) lo: u64,
    pub(crate) hi: u64,
}

/// The five keys: the instruction keys A and B, the data keys A and B, and
/// the generic key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PacKeys {
    pub(crate) ia: Key,
    pub(crate) ib: Key,
    pub(crate) da: Key,
    pub(crate) db: Key,
    pub(crate) ga: Key,
}

impl PacKeys {
    pub(crate) const ZERO: Self = {
        let zero = Key { lo: 0, hi: 0 };
        Self {
            ia: zero,
            ib: zero,
            da: zero,
            db: zero,
            ga: zero,
        }
    };

    /// The ten key registers in the order of their encodings,
    /// `APIAKeyLo_EL1` to `APGAKeyHi_EL1`: what a snapshot holds.
    pub(crate) const fn halves(&self) -> [u64; 10] {
        [
            self.ia.lo, self.ia.hi, self.ib.lo, self.ib.hi, self.da.lo, self.da.hi, self.db.lo, self.db.hi, self.ga.lo,
            self.ga.hi,
        ]
    }

    /// The inverse of [`PacKeys::halves`].
    pub(crate) const fn from_halves(h: [u64; 10]) -> Self {
        Self {
            ia: Key { lo: h[0], hi: h[1] },
            ib: Key { lo: h[2], hi: h[3] },
            da: Key { lo: h[4], hi: h[5] },
            db: Key { lo: h[6], hi: h[7] },
            ga: Key { lo: h[8], hi: h[9] },
        }
    }
}

/// The key an address-authenticating instruction uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PacKey {
    Ia,
    Ib,
    Da,
    Db,
}

impl PacKey {
    /// The instruction key, A or B as `b` says (the `M` bit of the
    /// branches, `op2<1>` of the hints, `opcode<0>` of the one-source
    /// instructions).
    const fn instruction(b: bool) -> Self {
        if b {
            Self::Ib
        } else {
            Self::Ia
        }
    }

    /// The data key, A or B as `b` says.
    const fn data_key(b: bool) -> Self {
        if b {
            Self::Db
        } else {
            Self::Da
        }
    }

    /// Whether the key authenticates data addresses (`data` in the
    /// pseudocode: top-byte-ignore for data rather than instructions).
    const fn data(self) -> bool {
        matches!(self, Self::Da | Self::Db)
    }

    /// `key_number`: 0 for the A keys, 1 for the B keys.
    const fn number(self) -> u64 {
        match self {
            Self::Ia | Self::Da => 0,
            Self::Ib | Self::Db => 1,
        }
    }

    /// The `SCTLR_EL1` bit enabling the key: EnIA, EnIB, EnDA, EnDB.
    const fn enable_bit(self) -> u32 {
        match self {
            Self::Ia => 31,
            Self::Ib => 30,
            Self::Da => 27,
            Self::Db => 13,
        }
    }
}

/// Whether an authentication is part of a combined instruction (a branch,
/// return or load that authenticates), which FEAT_FPAC alone does not
/// fault on (Arm `is_combined`, the pages' `auth_then_branch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthUse {
    Alone,
    Combined,
}

// --- QARMA5 (Arm ComputePACQARMA, isqarma3 = FALSE) ---

const ALPHA: u64 = 0xC0AC_29B7_C97C_50DD;
const RC: [u64; 5] = [
    0x0000_0000_0000_0000,
    0x1319_8A2E_0370_7344,
    0xA409_3822_299F_31D0,
    0x082E_FA98_EC4E_6C89,
    0x4528_21E6_38D0_1377,
];

/// PACSub and PACInvSub: the PRINCE-family 4-bit substitution and its
/// inverse.
const SUB: [u64; 16] = [0xB, 0x6, 0x8, 0xF, 0xC, 0x0, 0x9, 0xE, 0x3, 0x7, 0x4, 0x5, 0xD, 0x2, 0x1, 0xA];
const INV_SUB: [u64; 16] = [0x5, 0xE, 0xD, 0x8, 0xA, 0xB, 0x1, 0x9, 0x2, 0x6, 0xF, 0x0, 0x4, 0xC, 0x7, 0x3];

/// PACCellShuffle and PACCellInvShuffle: output cell `k` is input cell
/// `SHUFFLE[k]`.
const SHUFFLE: [u32; 16] = [13, 6, 11, 0, 7, 12, 1, 10, 8, 3, 14, 5, 2, 9, 4, 15];
const INV_SHUFFLE: [u32; 16] = [3, 6, 12, 9, 14, 11, 1, 4, 8, 13, 7, 2, 5, 0, 10, 15];

/// TweakShuffle and TweakInvShuffle: output cell `k` is input cell
/// `.0`, rotated (TweakCellRot, TweakCellInvRot) where `.1` says so.
const TWEAK: [(u32, bool); 16] = [
    (4, false),
    (5, false),
    (6, true),
    (7, false),
    (11, true),
    (2, false),
    (3, false),
    (8, true),
    (12, false),
    (13, false),
    (14, false),
    (15, true),
    (0, true),
    (1, false),
    (10, true),
    (9, true),
];
const INV_TWEAK: [(u32, bool); 16] = [
    (12, true),
    (13, false),
    (5, false),
    (6, false),
    (0, false),
    (1, false),
    (2, true),
    (3, false),
    (7, true),
    (15, true),
    (14, true),
    (4, true),
    (8, false),
    (9, false),
    (10, false),
    (11, true),
];

#[inline]
const fn cell(x: u64, i: u32) -> u64 {
    (x >> (4 * i)) & 0xF
}

fn substitute(x: u64, table: &[u64; 16]) -> u64 {
    (0..16).fold(0, |out, i| out | (table[cell(x, i) as usize] << (4 * i)))
}

fn shuffle(x: u64, map: &[u32; 16]) -> u64 {
    (0..16).fold(0, |out, k| out | (cell(x, map[k as usize]) << (4 * k)))
}

/// TweakCellRot: `c<0> XOR c<1>`, `c<3>`, `c<2>`, `c<1>`.
const fn tweak_cell_rot(c: u64) -> u64 {
    (c >> 1) | (((c ^ (c >> 1)) & 1) << 3)
}

/// TweakCellInvRot: `c<2>`, `c<1>`, `c<0>`, `c<0> XOR c<3>`.
const fn tweak_cell_inv_rot(c: u64) -> u64 {
    ((c << 1) & 0b1110) | ((c ^ (c >> 3)) & 1)
}

fn tweak_shuffle(x: u64, map: &[(u32, bool); 16], rot: fn(u64) -> u64) -> u64 {
    (0..16).fold(0, |out, k| {
        let (from, rotated) = map[k as usize];
        let c = cell(x, from);
        out | ((if rotated { rot(c) } else { c }) << (4 * k))
    })
}

/// ROL of a 4-bit cell.
const fn rol4(c: u64, n: u32) -> u64 {
    ((c << n) | (c >> (4 - n))) & 0xF
}

/// PACMult: the MixColumns-like step over each column of four cells.
fn mult(s: u64) -> u64 {
    let mut out = 0;
    for i in 0..4 {
        let (c0, c4, c8, c12) = (cell(s, i), cell(s, i + 4), cell(s, i + 8), cell(s, i + 12));
        let t0 = rol4(c8, 1) ^ rol4(c4, 2) ^ rol4(c0, 1);
        let t1 = rol4(c12, 1) ^ rol4(c4, 1) ^ rol4(c0, 2);
        let t2 = rol4(c12, 2) ^ rol4(c8, 1) ^ rol4(c0, 1);
        let t3 = rol4(c12, 1) ^ rol4(c8, 2) ^ rol4(c4, 1);
        out |= (t3 << (4 * i)) | (t2 << (4 * (i + 4))) | (t1 << (4 * (i + 8))) | (t0 << (4 * (i + 12)));
    }
    out
}

/// Arm ComputePAC with QARMA5: the 64-bit code for `data` under `modifier`
/// and the key `key0:key1` (`K[127:64]:K[63:0]`).
pub(crate) fn compute_pac(data: u64, modifier: u64, key0: u64, key1: u64) -> u64 {
    const ITERATIONS: usize = 4;
    let modk0 = (key0 << 63) | ((key0 >> 2) << 1) | (((key0 >> 63) ^ (key0 >> 1)) & 1);
    let mut running = modifier;
    let mut w = data ^ key0;
    for (i, rc) in RC.iter().enumerate() {
        w ^= key1 ^ running ^ rc;
        if i > 0 {
            w = mult(shuffle(w, &SHUFFLE));
        }
        w = substitute(w, &SUB);
        running = tweak_shuffle(running, &TWEAK, tweak_cell_rot);
    }
    w ^= modk0 ^ running;
    w = substitute(mult(shuffle(w, &SHUFFLE)), &SUB);
    w = mult(shuffle(w, &SHUFFLE));
    w ^= key1;
    w = shuffle(mult(substitute(shuffle(w, &INV_SHUFFLE), &INV_SUB)), &INV_SHUFFLE);
    w ^= key0 ^ running;
    for i in 0..=ITERATIONS {
        w = substitute(w, &INV_SUB);
        if i < ITERATIONS {
            w = shuffle(mult(w), &INV_SHUFFLE);
        }
        running = tweak_shuffle(running, &INV_TWEAK, tweak_cell_inv_rot);
        w ^= RC[ITERATIONS - i] ^ key1 ^ running ^ ALPHA;
    }
    w ^ modk0
}

/// Bits `hi:lo` of a 64-bit value, as a mask.
#[inline]
const fn field(hi: u32, lo: u32) -> u64 {
    (u64::MAX >> (63 - hi)) & !((1u64 << lo) - 1)
}

/// The bits a pointer's extension holds: all ones or all zeros by `bit`.
#[inline]
const fn replicate(bit: bool) -> u64 {
    if bit {
        u64::MAX
    } else {
        0
    }
}

/// `ptr` with its extension, the bits from `bottom` to bit 55 under
/// top-byte-ignore or to bit 63 without, all `ext`: what InsertPAC
/// computes the code of, Authenticate restores and Strip returns.
#[inline]
const fn with_extension(ptr: u64, ext: bool, bottom: u32, tbi: bool) -> u64 {
    let extension = field(if tbi { 55 } else { 63 }, bottom);
    (ptr & !extension) | (replicate(ext) & extension)
}

impl Cpu {
    /// Arm CalculateBottomPACBit: `64 - TxSZ` of the half of the address
    /// space `top_bit` selects, as the walk holds TxSZ.
    fn bottom_pac_bit(&self, top_bit: bool) -> u32 {
        64 - self.effective_txsz(top_bit)
    }

    /// Whether `key` is enabled (Arm IsAPIAKeyEnabled and its siblings):
    /// the `SCTLR_EL1` bit at EL1 and at EL0 alike, the EL1&0 regime being
    /// the only one.
    fn pac_key_enabled(&self, key: PacKey) -> bool {
        (self.sys.sctlr_el1 >> key.enable_bit()) & 1 == 1
    }

    fn pac_key(&self, key: PacKey) -> Key {
        match key {
            PacKey::Ia => self.pac_keys.ia,
            PacKey::Ib => self.pac_keys.ib,
            PacKey::Da => self.pac_keys.da,
            PacKey::Db => self.pac_keys.db,
        }
    }

    /// Arm AddPACIA, AddPACIB, AddPACDA, AddPACDB: `ptr` with a pointer
    /// authentication code for `modifier` inserted (InsertPAC), or `ptr`
    /// itself when `key` is disabled.
    pub(crate) fn add_pac(&self, ptr: u64, modifier: u64, key: PacKey) -> u64 {
        if !self.pac_key_enabled(key) {
            return ptr;
        }
        let k = self.pac_key(key);
        let data = key.data();
        let tbi = self.effective_tbi(ptr, !data);
        // The half of the address space the pointer is in: bit 55 where a
        // top-byte-ignore of either half applies to the access, bit 63
        // otherwise (`selbit`).
        let selbit = if self.tbi_applies(true, !data) || self.tbi_applies(false, !data) {
            (ptr >> 55) & 1 == 1
        } else {
            ptr >> 63 == 1
        };
        let bottom = self.bottom_pac_bit(selbit);
        let low = field(bottom - 1, 0);
        let mut pac = compute_pac(with_extension(ptr, selbit, bottom, tbi), modifier, k.hi, k.lo);
        let bit55 = if selbit { 1 << 55 } else { 0 };
        if !self.implements(Feature::PAuth2) {
            // A pointer without a canonical extension gets a corrupted
            // code.
            let top = if tbi { 55 } else { 63 };
            let extension = ptr & field(top, bottom);
            if extension != 0 && extension != field(top, bottom) {
                pac ^= 1 << (top - 1);
            }
            let upper = if tbi { ptr } else { pac };
            (upper & field(63, 56)) | bit55 | (pac & field(54, bottom)) | (ptr & low)
        } else {
            let upper = if tbi { ptr & field(63, 56) } else { (ptr ^ pac) & field(63, 56) };
            upper | bit55 | ((ptr ^ pac) & field(54, bottom)) | (ptr & low)
        }
    }

    /// Arm AddPACGA: the top 32 bits of the code for `x` under modifier `y`
    /// and the generic key, the low 32 zero. No enable bit governs it.
    pub(crate) fn add_pac_ga(&self, x: u64, y: u64) -> u64 {
        let k = self.pac_keys.ga;
        compute_pac(x, y, k.hi, k.lo) & field(63, 32)
    }

    /// Arm AuthIA, AuthIB, AuthDA, AuthDB: `ptr` with its code checked
    /// (Authenticate) and replaced by the address's extension, or `ptr`
    /// itself when `key` is disabled. A failed check corrupts the pointer;
    /// with FEAT_FPACCOMBINE, or FEAT_FPAC for an authentication alone, it
    /// raises the PAC Fail exception instead.
    pub(crate) fn auth(&self, ptr: u64, modifier: u64, key: PacKey, used: AuthUse) -> Result<u64, Exception> {
        if !self.pac_key_enabled(key) {
            return Ok(ptr);
        }
        let k = self.pac_key(key);
        let data = key.data();
        let tbi = self.effective_tbi(ptr, !data);
        let ptr55 = (ptr >> 55) & 1 == 1;
        let bottom = self.bottom_pac_bit(ptr55);
        let original = with_extension(ptr, ptr55, bottom, tbi);
        let pac = compute_pac(original, modifier, k.hi, k.lo);
        // The bits that hold the code: 54..bottom, and 63..56 without
        // top-byte-ignore.
        let code = if tbi { field(54, bottom) } else { field(63, 56) | field(54, bottom) };
        if !self.implements(Feature::PAuth2) {
            if (pac ^ ptr) & code == 0 {
                return Ok(original);
            }
            // The error code `key_number:NOT(key_number)` in the two bits
            // below the top of the extension.
            let error = (key.number() << 1) | (key.number() ^ 1);
            let at = if tbi { 53 } else { 61 };
            return Ok((original & !(3 << at)) | (error << at));
        }
        let result = ptr ^ (pac & code);
        let faults = self.implements(Feature::FpacCombine)
            || (self.implements(Feature::Fpac) && used == AuthUse::Alone);
        let checked = if tbi { field(54, bottom) } else { field(63, bottom) };
        if faults && result & checked != replicate((result >> 55) & 1 == 1) & checked {
            let syndrome = (u32::from(data) << 1) | key.number() as u32;
            return Err(Exception::new(ExceptionClass::PacFail, syndrome));
        }
        Ok(result)
    }

    /// The pointer authentication state a snapshot carries, for a processor
    /// implementing FEAT_PAuth: the ten key registers.
    pub(crate) fn save_pauth<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        if !self.implements(Feature::PAuth) {
            return Ok(());
        }
        for half in self.pac_keys.halves() {
            w.write_u64(half)?;
        }
        Ok(())
    }

    /// The inverse of [`Cpu::save_pauth`].
    pub(crate) fn restore_pauth<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        if !self.implements(Feature::PAuth) {
            return Ok(());
        }
        let mut halves = [0; 10];
        for half in halves.iter_mut() {
            *half = r.read_u64()?;
        }
        self.pac_keys = PacKeys::from_halves(halves);
        Ok(())
    }

    /// Arm Strip: `a` with its code replaced by the address's extension.
    pub(crate) fn strip_pac(&self, a: u64, data: bool) -> u64 {
        let a55 = (a >> 55) & 1 == 1;
        with_extension(a, a55, self.bottom_pac_bit(a55), self.effective_tbi(a, !data))
    }
}

/// The instructions.
impl Cpu {
    /// PACIA, PACIZA, AUTIA, AUTIZA and the B and data key forms, XPACI
    /// and XPACD: `1 1 0 11010110 00001 opcode Rn Rd`. `opcode` is `00 Z
    /// AUT D B` (`D` for a data key, `B` for key B), with `Z` taking the
    /// modifier as zero rather than from `Rn` (where 31 is SP), or `01000
    /// D` for XPAC.
    pub(crate) fn exec_pauth(&mut self, insn: u32) -> ExecResult {
        let rn = bits(insn, 9, 5);
        let rd = bits(insn, 4, 0);
        let x = self.x_or_zr(rd);
        let value = if bits(insn, 15, 11) == 0b01000 {
            self.strip_pac(x, bit(insn, 10))
        } else {
            let key = if bit(insn, 11) { PacKey::data_key(bit(insn, 10)) } else { PacKey::instruction(bit(insn, 10)) };
            let modifier = if bit(insn, 13) { 0 } else { self.x_or_sp(rn) };
            if bit(insn, 12) {
                self.auth(x, modifier, key, AuthUse::Alone)?
            } else {
                self.add_pac(x, modifier, key)
            }
        };
        self.set_x_or_zr(rd, value);
        Ok(Flow::Next)
    }

    /// PACGA: `1 0 0 11010110 Rm 001100 Rn Rd`, `Rm` = 31 being SP.
    pub(crate) fn exec_pacga(&mut self, insn: u32) -> ExecResult {
        let x = self.x_or_zr(bits(insn, 9, 5));
        let modifier = self.x_or_sp(bits(insn, 20, 16));
        let value = self.add_pac_ga(x, modifier);
        self.set_x_or_zr(bits(insn, 4, 0), value);
        Ok(Flow::Next)
    }

    /// The pointer authentication hints, by `CRm:op2`: XPACLRI (0000 111)
    /// strips X30; with CRm = 0001 (PACIA1716 and its siblings) X17 is
    /// signed or authenticated with X16 as the modifier, and with CRm =
    /// 0011 (PACIAZ, PACIASP and their siblings) X30 with zero or SP as
    /// `op2<0>` says. `op2<1>` picks key B, `op2<2>` authenticates.
    pub(crate) fn exec_pauth_hint(&mut self, insn: u32) -> ExecResult {
        let op2 = bits(insn, 7, 5);
        let (d, modifier) = match bits(insn, 11, 8) {
            0b0000 => {
                self.x[30] = self.strip_pac(self.x[30], false);
                return Ok(Flow::Next);
            }
            0b0001 => (17, self.x[16]),
            // CRm = 0011: the decoder sends no other hint here.
            _ => (30, if op2 & 1 == 1 { self.x_or_sp(31) } else { 0 }),
        };
        let key = PacKey::instruction(op2 & 0b010 != 0);
        let x = self.x[d];
        self.x[d] = if op2 & 0b100 != 0 {
            self.auth(x, modifier, key, AuthUse::Alone)?
        } else {
            self.add_pac(x, modifier, key)
        };
        Ok(Flow::Next)
    }

    /// BRAA, BRAAZ, BLRAA, BLRAAZ, RETAA, ERETAA and their key B forms:
    /// `1101011 Z 0 op 11111 00001 M Rn Rm`. The target is authenticated
    /// with key A or B as `M` says, under the modifier `Rm` (31 being SP)
    /// for the `Z = 1` branches, zero for the `Z = 0` ones and SP for the
    /// returns; a failure to authenticate faults with FEAT_FPACCOMBINE and
    /// otherwise leaves a target the fetch faults on. Whether the branch was
    /// fetched from a guarded page decides its BTYPE (FEAT_BTI).
    pub(crate) fn exec_branch_auth(&mut self, pc: u64, insn: u32, in_guarded_page: bool) -> ExecResult {
        let key = PacKey::instruction(bit(insn, 10));
        let rn = bits(insn, 9, 5);
        let rm = bits(insn, 4, 0);
        match bits(insn, 24, 21) {
            0b0010 => {
                let sp = self.x_or_sp(31);
                Ok(Flow::Jump(self.auth(self.x[30], sp, key, AuthUse::Combined)?))
            }
            0b0100 => {
                if self.pstate.el == El::El0 {
                    return self.undefined(pc, insn);
                }
                let sp = self.x_or_sp(31);
                let target = self.auth(self.sys.elr_el1, sp, key, AuthUse::Combined)?;
                Ok(Flow::Jump(self.exception_return(target)))
            }
            // BRAAZ, BLRAAZ (opc 000x) and BRAA, BLRAA (opc 100x): opc<0>
            // links.
            opc => {
                let modifier = if bit(insn, 24) { self.x_or_sp(rm) } else { 0 };
                let target = self.auth(self.x_or_zr(rn), modifier, key, AuthUse::Combined)?;
                if opc & 1 == 1 {
                    self.x[30] = pc.wrapping_add(4);
                    self.set_btype_call();
                } else {
                    self.set_btype_jump(in_guarded_page, rn);
                }
                Ok(Flow::Jump(target))
            }
        }
    }

    /// LDRAA, LDRAB: `11 111 0 00 M S 1 imm9 W 1 Rn Rt`. The base (`Rn`,
    /// 31 being SP) is authenticated with data key A or B as `M` says and
    /// a zero modifier, then offset by `S:imm9` doublewords; `W` writes the
    /// address back after the load, the loaded register first where the
    /// two are one (as [`super::ldst`] orders every writeback).
    pub(crate) fn exec_load_auth<B: PhysBus>(&mut self, bus: &mut B, insn: u32) -> ExecResult {
        let key = PacKey::data_key(bit(insn, 23));
        let rn = bits(insn, 9, 5);
        let rt = bits(insn, 4, 0);
        let base = self.x_or_sp(rn);
        let authenticated = self.auth(base, 0, key, AuthUse::Combined)?;
        self.check_sp_alignment(rn == 31, base)?;
        let offset = sext((u64::from(bit(insn, 22)) << 9) | u64::from(bits(insn, 20, 12)), 10) << 3;
        let address = authenticated.wrapping_add(offset);
        let value = self.read_mem(bus, address, 3, DataAccess::NORMAL)?;
        self.set_x_or_zr(rt, value);
        if bit(insn, 11) {
            self.set_x_or_sp(rn, address);
        }
        Ok(Flow::Next)
    }
}
