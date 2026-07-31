#![allow(dead_code, non_snake_case)]
//! Float64 addition and subtraction.
//! Ported from Berkeley SoftFloat 3e f64_addsub.c + s_addMagsF64.c + s_subMagsF64.c.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Berkeley SoftFloat `softfloat_addMagsF64`.
fn add_mags_f64(
    mut ui_a: u64,
    ui_b: u64,
    sign_z: bool,
    status: &mut SoftFloatStatus,
) -> float64 {
    let exp_a = exp_f64(ui_a);
    let mut sig_a = frac_f64(ui_a);
    let exp_b = exp_f64(ui_b);
    let mut sig_b = frac_f64(ui_b);

    if softfloat_denormalsAreZeros(status) {
        if exp_a == 0 {
            sig_a = 0;
            ui_a = pack_to_f64(sign_z, 0, 0);
        }
        if exp_b == 0 {
            sig_b = 0;
        }
    }

    let exp_diff = exp_a - exp_b;
    let mut exp_z;
    let mut sig_z;
    if exp_diff == 0 {
        if exp_a == 0 {
            let ui_z = ui_a.wrapping_add(sig_b);
            if (sig_a | sig_b) != 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
                let is_tiny = exp_f64(ui_z) == 0;
                if is_tiny {
                    if softfloat_flushUnderflowToZero(status) {
                        softfloat_raiseFlags(status, FLAG_UNDERFLOW | FLAG_INEXACT);
                        return pack_to_f64(sign_z, 0, 0);
                    }
                    if !softfloat_isMaskedException(status, FLAG_UNDERFLOW) {
                        softfloat_raiseFlags(status, FLAG_UNDERFLOW);
                    }
                }
            }
            return ui_z;
        }
        if exp_a == 0x7FF {
            if (sig_a | sig_b) != 0 {
                return softfloat_propagate_nan_f64(ui_a, ui_b, status);
            }
            return ui_a;
        }
        exp_z = exp_a;
        sig_z = 0x0020_0000_0000_0000u64
            .wrapping_add(sig_a)
            .wrapping_add(sig_b);
        sig_z <<= 9;
    } else {
        sig_a <<= 9;
        sig_b <<= 9;
        if exp_diff < 0 {
            if exp_b == 0x7FF {
                if sig_b != 0 {
                    return softfloat_propagate_nan_f64(ui_a, ui_b, status);
                }
                if sig_a != 0 && exp_a == 0 {
                    softfloat_raiseFlags(status, FLAG_DENORMAL);
                }
                return pack_to_f64(sign_z, 0x7FF, 0);
            }

            if (exp_a == 0 && sig_a != 0) || (exp_b == 0 && sig_b != 0) {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }

            exp_z = exp_b;
            if exp_a != 0 {
                sig_a = sig_a.wrapping_add(0x2000_0000_0000_0000);
            } else {
                sig_a <<= 1;
            }
            sig_a = shift_right_jam64(sig_a, (-exp_diff) as u32);
        } else {
            if exp_a == 0x7FF {
                if sig_a != 0 {
                    return softfloat_propagate_nan_f64(ui_a, ui_b, status);
                }
                if sig_b != 0 && exp_b == 0 {
                    softfloat_raiseFlags(status, FLAG_DENORMAL);
                }
                return ui_a;
            }

            if (exp_a == 0 && sig_a != 0) || (exp_b == 0 && sig_b != 0) {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }

            exp_z = exp_a;
            if exp_b != 0 {
                sig_b = sig_b.wrapping_add(0x2000_0000_0000_0000);
            } else {
                sig_b <<= 1;
            }
            sig_b = shift_right_jam64(sig_b, exp_diff as u32);
        }
        sig_z = 0x2000_0000_0000_0000u64
            .wrapping_add(sig_a)
            .wrapping_add(sig_b);
        if sig_z < 0x4000_0000_0000_0000 {
            exp_z -= 1;
            sig_z <<= 1;
        }
    }
    round_pack_to_f64(sign_z, exp_z, sig_z, status)
}

