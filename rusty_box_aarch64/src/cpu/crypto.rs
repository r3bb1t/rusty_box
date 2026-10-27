//! The ARMv8.0 Cryptographic Extension and CRC32: AES (FEAT_AES), SHA-1
//! (FEAT_SHA1), SHA-256 (FEAT_SHA256) and CRC32/CRC32C (FEAT_CRC32), as the
//! Cortex-A57 has them, and the 64-bit polynomial multiply PMULL uses
//! (FEAT_PMULL).
//!
//! The semantics are the Arm ARM pseudocode (DDI 0487): the AESE, AESD,
//! AESMC, AESIMC, SHA1C, SHA1P, SHA1M, SHA1H, SHA1SU0, SHA1SU1, SHA256H,
//! SHA256H2, SHA256SU0, SHA256SU1, PMULL and CRC32/CRC32C instruction pages
//! and the shared functions they call — `AESSubBytes`, `AESInvSubBytes`,
//! `AESShiftRows`, `AESInvShiftRows`, `AESMixColumns`, `AESInvMixColumns`,
//! `SHAchoose`, `SHAparity`, `SHAmajority`, `SHAhashSIGMA0`,
//! `SHAhashSIGMA1`, `SHA256hash`, `PolynomialMult`, `BitReverse` and
//! `Poly32Mod2`. They are cross-checked against QEMU, whose Cortex-A57 has
//! the same extensions: `target/arm/tcg/crypto_helper.c` (`crypto_aese`,
//! `crypto_aesmc`, `crypto_sha1_3reg`, `crypto_sha1su0`, `crypto_sha256h`,
//! `crypto_sha256su1`, ...), `crypto/clmul.c` (`clmul_64`) and
//! `target/arm/tcg/helper-a64.c` (`crc32_64`, `crc32c_64`), through the
//! Unicorn (QEMU TCG) vectors in `tests/vectors/unicorn.txt`; and against the
//! standards' own examples in `tests/crypto.rs`.
//!
//! The AES S-boxes and the GF(2^8) multiplication tables are computed at
//! compile time from their definitions in FIPS-197 (section 4.2 for the
//! field, 5.1.1 and 5.3.2 for the S-boxes), the source of the Arm ARM's
//! tables.

use super::dp::ones;
use super::exec::{bit, bits, ExecResult, Flow};
use super::simd::{elem, set_elem};
use super::Cpu;

// ----- AES -----

/// Multiplication in GF(2^8) modulo x^8 + x^4 + x^3 + x + 1 (FIPS-197 4.2).
const fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut product = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            product ^= a;
        }
        let carry = a & 0x80 != 0;
        a <<= 1;
        if carry {
            a ^= 0x1B;
        }
        b >>= 1;
    }
    product
}

/// The multiplicative inverse in GF(2^8), with 0 mapped to 0: `a^254`.
const fn gf_inverse(a: u8) -> u8 {
    let mut result = 1u8;
    let mut base = a;
    let mut exponent = 254u32;
    while exponent != 0 {
        if exponent & 1 != 0 {
            result = gf_mul(result, base);
        }
        base = gf_mul(base, base);
        exponent >>= 1;
    }
    result
}

/// The AES S-box (FIPS-197 5.1.1): the inverse, then the affine map.
const SBOX: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let b = gf_inverse(i as u8);
        table[i] = b ^ b.rotate_left(1) ^ b.rotate_left(2) ^ b.rotate_left(3) ^ b.rotate_left(4) ^ 0x63;
        i += 1;
    }
    table
};

/// The inverse S-box (FIPS-197 5.3.2).
const INV_SBOX: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        table[SBOX[i] as usize] = i as u8;
        i += 1;
    }
    table
};

/// Multiplication by `k` in GF(2^8), as a table: the Arm ARM's `FFmul02`,
/// `FFmul03`, `FFmul09`, `FFmul0B`, `FFmul0D` and `FFmul0E`.
const fn ff_mul_table(k: u8) -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = gf_mul(i as u8, k);
        i += 1;
    }
    table
}

