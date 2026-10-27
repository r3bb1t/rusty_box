//! Arm behaviour of the softfloat surface: agreement with the host's IEEE
//! arithmetic on ordinary inputs in round-to-nearest, and the Arm-specific
//! rules (default NaN, NaN propagation order, FPCR.DN, FPCR.FZ, tininess
//! before rounding, saturating conversions, FMIN/FMAX signed zeros).

extern crate std;

use super::*;

const IOC: u64 = FPSR_IOC;
const DZC: u64 = FPSR_DZC;
const OFC: u64 = FPSR_OFC;
const UFC: u64 = FPSR_UFC;
const IXC: u64 = FPSR_IXC;
const IDC: u64 = FPSR_IDC;

const FZ: u64 = 1 << 24;
const DN: u64 = 1 << 25;

fn status() -> ArmFpStatus {
    ArmFpStatus::from_fpcr(0)
}

fn status_with(fpcr: u64) -> ArmFpStatus {
    ArmFpStatus::from_fpcr(fpcr)
}

// Quiet and signaling NaNs with distinguishable payloads.
const Q1: u32 = 0x7FC0_0001;
const Q2: u32 = 0x7FC0_0002;
const S1: u32 = 0x7F80_0001;
const S2: u32 = 0x7F80_0002;
const S1_QUIET: u32 = 0x7FC0_0001;
const S2_QUIET: u32 = 0x7FC0_0002;
const DQ1: u64 = 0x7FF8_0000_0000_0001;
const DQ2: u64 = 0x7FF8_0000_0000_0002;
const DS1: u64 = 0x7FF0_0000_0000_0001;
const DS2: u64 = 0x7FF0_0000_0000_0002;

const ONE: u32 = 0x3F80_0000;
const POS_INF: u32 = 0x7F80_0000;
const NEG_INF: u32 = 0xFF80_0000;
const POS_ZERO: u32 = 0;
const NEG_ZERO: u32 = 0x8000_0000;
const DPOS_INF: u64 = 0x7FF0_0000_0000_0000;

/// Fixed-seed xorshift64 (Marsaglia), so every run checks the same inputs.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A finite single: either any finite bit pattern (subnormals and
    /// extremes included) or one with a moderate exponent, so sums and
    /// products exercise cancellation and rounding rather than saturating.
    fn finite_f32(&mut self) -> f32 {
        loop {
            let bits = self.next();
            let x = if bits & 1 == 0 {
                (bits >> 32) as u32
            } else {
                let exp = 0x7F - 24 + ((bits >> 8) % 48) as u32;
                ((bits >> 63) as u32) << 31 | exp << 23 | ((bits >> 20) as u32 & 0x007F_FFFF)
            };
            let f = f32::from_bits(x);
            if f.is_finite() {
                return f;
            }
        }
    }

    /// A finite double; see [`XorShift::finite_f32`].
    fn finite_f64(&mut self) -> f64 {
        loop {
            let bits = self.next();
            let x = if bits & 1 == 0 {
                self.next()
            } else {
                let exp = 0x3FF - 53 + ((bits >> 8) % 106);
                (bits >> 63) << 63 | exp << 52 | (self.next() & 0x000F_FFFF_FFFF_FFFF)
            };
            let f = f64::from_bits(x);
            if f.is_finite() {
                return f;
            }
        }
    }
}

const RANDOM_CASES: usize = 4000;

/// A host result against ours: bit-identical, except that a NaN the host
/// produces (from finite inputs only an invalid operation) must be the Arm
/// default NaN.
fn check32(ours: u32, host: f32, what: &str, a: f32, b: f32) {
    if host.is_nan() {
        assert_eq!(ours, 0x7FC0_0000, "{what}({a:e}, {b:e}) default NaN");
    } else {
        assert_eq!(ours, host.to_bits(), "{what}({a:e}, {b:e})");
    }
}

fn check64(ours: u64, host: f64, what: &str, a: f64, b: f64) {
    if host.is_nan() {
        assert_eq!(ours, 0x7FF8_0000_0000_0000, "{what}({a:e}, {b:e}) default NaN");
    } else {
        assert_eq!(ours, host.to_bits(), "{what}({a:e}, {b:e})");
    }
}

#[test]
fn f32_arithmetic_matches_the_host_in_round_to_nearest() {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    for _ in 0..RANDOM_CASES {
        let (a, b, c) = (rng.finite_f32(), rng.finite_f32(), rng.finite_f32());
        let (ab, bb, cb) = (a.to_bits(), b.to_bits(), c.to_bits());
        let mut fp = status();
        check32(f32_add(ab, bb, &mut fp), a + b, "add", a, b);
        check32(f32_sub(ab, bb, &mut fp), a - b, "sub", a, b);
        check32(f32_mul(ab, bb, &mut fp), a * b, "mul", a, b);
        check32(f32_div(ab, bb, &mut fp), a / b, "div", a, b);
        check32(f32_sqrt(ab, &mut fp), a.sqrt(), "sqrt", a, a);
        check32(
            f32_mul_add(FusedMulAdd::Fmadd, cb, ab, bb, &mut fp),
            a.mul_add(b, c),
            "fma",
            a,
            b,
        );
        assert_eq!(f32_to_f64(ab, &mut fp), f64::from(a).to_bits(), "f32_to_f64({a:e})");
    }
}

