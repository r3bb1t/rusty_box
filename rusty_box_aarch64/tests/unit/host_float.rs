//! Wherever the host is allowed to compute, it computes exactly what
//! SoftFloat computes. For every operand pair it accepts, under every
//! FPCR.RMode, FZ and DN, with inexact already raised or not, the bits are
//! SoftFloat's and SoftFloat raises nothing new.
//!
//! The operands are random, biased to the boundaries the rule guards:
//!
//! - results near the largest number;
//! - results near the smallest normal, and just below it;
//! - cancellation;
//! - products and quotients that round up to the smallest normal.
//!
//! A fused multiply-add is checked the same way, with addends that cancel
//! the product down to its rounding error, near the smallest normal, and
//! addends at the smallest normal with products below its last bit.
//!
//! The pure SoftFloat routines are the reference. The surface functions
//! in `softfloat/mod.rs` try the host first.

use super::super::softfloat::SoftFloatStatus;
use super::super::{f32_addsub, f32_div, f32_mul, f64_addsub, f64_div, f64_mul, ArmFpStatus};
use super::{f32_arith, f64_arith, Arith};

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

const OPS: [Arith; 4] = [Arith::Add, Arith::Sub, Arith::Mul, Arith::Div];
const FPSR_IXC: u64 = 1 << 4;

/// FPCR with RMode, FZ and DN from the low four bits of `r`.
fn fpcr(r: u64) -> u64 {
    ((r & 3) << 22) | (((r >> 2) & 3) << 24)
}

fn soft64(op: Arith, a: u64, b: u64, s: &mut SoftFloatStatus) -> u64 {
    match op {
        Arith::Add => f64_addsub::f64_add(a, b, s),
        Arith::Sub => f64_addsub::f64_sub(a, b, s),
        Arith::Mul => f64_mul::f64_mul(a, b, s),
        Arith::Div => f64_div::f64_div(a, b, s),
    }
}

fn soft32(op: Arith, a: u32, b: u32, s: &mut SoftFloatStatus) -> u32 {
    match op {
        Arith::Add => f32_addsub::f32_add(a, b, s),
        Arith::Sub => f32_addsub::f32_sub(a, b, s),
        Arith::Mul => f32_mul::f32_mul(a, b, s),
        Arith::Div => f32_div::f32_div(a, b, s),
    }
}

/// A binary64 operand pair for `op`, from one of the biased draws.
fn pair64(rng: &mut SplitMix64, op: Arith) -> [u64; 2] {
    const SIGN: u64 = 1 << 63;
    const MAN: u64 = (1 << 52) - 1;
    let r = rng.next();
    let (x, y) = (rng.next(), rng.next());
    let sign = |v: u64| v & SIGN;
    let number = |v: u64, exp: u64| sign(v) | (exp << 52) | (v & MAN);
    let nudge = |v: u64, by: u64| (v & !SIGN).wrapping_add(by % 7).wrapping_sub(3) | sign(v);
    match r % 6 {
        // Moderate magnitudes.
        0 => [number(x, 1023 - 60 + (r >> 8) % 120), number(y, 1023 - 60 + (r >> 16) % 120)],
        // Near the largest number: sums and products that may overflow.
        1 => match op {
            Arith::Add | Arith::Sub => [number(x, 2045 + (r >> 8) % 2), number(y, 2040 + (r >> 16) % 6)],
            Arith::Mul => {
                let e = 1024 + (r >> 8) % 1021;
                [number(x, e), number(y, (3069 - e).min(2046) - (r >> 24) % 3)]
            }
            Arith::Div => [number(x, 2044 + (r >> 8) % 3), number(y, 1 + (r >> 16) % 4)],
        },
        // Near the smallest normal: products and quotients that may be tiny.
        2 => match op {
            Arith::Add | Arith::Sub => [number(x, 1 + (r >> 8) % 3), number(y, 1 + (r >> 16) % 3)],
            Arith::Mul => {
                let e = 1 + (r >> 8) % 1022;
                [number(x, e), number(y, 1023 + 1 - e + (r >> 24) % 3)]
            }
            Arith::Div => [number(x, 1 + (r >> 8) % 4), number(y, 1022 + (r >> 16) % 6)],
        },
        // A result close to the smallest normal, a few ulps either side:
        // the exact value may be just below it and round up to it.
        3 => {
            let a = number(x, 1023 - 30 + (r >> 8) % 60);
            let min = f64::MIN_POSITIVE;
            let fa = f64::from_bits(a & !SIGN);
            match op {
                Arith::Mul => [a, nudge((min / fa).to_bits(), y) | sign(y)],
                Arith::Div => [nudge((min * fa).to_bits(), y) | sign(y), a],
                Arith::Add => [number(x, 1), nudge(number(x, 1), y) ^ SIGN],
                Arith::Sub => [number(x, 1), nudge(number(x, 1), y)],
            }
        }
        // Cancellation.
        4 => {
            let a = number(x, 1 + (r >> 8) % 2045);
            match op {
                Arith::Add => [a, nudge(a, y) ^ SIGN],
                _ => [a, nudge(a, y)],
            }
        }
        // Any bits.
        _ => [x, y],
    }
}

