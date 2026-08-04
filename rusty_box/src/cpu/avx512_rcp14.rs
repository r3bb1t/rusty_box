//! 14-bit reciprocal and reciprocal-square-root approximations.
//!
//! Ported from Bochs `cpu/avx/avx512_rcp14.cc` and `cpu/avx/avx512_rsqrt14.cc`.
//! These are the element functions behind VRCP14 and VRSQRT14: deliberately
//! approximate, table-driven, and rounded to 14 fraction bits by
//! round-to-nearest regardless of MXCSR.RC. They raise no exceptions at all —
//! not even on a signalling NaN, which is merely quietened.

use super::avx512_rcp14_tables::{RCP14_TABLE, RSQRT14_TABLE0, RSQRT14_TABLE1};
use super::softfloat3e::f32_class::f32_class;
use super::softfloat3e::f64_class::f64_class;
use super::softfloat3e::internals::{
    exp_f32, exp_f64, frac_f32, frac_f64, norm_subnormal_f32_sig, norm_subnormal_f64_sig,
    pack_to_f32, pack_to_f64, sign_f32, sign_f64,
};
use super::softfloat3e::softfloat::SoftFloatClass;
use super::softfloat3e::softfloat_types::{Float32, Float64};
use super::softfloat3e::specialize::{FLOAT32_DEFAULT_NAN, FLOAT64_DEFAULT_NAN};

/// Bochs avx512_rcp14.cc `rcp14_table_lookup`. `mant` is the top bits of the
/// significand; the returned value is the new 16-bit significand and `exp` is
/// updated in place.
#[inline]
fn rcp14_table_lookup(mant: u32, bias: i16, exp: &mut i16) -> u32 {
    let mut r_exp = 2 * bias - 1 - *exp;
    let result = if mant == 0 {
        // An exact power of two: no table entry, just adjust the exponent.
        r_exp += 1;
        mant
    } else {
        // The table is indexed by the 16 most significant bits of the 23-bit
        // significand.
        RCP14_TABLE[(mant >> 7) as usize] as u32
    };
    *exp = r_exp;
    result
}

/// Quieten a NaN the way Bochs `convert_to_QNaN` does.
#[inline]
fn quieten_f32(op: Float32) -> Float32 {
    op | 0x0040_0000
}

#[inline]
fn quieten_f64(op: Float64) -> Float64 {
    op | 0x0008_0000_0000_0000
}

/// Bochs `approximate_rcp14(float32)`.
pub(super) fn approximate_rcp14_f32(op: Float32, daz: bool, ftz: bool) -> Float32 {
    let sign = sign_f32(op);
    let mut fraction = frac_f32(op);
    let mut exp = exp_f32(op);

    match f32_class(op) {
        SoftFloatClass::Zero => return pack_to_f32(sign, 0xFF, 0),
        SoftFloatClass::NegativeInf | SoftFloatClass::PositiveInf => {
            return pack_to_f32(sign, 0, 0)
        }
        SoftFloatClass::SNaN | SoftFloatClass::QNaN => return quieten_f32(op),
        SoftFloatClass::Denormal => {
            if daz {
                return pack_to_f32(sign, 0xFF, 0);
            }
            let ns = norm_subnormal_f32_sig(fraction);
            exp = ns.exp;
            fraction = ns.sig & 0x7F_FFFF;
        }
        SoftFloatClass::Normalized => {}
    }

    fraction = rcp14_table_lookup(fraction, 0x7F, &mut exp) << 7;

    if exp >= 0xFF {
        return pack_to_f32(sign, 0xFF, 0); // overflow to infinity
    }
    if exp <= 0 {
        if ftz {
            return pack_to_f32(sign, 0, 0);
        }
        // -1 <= exp <= 0 here, so the shift cannot need rounding.
        fraction >>= 1 - exp;
        exp = 0;
    }
    pack_to_f32(sign, exp, fraction)
}

/// Bochs `approximate_rcp14(float64)`.
pub(super) fn approximate_rcp14_f64(op: Float64, daz: bool, ftz: bool) -> Float64 {
    let sign = sign_f64(op);
    let mut fraction = frac_f64(op);
    let mut exp = exp_f64(op);

    match f64_class(op) {
        SoftFloatClass::Zero => return pack_to_f64(sign, 0x7FF, 0),
        SoftFloatClass::NegativeInf | SoftFloatClass::PositiveInf => {
            return pack_to_f64(sign, 0, 0)
        }
        SoftFloatClass::SNaN | SoftFloatClass::QNaN => return quieten_f64(op),
        SoftFloatClass::Denormal => {
            if daz {
                return pack_to_f64(sign, 0x7FF, 0);
            }
            let ns = norm_subnormal_f64_sig(fraction);
            exp = ns.exp;
            fraction = ns.sig & 0xF_FFFF_FFFF_FFFF;
        }
        SoftFloatClass::Normalized => {}
    }

    // Narrow the 52-bit significand to the 23 bits the table expects, keeping
    // a sticky bit for everything shifted off.
    let narrowed = ((fraction >> 29) | u64::from((fraction & 0x1FFF_FFFF) != 0)) as u32;
    let mut fraction = u64::from(rcp14_table_lookup(narrowed, 0x3FF, &mut exp)) << 36;

    if exp >= 0x7FF {
        return pack_to_f64(sign, 0x7FF, 0);
    }
    if exp <= 0 {
        if ftz {
            return pack_to_f64(sign, 0, 0);
        }
        fraction >>= 1 - exp;
        exp = 0;
    }
    pack_to_f64(sign, exp, fraction)
}

