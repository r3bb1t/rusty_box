#![allow(dead_code, non_snake_case)]
//! Round float64 to an integral value (SSE4.1 ROUNDPD/ROUNDSD and the
//! AVX-512 VRNDSCALEPD/SD scaled forms).
//! Ported from Bochs softfloat3e/f64_roundToInt.cc.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Bochs softfloat3e `f64_roundToInt`. `scale` is the VRNDSCALE M field
/// (imm8[7:4]); it is 0 for plain ROUNDPD/ROUNDSD.
pub(in crate::cpu) fn f64_round_to_int_scaled(
    mut a: float64,
    scale: u8,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> float64 {
    let scale = (scale & 0xF) as i16;
    let exp = exp_f64(a);
    let mut frac = frac_f64(a);
    let sign = sign_f64(a);

    if 0x433 <= (exp + scale) {
        if exp == 0x7FF && frac != 0 {
            return softfloat_propagate_nan_f64(a, 0, status);
        }
        return a;
    }

    if softfloat_denormalsAreZeros(status) && exp == 0 {
        frac = 0;
        a = pack_to_f64(sign, 0, 0);
    }

    if (exp + scale) <= 0x3FE {
        if (exp as u64 | frac) == 0 {
            return a;
        }
        if exact {
            softfloat_raiseFlags(status, FLAG_INEXACT);
        }
        let mut ui_z = pack_to_f64(sign, 0, 0);
        match rounding_mode {
            // round_near_even falls through to round_near_maxMag unless the
            // fraction is zero, in which case the C switch breaks out.
            ROUND_NEAR_EVEN | ROUND_NEAR_MAXMAG => {
                if !(rounding_mode == ROUND_NEAR_EVEN && frac == 0)
                    && (exp + scale) == 0x3FE
                {
                    ui_z |= pack_to_f64(false, 0x3FF - scale, 0);
                }
            }
            ROUND_MIN => {
                if ui_z != 0 {
                    ui_z = pack_to_f64(true, 0x3FF - scale, 0);
                }
            }
            ROUND_MAX => {
                if ui_z == 0 {
                    ui_z = pack_to_f64(false, 0x3FF - scale, 0);
                }
            }
            _ => {}
        }
        return ui_z;
    }

    let mut ui_z = a;
    let last_bit_mask: u64 = 1u64 << (0x433 - exp - scale);
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
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    ui_z
}

/// Bochs softfloat.h `f64_roundToInt(a, status)` — scale 0, MXCSR rounding
/// mode, exact reporting on.
#[inline]
pub(in crate::cpu) fn f64_round_to_int(a: float64, status: &mut SoftFloatStatus) -> float64 {
    let rc = softfloat_getRoundingMode(status);
    f64_round_to_int_scaled(a, 0, rc, true, status)
}

/// Bochs softfloat.h `f64_roundToInt(a, scale, status)`.
#[inline]
pub(in crate::cpu) fn f64_round_to_int_with_scale(
    a: float64,
    scale: u8,
    status: &mut SoftFloatStatus,
) -> float64 {
    let rc = softfloat_getRoundingMode(status);
    f64_round_to_int_scaled(a, scale, rc, true, status)
}

#[cfg(test)]
mod tests {
    use super::super::f32_roundToInt::*;
    use super::*;

    fn r64(v: f64, rc: u8) -> f64 {
        let mut st = SoftFloatStatus {
            softfloat_roundingMode: rc,
            ..Default::default()
        };
        f64::from_bits(f64_round_to_int(v.to_bits(), &mut st))
    }
    fn r32(v: f32, rc: u8) -> f32 {
        let mut st = SoftFloatStatus {
            softfloat_roundingMode: rc,
            ..Default::default()
        };
        f32::from_bits(f32_round_to_int(v.to_bits(), &mut st))
    }

    #[test]
    fn round_to_int_honours_every_rounding_mode() {
        // ROUNDPD/ROUNDPS with imm8[2]=1 take the rounding mode from MXCSR.
        for (v, rne, rmin, rmax, rzero) in [
            (2.5f64, 2.0f64, 2.0f64, 3.0f64, 2.0f64),
            (3.5, 4.0, 3.0, 4.0, 3.0),
            (-2.5, -2.0, -3.0, -2.0, -2.0),
            (1.1, 1.0, 1.0, 2.0, 1.0),
            (-1.1, -1.0, -2.0, -1.0, -1.0),
            (0.5, 0.0, 0.0, 1.0, 0.0),
            (-0.5, -0.0, -1.0, -0.0, -0.0),
        ] {
            assert_eq!(r64(v, ROUND_NEAR_EVEN).to_bits(), rne.to_bits(), "rne {v}");
            assert_eq!(r64(v, ROUND_MIN).to_bits(), rmin.to_bits(), "min {v}");
            assert_eq!(r64(v, ROUND_MAX).to_bits(), rmax.to_bits(), "max {v}");
            assert_eq!(r64(v, ROUND_TO_ZERO).to_bits(), rzero.to_bits(), "rz {v}");

            let v32 = v as f32;
            assert_eq!(
                r32(v32, ROUND_NEAR_EVEN).to_bits(),
                (rne as f32).to_bits(),
                "f32 rne {v32}"
            );
            assert_eq!(
                r32(v32, ROUND_MIN).to_bits(),
                (rmin as f32).to_bits(),
                "f32 min {v32}"
            );
        }
    }

    #[test]
    fn round_to_int_passes_through_large_and_special_values() {
        assert_eq!(r64(1e300, ROUND_MAX).to_bits(), 1e300f64.to_bits());
        assert_eq!(
            r64(f64::INFINITY, ROUND_MIN).to_bits(),
            f64::INFINITY.to_bits()
        );
        assert!(r64(f64::NAN, ROUND_NEAR_EVEN).is_nan());
    }
}
