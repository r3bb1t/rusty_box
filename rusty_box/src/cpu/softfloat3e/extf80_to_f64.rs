#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 to float64 conversion.
//! Ported from Berkeley SoftFloat 3e: extF80_to_f64.c

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub fn extf80_to_f64(a: floatx80, status: &mut SoftFloatStatus) -> float64 {
    // Handle unsupported
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOAT64_DEFAULT_NAN;
    }

    let sign = sign_extf80(a.sign_exp);
    let exp = exp_extf80(a.sign_exp) as i32;
    let sig = a.signif;

    // Zero
    if (exp as u64 | sig) == 0 {
        return pack_float64(sign, 0, 0);
    }

    // Infinity or NaN
    if exp == 0x7FFF {
        if (sig & 0x7FFFFFFFFFFFFFFF) != 0 {
            // NaN → convert to f64 NaN (quieten)
            let nan_sig = (sig >> 1) | 0x0008000000000000;
            return pack_float64(sign, 0x7FF, nan_sig & 0x000FFFFFFFFFFFFF);
        }
        return pack_float64(sign, 0x7FF, 0); // Infinity
    }

    // Short shift right with jam
    let sig_jammed = short_shift_right_jam64(sig, 1);
    let mut adj_exp = exp - 0x3C01;
    if adj_exp < -0x1000 {
        adj_exp = -0x1000;
    }
    round_pack_to_f64(sign, adj_exp as i16, sig_jammed, status)
}