#[test]
fn f64_arithmetic_matches_the_host_in_round_to_nearest() {
    let mut rng = XorShift(0xD1B5_4A32_D192_ED03);
    for _ in 0..RANDOM_CASES {
        let (a, b, c) = (rng.finite_f64(), rng.finite_f64(), rng.finite_f64());
        let (ab, bb, cb) = (a.to_bits(), b.to_bits(), c.to_bits());
        let mut fp = status();
        check64(f64_add(ab, bb, &mut fp), a + b, "add", a, b);
        check64(f64_sub(ab, bb, &mut fp), a - b, "sub", a, b);
        check64(f64_mul(ab, bb, &mut fp), a * b, "mul", a, b);
        check64(f64_div(ab, bb, &mut fp), a / b, "div", a, b);
        check64(f64_sqrt(ab, &mut fp), a.sqrt(), "sqrt", a, a);
        check64(
            f64_mul_add(FusedMulAdd::Fmadd, cb, ab, bb, &mut fp),
            a.mul_add(b, c),
            "fma",
            a,
            b,
        );
        check32(f64_to_f32(ab, &mut fp), a as f32, "f64_to_f32", a as f32, a as f32);
    }
}

/// Rust's float-to-int `as` truncates toward zero, saturates and maps NaN
/// to 0 — exactly Arm FCVTZS/FCVTZU. Int-to-float `as` rounds to nearest
/// even, as SCVTF/UCVTF do under FPCR.RMode = 0b00.
#[test]
fn integer_conversions_match_the_host() {
    let mut rng = XorShift(0x2545_F491_4F6C_DD1D);
    for _ in 0..RANDOM_CASES {
        let (a, d) = (rng.finite_f32(), rng.finite_f64());
        // Scale some inputs into the integer range so they do not all saturate.
        let a = if rng.next() & 1 == 0 { a } else { a * 1.0e9 };
        let d = if rng.next() & 1 == 0 { d } else { (d % 1.0e20) * 1.0e9 };
        let (ab, db) = (a.to_bits(), d.to_bits());
        let mut fp = status();
        let z = FpRounding::Zero;
        assert_eq!(f32_to_i32(ab, z, &mut fp), a as i32, "f32_to_i32({a:e})");
        assert_eq!(f32_to_i64(ab, z, &mut fp), a as i64, "f32_to_i64({a:e})");
        assert_eq!(f32_to_u32(ab, z, &mut fp), a as u32, "f32_to_u32({a:e})");
        assert_eq!(f32_to_u64(ab, z, &mut fp), a as u64, "f32_to_u64({a:e})");
        assert_eq!(f64_to_i32(db, z, &mut fp), d as i32, "f64_to_i32({d:e})");
        assert_eq!(f64_to_i64(db, z, &mut fp), d as i64, "f64_to_i64({d:e})");
        assert_eq!(f64_to_u32(db, z, &mut fp), d as u32, "f64_to_u32({d:e})");
        assert_eq!(f64_to_u64(db, z, &mut fp), d as u64, "f64_to_u64({d:e})");

        let x = rng.next();
        let shift = (rng.next() % 64) as u32;
        let x = x >> shift;
        let te = FpRounding::TieEven;
        assert_eq!(i32_to_f32(x as i32, te, &mut fp), (x as i32 as f32).to_bits(), "i32_to_f32({x})");
        assert_eq!(i64_to_f32(x as i64, te, &mut fp), (x as i64 as f32).to_bits(), "i64_to_f32({x})");
        assert_eq!(u32_to_f32(x as u32, te, &mut fp), (x as u32 as f32).to_bits(), "u32_to_f32({x})");
        assert_eq!(u64_to_f32(x, te, &mut fp), (x as f32).to_bits(), "u64_to_f32({x})");
        assert_eq!(i32_to_f64(x as i32), (x as i32 as f64).to_bits(), "i32_to_f64({x})");
        assert_eq!(u32_to_f64(x as u32), (x as u32 as f64).to_bits(), "u32_to_f64({x})");
        assert_eq!(i64_to_f64(x as i64, te, &mut fp), (x as i64 as f64).to_bits(), "i64_to_f64({x})");
        assert_eq!(u64_to_f64(x, te, &mut fp), (x as f64).to_bits(), "u64_to_f64({x})");
    }
}

#[test]
fn invalid_operations_give_the_arm_default_nan() {
    let mut fp = status();
    assert_eq!(f32_mul(POS_ZERO, POS_INF, &mut fp), 0x7FC0_0000);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    let mut fp = status();
    assert_eq!(f64_sub(DPOS_INF, DPOS_INF, &mut fp), 0x7FF8_0000_0000_0000);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    let mut fp = status();
    assert_eq!(f32_sqrt(0xBF80_0000, &mut fp), 0x7FC0_0000);
    assert_eq!(f64_div(0, 0, &mut fp), 0x7FF8_0000_0000_0000);
    assert_eq!(f32_add(POS_INF, NEG_INF, &mut fp), 0x7FC0_0000);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    // Default NaN in half precision, through FPCR.DN on a conversion.
    let mut fp = status_with(DN);
    assert_eq!(f32_to_f16(Q1, &mut fp), 0x7E00);
    assert_eq!(f64_to_f16(DQ1, &mut fp), 0x7E00);
}

#[test]
fn two_operand_nan_propagation_follows_fpprocessnans() {
    let cases: [(u32, u32, u32, u64); 8] = [
        (Q1, Q2, Q1, 0),
        (Q2, Q1, Q2, 0),
        (Q1, S2, S2_QUIET, IOC),
        (S1, Q2, S1_QUIET, IOC),
        (S1, S2, S1_QUIET, IOC),
        (ONE, Q2, Q2, 0),
        (S2, ONE, S2_QUIET, IOC),
        (0xFFC0_0005, ONE, 0xFFC0_0005, 0),
    ];
    for (a, b, want, flags) in cases {
        for op in [f32_add, f32_sub, f32_mul, f32_div, f32_max, f32_min] {
            let mut fp = status();
            assert_eq!(op(a, b, &mut fp), want, "{a:#x} op {b:#x}");
            assert_eq!(fp.cumulative_fpsr(), flags, "{a:#x} op {b:#x} flags");
        }
    }
    let cases64: [(u64, u64, u64, u64); 5] = [
        (DQ1, DQ2, DQ1, 0),
        (DQ1, DS2, DS2 | 0x0008_0000_0000_0000, IOC),
        (DS1, DQ2, DS1 | 0x0008_0000_0000_0000, IOC),
        (DS1, DS2, DS1 | 0x0008_0000_0000_0000, IOC),
        (0x3FF0_0000_0000_0000, DQ2, DQ2, 0),
    ];
    for (a, b, want, flags) in cases64 {
        for op in [f64_add, f64_sub, f64_mul, f64_div, f64_max, f64_min] {
            let mut fp = status();
            assert_eq!(op(a, b, &mut fp), want, "{a:#x} op {b:#x}");
            assert_eq!(fp.cumulative_fpsr(), flags, "{a:#x} op {b:#x} flags");
        }
    }
    // A one-operand NaN is quietened and keeps its payload and sign.
    let mut fp = status();
    assert_eq!(f32_sqrt(0xFF80_0003, &mut fp), 0xFFC0_0003);
    assert_eq!(fp.cumulative_fpsr(), IOC);
}