/// A binary32 operand pair for `op`, from one of the biased draws.
fn pair32(rng: &mut SplitMix64, op: Arith) -> [u32; 2] {
    const SIGN: u32 = 1 << 31;
    const MAN: u32 = (1 << 23) - 1;
    let r = rng.next();
    let (x, y) = (rng.next() as u32, rng.next() as u32);
    let sign = |v: u32| v & SIGN;
    let number = |v: u32, exp: u32| sign(v) | (exp << 23) | (v & MAN);
    let nudge = |v: u32, by: u32| (v & !SIGN).wrapping_add(by % 7).wrapping_sub(3) | sign(v);
    let k = |shift: u32, n: u64| ((r >> shift) % n) as u32;
    match r % 6 {
        0 => [number(x, 127 - 30 + k(8, 60)), number(y, 127 - 30 + k(16, 60))],
        1 => match op {
            Arith::Add | Arith::Sub => [number(x, 253 + k(8, 2)), number(y, 248 + k(16, 6))],
            Arith::Mul => {
                let e = 128 + k(8, 125);
                [number(x, e), number(y, (381 - e).min(254) - k(24, 3))]
            }
            Arith::Div => [number(x, 252 + k(8, 3)), number(y, 1 + k(16, 4))],
        },
        2 => match op {
            Arith::Add | Arith::Sub => [number(x, 1 + k(8, 3)), number(y, 1 + k(16, 3))],
            Arith::Mul => {
                let e = 1 + k(8, 126);
                [number(x, e), number(y, 127 + 1 - e + k(24, 3))]
            }
            Arith::Div => [number(x, 1 + k(8, 4)), number(y, 126 + k(16, 6))],
        },
        3 => {
            let a = number(x, 127 - 15 + k(8, 30));
            let min = f32::MIN_POSITIVE;
            let fa = f32::from_bits(a & !SIGN);
            match op {
                Arith::Mul => [a, nudge((min / fa).to_bits(), y) | sign(y)],
                Arith::Div => [nudge((min * fa).to_bits(), y) | sign(y), a],
                Arith::Add => [number(x, 1), nudge(number(x, 1), y) ^ SIGN],
                Arith::Sub => [number(x, 1), nudge(number(x, 1), y)],
            }
        }
        4 => {
            let a = number(x, 1 + k(8, 253));
            match op {
                Arith::Add => [a, nudge(a, y) ^ SIGN],
                _ => [a, nudge(a, y)],
            }
        }
        _ => [x, y],
    }
}

/// How often the host answered, overall and with a result below twice
/// the smallest normal, and how often SoftFloat underflowed from normal
/// operands under the host's own conditions.
#[derive(Debug, Default)]
struct Reach {
    accepted: u32,
    near_smallest_normal: u32,
    underflowed: u32,
}

