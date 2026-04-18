#![allow(dead_code, non_snake_case, unused_assignments)]
//! Float16 fused multiply-add: a*b + c (with operation modifier).
//! Ported from Berkeley SoftFloat 3e f16_mulAdd.c.

use super::f128::{SOFTFLOAT_MULADD_SUB_C, SOFTFLOAT_MULADD_SUB_PROD};
use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Float16 fused multiply-add: a*b + c (with operation modifier).
/// op=0: a*b+c, op=1: a*b-c, op=2: -(a*b)+c, op=3: -(a*b)-c
pub(crate) fn f16_mul_add(
    a: float16,
    b: float16,
    c: float16,
    op: u8,
    status: &mut SoftFloatStatus,
) -> float16 {
    let ui_a = a;
    let ui_b = b;
    let ui_c = c;

    let sign_a = sign_f16(ui_a);
    let mut exp_a = exp_f16(ui_a);
    let mut sig_a = frac_f16(ui_a);
    let sign_b = sign_f16(ui_b);
    let mut exp_b = exp_f16(ui_b);
    let mut sig_b = frac_f16(ui_b);
    let sign_c = sign_f16(ui_c) ^ ((op & SOFTFLOAT_MULADD_SUB_C) != 0);
    let mut exp_c = exp_f16(ui_c);
    let mut sig_c = frac_f16(ui_c);
    let sign_prod = sign_a ^ sign_b ^ ((op & SOFTFLOAT_MULADD_SUB_PROD) != 0);

    // NaN handling
    let a_is_nan = (exp_a == 0x1F) && sig_a != 0;
    let b_is_nan = (exp_b == 0x1F) && sig_b != 0;
    let c_is_nan = (exp_c == 0x1F) && sig_c != 0;
    if a_is_nan | b_is_nan | c_is_nan {
        let ui_z = if a_is_nan | b_is_nan {
            softfloat_propagate_nan_f16(ui_a, ui_b, status)
        } else {
            0
        };
        return softfloat_propagate_nan_f16(ui_z, ui_c, status);
    }

    // Denormals-are-zeros
    if softfloat_denormalsAreZeros(status) {
        if exp_a == 0 { sig_a = 0; }
        if exp_b == 0 { sig_b = 0; }
        if exp_c == 0 { sig_c = 0; }
    }

    // Infinity handling for A
    if exp_a == 0x1F {
        let mag_bits = (exp_b as u16) | sig_b;
        return inf_prod_arg(sign_prod, sign_c, exp_a, sig_a, exp_b, sig_b, exp_c, sig_c, mag_bits, ui_c, status);
    }
    // Infinity handling for B
    if exp_b == 0x1F {
        let mag_bits = (exp_a as u16) | sig_a;
        return inf_prod_arg(sign_prod, sign_c, exp_a, sig_a, exp_b, sig_b, exp_c, sig_c, mag_bits, ui_c, status);
    }
    // Infinity handling for C
    if exp_c == 0x1F {
        if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_to_f16(sign_c, 0x1F, 0);
    }

    // Handle subnormals for A
    if exp_a == 0 {
        if sig_a == 0 {
            // Denormal check for sigB before zeroProd
            if sig_b != 0 && exp_b == 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return zero_prod_f16(sign_prod, sign_c, exp_c, sig_c, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f16_sig(sig_a);
        exp_a = norm.exp;
        sig_a = norm.sig;
    }
    // Handle subnormals for B
    if exp_b == 0 {
        if sig_b == 0 {
            return zero_prod_f16(sign_prod, sign_c, exp_c, sig_c, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f16_sig(sig_b);
        exp_b = norm.exp;
        sig_b = norm.sig;
    }

    // Compute product
    let mut exp_prod = exp_a + exp_b - 0xE;
    sig_a = (sig_a | 0x0400) << 4;
    sig_b = (sig_b | 0x0400) << 4;
    let mut sig_prod = (sig_a as u32) * (sig_b as u32);
    if sig_prod < 0x20000000 {
        exp_prod -= 1;
        sig_prod <<= 1;
    }
    let mut sign_z = sign_prod;
    let mut exp_z: i16;
    let mut sig_z: u16;

    // Handle subnormal C
    if exp_c == 0 {
        if sig_c == 0 {
            exp_z = exp_prod - 1;
            sig_z = (sig_prod >> 15) as u16 | (((sig_prod & 0x7FFF) != 0) as u16);
            return round_pack_to_f16(sign_z, exp_z, sig_z, status);
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_f16_sig(sig_c);
        exp_c = norm.exp;
        sig_c = norm.sig;
    }
    sig_c = (sig_c | 0x0400) << 3;

    // Align and add/subtract
    let exp_diff = exp_prod - exp_c;
    if sign_prod == sign_c {
        if exp_diff <= 0 {
            exp_z = exp_c;
            sig_z = sig_c.wrapping_add(shift_right_jam32(sig_prod, (16 - exp_diff) as u16) as u16);
        } else {
            exp_z = exp_prod;
            let sig32z = sig_prod.wrapping_add(shift_right_jam32((sig_c as u32) << 16, exp_diff as u16));
            sig_z = (sig32z >> 16) as u16 | (((sig32z & 0xFFFF) != 0) as u16);
        }
        if sig_z < 0x4000 {
            exp_z -= 1;
            sig_z <<= 1;
        }
    } else {
        let sig32c = (sig_c as u32) << 16;
        let sig32z: u32;
        if exp_diff < 0 {
            sign_z = sign_c;
            exp_z = exp_c;
            sig32z = sig32c.wrapping_sub(shift_right_jam32(sig_prod, (-exp_diff) as u16));
        } else if exp_diff == 0 {
            exp_z = exp_prod;
            let diff = sig_prod.wrapping_sub(sig32c);
            if diff == 0 {
                return pack_to_f16(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0);
            }
            if diff & 0x80000000 != 0 {
                sign_z = !sign_z;
                sig32z = 0u32.wrapping_sub(diff);
            } else {
                sig32z = diff;
            }
        } else {
            exp_z = exp_prod;
            sig32z = sig_prod.wrapping_sub(shift_right_jam32(sig32c, exp_diff as u16));
        }
        let shift_dist = count_leading_zeros32(sig32z) as i16 - 1;
        exp_z -= shift_dist;
        let shift_dist = shift_dist - 16;
        if shift_dist < 0 {
            sig_z = (sig32z >> ((-shift_dist) as u32)) as u16
                | (((sig32z << (shift_dist.wrapping_neg() as u32 & 31)) as u32 != 0) as u16);
        } else {
            sig_z = (sig32z as u16) << shift_dist;
        }
    }
    round_pack_to_f16(sign_z, exp_z, sig_z, status)
}

/// Handle the zeroProd case for f16 mulAdd.
fn zero_prod_f16(
    sign_prod: bool,
    sign_c: bool,
    exp_c: i16,
    sig_c: u16,
    status: &mut SoftFloatStatus,
) -> float16 {
    let mut ui_z = pack_to_f16(sign_c, exp_c, sig_c);
    if exp_c == 0 && sig_c != 0 {
        // Exact zero plus a denormal
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        if softfloat_flushUnderflowToZero(status) {
            softfloat_raiseFlags(status, FLAG_UNDERFLOW | FLAG_INEXACT);
            return pack_to_f16(sign_c, 0, 0);
        }
    }
    if ((exp_c as u16) | sig_c) == 0 && sign_prod != sign_c {
        ui_z = pack_to_f16(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0);
    }
    ui_z
}

/// Handle the infProdArg case for f16 mulAdd.
fn inf_prod_arg(
    sign_prod: bool,
    sign_c: bool,
    exp_a: i16,
    sig_a: u16,
    exp_b: i16,
    sig_b: u16,
    exp_c: i16,
    sig_c: u16,
    mag_bits: u16,
    ui_c: float16,
    status: &mut SoftFloatStatus,
) -> float16 {
    if mag_bits != 0 {
        let ui_z = pack_to_f16(sign_prod, 0x1F, 0);
        if sign_prod == sign_c || exp_c != 0x1F {
            if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) || (sig_c != 0 && exp_c == 0) {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }
            return ui_z;
        }
    }
    softfloat_raiseFlags(status, FLAG_INVALID);
    let ui_z = FLOAT16_DEFAULT_NAN;
    softfloat_propagate_nan_f16(ui_z, ui_c, status)
}
