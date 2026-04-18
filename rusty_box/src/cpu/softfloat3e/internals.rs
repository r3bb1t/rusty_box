#![allow(non_camel_case_types, dead_code, non_snake_case, unused_assignments)]
//! Internal routines: field extraction, normalization, round-and-pack.
//! Ported from Berkeley SoftFloat 3e internals.h / s_roundPackTo*.c / s_normRoundPackTo*.c.

use super::primitives::*;
use super::softfloat::*;
use super::softfloat_types::*;
use super::specialize::*;

// ============================================================
// Field extraction macros (from internals.h)
// ============================================================

// --- float16 ---
#[inline]
pub fn sign_f16(a: u16) -> bool {
    (a >> 15) != 0
}
#[inline]
pub fn exp_f16(a: u16) -> i16 {
    ((a >> 10) & 0x1F) as i16
}
#[inline]
pub fn frac_f16(a: u16) -> u16 {
    a & 0x03FF
}
#[inline]
pub fn pack_to_f16(sign: bool, exp: i16, sig: u16) -> u16 {
    ((sign as u16) << 15).wrapping_add((exp as u16) << 10).wrapping_add(sig)
}
#[inline]
pub fn is_nan_f16(a: u16) -> bool {
    ((!a & 0x7C00) == 0) && ((a & 0x03FF) != 0)
}

// --- float32 ---
#[inline]
pub fn sign_f32(a: u32) -> bool {
    (a >> 31) != 0
}
#[inline]
pub fn exp_f32(a: u32) -> i16 {
    ((a >> 23) & 0xFF) as i16
}
#[inline]
pub fn frac_f32(a: u32) -> u32 {
    a & 0x007FFFFF
}
#[inline]
pub fn pack_to_f32(sign: bool, exp: i16, sig: u32) -> u32 {
    ((sign as u32) << 31).wrapping_add((exp as u32) << 23).wrapping_add(sig)
}
#[inline]
pub fn is_nan_f32(a: u32) -> bool {
    ((!a & 0x7F800000) == 0) && ((a & 0x007FFFFF) != 0)
}

// --- float64 ---
#[inline]
pub fn sign_f64(a: u64) -> bool {
    (a >> 63) != 0
}
#[inline]
pub fn exp_f64(a: u64) -> i16 {
    ((a >> 52) & 0x7FF) as i16
}
#[inline]
pub fn frac_f64(a: u64) -> u64 {
    a & 0x000FFFFFFFFFFFFF
}
#[inline]
pub fn pack_to_f64(sign: bool, exp: i16, sig: u64) -> u64 {
    ((sign as u64) << 63).wrapping_add((exp as u64) << 52).wrapping_add(sig)
}
#[inline]
pub fn is_nan_f64(a: u64) -> bool {
    ((!a & 0x7FF0000000000000) == 0) && ((a & 0x000FFFFFFFFFFFFF) != 0)
}

// --- extFloat80 ---
#[inline]
pub fn sign_extf80(a64: u16) -> bool {
    (a64 >> 15) != 0
}
#[inline]
pub fn exp_extf80(a64: u16) -> u16 {
    a64 & 0x7FFF
}
#[inline]
pub fn pack_to_extf80_sign_exp(sign: bool, exp: u16) -> u16 {
    ((sign as u16) << 15) | exp
}

// ============================================================
// Normalization of subnormal significands
// ============================================================

pub struct ExpSig16 {
    pub(crate) exp: i16,
    pub(crate) sig: u16,
}

pub struct ExpSig32 {
    pub(crate) exp: i16,
    pub(crate) sig: u32,
}
pub struct ExpSig64 {
    pub(crate) exp: i16,
    pub(crate) sig: u64,
}
pub struct ExpSig64_32 {
    pub(crate) exp: i32,
    pub(crate) sig: u64,
}

pub fn norm_subnormal_f16_sig(sig: u16) -> ExpSig16 {
    let shift = count_leading_zeros16(sig) as i16 - 5;
    ExpSig16 {
        exp: 1 - shift,
        sig: sig << shift,
    }
}

