#![allow(non_camel_case_types, dead_code, non_snake_case)]
//! ExtFloat80 addition and subtraction.
//! Ported from Berkeley SoftFloat 3e: extF80_addsub.c, s_addMagsExtF80.c, s_subMagsExtF80.c

use super::internals::*;
use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

// ============================================================
// addMagsExtF80: internal add-magnitudes
// ============================================================

fn add_mags_extf80(
    ui_a64: u16,
    ui_a0: u64,
    ui_b64: u16,
    ui_b0: u64,
    sign_z: bool,
    status: &mut SoftFloatStatus,
) -> floatx80 {
    let mut exp_a = exp_extf80(ui_a64) as i32;
    let mut sig_a = ui_a0;
    let mut exp_b = exp_extf80(ui_b64) as i32;
    let mut sig_b = ui_b0;

    // NaN / Inf handling
    if exp_a == 0x7FFF {
        if (sig_a << 1) != 0 || ((exp_b == 0x7FFF) && (sig_b << 1) != 0) {
            return softfloat_propagate_nan_extf80(ui_a64, ui_a0, ui_b64, ui_b0, status);
        }
        if sig_b != 0 && exp_b == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_to_extf80(ui_a64, ui_a0);
    }
    if exp_b == 0x7FFF {
        if (sig_b << 1) != 0 {
            return softfloat_propagate_nan_extf80(ui_a64, ui_a0, ui_b64, ui_b0, status);
        }
        if sig_a != 0 && exp_a == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_floatx80(sign_z, 0x7FFF, 0x8000000000000000);
    }

    // Handle zeros and denormals for A
    if exp_a == 0 {
        if sig_a == 0 {
            if exp_b == 0 && sig_b != 0 {
                softfloat_raiseFlags(status, FLAG_DENORMAL);
                let norm = norm_subnormal_extf80_sig(sig_b);
                exp_b = norm.exp + 1;
                sig_b = norm.sig;
            }
            return round_pack_to_extf80(
                sign_z,
                exp_b,
                sig_b,
                0,
                softfloat_extF80_roundingPrecision(status),
                status,
            );
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_a);
        exp_a = norm.exp + 1;
        sig_a = norm.sig;
    }

    // Handle zeros and denormals for B
    if exp_b == 0 {
        if sig_b == 0 {
            return round_pack_to_extf80(
                sign_z,
                exp_a,
                sig_a,
                0,
                softfloat_extF80_roundingPrecision(status),
                status,
            );
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_b);
        exp_b = norm.exp + 1;
        sig_b = norm.sig;
    }

    // Main addition
    let exp_diff = exp_a - exp_b;
    let mut exp_z;
    let mut sig_z;
    let mut sig_z_extra;

    if exp_diff == 0 {
        sig_z = sig_a.wrapping_add(sig_b);
        sig_z_extra = 0u64;
        exp_z = exp_a;
        // Need to shift right 1 and set MSB
        let (v, e) = short_shift_right_jam64_extra(sig_z, sig_z_extra, 1);
        sig_z = v | 0x8000000000000000;
        sig_z_extra = e;
        exp_z += 1;
    } else if exp_diff < 0 {
        exp_z = exp_b;
        let (v, e) = shift_right_jam64_extra(sig_a, 0, (-exp_diff) as u32);
        sig_a = v;
        sig_z_extra = e;
        sig_z = sig_a.wrapping_add(sig_b);
        if (sig_z & 0x8000000000000000) != 0 {
            // Already normalized, go to roundAndPack
        } else {
            let (v2, e2) = short_shift_right_jam64_extra(sig_z, sig_z_extra, 1);
            sig_z = v2 | 0x8000000000000000;
            sig_z_extra = e2;
            exp_z += 1;
        }
    } else {
        exp_z = exp_a;
        let (v, e) = shift_right_jam64_extra(sig_b, 0, exp_diff as u32);
        sig_b = v;
        sig_z_extra = e;
        sig_z = sig_a.wrapping_add(sig_b);
        if (sig_z & 0x8000000000000000) != 0 {
            // Already normalized
        } else {
            let (v2, e2) = short_shift_right_jam64_extra(sig_z, sig_z_extra, 1);
            sig_z = v2 | 0x8000000000000000;
            sig_z_extra = e2;
            exp_z += 1;
        }
    }

    round_pack_to_extf80(
        sign_z,
        exp_z,
        sig_z,
        sig_z_extra,
        softfloat_extF80_roundingPrecision(status),
        status,
    )
}

// ============================================================
// subMagsExtF80: internal subtract-magnitudes
// ============================================================