const FF_MUL_02: [u8; 256] = ff_mul_table(0x02);
const FF_MUL_03: [u8; 256] = ff_mul_table(0x03);
const FF_MUL_09: [u8; 256] = ff_mul_table(0x09);
const FF_MUL_0B: [u8; 256] = ff_mul_table(0x0B);
const FF_MUL_0D: [u8; 256] = ff_mul_table(0x0D);
const FF_MUL_0E: [u8; 256] = ff_mul_table(0x0E);

/// `AESShiftRows`: byte `i` of the result is byte `SHIFT_ROWS[i]` of the
/// operand. The state is column-major, row `r` of column `c` in byte
/// `4c + r`, and row `r` rotates left by `r` columns.
const SHIFT_ROWS: [usize; 16] = [0, 5, 10, 15, 4, 9, 14, 3, 8, 13, 2, 7, 12, 1, 6, 11];

/// `AESInvShiftRows`: row `r` rotates right by `r` columns.
const INV_SHIFT_ROWS: [usize; 16] = [0, 13, 10, 7, 4, 1, 14, 11, 8, 5, 2, 15, 12, 9, 6, 3];

/// `AESSubBytes(AESShiftRows(state))`, as AESE applies them (the two
/// commute: one moves bytes, the other maps each).
fn aes_shift_sub(state: u128) -> u128 {
    let input = state.to_le_bytes();
    let mut output = [0u8; 16];
    for (i, byte) in output.iter_mut().enumerate() {
        *byte = SBOX[usize::from(input[SHIFT_ROWS[i]])];
    }
    u128::from_le_bytes(output)
}

/// `AESInvSubBytes(AESInvShiftRows(state))`, as AESD applies them.
fn aes_inv_shift_sub(state: u128) -> u128 {
    let input = state.to_le_bytes();
    let mut output = [0u8; 16];
    for (i, byte) in output.iter_mut().enumerate() {
        *byte = INV_SBOX[usize::from(input[INV_SHIFT_ROWS[i]])];
    }
    u128::from_le_bytes(output)
}

/// `AESMixColumns`: each 32-bit column times {03}x^3 + {01}x^2 + {01}x +
/// {02} modulo x^4 + 1.
fn aes_mix_columns(state: u128) -> u128 {
    let input = state.to_le_bytes();
    let mut output = [0u8; 16];
    for c in 0..4 {
        let [a0, a1, a2, a3] = [input[4 * c], input[4 * c + 1], input[4 * c + 2], input[4 * c + 3]];
        let mul = |table: &[u8; 256], a: u8| table[usize::from(a)];
        output[4 * c] = mul(&FF_MUL_02, a0) ^ mul(&FF_MUL_03, a1) ^ a2 ^ a3;
        output[4 * c + 1] = a0 ^ mul(&FF_MUL_02, a1) ^ mul(&FF_MUL_03, a2) ^ a3;
        output[4 * c + 2] = a0 ^ a1 ^ mul(&FF_MUL_02, a2) ^ mul(&FF_MUL_03, a3);
        output[4 * c + 3] = mul(&FF_MUL_03, a0) ^ a1 ^ a2 ^ mul(&FF_MUL_02, a3);
    }
    u128::from_le_bytes(output)
}

/// `AESInvMixColumns`: each column times {0B}x^3 + {0D}x^2 + {09}x + {0E}.
fn aes_inv_mix_columns(state: u128) -> u128 {
    let input = state.to_le_bytes();
    let mut output = [0u8; 16];
    for c in 0..4 {
        let [a0, a1, a2, a3] = [input[4 * c], input[4 * c + 1], input[4 * c + 2], input[4 * c + 3]];
        let mul = |table: &[u8; 256], a: u8| table[usize::from(a)];
        output[4 * c] = mul(&FF_MUL_0E, a0) ^ mul(&FF_MUL_0B, a1) ^ mul(&FF_MUL_0D, a2) ^ mul(&FF_MUL_09, a3);
        output[4 * c + 1] = mul(&FF_MUL_09, a0) ^ mul(&FF_MUL_0E, a1) ^ mul(&FF_MUL_0B, a2) ^ mul(&FF_MUL_0D, a3);
        output[4 * c + 2] = mul(&FF_MUL_0D, a0) ^ mul(&FF_MUL_09, a1) ^ mul(&FF_MUL_0E, a2) ^ mul(&FF_MUL_0B, a3);
        output[4 * c + 3] = mul(&FF_MUL_0B, a0) ^ mul(&FF_MUL_0D, a1) ^ mul(&FF_MUL_09, a2) ^ mul(&FF_MUL_0E, a3);
    }
    u128::from_le_bytes(output)
}