pub fn norm_subnormal_f32_sig(sig: u32) -> ExpSig32 {
    let shift = count_leading_zeros32(sig) as i16 - 8;
    ExpSig32 {
        exp: 1 - shift,
        sig: sig << shift,
    }
}

pub fn norm_subnormal_f64_sig(sig: u64) -> ExpSig64 {
    let shift = count_leading_zeros64(sig) as i16 - 11;
    ExpSig64 {
        exp: 1 - shift,
        sig: sig << shift,
    }
}

pub fn norm_subnormal_extf80_sig(sig: u64) -> ExpSig64_32 {
    let shift = count_leading_zeros64(sig) as i32;
    ExpSig64_32 {
        exp: -shift,
        sig: sig << shift,
    }
}

// ============================================================
// Round-and-pack to float16
// ============================================================

pub fn round_pack_to_f16(sign: bool, exp: i16, sig: u16, status: &mut SoftFloatStatus) -> float16 {
    let rounding_mode = softfloat_getRoundingMode(status);
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;
    let mut round_increment: u16 = 0x8;
    if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
        round_increment = if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
            0xF
        } else {
            0
        };
    }
    let round_bits = sig & 0xF;
    let mut exp = exp;
    let mut sig = sig;

    if 0x1D <= (exp as u16) {
        if exp < 0 {
            let is_tiny = (exp < -1) || ((sig as u32).wrapping_add(round_increment as u32) < 0x8000);
            sig = shift_right_jam32(sig as u32, (-(exp as i32)) as u16) as u16;
            exp = 0;
            let round_bits = sig & 0xF;
            if is_tiny {
                if !softfloat_isMaskedException(status, FLAG_UNDERFLOW) || round_bits != 0 {
                    softfloat_raiseFlags(status, FLAG_UNDERFLOW);
                }
                if softfloat_flushUnderflowToZero(status) {
                    softfloat_raiseFlags(status, FLAG_UNDERFLOW | FLAG_INEXACT);
                    return pack_to_f16(sign, 0, 0);
                }
            }
        } else if (0x1D < exp) || (0x8000 <= (sig as u32).wrapping_add(round_increment as u32)) {
            softfloat_raiseFlags(status, FLAG_OVERFLOW);
            if round_bits != 0 || softfloat_isMaskedException(status, FLAG_OVERFLOW) {
                softfloat_raiseFlags(status, FLAG_INEXACT);
                if round_increment != 0 {
                    softfloat_setRoundingUp(status);
                }
            }
            return pack_to_f16(sign, 0x1F, 0).wrapping_sub(if round_increment == 0 { 1 } else { 0 });
        }
    }
    let sig_ref = sig;
    sig = (sig.wrapping_add(round_increment)) >> 4;
    sig &= !((!((round_bits ^ 8) != 0) as u16) & (round_near_even as u16));
    if sig == 0 { exp = 0; }
    if round_bits != 0 {
        softfloat_raiseFlags(status, FLAG_INEXACT);
        if (sig << 4) > sig_ref {
            softfloat_setRoundingUp(status);
        }
    }
    pack_to_f16(sign, exp, sig)
}

// ============================================================
// Round-and-pack to float32
// ============================================================

