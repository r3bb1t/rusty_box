//! Float32 addition and subtraction.
//! Ported from Berkeley SoftFloat 3e f32_addsub.c + s_addMagsF32.c + s_subMagsF32.c.

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

/// Berkeley SoftFloat `softfloat_addMagsF32`.
fn add_mags_f32(mut ui_a: u32, ui_b: u32, status: &mut SoftFloatStatus) -> Float32 {
    let exp_a = exp_f32(ui_a);
    let mut sig_a = frac_f32(ui_a);
    let exp_b = exp_f32(ui_b);
    let mut sig_b = frac_f32(ui_b);
    let sign_z = sign_f32(ui_a);

    if softfloat_denormals_are_zeros(status) {
        if exp_a == 0 {
            sig_a = 0;
            ui_a = pack_to_f32(sign_z, 0, 0);
        }
        if exp_b == 0 {
            sig_b = 0;
        }
    }

    let exp_diff = exp_a - exp_b;
    let exp_z;
    let mut sig_z;
    if exp_diff == 0 {
        if exp_a == 0 {
            let ui_z = ui_a.wrapping_add(sig_b);
            if (sig_a | sig_b) != 0 {
                softfloat_raise_flags(status, FLAG_DENORMAL);
                let is_tiny = exp_f32(ui_z) == 0;
                if is_tiny {
                    if softfloat_flush_underflow_to_zero(status) {
                        softfloat_raise_flags(status, FLAG_UNDERFLOW | FLAG_INEXACT);
                        return pack_to_f32(sign_z, 0, 0);
                    }
                    if !softfloat_is_masked_exception(status, FLAG_UNDERFLOW) {
                        softfloat_raise_flags(status, FLAG_UNDERFLOW);
                    }
                }
            }
            return ui_z;
        }
        if exp_a == 0xFF {
            if (sig_a | sig_b) != 0 {
                return softfloat_propagate_nan_f32(ui_a, ui_b, status);
            }
            return ui_a;
        }
        exp_z = exp_a;
        sig_z = 0x0100_0000 + sig_a + sig_b;
        if (sig_z & 1) == 0 && exp_z < 0xFE {
            return pack_to_f32(sign_z, exp_z, sig_z >> 1);
        }
        sig_z <<= 6;
    } else {
        sig_a <<= 6;
        sig_b <<= 6;
        if exp_diff < 0 {
            if exp_b == 0xFF {
                if sig_b != 0 {
                    return softfloat_propagate_nan_f32(ui_a, ui_b, status);
                }
                if sig_a != 0 && exp_a == 0 {
                    softfloat_raise_flags(status, FLAG_DENORMAL);
                }
                return pack_to_f32(sign_z, 0xFF, 0);
            }
            if (exp_a == 0 && sig_a != 0) || (exp_b == 0 && sig_b != 0) {
                softfloat_raise_flags(status, FLAG_DENORMAL);
            }
            exp_z = exp_b;
            sig_a += if exp_a != 0 { 0x2000_0000 } else { sig_a };
            sig_a = shift_right_jam32(sig_a, (-exp_diff) as u16);
        } else {
            if exp_a == 0xFF {
                if sig_a != 0 {
                    return softfloat_propagate_nan_f32(ui_a, ui_b, status);
                }
                if sig_b != 0 && exp_b == 0 {
                    softfloat_raise_flags(status, FLAG_DENORMAL);
                }
                return ui_a;
            }
            if (exp_a == 0 && sig_a != 0) || (exp_b == 0 && sig_b != 0) {
                softfloat_raise_flags(status, FLAG_DENORMAL);
            }
            exp_z = exp_a;
            sig_b += if exp_b != 0 { 0x2000_0000 } else { sig_b };
            sig_b = shift_right_jam32(sig_b, exp_diff as u16);
        }
        sig_z = 0x2000_0000 + sig_a + sig_b;
        if sig_z < 0x4000_0000 {
            // NOTE: exp_z decremented below via mutable copy
            return round_pack_to_f32(sign_z, exp_z - 1, sig_z << 1, status);
        }
    }
    round_pack_to_f32(sign_z, exp_z, sig_z, status)
}