// ----- SHA -----

/// `SHAchoose`.
#[inline]
fn sha_choose(x: u32, y: u32, z: u32) -> u32 {
    ((y ^ z) & x) ^ z
}

/// `SHAparity`.
#[inline]
fn sha_parity(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}

/// `SHAmajority`.
#[inline]
fn sha_majority(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | ((x | y) & z)
}

/// `SHAhashSIGMA0`.
#[inline]
fn sha_hash_sigma0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}

/// `SHAhashSIGMA1`.
#[inline]
fn sha_hash_sigma1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}

/// Element `e` of a vector of four 32-bit elements.
#[inline]
fn word(v: u128, e: u32) -> u32 {
    elem(v, e, 32) as u32
}

/// Four 32-bit elements, element 0 first.
#[inline]
fn words(w: [u32; 4]) -> u128 {
    w.iter().rev().fold(0u128, |acc, &x| (acc << 32) | u128::from(x))
}

/// SHA1C, SHA1P and SHA1M: four rounds of the round function `f` over the
/// hash state `x` (A in element 0), with E in `y` and the schedule words of
/// `w`.
fn sha1_rounds(mut x: u128, mut y: u32, w: u128, f: fn(u32, u32, u32) -> u32) -> u128 {
    for e in 0..4 {
        let t = f(word(x, 1), word(x, 2), word(x, 3));
        y = y
            .wrapping_add(word(x, 0).rotate_left(5))
            .wrapping_add(t)
            .wrapping_add(word(w, e));
        let b = word(x, 1).rotate_left(30);
        set_elem(&mut x, 1, 32, u64::from(b));
        // <Y, X> = ROL(Y : X, 32).
        let top = word(x, 3);
        x = (x << 32) | u128::from(y);
        y = top;
    }
    x
}

/// SHA1SU1: the second half of the SHA-1 schedule update.
fn sha1_schedule_update1(d: u128, n: u128) -> u128 {
    let t = d ^ (n >> 32);
    words([
        word(t, 0).rotate_left(1),
        word(t, 1).rotate_left(1),
        word(t, 2).rotate_left(1),
        word(t, 3).rotate_left(1) ^ word(t, 0).rotate_left(2),
    ])
}

/// `SHA256hash(X, Y, W, part1)`.
fn sha256_hash(mut x: u128, mut y: u128, w: u128, part1: bool) -> u128 {
    for e in 0..4 {
        let chs = sha_choose(word(y, 0), word(y, 1), word(y, 2));
        let maj = sha_majority(word(x, 0), word(x, 1), word(x, 2));
        let t = word(y, 3)
            .wrapping_add(sha_hash_sigma1(word(y, 0)))
            .wrapping_add(chs)
            .wrapping_add(word(w, e));
        // X<127:96> and then Y<127:96>; the latter reads X<31:0>, which the
        // former leaves alone.
        let x3 = t.wrapping_add(word(x, 3));
        let y3 = t.wrapping_add(sha_hash_sigma0(word(x, 0))).wrapping_add(maj);
        set_elem(&mut x, 3, 32, u64::from(x3));
        set_elem(&mut y, 3, 32, u64::from(y3));
        // <Y, X> = ROL(Y : X, 32).
        let (x_top, y_top) = (word(x, 3), word(y, 3));
        x = (x << 32) | u128::from(y_top);
        y = (y << 32) | u128::from(x_top);
    }
    if part1 {
        x
    } else {
        y
    }
}

