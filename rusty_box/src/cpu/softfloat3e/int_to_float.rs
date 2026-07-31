#![allow(dead_code, non_snake_case)]
//! Signed-integer to float32/float64 conversions.
//! Ported from Bochs softfloat3e/i32_to_f32.cc, i32_to_f64.cc,
//! i64_to_f32.cc and i64_to_f64.cc.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Bochs softfloat3e `i32_to_f32`.
pub(in crate::cpu) fn i32_to_f32(a: i32, status: &mut SoftFloatStatus) -> float32 {
    let sign = a < 0;
    if (a & 0x7FFF_FFFF) == 0 {
        return if sign { pack_to_f32(true, 0x9E, 0) } else { 0 };
    }
    let abs_a = if sign {
        (a as u32).wrapping_neg()
    } else {
        a as u32
    };
    norm_round_pack_to_f32(sign, 0x9C, abs_a, status)
}

/// Bochs softfloat3e `i32_to_f64`. Always exact — never rounds, so it takes
/// no status word.
pub(in crate::cpu) fn i32_to_f64(a: i32) -> float64 {
    if a == 0 {
        return 0;
    }
    let sign = a < 0;
    let abs_a = if sign {
        (a as u32).wrapping_neg()
    } else {
        a as u32
    };
    let shift_dist = count_leading_zeros32(abs_a) as i16 + 21;
    pack_to_f64(sign, 0x432 - shift_dist, (abs_a as u64) << shift_dist)
}

/// Bochs softfloat3e `i64_to_f32`.
pub(in crate::cpu) fn i64_to_f32(a: i64, status: &mut SoftFloatStatus) -> float32 {
    let sign = a < 0;
    let abs_a = if sign {
        (a as u64).wrapping_neg()
    } else {
        a as u64
    };
    let mut shift_dist = count_leading_zeros64(abs_a) as i16 - 40;
    if 0 <= shift_dist {
        return if a != 0 {
            pack_to_f32(sign, 0x95 - shift_dist, (abs_a as u32) << shift_dist)
        } else {
            0
        };
    }
    shift_dist += 7;
    let sig = if shift_dist < 0 {
        short_shift_right_jam64(abs_a, (-shift_dist) as u8) as u32
    } else {
        (abs_a as u32) << shift_dist
    };
    round_pack_to_f32(sign, 0x9C - shift_dist, sig, status)
}

/// Bochs softfloat3e `i64_to_f64`.
pub(in crate::cpu) fn i64_to_f64(a: i64, status: &mut SoftFloatStatus) -> float64 {
    let sign = a < 0;
    if (a & 0x7FFF_FFFF_FFFF_FFFF) == 0 {
        return if sign {
            pack_to_f64(true, 0x43E, 0)
        } else {
            0
        };
    }
    let abs_a = if sign {
        (a as u64).wrapping_neg()
    } else {
        a as u64
    };
    norm_round_pack_to_f64(sign, 0x43C, abs_a, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_to_float_matches_native_rne() {
        let vals32: [i32; 12] = [
            0,
            1,
            -1,
            2,
            -2,
            123456789,
            -123456789,
            i32::MAX,
            i32::MIN,
            16_777_217,
            -16_777_217,
            0x0100_0001,
        ];
        for &v in &vals32 {
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f32::from_bits(i32_to_f32(v, &mut st)).to_bits(),
                (v as f32).to_bits(),
                "i32_to_f32({v})"
            );
            assert_eq!(
                f64::from_bits(i32_to_f64(v)).to_bits(),
                (v as f64).to_bits(),
                "i32_to_f64({v})"
            );
        }
        let vals64: [i64; 12] = [
            0,
            1,
            -1,
            i64::MAX,
            i64::MIN,
            9_007_199_254_740_993,
            -9_007_199_254_740_993,
            123456789012345,
            -123456789012345,
            1 << 62,
            (1 << 53) + 1,
            -((1 << 53) + 1),
        ];
        for &v in &vals64 {
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f32::from_bits(i64_to_f32(v, &mut st)).to_bits(),
                (v as f32).to_bits(),
                "i64_to_f32({v})"
            );
            let mut st = SoftFloatStatus::default();
            assert_eq!(
                f64::from_bits(i64_to_f64(v, &mut st)).to_bits(),
                (v as f64).to_bits(),
                "i64_to_f64({v})"
            );
        }
    }
}
