//! Round Float32 to an integral value (SSE4.1 ROUNDPS/ROUNDSS and the
//! AVX-512 VRNDSCALEPS/SS scaled forms).
//! Ported from Bochs softfloat3e/f32_round_to_int.cc.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Bochs softfloat3e `f32_round_to_int`. `scale` is the VRNDSCALE M field
/// (imm8[7:4]); it is 0 for plain ROUNDPS/ROUNDSS.
pub(in crate::cpu) fn f32_round_to_int_scaled(
    mut a: Float32,
    scale: u8,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> Float32 {
    let scale = (scale & 0xF) as i16;
    let exp = exp_f32(a);
    let mut frac = frac_f32(a);
    let sign = sign_f32(a);

    if 0x96 <= (exp + scale) {
        if exp == 0xFF && frac != 0 {
            return softfloat_propagate_nan_f32(a, 0, status);
        }
        return a;
    }

    if softfloat_denormals_are_zeros(status) && exp == 0 {
        frac = 0;
        a = pack_to_f32(sign, 0, 0);
    }

    if (exp + scale) <= 0x7E {
        if (exp as u32 | frac) == 0 {
            return a;
        }
        if exact {
            softfloat_raise_flags(status, FLAG_INEXACT);
        }
        let mut ui_z = pack_to_f32(sign, 0, 0);
        match rounding_mode {
            // round_near_even falls through to round_near_maxMag unless the
            // fraction is zero, in which case the C switch breaks out.
            ROUND_NEAR_EVEN | ROUND_NEAR_MAXMAG => {
                if !(rounding_mode == ROUND_NEAR_EVEN && frac == 0)
                    && (exp + scale) == 0x7E
                {
                    ui_z |= pack_to_f32(false, 0x7F - scale, 0);
                }
            }
            ROUND_MIN => {
                if ui_z != 0 {
                    ui_z = pack_to_f32(true, 0x7F - scale, 0);
                }
            }
            ROUND_MAX => {
                if ui_z == 0 {
                    ui_z = pack_to_f32(false, 0x7F - scale, 0);
                }
            }
            _ => {}
        }
        return ui_z;
    }

    let mut ui_z = a;
    let last_bit_mask: u32 = 1u32 << (0x96 - exp - scale);
    let round_bits_mask = last_bit_mask - 1;
    if rounding_mode == ROUND_NEAR_MAXMAG {
        ui_z = ui_z.wrapping_add(last_bit_mask >> 1);
    } else if rounding_mode == ROUND_NEAR_EVEN {
        ui_z = ui_z.wrapping_add(last_bit_mask >> 1);
        if (ui_z & round_bits_mask) == 0 {
            ui_z &= !last_bit_mask;
        }
    } else if rounding_mode == (if sign_f32(ui_z) { ROUND_MIN } else { ROUND_MAX }) {
        ui_z = ui_z.wrapping_add(round_bits_mask);
    }
    ui_z &= !round_bits_mask;
    if ui_z != a && exact {
        softfloat_raise_flags(status, FLAG_INEXACT);
    }
    ui_z
}

/// Bochs softfloat.h `f32_round_to_int(a, status)` — scale 0, MXCSR rounding
/// mode, exact reporting on.
#[inline]
pub(in crate::cpu) fn f32_round_to_int(a: Float32, status: &mut SoftFloatStatus) -> Float32 {
    let rc = softfloat_get_rounding_mode(status);
    f32_round_to_int_scaled(a, 0, rc, true, status)
}
