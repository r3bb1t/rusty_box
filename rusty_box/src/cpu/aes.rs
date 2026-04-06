//! AES-NI and PCLMULQDQ instruction handlers
//!
//! Based on Bochs cpu/aes.cc
//! Copyright (C) 2008-2018 Stanislav Shwartsman
//!
//! Implements:
//! - AESIMC (Inverse MixColumns)
//! - AESENC (AES Encrypt Round)
//! - AESENCLAST (AES Encrypt Last Round)
//! - AESDEC (AES Decrypt Round)
//! - AESDECLAST (AES Decrypt Last Round)
//! - AESKEYGENASSIST (AES Key Generation Assist)
//! - PCLMULQDQ (Carry-Less Multiplication)

use super::{
    cpu::BxCpuC,
    cpuid::BxCpuIdTrait,
    decoder::{BxSegregs, Instruction},
    xmm::BxPackedXmmRegister,
};

// ============================================================================
// AES S-box tables (from Bochs aes.cc)
// ============================================================================

#[rustfmt::skip]
static SBOX_TRANSFORMATION: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5,
    0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0,
    0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc,
    0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a,
    0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0,
    0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b,
    0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85,
    0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5,
    0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17,
    0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88,
    0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c,
    0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9,
    0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6,
    0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e,
    0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94,
    0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68,
    0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

#[rustfmt::skip]
static INVERSE_SBOX_TRANSFORMATION: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38,
    0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87,
    0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d,
    0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2,
    0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16,
    0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda,
    0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a,
    0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02,
    0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea,
    0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85,
    0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89,
    0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20,
    0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31,
    0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d,
    0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0,
    0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26,
    0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

// ============================================================================
// AES helper functions (matching Bochs aes.cc)
// ============================================================================

/// AES_STATE(s,a,b) = s.xmmubyte(b*4+a)
/// Macro for accessing bytes in column-major AES state matrix order.
macro_rules! aes_state {
    ($s:expr, $a:expr, $b:expr) => {
        $s.xmmubyte(($b) * 4 + ($a))
    };
}

macro_rules! aes_state_set {
    ($s:expr, $a:expr, $b:expr, $val:expr) => {
        $s.set_xmmubyte(($b) * 4 + ($a), $val)
    };
}

/// AES ShiftRows transformation (Bochs AES_ShiftRows)
///
/// Row 0: unchanged
/// Row 1: shift left 1 (bytes 1,5,9,D rotate)
/// Row 2: shift left 2 (bytes 2,6,A,E rotate)
/// Row 3: shift left 3 (bytes 3,7,B,F rotate)
///
/// Byte permutation: [0,5,A,F,4,9,E,3,8,D,2,7,C,1,6,B]
fn aes_shift_rows(state: &mut BxPackedXmmRegister) {
    let tmp = *state;
        state.set_xmmubyte(0x0, tmp.xmmubyte(0x0)); // A => A
        state.set_xmmubyte(0x1, tmp.xmmubyte(0x5));
        state.set_xmmubyte(0x2, tmp.xmmubyte(0xA));
        state.set_xmmubyte(0x3, tmp.xmmubyte(0xF));
        state.set_xmmubyte(0x4, tmp.xmmubyte(0x4)); // E => E
        state.set_xmmubyte(0x5, tmp.xmmubyte(0x9));
        state.set_xmmubyte(0x6, tmp.xmmubyte(0xE));
        state.set_xmmubyte(0x7, tmp.xmmubyte(0x3));
        state.set_xmmubyte(0x8, tmp.xmmubyte(0x8)); // I => I
        state.set_xmmubyte(0x9, tmp.xmmubyte(0xD));
        state.set_xmmubyte(0xA, tmp.xmmubyte(0x2));
        state.set_xmmubyte(0xB, tmp.xmmubyte(0x7));
        state.set_xmmubyte(0xC, tmp.xmmubyte(0xC)); // M => M
        state.set_xmmubyte(0xD, tmp.xmmubyte(0x1));
        state.set_xmmubyte(0xE, tmp.xmmubyte(0x6));
        state.set_xmmubyte(0xF, tmp.xmmubyte(0xB));
}