/// Berkeley SoftFloat `softfloat_subMagsF64`.
fn sub_mags_f64(
    ui_a: u64,
    ui_b: u64,
    mut sign_z: bool,
    status: &mut SoftFloatStatus,
) -> float64 {
    let mut exp_a = exp_f64(ui_a);
    let mut sig_a = frac_f64(ui_a);
    let exp_b = exp_f64(ui_b);
    let mut sig_b = frac_f64(ui_b);

    if softfloat_denormalsAreZeros(status) {
        if exp_a == 0 {
            sig_a = 0;
        }
        if exp_b == 0 {
            sig_b = 0;
        }
    }

    let exp_diff = exp_a - exp_b;
    if exp_diff == 0 {
        if exp_a == 0x7FF {
            if (sig_a | sig_b) != 0 {
                return softfloat_propagate_nan_f64(ui_a, ui_b, status);
            }
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOAT64_DEFAULT_NAN;
        }
        if exp_a == 0 && (sig_a | sig_b) != 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        let mut sig_diff = sig_a as i64 - sig_b as i64;
        if sig_diff == 0 {
            return pack_to_f64(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0);
        }
        if exp_a != 0 {
            exp_a -= 1;
        }
        if sig_diff < 0 {
            sign_z = !sign_z;
            sig_diff = -sig_diff;
        }
        let mut shift_dist = count_leading_zeros64(sig_diff as u64) as i16 - 11;
        let mut exp_z = exp_a - shift_dist;
        if exp_z < 0 {
            shift_dist = exp_a;
            exp_z = 0;
        }
        if exp_z == 0 && sig_diff != 0 {
            if softfloat_flushUnderflowToZero(status) {
                softfloat_raiseFlags(status, FLAG_UNDERFLOW | FLAG_INEXACT);
                return pack_to_f64(sign_z, 0, 0);
            }
            if !softfloat_isMaskedException(status, FLAG_UNDERFLOW) {
                softfloat_raiseFlags(status, FLAG_UNDERFLOW);
            }
        }
        pack_to_f64(sign_z, exp_z, (sig_diff << shift_dist) as u64)
    } else {
        sig_a <<= 10;
        sig_b <<= 10;
        let exp_z;
        let sig_z;
        if exp_diff < 0 {
            sign_z = !sign_z;
            if exp_b == 0x7FF {
                if sig_b != 0 {
                    return softfloat_propagate_nan_f64(ui_a, ui_b, status);
                }
                if sig_a != 0 && exp_a == 0 {
                    softfloat_raiseFlags(status, FLAG_DENORMAL);
                }
                return pack_to_f64(sign_z, 0x7FF, 0);
            }

            if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }

            sig_a = sig_a.wrapping_add(if exp_a != 0 {
                0x4000_0000_0000_0000
            } else {
                sig_a
            });
            sig_a = shift_right_jam64(sig_a, (-exp_diff) as u32);
            sig_b |= 0x4000_0000_0000_0000;
            exp_z = exp_b;
            sig_z = sig_b.wrapping_sub(sig_a);
        } else {
            if exp_a == 0x7FF {
                if sig_a != 0 {
                    return softfloat_propagate_nan_f64(ui_a, ui_b, status);
                }
                if sig_b != 0 && exp_b == 0 {
                    softfloat_raiseFlags(status, FLAG_DENORMAL);
                }
                return ui_a;
            }

            if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
            }

            sig_b = sig_b.wrapping_add(if exp_b != 0 {
                0x4000_0000_0000_0000
            } else {
                sig_b
            });
            sig_b = shift_right_jam64(sig_b, exp_diff as u32);
            sig_a |= 0x4000_0000_0000_0000;
            exp_z = exp_a;
            sig_z = sig_a.wrapping_sub(sig_b);
        }
        norm_round_pack_to_f64(sign_z, exp_z - 1, sig_z, status)
    }
}