pub fn round_pack_to_f32(sign: bool, exp: i16, sig: u32, status: &mut SoftFloatStatus) -> float32 {
    let rounding_mode = softfloat_getRoundingMode(status);
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;
    let mut round_increment: u32 = 0x40;
    if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
        round_increment = if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
            0x7F
        } else {
            0
        };
    }
    let round_bits = sig & 0x7F;
    let mut exp = exp;
    let mut sig = sig;

    if 0xFD <= (exp as u16) {
        if exp < 0 {
            let is_tiny = (exp < -1) || (sig.wrapping_add(round_increment) < 0x80000000);
            sig = shift_right_jam32(sig, (-(exp as i32)) as u16);
            exp = 0;
            let round_bits = sig & 0x7F;
            if round_bits != 0 {
                softfloat_raiseFlags(status, FLAG_INEXACT);
                if is_tiny {
                    softfloat_raiseFlags(status, FLAG_UNDERFLOW);
                }
            }
            sig = sig.wrapping_add(round_increment);
            exp = ((sig & 0x80000000) != 0) as i16;
            let ri = round_increment;
            if round_near_even && ((round_bits ^ 0x40) == 0) {
                sig &= !(ri);
            } else {
                sig >>= 7;
            }
            return pack_to_f32(sign, exp, sig);
        }
        if (0xFD < exp) || (0x80000000 <= sig.wrapping_add(round_increment)) {
            softfloat_raiseFlags(status, FLAG_OVERFLOW | FLAG_INEXACT);
            if round_near_even
                || rounding_mode == ROUND_NEAR_MAXMAG
                || rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX })
            {
                return pack_to_f32(sign, 0xFF, 0);
            } else {
                return pack_to_f32(sign, 0xFE, 0x007FFFFF);
            }
        }
    }
    sig = sig.wrapping_add(round_increment);
    if sig < round_increment {
        exp += 1;
    }
    if round_near_even && ((round_bits ^ 0x40) == 0) {
        sig &= !0x7Fu32;
    }
    if round_bits != 0 {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    sig >>= 7;
    if sig == 0 { exp = 0; }
    pack_to_f32(sign, exp, sig)
}

pub fn norm_round_pack_to_f32(
    sign: bool,
    exp: i16,
    sig: u32,
    status: &mut SoftFloatStatus,
) -> float32 {
    let shift = count_leading_zeros32(sig) as i16 - 1;
    round_pack_to_f32(sign, exp - shift, sig << shift, status)
}

// ============================================================
// Round-and-pack to float64
// ============================================================

pub fn round_pack_to_f64(sign: bool, exp: i16, sig: u64, status: &mut SoftFloatStatus) -> float64 {
    let rounding_mode = softfloat_getRoundingMode(status);
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;
    let mut round_increment: u64 = 0x200;
    if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
        round_increment = if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
            0x3FF
        } else {
            0
        };
    }
    let round_bits = (sig & 0x3FF) as u32;
    let mut exp = exp;
    let mut sig = sig;

    if 0x7FD <= (exp as u16) {
        if exp < 0 {
            let is_tiny = (exp < -1) || (sig.wrapping_add(round_increment) < 0x8000000000000000);
            sig = shift_right_jam64(sig, (-(exp as i32)) as u32);
            exp = 0;
            let round_bits = (sig & 0x3FF) as u32;
            if round_bits != 0 {
                softfloat_raiseFlags(status, FLAG_INEXACT);
                if is_tiny {
                    softfloat_raiseFlags(status, FLAG_UNDERFLOW);
                }
            }
            sig = sig.wrapping_add(round_increment);
            exp = ((sig & 0x8000000000000000) != 0) as i16;
            if round_near_even && ((round_bits ^ 0x200) == 0) {
                sig &= !0x3FFu64;
            } else {
                sig >>= 10;
            }
            return pack_to_f64(sign, exp, sig);
        }
        if (0x7FD < exp) || (0x8000000000000000 <= sig.wrapping_add(round_increment)) {
            softfloat_raiseFlags(status, FLAG_OVERFLOW | FLAG_INEXACT);
            if round_near_even
                || rounding_mode == ROUND_NEAR_MAXMAG
                || rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX })
            {
                return pack_to_f64(sign, 0x7FF, 0);
            } else {
                return pack_to_f64(sign, 0x7FE, 0x000FFFFFFFFFFFFF);
            }
        }
    }
    sig = sig.wrapping_add(round_increment);
    if sig < round_increment {
        exp += 1;
    }
    if round_near_even && ((round_bits ^ 0x200) == 0) {
        sig &= !0x3FFu64;
    }
    if round_bits != 0 {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    sig >>= 10;
    if sig == 0 { exp = 0; }
    pack_to_f64(sign, exp, sig)
}