#[test]
fn fused_multiply_add_nan_propagation_follows_fpprocessnans3() {
    let fma = |a: u32, n: u32, m: u32| {
        let mut fp = status();
        let r = f32_mul_add(FusedMulAdd::Fmadd, a, n, m, &mut fp);
        (r, fp.cumulative_fpsr())
    };
    // Quiet NaNs: the addend first, then op1, then op2.
    assert_eq!(fma(Q1, Q2, ONE), (Q1, 0));
    assert_eq!(fma(ONE, Q1, Q2), (Q1, 0));
    assert_eq!(fma(ONE, ONE, Q2), (Q2, 0));
    // A signaling NaN wins over any quiet NaN, in the same order.
    assert_eq!(fma(Q1, S2, ONE), (S2_QUIET, IOC));
    assert_eq!(fma(Q1, Q2, S1), (S1_QUIET, IOC));
    assert_eq!(fma(S1, S2, ONE), (S1_QUIET, IOC));
    // Infinity times zero plus a quiet NaN is the default NaN, invalid;
    // plus a signaling NaN it is that NaN, quietened.
    assert_eq!(fma(Q1, POS_INF, POS_ZERO), (0x7FC0_0000, IOC));
    assert_eq!(fma(Q1, NEG_ZERO, POS_INF), (0x7FC0_0000, IOC));
    assert_eq!(fma(S1, POS_ZERO, POS_INF), (S1_QUIET, IOC));
    // Without a NaN, infinity times zero is invalid too.
    assert_eq!(fma(ONE, POS_INF, POS_ZERO), (0x7FC0_0000, IOC));
    // Opposite infinities are invalid.
    assert_eq!(fma(NEG_INF, POS_INF, ONE), (0x7FC0_0000, IOC));

    // FPNeg flips a NaN operand's sign before propagation.
    let mut fp = status();
    assert_eq!(f32_mul_add(FusedMulAdd::Fmsub, ONE, Q1, ONE, &mut fp), Q1 | 0x8000_0000);
    assert_eq!(f32_mul_add(FusedMulAdd::Fnmadd, Q1, ONE, ONE, &mut fp), Q1 | 0x8000_0000);
    assert_eq!(f32_mul_add(FusedMulAdd::Fnmsub, Q1, ONE, ONE, &mut fp), Q1 | 0x8000_0000);
    assert_eq!(f32_mul_add(FusedMulAdd::Fnmsub, ONE, Q1, ONE, &mut fp), Q1);

    // The four forms on ordinary values: a = 1, n = 2, m = 3.
    let (a, n, m) = (1.0f64.to_bits(), 2.0f64.to_bits(), 3.0f64.to_bits());
    let mut fp = status();
    for (form, want) in [
        (FusedMulAdd::Fmadd, 7.0f64),
        (FusedMulAdd::Fmsub, -5.0),
        (FusedMulAdd::Fnmadd, -7.0),
        (FusedMulAdd::Fnmsub, 5.0),
    ] {
        assert_eq!(f64_mul_add(form, a, n, m, &mut fp), want.to_bits(), "{form:?}");
    }
    assert_eq!(fp.cumulative_fpsr(), 0);

    let mut fp = status();
    assert_eq!(f64_mul_add(FusedMulAdd::Fmadd, DQ1, DPOS_INF, 0, &mut fp), 0x7FF8_0000_0000_0000);
    assert_eq!(f64_mul_add(FusedMulAdd::Fmadd, DQ1, DS2, DQ2, &mut fp), DS2 | 0x0008_0000_0000_0000);
    assert_eq!(fp.cumulative_fpsr(), IOC);
}

/// A double fused multiply-add whose addend `a = -RN(n*m)` cancels the
/// high word of the 128-bit product exactly, leaving a low word whose top
/// set bit is bit 62: Berkeley `softfloat_mulAddF64` normalizes that
/// remainder with a left shift of 0. Arm FPMulAdd's result is the exact
/// sum `n*m + a`, the product's rounding error, which binary64 holds, so
/// it is the same in every rounding mode and raises nothing.
#[test]
fn fused_multiply_add_cancelling_the_high_product_word_is_exact() {
    const SIGN: u64 = 1 << 63;
    // [n, m, a, n*m + a]; the sums are the exact rationals
    // 1734089336883 * 2^-102 and 111467904765 * 2^-99.
    let cases: [[u64; 4]; 2] = [
        [
            0x3FFE_E6F8_F0B8_0AC5,
            0x3FFC_95EC_6976_DA5C,
            0xC00B_9AE1_9706_7916,
            0x3C19_3BFC_7683_3000,
        ],
        [
            0x3FF0_2040_ABA1_8322,
            0x3FF7_3F7C_DCF7_0AD0,
            0xBFF7_6E59_CE05_A09D,
            0x3C09_F401_3EFD_0000,
        ],
    ];
    for [n, m, a, sum] in cases {
        let host = f64::from_bits(n).mul_add(f64::from_bits(m), f64::from_bits(a));
        assert_eq!(
            host.to_bits(),
            sum,
            "host {n:#018x} * {m:#018x} + {a:#018x}"
        );
        // Each form computes n*m + a or its negation.
        for (form, addend, want) in [
            (FusedMulAdd::Fmadd, a, sum),
            (FusedMulAdd::Fmsub, a ^ SIGN, sum ^ SIGN),
            (FusedMulAdd::Fnmadd, a, sum ^ SIGN),
            (FusedMulAdd::Fnmsub, a ^ SIGN, sum),
        ] {
            for rmode in 0..4u64 {
                let mut fp = status_with(rmode << 22);
                let got = f64_mul_add(form, addend, n, m, &mut fp);
                assert_eq!(
                    got, want,
                    "{form:?} RMode {rmode}: {addend:#018x} with {n:#018x} * {m:#018x}"
                );
                assert_eq!(fp.cumulative_fpsr(), 0, "{form:?} RMode {rmode}");
            }
        }
    }
}

