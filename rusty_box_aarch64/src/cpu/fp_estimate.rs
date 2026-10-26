//! The reciprocal and reciprocal-square-root estimates (FRECPE, FRSQRTE,
//! URECPE, URSQRTE) and FRECPX. Their results are defined bit for bit by
//! table-free integer algorithms in the Arm ARM (`RecipEstimate`,
//! `RecipSqrtEstimate`, `FPRecipEstimate`, `FPRSqrtEstimate`, `FPRecpX`,
//! `UnsignedRecipEstimate`, `UnsignedRSqrtEstimate`); this follows them, in
//! the integer form QEMU uses (`target/arm/tcg/vfp_helper.c`:
//! `recip_estimate`, `do_recip_sqrt_estimate`, `call_recip_estimate`,
//! `recip_sqrt_estimate`, `HELPER(recpe_f32/f64)`, `HELPER(rsqrte_f32/f64)`,
//! `HELPER(recpe_u32)`, `HELPER(rsqrte_u32)`, `HELPER(frecpx_f32/f64)`).
//!
//! FEAT_RPRES (the increased-precision single estimates) is not part of
//! ARMv8.0 and is not modelled.

/// FPSR cumulative exception bits these operations can raise.
pub(crate) mod fpsr {
    pub(crate) const IOC: u64 = 1 << 0;
    pub(crate) const DZC: u64 = 1 << 1;
    pub(crate) const OFC: u64 = 1 << 2;
    pub(crate) const UFC: u64 = 1 << 3;
    pub(crate) const IXC: u64 = 1 << 4;
    pub(crate) const IDC: u64 = 1 << 7;
}

/// The FPCR fields an estimate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EstimateControl {
    /// `FPCR.RMode`: 0 nearest, 1 toward +infinity, 2 toward -infinity,
    /// 3 toward zero.
    pub(crate) rmode: u64,
    /// `FPCR.FZ`, for single and double precision.
    pub(crate) flush_to_zero: bool,
    /// `FPCR.FZ16`, for half precision (FEAT_FP16).
    pub(crate) flush_half: bool,
    /// `FPCR.DN`.
    pub(crate) default_nan: bool,
}

impl EstimateControl {
    pub(crate) const fn from_fpcr(fpcr: u64) -> Self {
        Self {
            rmode: (fpcr >> 22) & 3,
            flush_to_zero: (fpcr >> 24) & 1 == 1,
            flush_half: (fpcr >> 19) & 1 == 1,
            default_nan: (fpcr >> 25) & 1 == 1,
        }
    }

    /// Whether denormals of format `f` flush to zero: FPCR.FZ16 for half
    /// precision, FPCR.FZ otherwise (Arm FPUnpackBase, FPRecipEstimate).
    const fn flushes(self, f: Format) -> bool {
        if f.is_half() {
            self.flush_half
        } else {
            self.flush_to_zero
        }
    }

    /// Whether an overflowing result of sign `negative` rounds to infinity
    /// (otherwise to the largest finite value).
    const fn rounds_to_infinity(self, negative: bool) -> bool {
        match self.rmode {
            0 => true,
            1 => !negative,
            2 => negative,
            _ => false,
        }
    }
}

/// An estimate's result and the FPSR exception bits it raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Estimate {
    pub(crate) bits: u64,
    pub(crate) fpsr: u64,
}

/// The shape of one IEEE binary format.
#[derive(Clone, Copy)]
pub(crate) struct Format {
    exp_bits: u32,
    frac_bits: u32,
}

pub(crate) const HALF: Format = Format {
    exp_bits: 5,
    frac_bits: 10,
};
pub(crate) const SINGLE: Format = Format {
    exp_bits: 8,
    frac_bits: 23,
};
pub(crate) const DOUBLE: Format = Format {
    exp_bits: 11,
    frac_bits: 52,
};