/// AES InverseShiftRows transformation (Bochs AES_InverseShiftRows)
///
/// Byte permutation: [0,D,A,7,4,1,E,B,8,5,2,F,C,9,6,3]
fn aes_inverse_shift_rows(state: &mut BxPackedXmmRegister) {
    let tmp = *state;
        state.set_xmmubyte(0x0, tmp.xmmubyte(0x0)); // A => A
        state.set_xmmubyte(0x1, tmp.xmmubyte(0xD));
        state.set_xmmubyte(0x2, tmp.xmmubyte(0xA));
        state.set_xmmubyte(0x3, tmp.xmmubyte(0x7));
        state.set_xmmubyte(0x4, tmp.xmmubyte(0x4)); // E => E
        state.set_xmmubyte(0x5, tmp.xmmubyte(0x1));
        state.set_xmmubyte(0x6, tmp.xmmubyte(0xE));
        state.set_xmmubyte(0x7, tmp.xmmubyte(0xB));
        state.set_xmmubyte(0x8, tmp.xmmubyte(0x8)); // I => I
        state.set_xmmubyte(0x9, tmp.xmmubyte(0x5));
        state.set_xmmubyte(0xA, tmp.xmmubyte(0x2));
        state.set_xmmubyte(0xB, tmp.xmmubyte(0xF));
        state.set_xmmubyte(0xC, tmp.xmmubyte(0xC)); // M => M
        state.set_xmmubyte(0xD, tmp.xmmubyte(0x9));
        state.set_xmmubyte(0xE, tmp.xmmubyte(0x6));
        state.set_xmmubyte(0xF, tmp.xmmubyte(0x3));
}

/// Apply AES S-box substitution to each byte of state (Bochs AES_SubstituteBytes)
fn aes_substitute_bytes(state: &mut BxPackedXmmRegister) {
        for i in 0..16 {
            state.set_xmmubyte(i, SBOX_TRANSFORMATION[state.xmmubyte(i) as usize]);
        }
}

/// Apply inverse AES S-box substitution to each byte of state (Bochs AES_InverseSubstituteBytes)
fn aes_inverse_substitute_bytes(state: &mut BxPackedXmmRegister) {
        for i in 0..16 {
            state.set_xmmubyte(i, INVERSE_SBOX_TRANSFORMATION[state.xmmubyte(i) as usize]);
        }
}

/// Galois Field multiplication of a by b, modulo 0x11b (Bochs gf_mul)
///
/// Like arithmetic multiplication, except additions and subtractions are XOR.
/// From: http://www.darkside.com.au/ice/index.html
#[inline]
fn gf_mul(mut a: u32, mut b: u32) -> u32 {
    let mut res: u32 = 0;
    let m: u32 = 0x11b;

    while b != 0 {
        if b & 1 != 0 {
            res ^= a;
        }

        a <<= 1;
        b >>= 1;

        if a >= 256 {
            a ^= m;
        }
    }

    res
}

