#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! i32 to extFloat80 conversion.
//! Ported from Berkeley SoftFloat 3e: i32_to_extF80.c

use super::internals::*;
use super::primitives::*;
use super::softfloat_types::*;

pub fn i32_to_extf80(a: i32) -> floatx80 {
    if a == 0 {
        return floatx80 {
            signif: 0,
            sign_exp: 0,
        };
    }

    let sign = a < 0;
    let abs_a = if sign { (-(a as i64)) as u32 } else { a as u32 };
    let shift_dist = count_leading_zeros32(abs_a);
    let ui_z64 = pack_to_extf80_sign_exp(sign, (0x401E - shift_dist as i32) as u16);
    let ui_z0 = ((abs_a as u64) << shift_dist) << 32;

    floatx80 {
        signif: ui_z0,
        sign_exp: ui_z64,
    }
}