impl Format {
    const fn is_half(self) -> bool {
        self.exp_bits == 5
    }
    const fn sign_bit(self) -> u64 {
        1 << (self.exp_bits + self.frac_bits)
    }
    const fn exp_max(self) -> u64 {
        (1 << self.exp_bits) - 1
    }
    const fn frac_mask(self) -> u64 {
        (1 << self.frac_bits) - 1
    }
    const fn exponent(self, bits: u64) -> u64 {
        (bits >> self.frac_bits) & self.exp_max()
    }
    const fn fraction(self, bits: u64) -> u64 {
        bits & self.frac_mask()
    }
    const fn is_nan(self, bits: u64) -> bool {
        self.exponent(bits) == self.exp_max() && self.fraction(bits) != 0
    }
    const fn is_signalling_nan(self, bits: u64) -> bool {
        self.is_nan(bits) && (bits >> (self.frac_bits - 1)) & 1 == 0
    }
    const fn is_infinity(self, bits: u64) -> bool {
        self.exponent(bits) == self.exp_max() && self.fraction(bits) == 0
    }
    const fn is_zero(self, bits: u64) -> bool {
        bits & !self.sign_bit() == 0
    }
    const fn infinity(self, negative: bool) -> u64 {
        self.signed(negative, self.exp_max() << self.frac_bits)
    }
    const fn max_normal(self, negative: bool) -> u64 {
        self.signed(negative, ((self.exp_max() - 1) << self.frac_bits) | self.frac_mask())
    }
    const fn zero(self, negative: bool) -> u64 {
        self.signed(negative, 0)
    }
    const fn default_nan(self) -> u64 {
        (self.exp_max() << self.frac_bits) | (1 << (self.frac_bits - 1))
    }
    const fn signed(self, negative: bool, magnitude: u64) -> u64 {
        if negative {
            magnitude | self.sign_bit()
        } else {
            magnitude
        }
    }
}

/// Arm ARM `FPProcessNaN`: quieten a signalling NaN (raising Invalid
/// Operation), or produce the default NaN under `FPCR.DN`.
fn process_nan(f: Format, bits: u64, ctl: EstimateControl) -> Estimate {
    let fpsr = if f.is_signalling_nan(bits) { fpsr::IOC } else { 0 };
    let quiet = bits | (1 << (f.frac_bits - 1));
    Estimate {
        bits: if ctl.default_nan { f.default_nan() } else { quiet },
        fpsr,
    }
}

/// Input flushing: under `FPCR.FZ` (`FPCR.FZ16` for half precision) a
/// denormal input becomes a zero of its sign, raising Input Denormal for
/// single and double precision only (Arm FPUnpackBase).
fn flush_input(f: Format, bits: u64, ctl: EstimateControl, raised: &mut u64) -> u64 {
    if ctl.flushes(f) && f.exponent(bits) == 0 && f.fraction(bits) != 0 {
        if !f.is_half() {
            *raised |= fpsr::IDC;
        }
        bits & f.sign_bit()
    } else {
        bits
    }
}

/// Arm ARM `RecipEstimate` on a 9-bit fixed-point input in 256..512
/// (0.5 <= x < 1.0); the result is in 256..512 (1.0 <= r < 2.0).
const fn recip_estimate(input: u64) -> u64 {
    let a = input * 2 + 1;
    let b = (1 << 19) / a;
    (b + 1) >> 1
}

/// Arm ARM `RecipSqrtEstimate` on a 9-bit fixed-point input in 128..512
/// (0.25 <= x < 1.0); the result is in 256..512.
const fn recip_sqrt_estimate(input: u64) -> u64 {
    let a = if input < 256 {
        input * 2 + 1
    } else {
        ((input >> 1) << 1) * 2 + 2
    };
    let mut b = 512;
    while a * (b + 1) * (b + 1) < (1 << 28) {
        b += 1;
    }
    b.div_ceil(2)
}