pub fn norm_round_pack_to_f64(
    sign: bool,
    exp: i16,
    sig: u64,
    status: &mut SoftFloatStatus,
) -> float64 {
    let shift = count_leading_zeros64(sig) as i16 - 1;
    round_pack_to_f64(sign, exp - shift, sig << shift, status)
}

// ============================================================
// softfloat_shiftRightJam64Extra — used by precision-80 path
// ============================================================
#[inline]
pub fn shift_right_jam64_extra(a: u64, extra: u64, dist: u32) -> (u64, u64) {
    if dist < 64 {
        if dist == 0 {
            return (a, extra);
        }
        let v = a >> dist;
        let new_extra = (a << ((!dist).wrapping_add(1) & 63)) | ((extra != 0) as u64);
        (v, new_extra)
    } else {
        let v = 0u64;
        let new_extra = if dist == 64 {
            a | ((extra != 0) as u64)
        } else {
            ((a | extra) != 0) as u64
        };
        (v, new_extra)
    }
}

// ============================================================
// Round-and-pack to extFloat80 (the heart of SoftFloat)
// ============================================================

pub fn round_pack_to_extf80(
    sign: bool,
    mut exp: i32,
    mut sig: u64,
    sig_extra: u64,
    rounding_precision: u8,
    status: &mut SoftFloatStatus,
) -> floatx80 {
    let rounding_mode = softfloat_getRoundingMode(status);
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;

    if rounding_precision != 80 {
        // Reduced precision (32 or 64 bit)
        let (round_increment, round_mask): (u64, u64) = if rounding_precision == 64 {
            (0x0000000000000400, 0x00000000000007FF)
        } else if rounding_precision == 32 {
            (0x0000008000000000, 0x000000FFFFFFFFFF)
        } else {
            // Default to 80-bit precision for reserved value
            return round_pack_to_extf80_precision80(sign, exp, sig, sig_extra, status);
        };

        sig |= (sig_extra != 0) as u64;
        let mut round_increment = round_increment;
        if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
            round_increment = if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
                round_mask
            } else {
                0
            };
        }
        let round_bits = sig & round_mask;

        if 0x7FFD <= (exp.wrapping_sub(1) as u32) {
            if exp <= 0 {
                let is_tiny = (exp < 0) || (sig <= sig.wrapping_add(round_increment));
                if is_tiny && sig != 0 && !softfloat_isMaskedException(status, FLAG_UNDERFLOW) {
                    softfloat_raiseFlags(status, FLAG_UNDERFLOW);
                    exp += 0x6000;
                } else {
                    sig = shift_right_jam64(sig, (1 - exp) as u32);
                    let round_bits = sig & round_mask;
                    let sig_exact = sig;
                    sig = sig.wrapping_add(round_increment);
                    exp = ((sig & 0x8000000000000000) != 0) as i32;
                    let ri = round_mask + 1;
                    let mut round_mask = round_mask;
                    if round_near_even && (round_bits << 1 == ri) {
                        round_mask |= ri;
                    }
                    sig &= !round_mask;
                    if round_bits != 0 {
                        softfloat_raiseFlags(status, FLAG_INEXACT);
                        if sig > sig_exact {
                            softfloat_setRoundingUp(status);
                        }
                        if is_tiny {
                            softfloat_raiseFlags(status, FLAG_UNDERFLOW);
                        }
                    }
                    return pack_floatx80(sign, exp, sig);
                }
            }
            if (0x7FFE < exp) || ((exp == 0x7FFE) && (sig.wrapping_add(round_increment) < sig)) {
                if !softfloat_isMaskedException(status, FLAG_OVERFLOW) {
                    softfloat_raiseFlags(status, FLAG_OVERFLOW);
                    exp -= 0x6000;
                }
                if (0x7FFE < exp) || ((exp == 0x7FFE) && (sig.wrapping_add(round_increment) < sig))
                {
                    // overflow
                    softfloat_raiseFlags(status, FLAG_OVERFLOW | FLAG_INEXACT);
                    if round_near_even
                        || rounding_mode == ROUND_NEAR_MAXMAG
                        || rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX })
                    {
                        softfloat_setRoundingUp(status);
                        return pack_floatx80(sign, 0x7FFF, 0x8000000000000000);
                    } else {
                        return pack_floatx80(sign, 0x7FFE, !round_mask);
                    }
                }
            }
        }

        let sig_exact = sig;
        sig = sig.wrapping_add(round_increment);
        if sig < round_increment {
            exp += 1;
            sig = 0x8000000000000000;
        }
        let ri = round_mask + 1;
        let mut round_mask = round_mask;
        if round_near_even && (round_bits << 1 == ri) {
            round_mask |= ri;
        }
        sig &= !round_mask;
        if round_bits != 0 {
            softfloat_raiseFlags(status, FLAG_INEXACT);
            if sig > sig_exact {
                softfloat_setRoundingUp(status);
            }
        }
        return pack_floatx80(sign, exp, sig);
    }

    // 80-bit precision
    round_pack_to_extf80_precision80(sign, exp, sig, sig_extra, status)
}