/// Berkeley `softfloat_shortShiftLeft128` shifts a 128-bit value left; a
/// distance of 0 leaves it unchanged.
#[test]
fn a_128_bit_left_shift_by_0_to_63_is_the_shift_of_the_whole_value() {
    use super::primitives::short_shift_left128;
    let mut rng = XorShift(0x5EED_0128_5A1F_0000);
    for dist in 0..64u8 {
        for _ in 0..64 {
            let (a64, a0) = (rng.next(), rng.next());
            let whole = ((u128::from(a64) << 64) | u128::from(a0)) << dist;
            let want = ((whole >> 64) as u64, whole as u64);
            assert_eq!(
                short_shift_left128(a64, a0, dist),
                want,
                "{a64:#018x}_{a0:016x} << {dist}"
            );
        }
    }
}

#[test]
fn default_nan_mode_replaces_every_nan_result() {
    let mut fp = status_with(DN);
    assert_eq!(f32_add(Q1, ONE, &mut fp), 0x7FC0_0000);
    assert_eq!(fp.cumulative_fpsr(), 0);
    assert_eq!(f32_add(S1, ONE, &mut fp), 0x7FC0_0000);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    let mut fp = status_with(DN);
    assert_eq!(f64_mul(DQ1, DQ2, &mut fp), 0x7FF8_0000_0000_0000);
    assert_eq!(f32_sqrt(0xFFC0_0007, &mut fp), 0x7FC0_0000);
    assert_eq!(f32_max(Q1, ONE, &mut fp), 0x7FC0_0000);
    assert_eq!(f32_round_to_int(Q1, FpRounding::TieEven, InexactReport::Suppress, &mut fp), 0x7FC0_0000);
    assert_eq!(f32_mul_add(FusedMulAdd::Fmadd, Q1, Q2, ONE, &mut fp), 0x7FC0_0000);
    assert_eq!(f32_to_f64(Q1, &mut fp), 0x7FF8_0000_0000_0000);
    assert_eq!(f64_to_f32(DQ1, &mut fp), 0x7FC0_0000);
    assert_eq!(f16_to_f32(0x7E01, &mut fp), 0x7FC0_0000);
    assert_eq!(fp.cumulative_fpsr(), 0);
}

#[test]
fn nan_conversions_keep_sign_and_payload() {
    let mut fp = status();
    assert_eq!(f32_to_f16(0x7FC0_0000, &mut fp), 0x7E00);
    assert_eq!(f32_to_f16(0xFFC0_0000, &mut fp), 0xFE00);
    assert_eq!(f32_to_f64(0xFFC0_0001, &mut fp), 0xFFF8_0000_2000_0000);
    assert_eq!(f64_to_f16(0x7FF8_0000_0000_0000 | (0x155 << 42), &mut fp), 0x7F55);
    assert_eq!(f16_to_f64(0x7F55, &mut fp), 0x7FFD_5400_0000_0000);
    assert_eq!(fp.cumulative_fpsr(), 0);
    // A signaling NaN is quietened and raises invalid.
    assert_eq!(f16_to_f32(0x7D00, &mut fp), 0x7FE0_0000);
    assert_eq!(fp.cumulative_fpsr(), IOC);
}