/// FRECPE (floating-point): Arm ARM `FPRecipEstimate`.
pub(crate) fn frecpe(f: Format, operand: u64, ctl: EstimateControl) -> Estimate {
    let mut raised = 0;
    let bits = flush_input(f, operand, ctl, &mut raised);
    let negative = bits & f.sign_bit() != 0;
    let done = |bits: u64, more: u64| Estimate {
        bits,
        fpsr: raised | more,
    };
    if f.is_nan(bits) {
        return process_nan(f, bits, ctl);
    }
    if f.is_infinity(bits) {
        return done(f.zero(negative), 0);
    }
    if f.is_zero(bits) {
        return done(f.infinity(negative), fpsr::DZC);
    }
    // Abs(value) < 2^-(bias+2): the reciprocal overflows.
    if bits & !f.sign_bit() < 1 << (f.frac_bits - 2) {
        let result = if ctl.rounds_to_infinity(negative) {
            f.infinity(negative)
        } else {
            f.max_normal(negative)
        };
        return done(result, fpsr::OFC | fpsr::IXC);
    }
    // Abs(value) >= 2^(bias-1) under flush-to-zero: the reciprocal is tiny.
    let bias = f.exp_max() >> 1;
    if ctl.flushes(f) && f.exponent(bits) >= 2 * bias - 1 {
        return done(f.zero(negative), fpsr::UFC);
    }

    // Work on a double-shaped fraction with the binary point at bit 52.
    let mut exp = f.exponent(bits) as i64;
    let mut frac = f.fraction(bits) << (52 - f.frac_bits);
    if exp == 0 {
        if (frac >> 51) & 1 == 0 {
            exp = -1;
            frac <<= 2;
        } else {
            frac <<= 1;
        }
    }
    let scaled = (1 << 8) | ((frac >> 44) & 0xFF);
    let estimate = recip_estimate(scaled);
    let mut result_exp = 2 * bias as i64 - 1 - exp;
    let mut result_frac = (estimate & 0xFF) << 44;
    if result_exp == 0 {
        result_frac = (result_frac >> 1) | (1 << 51);
    } else if result_exp == -1 {
        result_frac = (result_frac >> 2) | (1 << 50);
        result_exp = 0;
    }
    let magnitude = ((result_exp as u64) << f.frac_bits) | (result_frac >> (52 - f.frac_bits));
    done(f.signed(negative, magnitude), 0)
}

/// FRSQRTE (floating-point): Arm ARM `FPRSqrtEstimate`.
pub(crate) fn frsqrte(f: Format, operand: u64, ctl: EstimateControl) -> Estimate {
    let mut raised = 0;
    let bits = flush_input(f, operand, ctl, &mut raised);
    let negative = bits & f.sign_bit() != 0;
    let done = |bits: u64, more: u64| Estimate {
        bits,
        fpsr: raised | more,
    };
    if f.is_nan(bits) {
        return process_nan(f, bits, ctl);
    }
    if f.is_zero(bits) {
        return done(f.infinity(negative), fpsr::DZC);
    }
    if negative {
        return done(f.default_nan(), fpsr::IOC);
    }
    if f.is_infinity(bits) {
        return done(f.zero(false), 0);
    }

    let mut exp = f.exponent(bits) as i64;
    let mut frac = f.fraction(bits) << (52 - f.frac_bits);
    if exp == 0 {
        while (frac >> 51) & 1 == 0 {
            frac <<= 1;
            exp -= 1;
        }
        frac = (frac & ((1 << 51) - 1)) << 1;
    }
    let scaled = if exp & 1 != 0 {
        (1 << 7) | ((frac >> 45) & 0x7F)
    } else {
        (1 << 8) | ((frac >> 44) & 0xFF)
    };
    let estimate = recip_sqrt_estimate(scaled);
    let bias = (f.exp_max() >> 1) as i64;
    let result_exp = (3 * bias - 1 - exp) / 2;
    let magnitude = ((result_exp as u64) << f.frac_bits) | ((estimate & 0xFF) << (f.frac_bits - 8));
    done(magnitude, 0)
}

/// FRECPX: Arm ARM `FPRecpX` — the exponent inverted, the fraction cleared.
pub(crate) fn frecpx(f: Format, operand: u64, ctl: EstimateControl) -> Estimate {
    if f.is_nan(operand) {
        return process_nan(f, operand, ctl);
    }
    let mut raised = 0;
    let bits = flush_input(f, operand, ctl, &mut raised);
    let exp = f.exponent(bits);
    let result_exp = if exp == 0 {
        f.exp_max() - 1
    } else {
        !exp & f.exp_max()
    };
    Estimate {
        bits: (bits & f.sign_bit()) | (result_exp << f.frac_bits),
        fpsr: raised,
    }
}