/// AES MixColumns transformation (Bochs AES_MixColumns)
///
/// For each column j:
///   new[0,j] = 2*[0,j] ^ 3*[1,j] ^ [2,j] ^ [3,j]
///   new[1,j] = [0,j] ^ 2*[1,j] ^ 3*[2,j] ^ [3,j]
///   new[2,j] = [0,j] ^ [1,j] ^ 2*[2,j] ^ 3*[3,j]
///   new[3,j] = 3*[0,j] ^ [1,j] ^ [2,j] ^ 2*[3,j]
fn aes_mix_columns(state: &mut BxPackedXmmRegister) {
    let tmp = *state;

    for j in 0..4usize {
            aes_state_set!(state, 0, j, (gf_mul(0x2, aes_state!(tmp, 0, j) as u32)
                ^ gf_mul(0x3, aes_state!(tmp, 1, j) as u32)
                ^ aes_state!(tmp, 2, j) as u32
                ^ aes_state!(tmp, 3, j) as u32) as u8);

            aes_state_set!(state, 1, j, (aes_state!(tmp, 0, j) as u32
                ^ gf_mul(0x2, aes_state!(tmp, 1, j) as u32)
                ^ gf_mul(0x3, aes_state!(tmp, 2, j) as u32)
                ^ aes_state!(tmp, 3, j) as u32) as u8);

            aes_state_set!(state, 2, j, (aes_state!(tmp, 0, j) as u32
                ^ aes_state!(tmp, 1, j) as u32
                ^ gf_mul(0x2, aes_state!(tmp, 2, j) as u32)
                ^ gf_mul(0x3, aes_state!(tmp, 3, j) as u32)) as u8);

            aes_state_set!(state, 3, j, (gf_mul(0x3, aes_state!(tmp, 0, j) as u32)
                ^ aes_state!(tmp, 1, j) as u32
                ^ aes_state!(tmp, 2, j) as u32
                ^ gf_mul(0x2, aes_state!(tmp, 3, j) as u32)) as u8);
    }
}

/// AES InverseMixColumns transformation (Bochs AES_InverseMixColumns)
///
/// Uses multipliers 0xE, 0xB, 0xD, 0x9
fn aes_inverse_mix_columns(state: &mut BxPackedXmmRegister) {
    let tmp = *state;

    for j in 0..4usize {
            aes_state_set!(state, 0, j, (gf_mul(0xE, aes_state!(tmp, 0, j) as u32)
                ^ gf_mul(0xB, aes_state!(tmp, 1, j) as u32)
                ^ gf_mul(0xD, aes_state!(tmp, 2, j) as u32)
                ^ gf_mul(0x9, aes_state!(tmp, 3, j) as u32)) as u8);

            aes_state_set!(state, 1, j, (gf_mul(0x9, aes_state!(tmp, 0, j) as u32)
                ^ gf_mul(0xE, aes_state!(tmp, 1, j) as u32)
                ^ gf_mul(0xB, aes_state!(tmp, 2, j) as u32)
                ^ gf_mul(0xD, aes_state!(tmp, 3, j) as u32)) as u8);

            aes_state_set!(state, 2, j, (gf_mul(0xD, aes_state!(tmp, 0, j) as u32)
                ^ gf_mul(0x9, aes_state!(tmp, 1, j) as u32)
                ^ gf_mul(0xE, aes_state!(tmp, 2, j) as u32)
                ^ gf_mul(0xB, aes_state!(tmp, 3, j) as u32)) as u8);

            aes_state_set!(state, 3, j, (gf_mul(0xB, aes_state!(tmp, 0, j) as u32)
                ^ gf_mul(0xD, aes_state!(tmp, 1, j) as u32)
                ^ gf_mul(0x9, aes_state!(tmp, 2, j) as u32)
                ^ gf_mul(0xE, aes_state!(tmp, 3, j) as u32)) as u8);
    }
}

/// Apply S-box substitution to each byte of a u32 (Bochs AES_SubWord)
#[inline]
fn aes_sub_word(x: u32) -> u32 {
    let b0 = SBOX_TRANSFORMATION[(x & 0xff) as usize] as u32;
    let b1 = SBOX_TRANSFORMATION[((x >> 8) & 0xff) as usize] as u32;
    let b2 = SBOX_TRANSFORMATION[((x >> 16) & 0xff) as usize] as u32;
    let b3 = SBOX_TRANSFORMATION[((x >> 24) & 0xff) as usize] as u32;

    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}

/// Rotate u32 right by 8 bits (Bochs AES_RotWord)
#[inline]
fn aes_rot_word(x: u32) -> u32 {
    x.rotate_right(8)
}