fn round_pack_to_extf80_precision80(
    sign: bool,
    mut exp: i32,
    mut sig: u64,
    sig_extra: u64,
    status: &mut SoftFloatStatus,
) -> floatx80 {
    let rounding_mode = softfloat_getRoundingMode(status);
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;

    let mut do_increment = 0x8000000000000000 <= sig_extra;
    if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
        do_increment =
            (rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX })) && sig_extra != 0;
    }

    if 0x7FFD <= (exp.wrapping_sub(1) as u32) {
        if exp <= 0 {
            let is_tiny = (exp < 0) || !do_increment || (sig < 0xFFFFFFFFFFFFFFFF);
            if is_tiny && sig != 0 && !softfloat_isMaskedException(status, FLAG_UNDERFLOW) {
                softfloat_raiseFlags(status, FLAG_UNDERFLOW);
                exp += 0x6000;
            } else {
                let (new_sig, new_extra) =
                    shift_right_jam64_extra(sig, sig_extra, (1 - exp) as u32);
                exp = 0;
                sig = new_sig;
                let sig_extra = new_extra;
                if sig_extra != 0 {
                    softfloat_raiseFlags(status, FLAG_INEXACT);
                    if is_tiny {
                        softfloat_raiseFlags(status, FLAG_UNDERFLOW);
                    }
                }
                do_increment = 0x8000000000000000 <= sig_extra;
                if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
                    do_increment = (rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }))
                        && sig_extra != 0;
                }
                if do_increment {
                    let sig_exact = sig;
                    sig = sig.wrapping_add(1);
                    sig &= !((((sig_extra & 0x7FFFFFFFFFFFFFFF) == 0) && round_near_even) as u64);
                    exp = ((sig & 0x8000000000000000) != 0) as i32;
                    if sig > sig_exact {
                        softfloat_setRoundingUp(status);
                    }
                }
                return pack_floatx80(sign, exp, sig);
            }
        }
        if (0x7FFE < exp) || ((exp == 0x7FFE) && (sig == 0xFFFFFFFFFFFFFFFF) && do_increment) {
            if !softfloat_isMaskedException(status, FLAG_OVERFLOW) {
                softfloat_raiseFlags(status, FLAG_OVERFLOW);
                exp -= 0x6000;
            }
            if (0x7FFE < exp) || ((exp == 0x7FFE) && (sig == 0xFFFFFFFFFFFFFFFF) && do_increment) {
                softfloat_raiseFlags(status, FLAG_OVERFLOW | FLAG_INEXACT);
                if round_near_even
                    || rounding_mode == ROUND_NEAR_MAXMAG
                    || rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX })
                {
                    softfloat_setRoundingUp(status);
                    return pack_floatx80(sign, 0x7FFF, 0x8000000000000000);
                } else {
                    return pack_floatx80(sign, 0x7FFE, 0xFFFFFFFFFFFFFFFF);
                }
            }
        }
    }

    if sig_extra != 0 {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    if do_increment {
        let sig_exact = sig;
        sig = sig.wrapping_add(1);
        if sig == 0 {
            exp += 1;
            sig = 0x8000000000000000;
        } else {
            sig &= !((((sig_extra & 0x7FFFFFFFFFFFFFFF) == 0) && round_near_even) as u64);
        }
        if sig > sig_exact {
            softfloat_setRoundingUp(status);
        }
    } else {
        if sig == 0 {
            exp = 0;
        }
    }
    pack_floatx80(sign, exp, sig)
}