/// Berkeley SoftFloat `f64_add`.
pub(crate) fn f64_add(a: float64, b: float64, status: &mut SoftFloatStatus) -> float64 {
    let sign_a = sign_f64(a);
    let sign_b = sign_f64(b);
    if sign_a == sign_b {
        add_mags_f64(a, b, sign_a, status)
    } else {
        sub_mags_f64(a, b, sign_a, status)
    }
}

/// Berkeley SoftFloat `f64_sub`.
pub(crate) fn f64_sub(a: float64, b: float64, status: &mut SoftFloatStatus) -> float64 {
    let sign_a = sign_f64(a);
    let sign_b = sign_f64(b);
    if sign_a == sign_b {
        sub_mags_f64(a, b, sign_a, status)
    } else {
        add_mags_f64(a, b, sign_a, status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(a: f64, b: f64) -> f64 {
        let mut st = SoftFloatStatus::default();
        f64::from_bits(f64_add(a.to_bits(), b.to_bits(), &mut st))
    }
    fn sub(a: f64, b: f64) -> f64 {
        let mut st = SoftFloatStatus::default();
        f64::from_bits(f64_sub(a.to_bits(), b.to_bits(), &mut st))
    }

    // Under default MXCSR (round-nearest-even, no DAZ/FTZ), softfloat results
    // must be bit-identical to the host's IEEE f64 arithmetic for all finite
    // outcomes (and NaN for NaN).
    #[test]
    fn f64_add_sub_match_native_rne() {
        let vals: [f64; 26] = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            2.0,
            -2.0,
            0.5,
            core::f64::consts::PI,
            -core::f64::consts::E,
            1e300,
            -1e300,
            1e-300,
            f64::MIN_POSITIVE,
            f64::MIN_POSITIVE / 2.0,
            123456.789,
            0.1,
            0.2,
            9_007_199_254_740_992.0,
            9_007_199_254_740_993.0,
            4_503_599_627_370_496.0,
            4_503_599_627_370_497.0,
            f64::MAX,
            f64::MIN,
            1.0000000000000002,
            0.9999999999999999,
            65504.0,
        ];
        for &a in &vals {
            for &b in &vals {
                let (na, sa) = (a + b, add(a, b));
                if na.is_nan() {
                    assert!(sa.is_nan(), "add {a} {b}");
                } else {
                    assert_eq!(sa.to_bits(), na.to_bits(), "add {a} + {b}");
                }
                let (ns, ss) = (a - b, sub(a, b));
                if ns.is_nan() {
                    assert!(ss.is_nan(), "sub {a} {b}");
                } else {
                    assert_eq!(ss.to_bits(), ns.to_bits(), "sub {a} - {b}");
                }
            }
        }
    }

    #[test]
    fn f64_add_sub_specials() {
        let inf = f64::INFINITY;
        assert_eq!(add(inf, 1.0).to_bits(), inf.to_bits());
        assert_eq!(add(f64::MAX, f64::MAX).to_bits(), inf.to_bits()); // overflow → inf
        assert!(add(inf, -inf).is_nan()); // inf - inf → NaN
        assert!(sub(inf, inf).is_nan());
        assert_eq!(sub(1.0, 1.0).to_bits(), 0.0f64.to_bits()); // exact zero, +0
    }

    // Subnormal arithmetic is where the native/softfloat split matters most:
    // these results are exact and denormal, and must round identically.
    #[test]
    fn f64_add_sub_subnormals() {
        let tiny = f64::from_bits(1); // smallest subnormal
        assert_eq!(add(tiny, tiny).to_bits(), 2);
        assert_eq!(sub(f64::MIN_POSITIVE, tiny).to_bits(), 0x000F_FFFF_FFFF_FFFF);
        assert_eq!(add(f64::MIN_POSITIVE, -f64::MIN_POSITIVE).to_bits(), 0);
    }
}
