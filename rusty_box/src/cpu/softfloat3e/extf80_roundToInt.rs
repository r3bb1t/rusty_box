#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 round-to-integer.
//! Ported from Berkeley SoftFloat 3e: extF80_roundToInt.c

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Round extFloat80 to integer using given rounding mode.
pub fn extf80_round_to_int(
    a: floatx80,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> floatx80 {
    // Handle unsupported
    if extf80_is_unsupported(a) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    let ui_a64 = a.sign_exp;
    let sign_ui64 = ui_a64 & pack_to_extf80_sign_exp(true, 0);
    let exp = exp_extf80(ui_a64) as i32;
    let sig_a = a.signif;

    // Already integer (or infinity)
    if 0x403E <= exp {
        if (exp == 0x7FFF) && ((sig_a << 1) != 0) {
            return softfloat_propagate_nan_extf80(ui_a64, sig_a, 0, 0, status);
        }
        return a;
    }

    // Less than 1.0
    if exp <= 0x3FFE {
        if exp == 0 {
            if (sig_a << 1) == 0 {
                return a; // ±0
            }
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        if exact {
            softfloat_raiseFlags(status, FLAG_INEXACT);
        }
        match rounding_mode {
            ROUND_NEAR_EVEN => {
                if (sig_a & 0x7FFFFFFFFFFFFFFF) == 0 {
                    // Exactly 0.5 with no trailing bits — rounds to even (0)
                    return pack_to_extf80(sign_ui64, 0);
                }
                if exp == 0x3FFE {
                    // >= 0.5: round up to 1.0
                    softfloat_setRoundingUp(status);
                    return pack_floatx80(sign_extf80(sign_ui64), 0x3FFF, 0x8000000000000000);
                }
                return pack_to_extf80(sign_ui64, 0);
            }
            ROUND_NEAR_MAXMAG => {
                if exp == 0x3FFE {
                    softfloat_setRoundingUp(status);
                    return pack_floatx80(sign_extf80(sign_ui64), 0x3FFF, 0x8000000000000000);
                }
                return pack_to_extf80(sign_ui64, 0);
            }
            ROUND_MIN => {
                if sign_ui64 != 0 {
                    softfloat_setRoundingUp(status);
                    return pack_floatx80(true, 0x3FFF, 0x8000000000000000);
                }
                return pack_to_extf80(sign_ui64, 0);
            }
            ROUND_MAX => {
                if sign_ui64 == 0 {
                    softfloat_setRoundingUp(status);
                    return pack_floatx80(false, 0x3FFF, 0x8000000000000000);
                }
                return pack_to_extf80(sign_ui64, 0);
            }
            _ => {
                // ROUND_MINMAG (toward zero)
                return pack_to_extf80(sign_ui64, 0);
            }
        }
    }

    // Normal case: 1.0 <= |a| < 2^63
    let mut ui_z64 = sign_ui64 | (exp as u16);
    let last_bit_mask: u64 = 1u64 << (0x403E - exp);
    let round_bits_mask = last_bit_mask - 1;
    let mut sig_z = sig_a;

    match rounding_mode {
        ROUND_NEAR_MAXMAG => {
            sig_z = sig_z.wrapping_add(last_bit_mask >> 1);
        }
        ROUND_NEAR_EVEN => {
            sig_z = sig_z.wrapping_add(last_bit_mask >> 1);
            if (sig_z & round_bits_mask) == 0 {
                sig_z &= !last_bit_mask;
            }
        }
        ROUND_MIN | ROUND_MAX => {
            let round_up = if sign_ui64 != 0 {
                rounding_mode == ROUND_MIN
            } else {
                rounding_mode == ROUND_MAX
            };
            if round_up {
                sig_z = sig_z.wrapping_add(round_bits_mask);
            }
        }
        _ => {} // ROUND_MINMAG: truncate
    }

    sig_z &= !round_bits_mask;
    if sig_z == 0 {
        ui_z64 = ui_z64.wrapping_add(1);
        sig_z = 0x8000000000000000;
        softfloat_setRoundingUp(status);
    }
    if sig_z != sig_a {
        if exact {
            softfloat_raiseFlags(status, FLAG_INEXACT);
        }
        if sig_z > sig_a {
            softfloat_setRoundingUp(status);
        }
    }
    pack_to_extf80(ui_z64, sig_z)
}
