#![allow(dead_code, non_snake_case, unused_assignments)]
//! Float64 fused multiply-add: a*b + c (with operation modifier).
//! Ported from Berkeley SoftFloat 3e f64_mulAdd.c.

use super::f128::{SOFTFLOAT_MULADD_SUB_C, SOFTFLOAT_MULADD_SUB_PROD, short_shift_right_jam128};
use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Float64 fused multiply-add: a*b + c (with operation modifier).
/// op=0: a*b+c, op=1: a*b-c, op=2: -(a*b)+c, op=3: -(a*b)-c
pub(crate) fn f64_mul_add(
    a: float64,
    b: float64,
    c: float64,
    op: u8,
    status: &mut SoftFloatStatus,
) -> float64 {
    let ui_a = a;
    let ui_b = b;
    let ui_c = c;

    let sign_a = sign_f64(ui_a);
    let mut exp_a = exp_f64(ui_a);
    let mut sig_a = frac_f64(ui_a);
    let sign_b = sign_f64(ui_b);
    let mut exp_b = exp_f64(ui_b);
    let mut sig_b = frac_f64(ui_b);
    let sign_c = sign_f64(ui_c) ^ ((op & SOFTFLOAT_MULADD_SUB_C) != 0);
    let mut exp_c = exp_f64(ui_c);
    let mut sig_c = frac_f64(ui_c);
    let mut sign_z = sign_a ^ sign_b ^ ((op & SOFTFLOAT_MULADD_SUB_PROD) != 0);

    // NaN handling
    let a_is_nan = (exp_a == 0x7FF) && sig_a != 0;
    let b_is_nan = (exp_b == 0x7FF) && sig_b != 0;
    let c_is_nan = (exp_c == 0x7FF) && sig_c != 0;
    if a_is_nan | b_is_nan | c_is_nan {
        let ui_z = if a_is_nan | b_is_nan {
            softfloat_propagate_nan_f64(ui_a, ui_b, status)
        } else {
            0
        };
        return softfloat_propagate_nan_f64(ui_z, ui_c, status);
    }

    // Denormals-are-zeros
    if softfloat_denormalsAreZeros(status) {
        if exp_a == 0 { sig_a = 0; }
        if exp_b == 0 { sig_b = 0; }
        if exp_c == 0 { sig_c = 0; }
    }

    // Infinity handling for A
    if exp_a == 0x7FF {
        let mag_bits = (exp_b as u64) | sig_b;
        return inf_prod_arg_f64(sign_z, sign_c, exp_a, sig_a, exp_b, sig_b, exp_c, sig_c, mag_bits, ui_c, status);
    }
    // Infinity handling for B
    if exp_b == 0x7FF {
        let mag_bits = (exp_a as u64) | sig_a;
        return inf_prod_arg_f64(sign_z, sign_c, exp_a, sig_a, exp_b, sig_b, exp_c, sig_c, mag_bits, ui_c, status);
    }
    // Infinity handling for C
    if exp_c == 0x7FF {
        if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_to_f64(sign_c, 0x7FF, 0);
    }

    // Handle subnormals for A
    if exp_a == 0 {
        if sig_a == 0 {
            // Denormal check for sigB before zeroProd
            if sig_b != 0 && exp_b == 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return zero_prod_f64(sign_z, sign_c, exp_c, sig_c, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f64_sig(sig_a);
        exp_a = norm.exp;
        sig_a = norm.sig;
    }
    // Handle subnormals for B
    if exp_b == 0 {
        if sig_b == 0 {
            return zero_prod_f64(sign_z, sign_c, exp_c, sig_c, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f64_sig(sig_b);
        exp_b = norm.exp;
        sig_b = norm.sig;
    }

    // Compute product
    let mut exp_z = exp_a + exp_b - 0x3FE;
    sig_a = (sig_a | 0x0010000000000000) << 10;
    sig_b = (sig_b | 0x0010000000000000) << 10;
    let (mut sig128z_64, mut sig128z_0) = mul64_to_128(sig_a, sig_b);
    if sig128z_64 < 0x2000000000000000 {
        exp_z -= 1;
        let (new64, new0) = add128(sig128z_64, sig128z_0, sig128z_64, sig128z_0);
        sig128z_64 = new64;
        sig128z_0 = new0;
    }

    // Handle subnormal C
    if exp_c == 0 {
        if sig_c == 0 {
            exp_z -= 1;
            let sig_z = (sig128z_64 << 1) | ((sig128z_0 != 0) as u64);
            return round_pack_to_f64(sign_z, exp_z, sig_z, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f64_sig(sig_c);
        exp_c = norm.exp;
        sig_c = norm.sig;
    }
    sig_c = (sig_c | 0x0010000000000000) << 9;

    // Align product and addend
    let exp_diff = exp_z - exp_c;
    if exp_diff < 0 {
        exp_z = exp_c;
        if (sign_z == sign_c) || (exp_diff < -1) {
            sig128z_64 = shift_right_jam64(sig128z_64, (-exp_diff) as u32);
        } else {
            let (new64, new0) = short_shift_right_jam128(sig128z_64, sig128z_0, 1);
            sig128z_64 = new64;
            sig128z_0 = new0;
        }
    }
    let (sig128c_64, sig128c_0) = if exp_diff > 0 {
        shift_right_jam128(sig_c, 0, exp_diff as u32)
    } else {
        (sig_c, 0u64)
    };

    let mut sig_z: u64;
    if sign_z == sign_c {
        if exp_diff <= 0 {
            sig_z = (sig_c.wrapping_add(sig128z_64)) | ((sig128z_0 != 0) as u64);
        } else {
            let (sum64, sum0) = add128(sig128z_64, sig128z_0, sig128c_64, sig128c_0);
            sig_z = sum64 | ((sum0 != 0) as u64);
        }
        if sig_z < 0x4000000000000000 {
            exp_z -= 1;
            sig_z = sig_z << 1;
        }
    } else {
        if exp_diff < 0 {
            sign_z = sign_c;
            let (d64, d0) = sub128(sig_c, 0, sig128z_64, sig128z_0);
            sig128z_64 = d64;
            sig128z_0 = d0;
        } else if exp_diff == 0 {
            sig128z_64 = sig128z_64.wrapping_sub(sig_c);
            if (sig128z_64 | sig128z_0) == 0 {
                return pack_to_f64(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0);
            }
            if sig128z_64 & 0x8000000000000000 != 0 {
                sign_z = !sign_z;
                let (d64, d0) = sub128(0, 0, sig128z_64, sig128z_0);
                sig128z_64 = d64;
                sig128z_0 = d0;
            }
        } else {
            let (d64, d0) = sub128(sig128z_64, sig128z_0, sig128c_64, sig128c_0);
            sig128z_64 = d64;
            sig128z_0 = d0;
        }

        if sig128z_64 == 0 {
            exp_z -= 64;
            sig128z_64 = sig128z_0;
            sig128z_0 = 0;
        }
        let shift_dist = count_leading_zeros64(sig128z_64) as i16 - 1;
        exp_z -= shift_dist;
        if shift_dist < 0 {
            sig_z = short_shift_right_jam64(sig128z_64, (-shift_dist) as u8);
        } else {
            let (new64, new0) = short_shift_left128(sig128z_64, sig128z_0, shift_dist as u8);
            sig128z_64 = new64;
            sig128z_0 = new0;
            sig_z = sig128z_64;
        }
        sig_z = sig_z | ((sig128z_0 != 0) as u64);
    }
    round_pack_to_f64(sign_z, exp_z, sig_z, status)
}

/// Handle the zeroProd case for f64 mulAdd.
fn zero_prod_f64(
    sign_prod: bool,
    sign_c: bool,
    exp_c: i16,
    sig_c: u64,
    status: &mut SoftFloatStatus,
) -> float64 {
    let mut ui_z = pack_to_f64(sign_c, exp_c, sig_c);
    if exp_c == 0 && sig_c != 0 {
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        if softfloat_flushUnderflowToZero(status) {
            softfloat_raiseFlags(status, FLAG_UNDERFLOW | FLAG_INEXACT);
            return pack_to_f64(sign_c, 0, 0);
        }
    }
    if ((exp_c as u64) | sig_c) == 0 && sign_prod != sign_c {
        ui_z = pack_to_f64(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0);
    }
    ui_z
}

/// Handle the infProdArg case for f64 mulAdd.
fn inf_prod_arg_f64(
    sign_z: bool,
    sign_c: bool,
    exp_a: i16,
    sig_a: u64,
    exp_b: i16,
    sig_b: u64,
    exp_c: i16,
    sig_c: u64,
    mag_bits: u64,
    ui_c: float64,
    status: &mut SoftFloatStatus,
) -> float64 {
    if mag_bits != 0 {
        let ui_z = pack_to_f64(sign_z, 0x7FF, 0);
        if sign_z == sign_c || exp_c != 0x7FF {
            if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) || (sig_c != 0 && exp_c == 0) {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return ui_z;
        }
    }
    softfloat_raiseFlags(status, FLAG_INVALID);
    let ui_z = FLOAT64_DEFAULT_NAN;
    softfloat_propagate_nan_f64(ui_z, ui_c, status)
}
