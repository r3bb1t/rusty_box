#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! i64 to extFloat80 conversion.
//! Ported from Berkeley SoftFloat 3e: i64_to_extF80.c

use super::softfloat_types::*;
use super::primitives::*;
use super::internals::*;

pub fn i64_to_extf80(a: i64) -> floatx80 {
    if a == 0 {
        return floatx80 { signif: 0, sign_exp: 0 };
    }

    let sign = a < 0;
    let abs_a = if sign { (-(a as i128)) as u64 } else { a as u64 };
    let shift_dist = count_leading_zeros64(abs_a);
    let ui_z64 = pack_to_extf80_sign_exp(sign, (0x403E - shift_dist as i32) as u16);
    let ui_z0 = abs_a << shift_dist;

    floatx80 {
        signif: ui_z0,
        sign_exp: ui_z64,
    }
}