/// URECPE: Arm ARM `UnsignedRecipEstimate` on a 32-bit element.
pub(crate) const fn urecpe(operand: u32) -> u32 {
    if operand & 0x8000_0000 == 0 {
        return u32::MAX;
    }
    (recip_estimate(((operand >> 23) & 0x1FF) as u64) as u32) << 23
}

/// URSQRTE: Arm ARM `UnsignedRSqrtEstimate` on a 32-bit element.
pub(crate) const fn ursqrte(operand: u32) -> u32 {
    if operand & 0xC000_0000 == 0 {
        return u32::MAX;
    }
    (recip_sqrt_estimate(((operand >> 23) & 0x1FF) as u64) as u32) << 23
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEAREST: EstimateControl = EstimateControl::from_fpcr(0);

    #[test]
    fn reciprocal_estimate_of_two_is_about_a_half() {
        // FRECPE(2.0) = 0.4990234375 (0x3EFF8000): 8 significant bits.
        let e = frecpe(SINGLE, 0x4000_0000, NEAREST);
        assert_eq!(e.bits, 0x3EFF_8000);
        assert_eq!(e.fpsr, 0);
        let e = frecpe(DOUBLE, 0x4000_0000_0000_0000, NEAREST);
        assert_eq!(e.bits, 0x3FDF_F000_0000_0000);
    }

    #[test]
    fn reciprocal_sqrt_estimate_of_four_is_about_a_half() {
        // FRSQRTE(4.0) = 0.4990234375 (0x3EFF8000), as Unicorn computes it.
        let e = frsqrte(SINGLE, 0x4080_0000, NEAREST);
        assert_eq!(e.bits, 0x3EFF_8000);
        // Negative inputs are invalid.
        let e = frsqrte(SINGLE, 0xC080_0000, NEAREST);
        assert_eq!((e.bits, e.fpsr), (0x7FC0_0000, fpsr::IOC));
    }

    #[test]
    fn special_inputs_follow_the_pseudocode() {
        assert_eq!(frecpe(SINGLE, 0x8000_0000, NEAREST), Estimate { bits: 0xFF80_0000, fpsr: fpsr::DZC });
        assert_eq!(frecpe(SINGLE, 0x7F80_0000, NEAREST).bits, 0);
        // The smallest denormal overflows: infinity when rounding to
        // nearest, the largest normal when rounding toward zero.
        assert_eq!(frecpe(SINGLE, 1, NEAREST), Estimate { bits: 0x7F80_0000, fpsr: fpsr::OFC | fpsr::IXC });
        let rz = EstimateControl::from_fpcr(3 << 22);
        assert_eq!(frecpe(SINGLE, 1, rz).bits, 0x7F7F_FFFF);
        // Under FZ the denormal is a zero first.
        let fz = EstimateControl::from_fpcr(1 << 24);
        assert_eq!(frecpe(SINGLE, 1, fz), Estimate { bits: 0x7F80_0000, fpsr: fpsr::IDC | fpsr::DZC });
        // A signalling NaN is quietened, or replaced under DN.
        assert_eq!(frecpe(SINGLE, 0x7F80_0001, NEAREST), Estimate { bits: 0x7FC0_0001, fpsr: fpsr::IOC });
        let dn = EstimateControl::from_fpcr(1 << 25);
        assert_eq!(frecpe(SINGLE, 0x7F80_0001, dn).bits, 0x7FC0_0000);
    }

    #[test]
    fn frecpx_inverts_the_exponent() {
        // 2.0 (exponent 0x80) -> exponent 0x7F with the fraction cleared.
        assert_eq!(frecpx(SINGLE, 0x4012_3456, NEAREST).bits, 0x3F80_0000);
        // Zero and denormals take the largest finite exponent.
        assert_eq!(frecpx(SINGLE, 0x8000_0001, NEAREST).bits, 0xFF00_0000);
    }

    #[test]
    fn unsigned_estimates_saturate_small_inputs() {
        assert_eq!(urecpe(0x7FFF_FFFF), u32::MAX);
        assert_eq!(urecpe(0x8000_0000), 0xFF80_0000);
        assert_eq!(ursqrte(0x3FFF_FFFF), u32::MAX);
        assert_eq!(ursqrte(0x4000_0000), 0xFF80_0000);
    }
}