#[test]
fn the_host_computes_what_softfloat_computes_wherever_it_answers() {
    let mut rng = SplitMix64(0x4A2D_F00D_0001_2345);
    let mut reach64 = Reach::default();
    let mut reach32 = Reach::default();
    for i in 0..300_000u32 {
        let op = OPS[i as usize % OPS.len()];
        let draw = rng.next();
        // The host's own conditions (to nearest, inexact raised), and a
        // random FPCR and FPSR.IXC.
        let statuses = [
            ArmFpStatus::with_fpsr(fpcr(draw & 0b1100), FPSR_IXC),
            ArmFpStatus::with_fpsr(fpcr(draw >> 4), if draw & (1 << 8) != 0 { FPSR_IXC } else { 0 }),
        ];

        let [a, b] = pair64(&mut rng, op);
        for (n, fp) in statuses.iter().enumerate() {
            let mut soft = *fp;
            let want = soft64(op, a, b, &mut soft.status);
            match f64_arith(op, a, b, fp) {
                Some(got) => {
                    assert_eq!(got, want, "f64 {op:?} {a:#018x} {b:#018x} under {fp:?}");
                    assert_eq!(soft, *fp, "f64 {op:?} {a:#018x} {b:#018x}: SoftFloat raised more");
                    reach64.accepted += 1;
                    if want & !(1 << 63) < 2 * f64::MIN_POSITIVE.to_bits() {
                        reach64.near_smallest_normal += 1;
                    }
                }
                None => {
                    let normal = |v: u64| (1..0x7FF).contains(&((v >> 52) & 0x7FF));
                    if n == 0 && normal(a) && normal(b) && soft.cumulative_fpsr() & (1 << 3) != 0 {
                        reach64.underflowed += 1;
                    }
                }
            }
        }

        let [a, b] = pair32(&mut rng, op);
        for (n, fp) in statuses.iter().enumerate() {
            let mut soft = *fp;
            let want = soft32(op, a, b, &mut soft.status);
            match f32_arith(op, a, b, fp) {
                Some(got) => {
                    assert_eq!(got, want, "f32 {op:?} {a:#010x} {b:#010x} under {fp:?}");
                    assert_eq!(soft, *fp, "f32 {op:?} {a:#010x} {b:#010x}: SoftFloat raised more");
                    reach32.accepted += 1;
                    if want & !(1 << 31) < 2 * f32::MIN_POSITIVE.to_bits() {
                        reach32.near_smallest_normal += 1;
                    }
                }
                None => {
                    let normal = |v: u32| (1..0xFF).contains(&((v >> 23) & 0xFF));
                    if n == 0 && normal(a) && normal(b) && soft.cumulative_fpsr() & (1 << 3) != 0 {
                        reach32.underflowed += 1;
                    }
                }
            }
        }
    }
    // The host answered often, right up to the smallest normal, and the
    // draws reached the results it must leave to SoftFloat.
    for (name, reach) in [("f64", &reach64), ("f32", &reach32)] {
        assert!(reach.accepted >= 150_000, "{name}: {reach:?}");
        assert!(reach.near_smallest_normal >= 1_000, "{name}: {reach:?}");
        assert!(reach.underflowed >= 1_000, "{name}: {reach:?}");
    }
}

/// The fused multiply-add, which the host computes only with `std`, and
/// only where its `mul_add` rounds once (`HOST_FUSES_ONCE`).
#[cfg(feature = "std")]
mod fused {
    use super::super::super::{f32_mul_add, f64_mul_add, ArmFpStatus, FusedMulAdd};
    use super::super::{f32_fused, f64_fused, HOST_FUSES_ONCE};
    use super::{fpcr, Reach, SplitMix64, FPSR_IXC};

    /// A binary64 multiply-add `[addend, n, m]`.
    ///
    /// On half the draws the product of normal `n` and `m` is anywhere,
    /// and on one in four of those one operand is then made a subnormal, a
    /// zero, an infinity or a NaN. On a quarter the product is near
    /// `2^-970`, where its rounding error is near the smallest normal. On a
    /// quarter it is near `2^-1075`, below the smallest normal's last bit,
    /// and the addend is the smallest normal or a few ulps above it, of
    /// either sign, so that the exact sum may lie just below the smallest
    /// normal and round up to it. On the other draws the addend is any
    /// normal number, or, when `cancel` asks for it, the host's rounded
    /// `-(n*m)` moved by -4 to 3 ulps, which leaves the product's rounding
    /// error and those ulps.
    fn triple64(rng: &mut SplitMix64, cancel: bool) -> [u64; 3] {
        const SIGN: u64 = 1 << 63;
        const MAN: u64 = (1 << 52) - 1;
        const EXP: u64 = 0x7FF << 52;
        let r = rng.next();
        let (x, y, z) = (rng.next(), rng.next(), rng.next());
        let number = |v: u64, exp: u64| (v & SIGN) | (exp << 52) | (v & MAN);
        // `n` and `m` whose biased exponents add up to `sum`: a product from
        // `2^(sum - 2046)` to just under four times that.
        let multiplicands = |sum: u64| {
            let lo = sum.saturating_sub(2046).max(1);
            let hi = (sum - 1).min(2046);
            let en = lo + (r >> 8) % (hi - lo + 1);
            [number(y, en), number(z, sum - en)]
        };
        let [n, m] = match r % 8 {
            0..=3 => [
                number(y, 1 + (r >> 8) % 2046),
                number(z, 1 + (r >> 20) % 2046),
            ],
            4 | 5 => multiplicands(1072 + (r >> 32) % 8),
            _ => {
                let [n, m] = multiplicands(969 + (r >> 32) % 8);
                return [(x & SIGN) | (1 << 52) | ((x >> 8) % 4), n, m];
            }
        };
        let addend = if cancel {
            let product = (-(f64::from_bits(n) * f64::from_bits(m))).to_bits();
            ((product & !SIGN).wrapping_add(x & 7).wrapping_sub(4)) | (product & SIGN)
        } else {
            number(x, 1 + (x >> 52) % 2046)
        };
        let mut triple = [addend, n, m];
        if r % 8 == 3 {
            // The operand keeps its sign, and its fraction where it has one.
            let v = triple[((r >> 40) % 3) as usize];
            triple[((r >> 40) % 3) as usize] = match (r >> 44) % 4 {
                0 => (v & SIGN) | (v & MAN).max(1),
                1 => v & SIGN,
                2 => (v & SIGN) | EXP,
                _ => (v & SIGN) | EXP | (v & MAN).max(1),
            };
        }
        triple
    }