#[test]
fn float_to_integer_conversions_saturate() {
    let nan = 0x7FC0_0000u32;
    let dnan = 0x7FF8_0000_0000_0000u64;
    let z = FpRounding::Zero;
    let conv32 = |f: fn(u32, FpRounding, &mut ArmFpStatus) -> i64, a: f32| {
        let mut fp = status();
        let r = f(a.to_bits(), z, &mut fp);
        (r, fp.cumulative_fpsr())
    };
    let i32_of = |a: u32, r: FpRounding, fp: &mut ArmFpStatus| i64::from(f32_to_i32(a, r, fp));
    let u32_of = |a: u32, r: FpRounding, fp: &mut ArmFpStatus| i64::from(f32_to_u32(a, r, fp));
    let i64_of = |a: u32, r: FpRounding, fp: &mut ArmFpStatus| f32_to_i64(a, r, fp);

    assert_eq!(conv32(i32_of, f32::from_bits(nan)), (0, IOC));
    assert_eq!(conv32(i32_of, f32::INFINITY), (i64::from(i32::MAX), IOC));
    assert_eq!(conv32(i32_of, f32::NEG_INFINITY), (i64::from(i32::MIN), IOC));
    assert_eq!(conv32(i32_of, 3.0e9), (i64::from(i32::MAX), IOC));
    assert_eq!(conv32(i32_of, -3.0e9), (i64::from(i32::MIN), IOC));
    assert_eq!(conv32(i32_of, -2_147_483_648.0), (i64::from(i32::MIN), 0));
    assert_eq!(conv32(u32_of, -1.0), (0, IOC));
    assert_eq!(conv32(u32_of, -0.5), (0, IXC));
    assert_eq!(conv32(u32_of, 5.0e9), (i64::from(u32::MAX), IOC));
    assert_eq!(conv32(u32_of, f32::from_bits(nan)), (0, IOC));
    assert_eq!(conv32(i64_of, f32::from_bits(nan)), (0, IOC));
    assert_eq!(conv32(i64_of, 1.0e19), (i64::MAX, IOC));
    assert_eq!(conv32(i64_of, -1.0e19), (i64::MIN, IOC));

    let mut fp = status();
    assert_eq!(f64_to_u64(dnan, z, &mut fp), 0);
    assert_eq!(f64_to_u64((-1.0f64).to_bits(), z, &mut fp), 0);
    assert_eq!(f64_to_u64(f64::INFINITY.to_bits(), z, &mut fp), u64::MAX);
    assert_eq!(f64_to_i64(f64::NEG_INFINITY.to_bits(), z, &mut fp), i64::MIN);
    assert_eq!(f64_to_i32(dnan, z, &mut fp), 0);
    assert_eq!(f64_to_u32((-7.0f64).to_bits(), z, &mut fp), 0);
    assert_eq!(fp.cumulative_fpsr(), IOC);

    // Rounding past the range is invalid, not inexact.
    let mut fp = status();
    assert_eq!(f64_to_i32(2_147_483_647.5f64.to_bits(), FpRounding::TieEven, &mut fp), i32::MAX);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    let mut fp = status();
    assert_eq!(f64_to_i32((-2_147_483_648.5f64).to_bits(), FpRounding::TieEven, &mut fp), i32::MIN);
    assert_eq!(fp.cumulative_fpsr(), IXC);

    // Every rounding mode on the halfway cases.
    for (v, even, away, pos, neg, zero) in [
        (2.5f64, 2i64, 3i64, 3i64, 2i64, 2i64),
        (-2.5, -2, -3, -2, -3, -2),
        (0.5, 0, 1, 1, 0, 0),
        (-1.5, -2, -2, -1, -2, -1),
    ] {
        for (rounding, want) in [
            (FpRounding::TieEven, even),
            (FpRounding::TieAway, away),
            (FpRounding::PosInf, pos),
            (FpRounding::NegInf, neg),
            (FpRounding::Zero, zero),
        ] {
            let mut fp = status();
            assert_eq!(f64_to_i64(v.to_bits(), rounding, &mut fp), want, "{v} {rounding:?}");
            assert_eq!(fp.cumulative_fpsr(), IXC, "{v} {rounding:?} flags");
            let mut fp = status();
            assert_eq!(i64::from(f32_to_i32((v as f32).to_bits(), rounding, &mut fp)), want);
        }
    }
    // A negative value an unsigned conversion rounds to zero is exact zero
    // plus inexact; one it rounds below zero saturates.
    let mut fp = status();
    assert_eq!(f64_to_u32((-0.5f64).to_bits(), FpRounding::TieEven, &mut fp), 0);
    assert_eq!(fp.cumulative_fpsr(), IXC);
    let mut fp = status();
    assert_eq!(f64_to_u32((-0.5f64).to_bits(), FpRounding::TieAway, &mut fp), 0);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    let mut fp = status();
    assert_eq!(f32_to_u64((-0.25f32).to_bits(), FpRounding::NegInf, &mut fp), 0);
    assert_eq!(fp.cumulative_fpsr(), IOC);
}

/// 0x3F34F93B * 0x00B510AC is 2^-126 * (1 - 1.39e-8) exactly: below the
/// minimum normal before rounding, the minimum normal after. Arm detects
/// tininess before rounding, so the inexact result also underflows (an
/// after-rounding implementation, x86 for one, raises inexact alone).
#[test]
fn tininess_is_detected_before_rounding() {
    let (a, b) = (0x3F34_F93Bu32, 0x00B5_10ACu32);
    let host = f32::from_bits(a) * f32::from_bits(b);
    assert_eq!(host.to_bits(), 0x0080_0000);
    let mut fp = status();
    assert_eq!(f32_mul(a, b, &mut fp), 0x0080_0000);
    assert_eq!(fp.cumulative_fpsr(), UFC | IXC);
    // The same product under FPCR.FZ is flushed: tiny before rounding.
    let mut fp = status_with(FZ);
    assert_eq!(f32_mul(a, b, &mut fp), 0);
    assert_eq!(fp.cumulative_fpsr(), UFC);
    // An exact subnormal result does not underflow.
    let mut fp = status();
    assert_eq!(f32_mul(0x0080_0000, 0x3F00_0000, &mut fp), 0x0040_0000);
    assert_eq!(fp.cumulative_fpsr(), 0);
    // Double precision: this product is 2^-1022 * (1 - 0.183 * 2^-52),
    // tiny before rounding and the minimum normal after.
    let (a, b) = (0x3FE6_A09E_667C_2E8Cu64, 0x0016_A09E_6682_490Du64);
    let host = f64::from_bits(a) * f64::from_bits(b);
    assert_eq!(host.to_bits(), 0x0010_0000_0000_0000);
    let mut fp = status();
    assert_eq!(f64_mul(a, b, &mut fp), 0x0010_0000_0000_0000);
    assert_eq!(fp.cumulative_fpsr(), UFC | IXC);
    // Fused multiply-add rounds the exact sum once: the same product plus a
    // zero addend is tiny before rounding too.
    let mut fp = status();
    assert_eq!(f64_mul_add(FusedMulAdd::Fmadd, 0, a, b, &mut fp), 0x0010_0000_0000_0000);
    assert_eq!(fp.cumulative_fpsr(), UFC | IXC);
}