/// SHA256SU0: the first half of the SHA-256 schedule update.
fn sha256_schedule_update0(d: u128, n: u128) -> u128 {
    // T = Vn<31:0> : Vd<127:32>.
    let t = (n << 96) | (d >> 32);
    let mut result = 0u128;
    for e in 0..4 {
        let elt = word(t, e);
        let sigma = elt.rotate_right(7) ^ elt.rotate_right(18) ^ (elt >> 3);
        set_elem(&mut result, e, 32, u64::from(sigma.wrapping_add(word(d, e))));
    }
    result
}

/// SHA256SU1: the second half of the SHA-256 schedule update.
fn sha256_schedule_update1(d: u128, n: u128, m: u128) -> u128 {
    // T0 = Vm<31:0> : Vn<127:32>; the first two new words take their
    // sigma-1 input from Vm<127:64>, the last two from the first two.
    let t0 = (m << 96) | (n >> 32);
    let sigma1 = |x: u32| x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10);
    let mut result = 0u128;
    for e in 0..4 {
        let source = if e < 2 { word(m, e + 2) } else { word(result, e - 2) };
        let elt = sigma1(source).wrapping_add(word(d, e)).wrapping_add(word(t0, e));
        set_elem(&mut result, e, 32, u64::from(elt));
    }
    result
}

// ----- PMULL -----

/// `PolynomialMult` of two `width`-bit operands (the low `width` bits of `a`
/// and `b`, `width` up to 64): their product as polynomials over GF(2),
/// `2 * width` bits. PMUL and PMULL on bytes, and PMULL and PMULL2 on 64-bit
/// elements (FEAT_PMULL).
pub(crate) fn polynomial_mult(a: u64, b: u64, width: u32) -> u128 {
    let a = u128::from(a & ones(width));
    let mut product = 0u128;
    for i in 0..width {
        if (b >> i) & 1 != 0 {
            product ^= a << i;
        }
    }
    product
}

// ----- CRC32 -----

/// The CRC-32 polynomial (IEEE 802.3) of CRC32B/H/W/X.
const CRC32_POLY: u32 = 0x04C1_1DB7;
/// The CRC-32C polynomial (Castagnoli) of CRC32CB/CH/CW/CX.
const CRC32C_POLY: u32 = 0x1EDC_6F41;

/// The reflected byte table of `poly`: entry `i` is the remainder of byte
/// `i` shifted through the bit-reversed polynomial eight times.
const fn reflected_table(poly: u32) -> [u32; 256] {
    let reflected = poly.reverse_bits();
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut shifts = 0;
        while shifts < 8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ reflected } else { crc >> 1 };
            shifts += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

const CRC32_TABLE: [u32; 256] = reflected_table(CRC32_POLY);
const CRC32C_TABLE: [u32; 256] = reflected_table(CRC32C_POLY);

/// One CRC32 or CRC32C instruction: `acc` updated with the low `bytes`
/// bytes of `value`, least significant first. The pseudocode computes
/// `BitReverse(Poly32Mod2(BitReverse(acc):Zeros(size) EOR
/// BitReverse(val):Zeros(32), poly))`; in the bit-reversed (reflected)
/// domain the reversals cancel, and the division goes a byte at a time
/// through `table`. tests/unit/crypto.rs checks the two agree.
fn crc32_update(mut acc: u32, value: u64, bytes: u32, table: &[u32; 256]) -> u32 {
    for i in 0..bytes {
        let byte = (value >> (8 * i)) as u8;
        acc = (acc >> 8) ^ table[usize::from(acc as u8 ^ byte)];
    }
    acc
}

// ----- the instructions -----