/// Bochs `approximate_rsqrt14(float32)`. A negative operand is invalid and
/// yields the default NaN; unlike VRCP14 there is no underflow path, because
/// the result of a reciprocal square root cannot be denormal.
pub(super) fn approximate_rsqrt14_f32(op: Float32, daz: bool) -> Float32 {
    let sign = sign_f32(op);
    let mut fraction = frac_f32(op);
    let mut exp = exp_f32(op);

    match f32_class(op) {
        SoftFloatClass::Zero => return pack_to_f32(sign, 0xFF, 0),
        SoftFloatClass::PositiveInf => return 0,
        SoftFloatClass::NegativeInf => return FLOAT32_DEFAULT_NAN,
        SoftFloatClass::SNaN | SoftFloatClass::QNaN => return quieten_f32(op),
        SoftFloatClass::Denormal => {
            if daz {
                return pack_to_f32(sign, 0xFF, 0);
            }
            let ns = norm_subnormal_f32_sig(fraction);
            exp = ns.exp;
            fraction = ns.sig & 0x7F_FFFF;
        }
        SoftFloatClass::Normalized => {}
    }

    if sign {
        return FLOAT32_DEFAULT_NAN;
    }

    // Two tables, selected by the parity of the exponent: halving an odd
    // exponent leaves a factor of sqrt(2) that the table has to absorb.
    let table = if exp & 1 != 0 {
        &RSQRT14_TABLE1
    } else {
        &RSQRT14_TABLE0
    };
    exp = 0x7E - ((exp - 0x7F) >> 1);
    let mut sig = 0u32;
    if fraction != 0 {
        sig = table[(fraction >> 8) as usize] as u32;
    } else {
        // NOTE: upstream bug, reproduced deliberately. This shortcut is only
        // valid on the even-exponent table, where 1/sqrt(2^2k) really is a
        // power of two. On the odd table the significand should come from
        // entry 0 (~0.4142) instead, so Bochs answers 1.0 for rsqrt14(2.0)
        // where hardware answers ~0.7071 — and likewise for 8.0, 32.0, 0.5
        // and every other exact power of two with an odd unbiased exponent.
        // See docs/bochs-upstream-bugs.md.
        exp += 1;
    }
    pack_to_f32(false, exp, sig << 7)
}