/// XOR two XMM registers (Bochs xmm_xorps)
#[inline]
fn xmm_xorps(dst: &mut BxPackedXmmRegister, src: &BxPackedXmmRegister) {
        dst.set_xmm64u(0, dst.xmm64u(0) ^ src.xmm64u(0));
        dst.set_xmm64u(1, dst.xmm64u(1) ^ src.xmm64u(1));
}

/// Carry-less multiplication of two 64-bit values (Bochs xmm_pclmulqdq)
///
/// Returns a 128-bit result in an XMM register.
fn xmm_pclmulqdq(a: u64, b: u64) -> BxPackedXmmRegister {
    let mut result = BxPackedXmmRegister::default();
    let mut tmp_lo: u64 = a;
    let mut tmp_hi: u64 = 0;
    let mut r_lo: u64 = 0;
    let mut r_hi: u64 = 0;
    let mut b = b;

    for _ in 0..64 {
        if b == 0 {
            break;
        }
        if b & 1 != 0 {
            r_lo ^= tmp_lo;
            r_hi ^= tmp_hi;
        }
        tmp_hi = (tmp_hi << 1) | (tmp_lo >> 63);
        tmp_lo <<= 1;
        b >>= 1;
    }

        result.set_xmm64u(0, r_lo);
        result.set_xmm64u(1, r_hi);
    result
}

