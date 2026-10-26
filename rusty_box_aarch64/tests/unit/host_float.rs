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
