//! Round Float64 to an integral value in floating-point format (Arm FRINT*).
//! Ported from Berkeley SoftFloat 3e f64_roundToInt.c (via Bochs
//! softfloat3e/f64_round_to_int.cc without its x86 VRNDSCALE scale field).

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Berkeley `f64_roundToInt`; see [`super::f32_round_to_int::f32_round_to_int`].
pub(crate) fn f64_round_to_int(
    a: Float64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> Float64 {
    let exp = exp_f64(a);
    let frac = frac_f64(a);
    let sign = sign_f64(a);

    if 0x433 <= exp {
        if exp == 0x7FF && frac != 0 {
            return softfloat_propagate_nan_f64(a, 0, status);
        }
        return a;
    }

    if exp <= 0x3FE {
        if (exp as u64 | frac) == 0 {
            return a;
        }
        if exact {
            softfloat_raise_flags(status, FLAG_INEXACT);
        }
        let mut ui_z = pack_to_f64(sign, 0, 0);
        match rounding_mode {
            // round_near_even falls through to round_near_maxMag unless the
            // fraction is zero (exactly 0.5), in which case the C switch
            // breaks out.
            ROUND_NEAR_EVEN | ROUND_NEAR_MAXMAG => {
                if !(rounding_mode == ROUND_NEAR_EVEN && frac == 0) && exp == 0x3FE {
                    ui_z |= pack_to_f64(false, 0x3FF, 0);
                }
            }
            ROUND_MIN if ui_z != 0 => ui_z = pack_to_f64(true, 0x3FF, 0),
            ROUND_MAX if ui_z == 0 => ui_z = pack_to_f64(false, 0x3FF, 0),
            _ => {}
        }
        return ui_z;
    }

    let mut ui_z = a;
    let last_bit_mask: u64 = 1u64 << (0x433 - exp);
    let round_bits_mask = last_bit_mask - 1;
    if rounding_mode == ROUND_NEAR_MAXMAG {
        ui_z = ui_z.wrapping_add(last_bit_mask >> 1);
    } else if rounding_mode == ROUND_NEAR_EVEN {
        ui_z = ui_z.wrapping_add(last_bit_mask >> 1);
        if (ui_z & round_bits_mask) == 0 {
            ui_z &= !last_bit_mask;
        }
    } else if rounding_mode == (if sign_f64(ui_z) { ROUND_MIN } else { ROUND_MAX }) {
        ui_z = ui_z.wrapping_add(round_bits_mask);
    }
    ui_z &= !round_bits_mask;
    if ui_z != a && exact {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    ui_z
}
