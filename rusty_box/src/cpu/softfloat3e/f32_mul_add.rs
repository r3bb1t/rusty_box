#![allow(dead_code, non_snake_case, unused_assignments)]
//! Float32 fused multiply-add: a*b + c (with operation modifier).
//! Ported from Berkeley SoftFloat 3e f32_mulAdd.c.

use super::f128::{SOFTFLOAT_MULADD_SUB_C, SOFTFLOAT_MULADD_SUB_PROD};
use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Float32 fused multiply-add: a*b + c (with operation modifier).
/// op=0: a*b+c, op=1: a*b-c, op=2: -(a*b)+c, op=3: -(a*b)-c
pub(crate) fn f32_mul_add(
    a: float32,
    b: float32,
    c: float32,
    op: u8,
    status: &mut SoftFloatStatus,
) -> float32 {
    let ui_a = a;
    let ui_b = b;
    let ui_c = c;

    let sign_a = sign_f32(ui_a);
    let mut exp_a = exp_f32(ui_a);
    let mut sig_a = frac_f32(ui_a);
    let sign_b = sign_f32(ui_b);
    let mut exp_b = exp_f32(ui_b);
    let mut sig_b = frac_f32(ui_b);
    let sign_c = sign_f32(ui_c) ^ ((op & SOFTFLOAT_MULADD_SUB_C) != 0);
    let mut exp_c = exp_f32(ui_c);
    let mut sig_c = frac_f32(ui_c);
    let sign_prod = sign_a ^ sign_b ^ ((op & SOFTFLOAT_MULADD_SUB_PROD) != 0);

    // NaN handling
    let a_is_nan = (exp_a == 0xFF) && sig_a != 0;
    let b_is_nan = (exp_b == 0xFF) && sig_b != 0;
    let c_is_nan = (exp_c == 0xFF) && sig_c != 0;
    if a_is_nan | b_is_nan | c_is_nan {
        let ui_z = if a_is_nan | b_is_nan {
            softfloat_propagate_nan_f32(ui_a, ui_b, status)
        } else {
            0
        };
        return softfloat_propagate_nan_f32(ui_z, ui_c, status);
    }

    // Denormals-are-zeros
    if softfloat_denormalsAreZeros(status) {
        if exp_a == 0 { sig_a = 0; }
        if exp_b == 0 { sig_b = 0; }
        if exp_c == 0 { sig_c = 0; }
    }

    // Infinity handling for A
    if exp_a == 0xFF {
        let mag_bits = (exp_b as u32) | sig_b;
        return inf_prod_arg_f32(sign_prod, sign_c, exp_a, sig_a, exp_b, sig_b, exp_c, sig_c, mag_bits, ui_c, status);
    }
    // Infinity handling for B
    if exp_b == 0xFF {
        let mag_bits = (exp_a as u32) | sig_a;
        return inf_prod_arg_f32(sign_prod, sign_c, exp_a, sig_a, exp_b, sig_b, exp_c, sig_c, mag_bits, ui_c, status);
    }
    // Infinity handling for C
    if exp_c == 0xFF {
        if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_to_f32(sign_c, 0xFF, 0);
    }

    // Handle subnormals for A
    if exp_a == 0 {
        if sig_a == 0 {
            // Denormal check for sigB before zeroProd
            if sig_b != 0 && exp_b == 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return zero_prod_f32(sign_prod, sign_c, exp_c, sig_c, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f32_sig(sig_a);
        exp_a = norm.exp;
        sig_a = norm.sig;
    }
    // Handle subnormals for B
    if exp_b == 0 {
        if sig_b == 0 {
            return zero_prod_f32(sign_prod, sign_c, exp_c, sig_c, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f32_sig(sig_b);
        exp_b = norm.exp;
        sig_b = norm.sig;
    }

    // Compute product
    let mut exp_prod = exp_a + exp_b - 0x7E;
    sig_a = (sig_a | 0x00800000) << 7;
    sig_b = (sig_b | 0x00800000) << 7;
    let mut sig_prod = (sig_a as u64) * (sig_b as u64);
    if sig_prod < 0x2000000000000000 {
        exp_prod -= 1;
        sig_prod <<= 1;
    }
    let mut sign_z = sign_prod;
    let mut exp_z: i16;
    let mut sig_z: u32;

    // Handle subnormal C
    if exp_c == 0 {
        if sig_c == 0 {
            exp_z = exp_prod - 1;
            sig_z = short_shift_right_jam64(sig_prod, 31) as u32;
            return round_pack_to_f32(sign_z, exp_z, sig_z, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f32_sig(sig_c);
        exp_c = norm.exp;
        sig_c = norm.sig;
    }
    sig_c = (sig_c | 0x00800000) << 6;

    // Align and add/subtract
    let exp_diff = exp_prod - exp_c;
    if sign_prod == sign_c {
        if exp_diff <= 0 {
            exp_z = exp_c;
            sig_z = sig_c.wrapping_add(shift_right_jam64(sig_prod, (32 - exp_diff) as u32) as u32);
        } else {
            exp_z = exp_prod;
            let sig64z = sig_prod.wrapping_add(shift_right_jam64((sig_c as u64) << 32, exp_diff as u32));
            sig_z = short_shift_right_jam64(sig64z, 32) as u32;
        }
        if sig_z < 0x40000000 {
            exp_z -= 1;
            sig_z = sig_z << 1;
        }
    } else {
        let sig64c = (sig_c as u64) << 32;
        let sig64z: u64;
        if exp_diff < 0 {
            sign_z = sign_c;
            exp_z = exp_c;
            sig64z = sig64c.wrapping_sub(shift_right_jam64(sig_prod, (-exp_diff) as u32));
        } else if exp_diff == 0 {
            exp_z = exp_prod;
            let diff = sig_prod.wrapping_sub(sig64c);
            if diff == 0 {
                return pack_to_f32(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0);
            }
            if diff & 0x8000000000000000 != 0 {
                sign_z = !sign_z;
                sig64z = 0u64.wrapping_sub(diff);
            } else {
                sig64z = diff;
            }
        } else {
            exp_z = exp_prod;
            sig64z = sig_prod.wrapping_sub(shift_right_jam64(sig64c, exp_diff as u32));
        }
        let shift_dist = count_leading_zeros64(sig64z) as i16 - 1;
        exp_z -= shift_dist;
        let shift_dist = shift_dist - 32;
        if shift_dist < 0 {
            sig_z = short_shift_right_jam64(sig64z, (-shift_dist) as u8) as u32;
        } else {
            sig_z = (sig64z as u32) << shift_dist;
        }
    }
    round_pack_to_f32(sign_z, exp_z, sig_z, status)
}

/// Handle the zeroProd case for f32 mulAdd.
fn zero_prod_f32(
    sign_prod: bool,
    sign_c: bool,
    exp_c: i16,
    sig_c: u32,
    status: &mut SoftFloatStatus,
) -> float32 {
    let mut ui_z = pack_to_f32(sign_c, exp_c, sig_c);
    if exp_c == 0 && sig_c != 0 {
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        if softfloat_flushUnderflowToZero(status) {
            softfloat_raiseFlags(status, FLAG_UNDERFLOW | FLAG_INEXACT);
            return pack_to_f32(sign_c, 0, 0);
        }
    }
    if ((exp_c as u32) | sig_c) == 0 && sign_prod != sign_c {
        ui_z = pack_to_f32(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0);
    }
    ui_z
}

/// Handle the infProdArg case for f32 mulAdd.
fn inf_prod_arg_f32(
    sign_prod: bool,
    sign_c: bool,
    exp_a: i16,
    sig_a: u32,
    exp_b: i16,
    sig_b: u32,
    exp_c: i16,
    sig_c: u32,
    mag_bits: u32,
    ui_c: float32,
    status: &mut SoftFloatStatus,
) -> float32 {
    if mag_bits != 0 {
        let ui_z = pack_to_f32(sign_prod, 0xFF, 0);
        if sign_prod == sign_c || exp_c != 0xFF {
            if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) || (sig_c != 0 && exp_c == 0) {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return ui_z;
        }
    }
    softfloat_raiseFlags(status, FLAG_INVALID);
    let ui_z = FLOAT32_DEFAULT_NAN;
    softfloat_propagate_nan_f32(ui_z, ui_c, status)
}