    /// A binary32 multiply-add `[addend, n, m]`, drawn as [`triple64`]
    /// draws one: products anywhere, near `2^-103` and near `2^-150`.
    fn triple32(rng: &mut SplitMix64, cancel: bool) -> [u32; 3] {
        const SIGN: u32 = 1 << 31;
        const MAN: u32 = (1 << 23) - 1;
        const EXP: u32 = 0xFF << 23;
        let r = rng.next();
        let (x, y, z) = (rng.next(), rng.next() as u32, rng.next() as u32);
        let number = |v: u32, exp: u64| (v & SIGN) | ((exp as u32) << 23) | (v & MAN);
        let multiplicands = |sum: u64| {
            let lo = sum.saturating_sub(254).max(1);
            let hi = (sum - 1).min(254);
            let en = lo + (r >> 8) % (hi - lo + 1);
            [number(y, en), number(z, sum - en)]
        };
        let [n, m] = match r % 8 {
            0..=3 => [
                number(y, 1 + (r >> 8) % 254),
                number(z, 1 + (r >> 20) % 254),
            ],
            4 | 5 => multiplicands(147 + (r >> 32) % 8),
            _ => {
                let [n, m] = multiplicands(102 + (r >> 32) % 8);
                return [(x as u32 & SIGN) | (1 << 23) | ((x >> 8) % 4) as u32, n, m];
            }
        };
        let addend = if cancel {
            let product = (-(f32::from_bits(n) * f32::from_bits(m))).to_bits();
            ((product & !SIGN).wrapping_add(x as u32 & 7).wrapping_sub(4)) | (product & SIGN)
        } else {
            number(x as u32, 1 + (x >> 32) % 254)
        };
        let mut triple = [addend, n, m];
        if r % 8 == 3 {
            let v = triple[((r >> 40) % 3) as usize];
            triple[((r >> 40) % 3) as usize] = match (r >> 44) % 4 {
                0 => (v & SIGN) | (v & MAN).max(1),
                1 => v & SIGN,
                2 => (v & SIGN) | EXP,
                _ => (v & SIGN) | EXP | (v & MAN).max(1),
            };
        }
        triple
    }