impl Cpu {
    /// AESE, AESD, AESMC, AESIMC (Arm class `cryptoaes`):
    /// `01001110 00 10100 001 M D 10 Rn Rd`, where bit 13 (`M`) selects the
    /// MixColumns forms and bit 12 (`D`, as Arm names it) the decryption
    /// ones.
    pub(crate) fn exec_aes(&mut self, insn: u32) -> ExecResult {
        self.simd_check()?;
        let n = self.v[bits(insn, 9, 5) as usize];
        let rd = bits(insn, 4, 0) as usize;
        self.v[rd] = match (bit(insn, 13), bit(insn, 12)) {
            // AESE: AddRoundKey, then ShiftRows and SubBytes.
            (false, false) => aes_shift_sub(self.v[rd] ^ n),
            // AESD: AddRoundKey, then InvShiftRows and InvSubBytes.
            (false, true) => aes_inv_shift_sub(self.v[rd] ^ n),
            // AESMC, AESIMC.
            (true, false) => aes_mix_columns(n),
            (true, true) => aes_inv_mix_columns(n),
        };
        Ok(Flow::Next)
    }

    /// SHA1C, SHA1P, SHA1M, SHA1SU0, SHA256H, SHA256H2, SHA256SU1 (Arm
    /// class `cryptosha3`): `01011110 00 0 Rm 0 opcode 00 Rn Rd`.
    pub(crate) fn exec_sha_three(&mut self, pc: u64, insn: u32) -> ExecResult {
        let rd = bits(insn, 4, 0) as usize;
        let d = self.v[rd];
        let n = self.v[bits(insn, 9, 5) as usize];
        let m = self.v[bits(insn, 20, 16) as usize];
        let result = match bits(insn, 14, 12) {
            0b000 => sha1_rounds(d, word(n, 0), m, sha_choose),
            0b001 => sha1_rounds(d, word(n, 0), m, sha_parity),
            0b010 => sha1_rounds(d, word(n, 0), m, sha_majority),
            // SHA1SU0: (Vn<63:0> : Vd<127:64>) EOR Vd EOR Vm.
            0b011 => ((n << 64) | (d >> 64)) ^ d ^ m,
            0b100 => sha256_hash(d, n, m, true),
            0b101 => sha256_hash(n, d, m, false),
            0b110 => sha256_schedule_update1(d, n, m),
            // 111 is unallocated; the decoder hands this class none.
            _ => return self.undefined(pc, insn),
        };
        self.simd_check()?;
        self.v[rd] = result;
        Ok(Flow::Next)
    }

    /// SHA1H, SHA1SU1, SHA256SU0 (Arm class `cryptosha2`):
    /// `01011110 00 10100 000 opcode<1:0> 10 Rn Rd`.
    pub(crate) fn exec_sha_two(&mut self, pc: u64, insn: u32) -> ExecResult {
        let rd = bits(insn, 4, 0) as usize;
        let n = self.v[bits(insn, 9, 5) as usize];
        let result = match (bit(insn, 13), bit(insn, 12)) {
            // SHA1H: ROL(Vn<31:0>, 30), zero-extended: Sd is written.
            (false, false) => u128::from(word(n, 0).rotate_left(30)),
            (false, true) => sha1_schedule_update1(self.v[rd], n),
            (true, false) => sha256_schedule_update0(self.v[rd], n),
            // 00011 is unallocated; the decoder hands this class none.
            (true, true) => return self.undefined(pc, insn),
        };
        self.simd_check()?;
        self.v[rd] = result;
        Ok(Flow::Next)
    }

    /// CRC32B/H/W/X and CRC32CB/CH/CW/CX (Arm class `dp_2src`):
    /// `sf 0 0 11010110 Rm 010 C sz Rn Rd`, with `sz = 11` exactly when
    /// `sf = 1`, as the decoder hands them over. General-purpose
    /// instructions: `CPACR_EL1.FPEN` does not trap them.
    pub(crate) fn exec_crc32(&mut self, insn: u32) -> ExecResult {
        let bytes = 1 << bits(insn, 11, 10);
        let table = if bit(insn, 12) { &CRC32C_TABLE } else { &CRC32_TABLE };
        let acc = self.x_or_zr(bits(insn, 9, 5)) as u32;
        let value = self.x_or_zr(bits(insn, 20, 16));
        let crc = crc32_update(acc, value, bytes, table);
        self.set_x_or_zr_sized(bits(insn, 4, 0), u64::from(crc), false);
        Ok(Flow::Next)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/crypto.rs"]
mod tests;