// ============================================================================
// Instruction handlers
// ============================================================================

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// Read source XMM operand from register or memory.
    /// Matches Bochs LOAD_Wdq pattern: if mod==11b read register, else read 128-bit
    /// from memory via paging-aware access.
    #[inline]
    fn read_xmm_src(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<BxPackedXmmRegister> {
        if instr.mod_c0() {
            Ok(self.read_xmm_reg(instr.src()))
        } else {
            let eaddr = self.resolve_addr(instr);
            let seg = BxSegregs::from(instr.seg());
            self.v_read_xmmword(seg, eaddr)
        }
    }

    /// AESIMC VdqWdq — 66 0F 38 DB
    ///
    /// Perform Inverse MixColumns on the source XMM operand and store
    /// the result in the destination XMM register.
    pub(super) fn aesimc_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut op = self.read_xmm_src(instr)?;

        aes_inverse_mix_columns(&mut op);

        self.write_xmm_result(instr, instr.dst(), op);
        Ok(())
    }

    /// Write XMM result: VEX clears upper YMM, legacy SSE preserves it.
    /// Bochs: VEX uses BX_WRITE_XMM_REG_CLEAR_HIGH, legacy uses BX_WRITE_XMM_REG.
    #[inline]
    fn write_xmm_result(&mut self, instr: &Instruction, idx: u8, val: BxPackedXmmRegister) {
        if instr.is_vex() {
            self.write_xmm_reg(idx, val); // VEX: clear upper YMM/ZMM
        } else {
            self.write_xmm_reg_lo128(idx, val); // Legacy SSE: preserve upper
        }
    }

    /// Read the AES state operand: for legacy SSE, state is dst (destructive).
    /// For VEX, state is src2 (vvv, non-destructive 3-operand form).
    #[inline]
    fn read_aes_state(&self, instr: &Instruction) -> BxPackedXmmRegister {
        if instr.is_vex() {
            self.read_xmm_reg(instr.src2()) // VEX: state = vvv
        } else {
            self.read_xmm_reg(instr.dst()) // Legacy: state = dst (destructive)
        }
    }

    /// AESENC VdqWdq — 66 0F 38 DC / VEX.128.66.0F38 DC
    ///
    /// Perform one round of AES encryption:
    /// ShiftRows(state), SubBytes(state), MixColumns(state), XOR with round key.
    pub(super) fn aesenc_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut state = self.read_aes_state(instr);
        let round_key = self.read_xmm_src(instr)?;

        aes_shift_rows(&mut state);
        aes_substitute_bytes(&mut state);
        aes_mix_columns(&mut state);
        xmm_xorps(&mut state, &round_key);

        self.write_xmm_result(instr, instr.dst(), state);
        Ok(())
    }

    /// AESENCLAST VdqWdq — 66 0F 38 DD / VEX.128.66.0F38 DD
    ///
    /// Perform the last round of AES encryption (no MixColumns):
    /// ShiftRows(state), SubBytes(state), XOR with round key.
    pub(super) fn aesenclast_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut state = self.read_aes_state(instr);
        let round_key = self.read_xmm_src(instr)?;

        aes_shift_rows(&mut state);
        aes_substitute_bytes(&mut state);
        xmm_xorps(&mut state, &round_key);

        self.write_xmm_result(instr, instr.dst(), state);
        Ok(())
    }

    /// AESDEC VdqWdq — 66 0F 38 DE / VEX.128.66.0F38 DE
    ///
    /// Perform one round of AES decryption:
    /// InverseShiftRows, InverseSubBytes, InverseMixColumns, XOR with round key.
    pub(super) fn aesdec_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut state = self.read_aes_state(instr);
        let round_key = self.read_xmm_src(instr)?;

        aes_inverse_shift_rows(&mut state);
        aes_inverse_substitute_bytes(&mut state);
        aes_inverse_mix_columns(&mut state);
        xmm_xorps(&mut state, &round_key);

        self.write_xmm_result(instr, instr.dst(), state);
        Ok(())
    }

    /// AESDECLAST VdqWdq — 66 0F 38 DF / VEX.128.66.0F38 DF
    ///
    /// Perform the last round of AES decryption (no InverseMixColumns):
    /// InverseShiftRows, InverseSubBytes, XOR with round key.
    pub(super) fn aesdeclast_vdq_wdq(&mut self, instr: &Instruction) -> super::Result<()> {
        let mut state = self.read_aes_state(instr);
        let round_key = self.read_xmm_src(instr)?;

        aes_inverse_shift_rows(&mut state);
        aes_inverse_substitute_bytes(&mut state);
        xmm_xorps(&mut state, &round_key);

        self.write_xmm_result(instr, instr.dst(), state);
        Ok(())
    }

    /// AESKEYGENASSIST VdqWdqIb — 66 0F 3A DF
    ///
    /// Assist in AES round key generation using the RCON immediate.
    pub(super) fn aeskeygenassist_vdq_wdq_ib(
        &mut self,
        instr: &Instruction,
    ) -> super::Result<()> {
        let op = self.read_xmm_src(instr)?;
        let rcon32 = instr.ib() as u32;
        let mut result = BxPackedXmmRegister::default();

            result.set_xmm32u(0, aes_sub_word(op.xmm32u(1)));
            result.set_xmm32u(1, aes_rot_word(result.xmm32u(0)) ^ rcon32);
            result.set_xmm32u(2, aes_sub_word(op.xmm32u(3)));
            result.set_xmm32u(3, aes_rot_word(result.xmm32u(2)) ^ rcon32);

        self.write_xmm_result(instr, instr.dst(), result);
        Ok(())
    }

    /// PCLMULQDQ VdqWdqIb — 66 0F 3A 44 / VEX.128.66.0F3A 44
    ///
    /// Carry-less multiplication of two 64-bit values selected by the
    /// immediate byte (bit 0 selects qword from op1, bit 4 from op2).
    pub(super) fn pclmulqdq_vdq_wdq_ib(&mut self, instr: &Instruction) -> super::Result<()> {
        let imm8 = instr.ib();
        // VEX: op1 = vvv (src2), legacy: op1 = dst (destructive)
        let op1 = if instr.is_vex() {
            self.read_xmm_reg(instr.src2())
        } else {
            self.read_xmm_reg(instr.dst())
        };
        let op2 = self.read_xmm_src(instr)?;

        let a = op1.xmm64u((imm8 & 1) as usize);
        let b = op2.xmm64u(((imm8 >> 4) & 1) as usize);

        let result = xmm_pclmulqdq(a, b);

        self.write_xmm_result(instr, instr.dst(), result);
        Ok(())
    }
}