#[test]
fn flush_to_zero_flushes_inputs_and_tiny_results() {
    let mut fp = status_with(FZ);
    assert_eq!(f32_add(0x0000_0001, POS_ZERO, &mut fp), POS_ZERO);
    assert_eq!(fp.cumulative_fpsr(), IDC);
    let mut fp = status_with(FZ);
    assert_eq!(f32_add(0x8000_0001, NEG_ZERO, &mut fp), NEG_ZERO);
    assert_eq!(fp.cumulative_fpsr(), IDC);
    // A tiny result becomes a signed zero, raising underflow without inexact.
    let mut fp = status_with(FZ);
    assert_eq!(f32_mul(0x0080_0000, 0x3F00_0000, &mut fp), POS_ZERO);
    assert_eq!(fp.cumulative_fpsr(), UFC);
    let mut fp = status_with(FZ);
    assert_eq!(f64_mul(0x8010_0000_0000_0000, 0.5f64.to_bits(), &mut fp), 0x8000_0000_0000_0000);
    assert_eq!(fp.cumulative_fpsr(), UFC);
    let mut fp = status_with(FZ);
    assert_eq!(f64_to_f32(1.0e-40f64.to_bits(), &mut fp), POS_ZERO);
    assert_eq!(fp.cumulative_fpsr(), UFC);
    // The difference of two numbers of one exponent is exact: flushed when
    // it is subnormal, kept when it is normal, the smallest normal included.
    let mut fp = status_with(FZ);
    assert_eq!(f32_sub(0x0120_0000, 0x0100_0000, &mut fp), POS_ZERO);
    assert_eq!(f64_sub(0x0024_0000_0000_0000, 0x0020_0000_0000_0000, &mut fp), 0);
    assert_eq!(fp.cumulative_fpsr(), UFC);
    let mut fp = status_with(FZ);
    assert_eq!(f32_sub(0x0178_0000, 0x0100_0000, &mut fp), 0x00F0_0000);
    assert_eq!(f32_sub(0x01F0_4AC3, 0x01C3_8956, &mut fp), 0x00B3_05B4);
    assert_eq!(f32_sub(0x0140_0000, 0x0100_0000, &mut fp), 0x0080_0000);
    assert_eq!(f64_sub(0x002F_0000_0000_0000, 0x0020_0000_0000_0000, &mut fp), 0x001E_0000_0000_0000);
    assert_eq!(f64_add(0x8028_0000_0000_0000, 0x0020_0000_0000_0000, &mut fp), 0x8010_0000_0000_0000);
    assert_eq!(fp.cumulative_fpsr(), 0);
    // Comparisons and conversions see the flushed operand.
    let mut fp = status_with(FZ);
    assert_eq!(f32_compare(0x0000_0001, POS_ZERO, NanSignal::Quiet, &mut fp), FpOrdering::Equal);
    assert_eq!(f32_to_f64(0x8000_0001, &mut fp), 0x8000_0000_0000_0000);
    assert_eq!(f32_to_i32(0x0000_0001, FpRounding::PosInf, &mut fp), 0);
    assert_eq!(fp.cumulative_fpsr(), IDC);
    // A flushed operand is a zero for the inf*0 + qNaN rule.
    let mut fp = status_with(FZ);
    assert_eq!(f32_mul_add(FusedMulAdd::Fmadd, Q1, POS_INF, 0x0000_0001, &mut fp), 0x7FC0_0000);
    assert_eq!(fp.cumulative_fpsr(), IOC | IDC);
    // Half precision is never flushed.
    let mut fp = status_with(FZ);
    assert_eq!(f16_to_f32(0x0001, &mut fp), 0x3380_0000);
    assert_eq!(f64_to_f16((5.960_464_477_539_063e-8f64).to_bits(), &mut fp), 0x0001);
    assert_eq!(fp.cumulative_fpsr(), 0);
    // Without FZ, denormals are ordinary operands and exact results.
    let mut fp = status();
    assert_eq!(f32_add(0x0000_0001, 0x0000_0001, &mut fp), 0x0000_0002);
    assert_eq!(fp.cumulative_fpsr(), 0);
}

#[test]
fn min_max_order_signed_zeros_and_handle_nans() {
    let mut fp = status();
    for (a, b, max, min) in [
        (NEG_ZERO, POS_ZERO, POS_ZERO, NEG_ZERO),
        (POS_ZERO, NEG_ZERO, POS_ZERO, NEG_ZERO),
        (NEG_ZERO, NEG_ZERO, NEG_ZERO, NEG_ZERO),
        (POS_ZERO, POS_ZERO, POS_ZERO, POS_ZERO),
        (ONE, 0x4000_0000, 0x4000_0000, ONE),
        (0xBF80_0000, 0x4000_0000, 0x4000_0000, 0xBF80_0000),
        (NEG_INF, ONE, ONE, NEG_INF),
    ] {
        assert_eq!(f32_max(a, b, &mut fp), max, "max({a:#x}, {b:#x})");
        assert_eq!(f32_min(a, b, &mut fp), min, "min({a:#x}, {b:#x})");
        assert_eq!(f32_maxnum(a, b, &mut fp), max, "maxnum({a:#x}, {b:#x})");
        assert_eq!(f32_minnum(a, b, &mut fp), min, "minnum({a:#x}, {b:#x})");
    }
    assert_eq!(fp.cumulative_fpsr(), 0);
    let d0 = 0x8000_0000_0000_0000u64;
    assert_eq!(f64_max(d0, 0, &mut fp), 0);
    assert_eq!(f64_min(0, d0, &mut fp), d0);
    assert_eq!(f64_maxnum(d0, 0, &mut fp), 0);
    assert_eq!(f64_minnum(0, d0, &mut fp), d0);

    // FMAXNM/FMINNM: a single quiet NaN loses to a number.
    let mut fp = status();
    assert_eq!(f32_maxnum(Q1, ONE, &mut fp), ONE);
    assert_eq!(f32_minnum(ONE, Q1, &mut fp), ONE);
    assert_eq!(f32_minnum(Q1, NEG_INF, &mut fp), NEG_INF);
    assert_eq!(f32_maxnum(Q1, NEG_INF, &mut fp), NEG_INF);
    assert_eq!(f64_maxnum(DQ1, 0x3FF0_0000_0000_0000, &mut fp), 0x3FF0_0000_0000_0000);
    assert_eq!(f32_maxnum(Q1, Q2, &mut fp), Q1);
    assert_eq!(fp.cumulative_fpsr(), 0);
    // FMAX/FMIN propagate the NaN.
    assert_eq!(f32_max(Q1, ONE, &mut fp), Q1);
    assert_eq!(f32_min(ONE, Q2, &mut fp), Q2);
    assert_eq!(fp.cumulative_fpsr(), 0);
    // A signaling NaN propagates through FMAXNM too, invalid.
    let mut fp = status();
    assert_eq!(f32_maxnum(S1, ONE, &mut fp), S1_QUIET);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    let mut fp = status();
    assert_eq!(f32_minnum(Q1, S2, &mut fp), S2_QUIET);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    // Under FZ a denormal operand is a signed zero for the zero rule.
    let mut fp = status_with(FZ);
    assert_eq!(f32_max(0x8000_0001, POS_ZERO, &mut fp), POS_ZERO);
    assert_eq!(f32_min(0x0000_0001, NEG_ZERO, &mut fp), NEG_ZERO);
    assert_eq!(fp.cumulative_fpsr(), IDC);
}