pub fn norm_round_pack_to_extf80(
    sign: bool,
    mut exp: i32,
    mut sig: u64,
    mut sig_extra: u64,
    rounding_precision: u8,
    status: &mut SoftFloatStatus,
) -> floatx80 {
    if sig == 0 {
        exp -= 64;
        sig = sig_extra;
        sig_extra = 0;
    }
    let shift = count_leading_zeros64(sig);
    exp -= shift as i32;
    if shift != 0 {
        let (new_sig, new_extra) = short_shift_left128(sig, sig_extra, shift);
        sig = new_sig;
        sig_extra = new_extra;
    }
    round_pack_to_extf80(sign, exp, sig, sig_extra, rounding_precision, status)
}

// ============================================================
// Round-to-integer helpers (for extF80→i32/i64 conversions)
// ============================================================

pub fn softfloat_round_to_i32(
    sign: bool,
    sig: u64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i32 {
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;
    let mut round_increment: u32 = 0x800;
    if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
        round_increment = if rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX }) {
            0xFFF
        } else {
            0
        };
    }
    let round_bits = (sig & 0xFFF) as u32;
    let sig = (sig + round_increment as u64) >> 12;
    let sig = if round_near_even && (round_bits == 0x800) {
        sig & !1
    } else {
        sig
    };
    let mut z = sig as i32;
    if sign {
        z = -z;
    }
    if z != 0 && ((z < 0) ^ sign) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return if sign {
            I32_FROM_NEG_OVERFLOW
        } else {
            I32_FROM_POS_OVERFLOW
        };
    }
    if round_bits != 0 && exact {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    z
}

pub fn softfloat_round_to_i64(
    sign: bool,
    sig: u64,
    sig_extra: u64,
    rounding_mode: u8,
    exact: bool,
    status: &mut SoftFloatStatus,
) -> i64 {
    let round_near_even = rounding_mode == ROUND_NEAR_EVEN;
    let mut do_increment = 0x8000000000000000 <= sig_extra;
    if !round_near_even && rounding_mode != ROUND_NEAR_MAXMAG {
        do_increment =
            (rounding_mode == (if sign { ROUND_MIN } else { ROUND_MAX })) && sig_extra != 0;
    }
    let mut sig = sig;
    if do_increment {
        sig = sig.wrapping_add(1);
        if sig == 0 {
            softfloat_raiseFlags(status, FLAG_INVALID);
            return if sign {
                I64_FROM_NEG_OVERFLOW
            } else {
                I64_FROM_POS_OVERFLOW
            };
        }
        if round_near_even && (sig_extra == 0x8000000000000000) {
            sig &= !1;
        }
    }
    let mut z = sig as i64;
    if sign {
        z = -z;
    }
    if z != 0 && ((z < 0) ^ sign) {
        softfloat_raiseFlags(status, FLAG_INVALID);
        return if sign {
            I64_FROM_NEG_OVERFLOW
        } else {
            I64_FROM_POS_OVERFLOW
        };
    }
    if sig_extra != 0 && exact {
        softfloat_raiseFlags(status, FLAG_INEXACT);
    }
    z
}
