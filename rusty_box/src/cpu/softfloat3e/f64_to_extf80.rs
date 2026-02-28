#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! Float64 to extFloat80 conversion.
//! Ported from Berkeley SoftFloat 3e: f64_to_extF80.c

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub fn f64_to_extf80(a: float64, status: &mut SoftFloatStatus) -> floatx80 {
    let sign = sign_f64(a);
    let exp = exp_f64(a);
    let frac = frac_f64(a);

    // Infinity or NaN
    if exp == 0x7FF {
        if frac != 0 {
            // NaN: convert to extF80 NaN
            softfloat_raiseFlags(status, FLAG_INVALID);
            let sig = (frac | 0x0008000000000000) << 11 | 0x8000000000000000;
            return pack_floatx80(sign, 0x7FFF, sig);
        }
        return pack_floatx80(sign, 0x7FFF, 0x8000000000000000);
    }

    // Zero or denormal
    if exp == 0 {
        if frac == 0 {
            return pack_floatx80(sign, 0, 0);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f64_sig(frac);
        let exp = norm.exp;
        let frac = norm.sig;
        let ui_z64 = pack_to_extf80_sign_exp(sign, (exp as i32 + 0x3C00) as u16);
        let ui_z0 = (frac | 0x0010000000000000) << 11;
        return pack_to_extf80(ui_z64, ui_z0);
    }

    // Normal
    let ui_z64 = pack_to_extf80_sign_exp(sign, (exp as i32 + 0x3C00) as u16);
    let ui_z0 = (frac | 0x0010000000000000) << 11;
    pack_to_extf80(ui_z64, ui_z0)
}