/// Berkeley SoftFloat `softfloat_subMagsF32`.
fn sub_mags_f32(ui_a: u32, ui_b: u32, status: &mut SoftFloatStatus) -> Float32 {
    let mut exp_a = exp_f32(ui_a);
    let mut sig_a = frac_f32(ui_a);
    let exp_b = exp_f32(ui_b);
    let mut sig_b = frac_f32(ui_b);

    if softfloat_denormals_are_zeros(status) {
        if exp_a == 0 {
            sig_a = 0;
        }
        if exp_b == 0 {
            sig_b = 0;
        }
    }

    let exp_diff = exp_a - exp_b;
    if exp_diff == 0 {
        if exp_a == 0xFF {
            if (sig_a | sig_b) != 0 {
                return softfloat_propagate_nan_f32(ui_a, ui_b, status);
            }
            softfloat_raise_flags(status, FLAG_INVALID);
            return FLOAT32_DEFAULT_NAN;
        }
        if exp_a == 0 && (sig_a | sig_b) != 0 {
            softfloat_raise_flags(status, FLAG_DENORMAL);
        }
        let mut sig_diff = sig_a as i32 - sig_b as i32;
        if sig_diff == 0 {
            return pack_to_f32(softfloat_get_rounding_mode(status) == ROUND_MIN, 0, 0);
        }
        if exp_a != 0 {
            exp_a -= 1;
        }
        let mut sign_z = sign_f32(ui_a);
        if sig_diff < 0 {
            sign_z = !sign_z;
            sig_diff = -sig_diff;
        }
        let mut shift_dist = count_leading_zeros32(sig_diff as u32) as i16 - 8;
        let mut exp_z = exp_a - shift_dist;
        if exp_z < 0 {
            shift_dist = exp_a;
            exp_z = 0;
        }
        if exp_z == 0 && sig_diff != 0 {
            if softfloat_flush_underflow_to_zero(status) {
                softfloat_raise_flags(status, FLAG_UNDERFLOW | FLAG_INEXACT);
                return pack_to_f32(sign_z, 0, 0);
            }
            if !softfloat_is_masked_exception(status, FLAG_UNDERFLOW) {
                softfloat_raise_flags(status, FLAG_UNDERFLOW);
            }
        }
        pack_to_f32(sign_z, exp_z, (sig_diff << shift_dist) as u32)
    } else {
        let mut sign_z = sign_f32(ui_a);
        sig_a <<= 7;
        sig_b <<= 7;
        let exp_z;
        let sig_x;
        let sig_y;
        let jam_dist;
        if exp_diff < 0 {
            sign_z = !sign_z;
            if exp_b == 0xFF {
                if sig_b != 0 {
                    return softfloat_propagate_nan_f32(ui_a, ui_b, status);
                }
                if sig_a != 0 && exp_a == 0 {
                    softfloat_raise_flags(status, FLAG_DENORMAL);
                }
                return pack_to_f32(sign_z, 0xFF, 0);
            }
            if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) {
                softfloat_raise_flags(status, FLAG_DENORMAL);
            }
            exp_z = exp_b - 1;
            sig_x = sig_b | 0x4000_0000;
            sig_y = sig_a + if exp_a != 0 { 0x4000_0000 } else { sig_a };
            jam_dist = (-exp_diff) as u16;
        } else {
            if exp_a == 0xFF {
                if sig_a != 0 {
                    return softfloat_propagate_nan_f32(ui_a, ui_b, status);
                }
                if sig_b != 0 && exp_b == 0 {
                    softfloat_raise_flags(status, FLAG_DENORMAL);
                }
                return ui_a;
            }
            if (sig_a != 0 && exp_a == 0) || (sig_b != 0 && exp_b == 0) {
                softfloat_raise_flags(status, FLAG_DENORMAL);
            }
            exp_z = exp_a - 1;
            sig_x = sig_a | 0x4000_0000;
            sig_y = sig_b + if exp_b != 0 { 0x4000_0000 } else { sig_b };
            jam_dist = exp_diff as u16;
        }
        norm_round_pack_to_f32(
            sign_z,
            exp_z,
            sig_x.wrapping_sub(shift_right_jam32(sig_y, jam_dist)),
            status,
        )
    }
}

/// Berkeley SoftFloat `f32_add`.
pub(in crate::cpu) fn f32_add(a: Float32, b: Float32, status: &mut SoftFloatStatus) -> Float32 {
    if sign_f32(a ^ b) {
        sub_mags_f32(a, b, status)
    } else {
        add_mags_f32(a, b, status)
    }
}

/// Berkeley SoftFloat `f32_sub`.
pub(in crate::cpu) fn f32_sub(a: Float32, b: Float32, status: &mut SoftFloatStatus) -> Float32 {
    if sign_f32(a ^ b) {
        add_mags_f32(a, b, status)
    } else {
        sub_mags_f32(a, b, status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(a: f32, b: f32) -> f32 {
        let mut st = SoftFloatStatus::default();
        f32::from_bits(f32_add(a.to_bits(), b.to_bits(), &mut st))
    }
    fn sub(a: f32, b: f32) -> f32 {
        let mut st = SoftFloatStatus::default();
        f32::from_bits(f32_sub(a.to_bits(), b.to_bits(), &mut st))
    }

    // Under default MXCSR (round-nearest-even, no DAZ/FTZ), softfloat results
    // must be bit-identical to the host's IEEE f32 arithmetic for all finite
    // outcomes (and NaN for NaN).
    #[test]
    fn f32_add_sub_match_native_rne() {
        let vals: [f32; 26] = [
            0.0, -0.0, 1.0, -1.0, 2.0, -2.0, 0.5, 3.14159, -2.71828,
            1e30, -1e30, 1e-30, f32::MIN_POSITIVE, f32::MIN_POSITIVE / 2.0,
            123456.789, 0.1, 0.2, 16_777_216.0, 16_777_217.0, 8_388_608.0,
            8_388_609.0, f32::MAX, f32::MIN, 1.0000001, 0.9999999, 65504.0,
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
    fn f32_add_sub_specials() {
        let inf = f32::INFINITY;
        assert_eq!(add(inf, 1.0).to_bits(), inf.to_bits());
        assert_eq!(add(f32::MAX, f32::MAX).to_bits(), inf.to_bits()); // overflow → inf
        assert!(add(inf, -inf).is_nan()); // inf - inf → NaN
        assert!(sub(inf, inf).is_nan());
        assert_eq!(sub(1.0, 1.0).to_bits(), 0.0f32.to_bits()); // exact zero, +0
    }
}