#[test]
fn compare_reports_ordering_and_signals_per_fcmp_or_fcmpe() {
    let mut fp = status();
    assert_eq!(f32_compare(ONE, 0x4000_0000, NanSignal::Quiet, &mut fp), FpOrdering::Less);
    assert_eq!(f32_compare(0x4000_0000, ONE, NanSignal::Quiet, &mut fp), FpOrdering::Greater);
    assert_eq!(f32_compare(NEG_ZERO, POS_ZERO, NanSignal::Signaling, &mut fp), FpOrdering::Equal);
    assert_eq!(f32_compare(NEG_INF, 0xFF7F_FFFF, NanSignal::Quiet, &mut fp), FpOrdering::Less);
    assert_eq!(f64_compare(DPOS_INF, DPOS_INF, NanSignal::Quiet, &mut fp), FpOrdering::Equal);
    assert_eq!(fp.cumulative_fpsr(), 0);
    assert_eq!(f32_compare(Q1, ONE, NanSignal::Quiet, &mut fp), FpOrdering::Unordered);
    assert_eq!(fp.cumulative_fpsr(), 0);
    assert_eq!(f32_compare(Q1, ONE, NanSignal::Signaling, &mut fp), FpOrdering::Unordered);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    let mut fp = status();
    assert_eq!(f64_compare(0x3FF0_0000_0000_0000, DS1, NanSignal::Quiet, &mut fp), FpOrdering::Unordered);
    assert_eq!(fp.cumulative_fpsr(), IOC);
    assert_eq!(FpOrdering::Less.nzcv(), 0b1000);
    assert_eq!(FpOrdering::Equal.nzcv(), 0b0110);
    assert_eq!(FpOrdering::Greater.nzcv(), 0b0010);
    assert_eq!(FpOrdering::Unordered.nzcv(), 0b0011);
}

#[test]
fn round_to_int_honours_every_arm_mode() {
    let frint = |v: f64, r: FpRounding| {
        let mut fp = status();
        let z = f64_round_to_int(v.to_bits(), r, InexactReport::Suppress, &mut fp);
        assert_eq!(fp.cumulative_fpsr(), 0, "FRINT {v} {r:?} raises nothing");
        let mut fp = status();
        let z32 = f32_round_to_int((v as f32).to_bits(), r, InexactReport::Suppress, &mut fp);
        assert_eq!(z32, (f64::from_bits(z) as f32).to_bits(), "f32 FRINT {v} {r:?}");
        f64::from_bits(z)
    };
    for (v, n, a, p, m, z) in [
        (2.5f64, 2.0f64, 3.0f64, 3.0f64, 2.0f64, 2.0f64),
        (-2.5, -2.0, -3.0, -2.0, -3.0, -2.0),
        (0.5, 0.0, 1.0, 1.0, 0.0, 0.0),
        (-0.5, -0.0, -1.0, -0.0, -1.0, -0.0),
        (1.5, 2.0, 2.0, 2.0, 1.0, 1.0),
        (0.3, 0.0, 0.0, 1.0, 0.0, 0.0),
        (-0.3, -0.0, -0.0, -0.0, -1.0, -0.0),
    ] {
        assert_eq!(frint(v, FpRounding::TieEven).to_bits(), n.to_bits(), "FRINTN {v}");
        assert_eq!(frint(v, FpRounding::TieAway).to_bits(), a.to_bits(), "FRINTA {v}");
        assert_eq!(frint(v, FpRounding::PosInf).to_bits(), p.to_bits(), "FRINTP {v}");
        assert_eq!(frint(v, FpRounding::NegInf).to_bits(), m.to_bits(), "FRINTM {v}");
        assert_eq!(frint(v, FpRounding::Zero).to_bits(), z.to_bits(), "FRINTZ {v}");
    }
    // FRINTX reports inexactness; an integral operand is exact.
    let mut fp = status();
    assert_eq!(f32_round_to_int(0x4020_0000, FpRounding::TieEven, InexactReport::Raise, &mut fp), 0x4000_0000);
    assert_eq!(fp.cumulative_fpsr(), IXC);
    let mut fp = status();
    assert_eq!(f64_round_to_int(2.0f64.to_bits(), FpRounding::TieEven, InexactReport::Raise, &mut fp), 2.0f64.to_bits());
    assert_eq!(fp.cumulative_fpsr(), 0);
    // Large values and infinities pass through; a signaling NaN is quietened.
    let mut fp = status();
    assert_eq!(f64_round_to_int(1.0e300f64.to_bits(), FpRounding::PosInf, InexactReport::Raise, &mut fp), 1.0e300f64.to_bits());
    assert_eq!(f32_round_to_int(NEG_INF, FpRounding::Zero, InexactReport::Raise, &mut fp), NEG_INF);
    assert_eq!(fp.cumulative_fpsr(), 0);
    assert_eq!(f32_round_to_int(S1, FpRounding::Zero, InexactReport::Raise, &mut fp), S1_QUIET);
    assert_eq!(fp.cumulative_fpsr(), IOC);
}