    #[test]
    fn the_host_fuses_what_softfloat_fuses_wherever_it_answers() {
        let mut rng = SplitMix64(0x0F05_ED00_0003_A11D);
        let mut reach64 = Reach::default();
        let mut reach32 = Reach::default();
        for i in 0..300_000u32 {
            // Even draws add any addend, odd ones cancel the product.
            let cancel = i % 2 == 1;
            let draw = rng.next();
            let statuses = [
                ArmFpStatus::with_fpsr(fpcr(draw & 0b1100), FPSR_IXC),
                ArmFpStatus::with_fpsr(
                    fpcr(draw >> 4),
                    if draw & (1 << 8) != 0 { FPSR_IXC } else { 0 },
                ),
            ];

            let [addend, n, m] = triple64(&mut rng, cancel);
            for (k, fp) in statuses.iter().enumerate() {
                let mut soft = *fp;
                let want = f64_mul_add::f64_mul_add(n, m, addend, &mut soft.status);
                match f64_fused(addend, n, m, fp) {
                    Some(got) => {
                        assert_eq!(
                            got, want,
                            "f64 {addend:#018x} + {n:#018x} * {m:#018x} under {fp:?}"
                        );
                        assert_eq!(
                            soft, *fp,
                            "f64 {addend:#018x} + {n:#018x} * {m:#018x}: SoftFloat raised more"
                        );
                        reach64.accepted += 1;
                        if want & !(1 << 63) < 2 * f64::MIN_POSITIVE.to_bits() {
                            reach64.near_smallest_normal += 1;
                        }
                    }
                    None => {
                        let normal = |v: u64| (1..0x7FF).contains(&((v >> 52) & 0x7FF));
                        let operands_normal = normal(addend) && normal(n) && normal(m);
                        if k == 0 && operands_normal && soft.cumulative_fpsr() & (1 << 3) != 0 {
                            reach64.underflowed += 1;
                        }
                    }
                }
            }

            let [addend, n, m] = triple32(&mut rng, cancel);
            for (k, fp) in statuses.iter().enumerate() {
                let mut soft = *fp;
                let want = f32_mul_add::f32_mul_add(n, m, addend, &mut soft.status);
                match f32_fused(addend, n, m, fp) {
                    Some(got) => {
                        assert_eq!(
                            got, want,
                            "f32 {addend:#010x} + {n:#010x} * {m:#010x} under {fp:?}"
                        );
                        assert_eq!(
                            soft, *fp,
                            "f32 {addend:#010x} + {n:#010x} * {m:#010x}: SoftFloat raised more"
                        );
                        reach32.accepted += 1;
                        if want & !(1 << 31) < 2 * f32::MIN_POSITIVE.to_bits() {
                            reach32.near_smallest_normal += 1;
                        }
                    }
                    None => {
                        let normal = |v: u32| (1..0xFF).contains(&((v >> 23) & 0xFF));
                        let operands_normal = normal(addend) && normal(n) && normal(m);
                        if k == 0 && operands_normal && soft.cumulative_fpsr() & (1 << 3) != 0 {
                            reach32.underflowed += 1;
                        }
                    }
                }
            }
        }
        // Where the host fuses, it answered often, right up to the smallest
        // normal, and the draws reached the results it must leave to
        // SoftFloat. Elsewhere it never answered.
        for (name, reach) in [("f64", &reach64), ("f32", &reach32)] {
            if HOST_FUSES_ONCE {
                assert!(reach.accepted >= 100_000, "{name}: {reach:?}");
                assert!(reach.near_smallest_normal >= 1_000, "{name}: {reach:?}");
            } else {
                assert_eq!(reach.accepted, 0, "{name}: {reach:?}");
            }
            assert!(reach.underflowed >= 1_000, "{name}: {reach:?}");
        }
    }

    /// FMSUB, FNMADD and FNMSUB negate their operands before the host sees
    /// them. With inexact already raised and every result normal and
    /// inexact, each form gives the bits and the FPSR SoftFloat gives for
    /// the negated operands, and the host answers for those operands.
    #[test]
    fn the_host_fuses_after_the_forms_negations() {
        // (form, Ra negated, Rn negated), as Arm FPMulAdd's callers negate.
        const FORMS: [(FusedMulAdd, bool, bool); 4] = [
            (FusedMulAdd::Fmadd, false, false),
            (FusedMulAdd::Fmsub, false, true),
            (FusedMulAdd::Fnmadd, true, true),
            (FusedMulAdd::Fnmsub, true, false),
        ];
        let fp = ArmFpStatus::with_fpsr(fpcr(0), FPSR_IXC);
        for (form, negate_addend, negate_n) in FORMS {
            // 1.5 + 0.1 * 0.3, with the form's signs.
            let [addend, n, m] = [1.5f64.to_bits(), 0.1f64.to_bits(), 0.3f64.to_bits()];
            let a = if negate_addend {
                addend ^ (1 << 63)
            } else {
                addend
            };
            let x = if negate_n { n ^ (1 << 63) } else { n };
            let mut soft = fp;
            let want = f64_mul_add::f64_mul_add(x, m, a, &mut soft.status);
            let mut surface = fp;
            assert_eq!(
                f64_mul_add(form, addend, n, m, &mut surface),
                want,
                "f64 {form:?}"
            );
            assert_eq!(surface, soft, "f64 {form:?}");
            assert_eq!(
                f64_fused(a, x, m, &fp).is_some(),
                HOST_FUSES_ONCE,
                "f64 {form:?}"
            );

            let [addend, n, m] = [1.5f32.to_bits(), 0.1f32.to_bits(), 0.3f32.to_bits()];
            let a = if negate_addend {
                addend ^ (1 << 31)
            } else {
                addend
            };
            let x = if negate_n { n ^ (1 << 31) } else { n };
            let mut soft = fp;
            let want = f32_mul_add::f32_mul_add(x, m, a, &mut soft.status);
            let mut surface = fp;
            assert_eq!(
                f32_mul_add(form, addend, n, m, &mut surface),
                want,
                "f32 {form:?}"
            );
            assert_eq!(surface, soft, "f32 {form:?}");
            assert_eq!(
                f32_fused(a, x, m, &fp).is_some(),
                HOST_FUSES_ONCE,
                "f32 {form:?}"
            );
        }
    }
}