/// Bochs `approximate_rsqrt14(float64)`.
pub(super) fn approximate_rsqrt14_f64(op: Float64, daz: bool) -> Float64 {
    let sign = sign_f64(op);
    let mut fraction = frac_f64(op);
    let mut exp = exp_f64(op);

    match f64_class(op) {
        SoftFloatClass::Zero => return pack_to_f64(sign, 0x7FF, 0),
        SoftFloatClass::PositiveInf => return 0,
        SoftFloatClass::NegativeInf => return FLOAT64_DEFAULT_NAN,
        SoftFloatClass::SNaN | SoftFloatClass::QNaN => return quieten_f64(op),
        SoftFloatClass::Denormal => {
            if daz {
                return pack_to_f64(sign, 0x7FF, 0);
            }
            let ns = norm_subnormal_f64_sig(fraction);
            exp = ns.exp;
            fraction = ns.sig & 0xF_FFFF_FFFF_FFFF;
        }
        SoftFloatClass::Normalized => {}
    }

    if sign {
        return FLOAT64_DEFAULT_NAN;
    }

    let narrowed = ((fraction >> 29) | u64::from((fraction & 0x1FFF_FFFF) != 0)) as u32;
    let table = if exp & 1 != 0 {
        &RSQRT14_TABLE1
    } else {
        &RSQRT14_TABLE0
    };
    exp = 0x3FE - ((exp - 0x3FF) >> 1);
    let mut sig = 0u64;
    if narrowed != 0 {
        sig = table[(narrowed >> 8) as usize] as u64;
    } else {
        // Same upstream bug as in the single-precision path above.
        exp += 1;
    }
    pack_to_f64(false, exp, sig << 36)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The approximation carries 14 fraction bits, so a relative error of
    /// 2^-14 is the architectural bound. These check the shape of the result
    /// rather than exact bits, plus every special case exactly.
    fn rel_err_f32(got: Float32, want: f32) -> f32 {
        (f32::from_bits(got) - want).abs() / want.abs()
    }

    #[test]
    fn rcp14_is_within_the_architectural_error_bound() {
        for v in [1.0f32, 2.0, 0.5, 3.0, 7.5, 1e10, 1e-10, -4.0] {
            let got = approximate_rcp14_f32(v.to_bits(), false, false);
            assert!(
                rel_err_f32(got, 1.0 / v) <= 2.0f32.powi(-14),
                "rcp14({v}) = {} is outside 2^-14",
                f32::from_bits(got)
            );
        }
        for v in [1.0f64, 2.0, 0.5, 3.0, 7.5, 1e100, 1e-100, -4.0] {
            let got = f64::from_bits(approximate_rcp14_f64(v.to_bits(), false, false));
            assert!(
                ((got - 1.0 / v) / (1.0f64 / v)).abs() <= 2.0f64.powi(-14),
                "rcp14({v}) = {got} is outside 2^-14"
            );
        }
    }

    #[test]
    fn rsqrt14_is_within_the_architectural_error_bound() {
        // Exact powers of two with an *odd* unbiased exponent are excluded:
        // upstream mishandles them, and `rsqrt14_reproduces_the_upstream_power_
        // of_two_bug` pins what it does instead.
        for v in [1.0f32, 0.25, 3.0, 16.0, 1.5, 7.0, 1e10] {
            let got = approximate_rsqrt14_f32(v.to_bits(), false);
            assert!(
                rel_err_f32(got, 1.0 / v.sqrt()) <= 2.0f32.powi(-14),
                "rsqrt14({v}) = {} is outside 2^-14",
                f32::from_bits(got)
            );
        }
        for v in [1.0f64, 0.25, 3.0, 16.0, 1.5, 7.0] {
            let got = f64::from_bits(approximate_rsqrt14_f64(v.to_bits(), false));
            let want = 1.0 / v.sqrt();
            assert!(
                ((got - want) / want).abs() <= 2.0f64.powi(-14),
                "rsqrt14({v}) = {got} is outside 2^-14"
            );
        }
    }

    #[test]
    fn rsqrt14_reproduces_the_upstream_power_of_two_bug() {
        // Bochs takes an `exp++` shortcut when the significand is zero, which
        // is only valid on the even-exponent table. For an exact power of two
        // with an odd unbiased exponent it therefore returns the reciprocal of
        // the square root of the *next* power of two: 1.0 for 2.0, 0.5 for
        // 8.0. Hardware returns ~0.7071 and ~0.35355. Reproduced deliberately
        // — parity with upstream is the invariant here — and recorded in
        // docs/bochs-upstream-bugs.md.
        for (v, bochs) in [(2.0f32, 1.0f32), (8.0, 0.5), (0.5, 2.0), (32.0, 0.25)] {
            assert_eq!(
                approximate_rsqrt14_f32(v.to_bits(), false),
                bochs.to_bits(),
                "rsqrt14({v}) must match Bochs, not hardware"
            );
        }
        assert_eq!(
            approximate_rsqrt14_f64(2.0f64.to_bits(), false),
            1.0f64.to_bits()
        );
        // The even-exponent powers of two are unaffected and exact.
        assert_eq!(
            approximate_rsqrt14_f32(4.0f32.to_bits(), false),
            0.5f32.to_bits()
        );
        assert_eq!(
            approximate_rsqrt14_f32(1.0f32.to_bits(), false),
            1.0f32.to_bits()
        );
    }

    #[test]
    fn the_special_cases_are_exact() {
        // Zero reciprocates to infinity of the same sign, infinity to zero.
        assert_eq!(approximate_rcp14_f32(0.0f32.to_bits(), false, false), 0x7F80_0000);
        assert_eq!(
            approximate_rcp14_f32((-0.0f32).to_bits(), false, false),
            0xFF80_0000
        );
        assert_eq!(
            approximate_rcp14_f32(f32::INFINITY.to_bits(), false, false),
            0
        );
        assert_eq!(
            approximate_rcp14_f32(f32::NEG_INFINITY.to_bits(), false, false),
            0x8000_0000
        );

        // A signalling NaN is quietened, not signalled — VRCP14 raises nothing.
        let snan = 0x7F80_0001u32;
        assert_eq!(approximate_rcp14_f32(snan, false, false), snan | 0x0040_0000);

        // rsqrt of a negative is the default NaN; of +inf is zero.
        assert_eq!(
            approximate_rsqrt14_f32((-4.0f32).to_bits(), false),
            FLOAT32_DEFAULT_NAN
        );
        assert_eq!(approximate_rsqrt14_f32(f32::INFINITY.to_bits(), false), 0);
        assert_eq!(
            approximate_rsqrt14_f32(0.0f32.to_bits(), false),
            0x7F80_0000
        );

        // DAZ turns a denormal operand into a zero operand. The smallest
        // denormal reciprocates past the top of the range either way, so use
        // one whose true reciprocal is finite to tell the two paths apart.
        let denorm = 0x0000_0001u32;
        assert_eq!(approximate_rcp14_f32(denorm, true, false), 0x7F80_0000);
        let big_denorm = 0x007F_FFFFu32; // just below the smallest normal
        assert_eq!(approximate_rcp14_f32(big_denorm, true, false), 0x7F80_0000);
        assert_ne!(
            approximate_rcp14_f32(big_denorm, false, false),
            0x7F80_0000,
            "without DAZ a denormal is normalised and reciprocated properly"
        );
    }
}