fn sub_mags_extf80(
    ui_a64: u16,
    ui_a0: u64,
    ui_b64: u16,
    ui_b0: u64,
    mut sign_z: bool,
    status: &mut SoftFloatStatus,
) -> floatx80 {
    let mut exp_a = exp_extf80(ui_a64) as i32;
    let mut sig_a = ui_a0;
    let mut exp_b = exp_extf80(ui_b64) as i32;
    let mut sig_b = ui_b0;

    // NaN / Inf handling
    if exp_a == 0x7FFF {
        if (sig_a << 1) != 0 {
            return softfloat_propagate_nan_extf80(ui_a64, ui_a0, ui_b64, ui_b0, status);
        }
        if exp_b == 0x7FFF {
            if (sig_b << 1) != 0 {
                return softfloat_propagate_nan_extf80(ui_a64, ui_a0, ui_b64, ui_b0, status);
            }
            // Inf - Inf = invalid
            softfloat_raiseFlags(status, FLAG_INVALID);
            return FLOATX80_DEFAULT_NAN;
        }
        if sig_b != 0 && exp_b == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_to_extf80(ui_a64, ui_a0);
    }
    if exp_b == 0x7FFF {
        if (sig_b << 1) != 0 {
            return softfloat_propagate_nan_extf80(ui_a64, ui_a0, ui_b64, ui_b0, status);
        }
        if sig_a != 0 && exp_a == 0 {
            softfloat_raiseFlags(status, FLAG_DENORMAL);
        }
        return pack_floatx80(!sign_z, 0x7FFF, 0x8000000000000000);
    }

    // Handle A denormals/zeros
    if exp_a == 0 {
        if sig_a == 0 {
            if exp_b == 0 {
                if sig_b != 0 {
                    softfloat_raiseFlags(status, FLAG_DENORMAL);
                    let norm = norm_subnormal_extf80_sig(sig_b);
                    let exp_b2 = norm.exp + 1;
                    let sig_b2 = norm.sig;
                    return round_pack_to_extf80(
                        !sign_z,
                        exp_b2,
                        sig_b2,
                        0,
                        softfloat_extF80_roundingPrecision(status),
                        status,
                    );
                }
                // 0 - 0
                return pack_floatx80(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0);
            }
            // 0 - B
            return round_pack_to_extf80(
                !sign_z,
                exp_b,
                sig_b,
                0,
                softfloat_extF80_roundingPrecision(status),
                status,
            );
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_a);
        exp_a = norm.exp + 1;
        sig_a = norm.sig;
    }

    // Handle B denormals/zeros
    if exp_b == 0 {
        if sig_b == 0 {
            return round_pack_to_extf80(
                sign_z,
                exp_a,
                sig_a,
                0,
                softfloat_extF80_roundingPrecision(status),
                status,
            );
        }
        softfloat_raiseFlags(status, FLAG_DENORMAL);
        let norm = norm_subnormal_extf80_sig(sig_b);
        exp_b = norm.exp + 1;
        sig_b = norm.sig;
    }

    // Main subtraction
    let exp_diff = exp_a - exp_b;

    if exp_diff > 0 {
        // expA bigger
        let (s64, s0) = shift_right_jam128(sig_b, 0, exp_diff as u32);
        sig_b = s64;
        let sig_extra = s0;
        let exp_z = exp_a;
        let (r64, r0) = sub128(sig_a, 0, sig_b, sig_extra);
        return norm_round_pack_to_extf80(
            sign_z,
            exp_z,
            r64,
            r0,
            softfloat_extF80_roundingPrecision(status),
            status,
        );
    }
    if exp_diff < 0 {
        // expB bigger
        let (s64, s0) = shift_right_jam128(sig_a, 0, (-exp_diff) as u32);
        sig_a = s64;
        let sig_extra = s0;
        let exp_z = exp_b;
        sign_z = !sign_z;
        let (r64, r0) = sub128(sig_b, 0, sig_a, sig_extra);
        return norm_round_pack_to_extf80(
            sign_z,
            exp_z,
            r64,
            r0,
            softfloat_extF80_roundingPrecision(status),
            status,
        );
    }

    // Equal exponents
    let exp_z = exp_a;
    if sig_b < sig_a {
        let (r64, r0) = sub128(sig_a, 0, sig_b, 0);
        return norm_round_pack_to_extf80(
            sign_z,
            exp_z,
            r64,
            r0,
            softfloat_extF80_roundingPrecision(status),
            status,
        );
    }
    if sig_a < sig_b {
        sign_z = !sign_z;
        let (r64, r0) = sub128(sig_b, 0, sig_a, 0);
        return norm_round_pack_to_extf80(
            sign_z,
            exp_z,
            r64,
            r0,
            softfloat_extF80_roundingPrecision(status),
            status,
        );
    }
    // Equal: result is ±0
    pack_floatx80(softfloat_getRoundingMode(status) == ROUND_MIN, 0, 0)
}

// ============================================================
// Public API: extF80_add, extF80_sub
// ============================================================

pub fn extf80_add(a: floatx80, b: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
    if extf80_is_unsupported(a) || extf80_is_unsupported(b) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    let sign_a = sign_extf80(a.sign_exp);
    let sign_b = sign_extf80(b.sign_exp);

    if sign_a == sign_b {
        add_mags_extf80(a.sign_exp, a.signif, b.sign_exp, b.signif, sign_a, status)
    } else {
        sub_mags_extf80(a.sign_exp, a.signif, b.sign_exp, b.signif, sign_a, status)
    }
}

pub fn extf80_sub(a: floatx80, b: floatx80, status: &mut SoftFloatStatus) -> floatx80 {
    if extf80_is_unsupported(a) || extf80_is_unsupported(b) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return FLOATX80_DEFAULT_NAN;
    }

    let sign_a = sign_extf80(a.sign_exp);
    let sign_b = sign_extf80(b.sign_exp);

    if sign_a == sign_b {
        sub_mags_extf80(a.sign_exp, a.signif, b.sign_exp, b.signif, sign_a, status)
    } else {
        add_mags_extf80(a.sign_exp, a.signif, b.sign_exp, b.signif, sign_a, status)
    }
}