#[test]
fn fpcr_controls_and_fpsr_bits_round_trip() {
    for (rmode, rounding) in [
        (0u64, FpRounding::TieEven),
        (1, FpRounding::PosInf),
        (2, FpRounding::NegInf),
        (3, FpRounding::Zero),
    ] {
        let fp = status_with(rmode << 22 | 1 << 26);
        assert_eq!(fp.rounding(), rounding);
    }
    // FPCR.RMode steers ordinary arithmetic.
    let tiny = 0x3080_0000; // 2^-30
    for (rmode, pos, neg) in [
        (0u64, ONE, 0xBF80_0000u32),
        (1, 0x3F80_0001, 0xBF80_0000),
        (2, ONE, 0xBF80_0001),
        (3, ONE, 0xBF80_0000),
    ] {
        let mut fp = status_with(rmode << 22);
        assert_eq!(f32_add(ONE, tiny, &mut fp), pos, "rmode {rmode}");
        assert_eq!(f32_add(0xBF80_0000, tiny | 0x8000_0000, &mut fp), neg, "rmode {rmode}");
    }
    // An exact zero difference is -0 only when rounding toward -infinity.
    let mut fp = status_with(2 << 22);
    assert_eq!(f32_sub(ONE, ONE, &mut fp), NEG_ZERO);
    let mut fp = status();
    assert_eq!(f32_sub(ONE, ONE, &mut fp), POS_ZERO);
    // FPCR.RMode reaches SCVTF through `rounding()`.
    let mut fp = status_with(1 << 22);
    let r = fp.rounding();
    assert_eq!(i32_to_f32(0x0100_0001, r, &mut fp), 0x4B80_0001);
    assert_eq!(fp.cumulative_fpsr(), IXC);

    // Every cumulative bit maps to its own FPSR position.
    let mut fp = status_with(FZ);
    assert_eq!(f32_mul(POS_ZERO, POS_INF, &mut fp), 0x7FC0_0000); // IOC
    assert_eq!(f32_div(ONE, POS_ZERO, &mut fp), POS_INF); // DZC
    assert_eq!(f32_mul(0x7F7F_FFFF, 0x4000_0000, &mut fp), POS_INF); // OFC, IXC
    assert_eq!(f32_mul(0x0080_0000, 0x3F00_0000, &mut fp), POS_ZERO); // UFC (flushed)
    assert_eq!(f32_add(0x0000_0001, POS_ZERO, &mut fp), POS_ZERO); // IDC
    assert_eq!(fp.cumulative_fpsr(), IOC | DZC | OFC | UFC | IXC | IDC);
    assert_eq!(fp.cumulative_fpsr(), 0x9F);
    fp.set_cumulative_fpsr(0);
    assert_eq!(fp.cumulative_fpsr(), 0);
    fp.set_cumulative_fpsr(u64::MAX);
    assert_eq!(fp.cumulative_fpsr(), FPSR_CUMULATIVE_MASK);
    // Changing FPCR keeps the accumulated bits.
    fp.set_fpcr(0);
    assert_eq!(fp.cumulative_fpsr(), FPSR_CUMULATIVE_MASK);
    // A denormal used without FZ reaches no FPSR bit.
    let mut fp = status();
    assert_eq!(f32_mul(0x0000_0001, ONE, &mut fp), 0x0000_0001);
    assert_eq!(fp.cumulative_fpsr(), 0);
}

#[test]
fn half_precision_conversions_round_and_overflow() {
    let mut fp = status();
    assert_eq!(f16_to_f32(0x3C00, &mut fp), ONE);
    assert_eq!(f16_to_f64(0xC000, &mut fp), (-2.0f64).to_bits());
    assert_eq!(f16_to_f64(0x0001, &mut fp), (5.960_464_477_539_063e-8f64).to_bits());
    assert_eq!(f32_to_f16(65504.0f32.to_bits(), &mut fp), 0x7BFF);
    assert_eq!(f64_to_f16(0.5f64.to_bits(), &mut fp), 0x3800);
    assert_eq!(fp.cumulative_fpsr(), 0);
    let mut fp = status();
    assert_eq!(f32_to_f16(65520.0f32.to_bits(), &mut fp), 0x7C00);
    assert_eq!(fp.cumulative_fpsr(), OFC | IXC);
    let mut fp = status();
    assert_eq!(f32_to_f16(0x3F80_1000, &mut fp), 0x3C00); // 1 + 2^-11: tie to even
    assert_eq!(fp.cumulative_fpsr(), IXC);
    let mut fp = status();
    assert_eq!(f64_to_f16((2.0f64).powi(-25).to_bits(), &mut fp), 0); // half the least subnormal
    assert_eq!(fp.cumulative_fpsr(), UFC | IXC);
    // Toward zero, 65520 rounds to the largest finite half (65504): inexact,
    // not an overflow. 65536 rounds past it: overflow to the largest finite.
    let mut fp = status_with(3 << 22);
    assert_eq!(f32_to_f16(65520.0f32.to_bits(), &mut fp), 0x7BFF);
    assert_eq!(fp.cumulative_fpsr(), IXC);
    let mut fp = status_with(3 << 22);
    assert_eq!(f32_to_f16(65536.0f32.to_bits(), &mut fp), 0x7BFF);
    assert_eq!(fp.cumulative_fpsr(), OFC | IXC);
}
