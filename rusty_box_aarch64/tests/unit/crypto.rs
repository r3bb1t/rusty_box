//! Unit tests of `src/cpu/crypto.rs`: its tables and helper functions
//! against the definitions they are computed from. The instructions
//! themselves are tested as a guest runs them, in `tests/crypto.rs`, and
//! against Unicorn in the vector replay.

use super::*;

/// FIPS-197 Figure 7 and Figure 14: entries of the S-box and its inverse.
#[test]
fn the_s_boxes_are_the_fips_197_tables() {
    assert_eq!(SBOX[0x00], 0x63);
    assert_eq!(SBOX[0x01], 0x7C);
    assert_eq!(SBOX[0x53], 0xED);
    assert_eq!(SBOX[0xFF], 0x16);
    assert_eq!(INV_SBOX[0x63], 0x00);
    assert_eq!(INV_SBOX[0x00], 0x52);
    for i in 0..=255u8 {
        assert_eq!(INV_SBOX[usize::from(SBOX[usize::from(i)])], i);
    }
}

/// The MixColumns example column db 13 53 45 -> 8e 4d a1 bc, and
/// InvMixColumns undoing it.
#[test]
fn mix_columns_and_its_inverse() {
    let column = u128::from_le_bytes([0xDB, 0x13, 0x53, 0x45, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1]);
    let mixed = aes_mix_columns(column);
    assert_eq!(mixed.to_le_bytes()[..4], [0x8E, 0x4D, 0xA1, 0xBC]);
    assert_eq!(mixed.to_le_bytes()[12..], [1, 1, 1, 1], "{{01}} columns are fixed points");
    assert_eq!(aes_inv_mix_columns(mixed), column);
}

/// Carry-less multiplication: (x + 1)^2 = x^2 + 1 over GF(2), one is the
/// identity, the top bits of both operands meet at bit `2 * width - 2`, and
/// only the low `width` bits of each operand take part.
#[test]
fn polynomial_multiplication_is_carry_less() {
    assert_eq!(polynomial_mult(0b11, 0b11, 8), 0b101);
    assert_eq!(polynomial_mult(0xFF, 0xFF, 8), 0x5555);
    assert_eq!(polynomial_mult(0x1FF, 0x101, 8), 0xFF, "bits above the width are not operands");
    assert_eq!(polynomial_mult(0xDEAD_BEEF_0123_4567, 1, 64), 0xDEAD_BEEF_0123_4567);
    assert_eq!(polynomial_mult(1 << 63, 1 << 63, 64), 1 << 126);
    assert_eq!(polynomial_mult(u64::MAX, 0, 64), 0);
}

/// `Poly32Mod2(data, poly)` (Arm ARM shared pseudocode) for `data` of
/// `width` bits: the remainder of `data` divided by `x^32 + poly` over
/// GF(2).
fn poly32_mod2(mut data: u128, width: u32, poly: u32) -> u32 {
    for i in (32..width).rev() {
        if (data >> i) & 1 != 0 {
            data ^= u128::from(poly) << (i - 32);
        }
    }
    data as u32
}

/// CRC32 and CRC32C exactly as their pseudocode writes them, for `size`
/// bits of `value`: `BitReverse(Poly32Mod2(BitReverse(acc):Zeros(size) EOR
/// BitReverse(val):Zeros(32), poly))`.
fn pseudocode_crc32(acc: u32, value: u64, size: u32, poly: u32) -> u32 {
    let reversed_value = (value & ones(size)).reverse_bits() >> (64 - size);
    let temp_acc = u128::from(acc.reverse_bits()) << size;
    let temp_val = u128::from(reversed_value) << 32;
    poly32_mod2(temp_acc ^ temp_val, size + 32, poly).reverse_bits()
}

/// The byte tables give what the pseudocode gives, for every size and both
/// polynomials.
#[test]
fn the_table_crc_is_the_pseudocode_crc() {
    let mut seed = 0x1234_5678_9ABC_DEF0u64;
    for _ in 0..2000 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let acc = (seed >> 32) as u32;
        let value = seed.rotate_left(17) ^ 0xA5A5_5A5A_0F0F_F0F0;
        for (poly, table) in [(CRC32_POLY, &CRC32_TABLE), (CRC32C_POLY, &CRC32C_TABLE)] {
            for bytes in [1u32, 2, 4, 8] {
                assert_eq!(
                    crc32_update(acc, value, bytes, table),
                    pseudocode_crc32(acc, value, 8 * bytes, poly),
                    "{poly:#x}, {bytes} bytes, acc {acc:#x}, value {value:#x}"
                );
            }
        }
    }
}
