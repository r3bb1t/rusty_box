#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! Float32 to extFloat80 conversion.
//! Ported from Berkeley SoftFloat 3e: f32_to_extF80.c

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

pub fn f32_to_extf80(a: float32, status: &mut SoftFloatStatus) -> floatx80 {
    let sign = sign_f32(a);
    let exp = exp_f32(a);
    let frac = frac_f32(a);

    // Infinity or NaN
    if exp == 0xFF {
        if frac != 0 {
            // NaN: convert to extF80 NaN
            softfloat_raiseFlags(status, FLAG_INVALID);
            let sig = ((frac as u64) | 0x00400000) << 40 | 0x8000000000000000;
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
        let norm = norm_subnormal_f32_sig(frac);
        let exp = norm.exp;
        let frac = norm.sig;
        let ui_z64 = pack_to_extf80_sign_exp(sign, (exp + 0x3F80) as u16);
        let ui_z0 = ((frac | 0x00800000) as u64) << 40;
        return pack_to_extf80(ui_z64, ui_z0);
    }

    // Normal
    let ui_z64 = pack_to_extf80_sign_exp(sign, (exp as i32 + 0x3F80) as u16);
    let ui_z0 = ((frac | 0x00800000) as u64) << 40;
    pack_to_extf80(ui_z64, ui_z0)
}
