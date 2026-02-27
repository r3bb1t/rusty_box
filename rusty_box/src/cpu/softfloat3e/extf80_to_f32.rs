#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 to float32 conversion.
//! Ported from Berkeley SoftFloat 3e: extF80_to_f32.c

use super::softfloat_types::*;
use super::softfloat::*;
use super::primitives::*;
use super::specialize::*;
use super::internals::*;

pub fn extf80_to_f32(a: floatx80, status: &mut SoftFloatStatus) -> float32 {
    // Handle unsupported
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOAT32_DEFAULT_NAN;
    }

    let sign = sign_extf80(a.sign_exp);
    let exp = exp_extf80(a.sign_exp) as i32;
    let sig = a.signif;

    // Infinity or NaN
    if exp == 0x7FFF {
        if (sig & 0x7FFFFFFFFFFFFFFF) != 0 {
            // NaN → convert to f32 NaN (quieten)
            let nan_sig = (sig >> 41) as u32 | 0x00400000;
            return pack_float32(sign, 0xFF, nan_sig & 0x003FFFFF);
        }
        return pack_float32(sign, 0xFF, 0); // Infinity
    }

    // Short shift right with jam to get 32-bit significand
    let sig32 = short_shift_right_jam64(sig, 33) as u32;
    if (exp as u32 | sig32 as u32) == 0 {
        return pack_float32(sign, 0, 0);
    }

    let mut adj_exp = exp - 0x3F81;
    // Clamp exponent for very small values
    if adj_exp < -0x1000 {
        adj_exp = -0x1000;
    }
    round_pack_to_f32(sign, adj_exp as i16, sig32, status)
}
