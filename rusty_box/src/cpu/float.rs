pub(crate) trait FloatExt: Sized {
    fn sqrt(self) -> Self;
    fn floor(self) -> Self;
    fn ceil(self) -> Self;
    fn trunc(self) -> Self;
    fn round_ties_even(self) -> Self;
    fn mul_add(self, a: Self, b: Self) -> Self;
    fn powi(self, n: i32) -> Self;
}

impl FloatExt for f32 {
    #[inline]
    fn sqrt(self) -> Self {
        #[cfg(feature = "std")]
        {
            f32::sqrt(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::sqrtf(self)
        }
    }

    #[inline]
    fn floor(self) -> Self {
        #[cfg(feature = "std")]
        {
            f32::floor(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::floorf(self)
        }
    }

    #[inline]
    fn ceil(self) -> Self {
        #[cfg(feature = "std")]
        {
            f32::ceil(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::ceilf(self)
        }
    }

    #[inline]
    fn trunc(self) -> Self {
        #[cfg(feature = "std")]
        {
            f32::trunc(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::truncf(self)
        }
    }

    #[inline]
    fn round_ties_even(self) -> Self {
        #[cfg(feature = "std")]
        {
            f32::round_ties_even(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::roundevenf(self)
        }
    }

    #[inline]
    fn mul_add(self, a: Self, b: Self) -> Self {
        #[cfg(feature = "std")]
        {
            f32::mul_add(self, a, b)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::fmaf(self, a, b)
        }
    }

    #[inline]
    fn powi(self, n: i32) -> Self {
        #[cfg(feature = "std")]
        {
            f32::powi(self, n)
        }
        #[cfg(not(feature = "std"))]
        {
            powi_f32(self, n)
        }
    }
}

impl FloatExt for f64 {
    #[inline]
    fn sqrt(self) -> Self {
        #[cfg(feature = "std")]
        {
            f64::sqrt(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::sqrt(self)
        }
    }

    #[inline]
    fn floor(self) -> Self {
        #[cfg(feature = "std")]
        {
            f64::floor(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::floor(self)
        }
    }

    #[inline]
    fn ceil(self) -> Self {
        #[cfg(feature = "std")]
        {
            f64::ceil(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::ceil(self)
        }
    }

    #[inline]
    fn trunc(self) -> Self {
        #[cfg(feature = "std")]
        {
            f64::trunc(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::trunc(self)
        }
    }

    #[inline]
    fn round_ties_even(self) -> Self {
        #[cfg(feature = "std")]
        {
            f64::round_ties_even(self)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::roundeven(self)
        }
    }

    #[inline]
    fn mul_add(self, a: Self, b: Self) -> Self {
        #[cfg(feature = "std")]
        {
            f64::mul_add(self, a, b)
        }
        #[cfg(not(feature = "std"))]
        {
            libm::fma(self, a, b)
        }
    }

    #[inline]
    fn powi(self, n: i32) -> Self {
        #[cfg(feature = "std")]
        {
            f64::powi(self, n)
        }
        #[cfg(not(feature = "std"))]
        {
            powi_f64(self, n)
        }
    }
}

#[cfg(not(feature = "std"))]
#[inline]
fn powi_f32(base: f32, exponent: i32) -> f32 {
    powi_by_squaring_f32(base, exponent)
}

#[cfg(not(feature = "std"))]
#[inline]
fn powi_f64(base: f64, exponent: i32) -> f64 {
    powi_by_squaring_f64(base, exponent)
}

#[cfg(not(feature = "std"))]
fn powi_by_squaring_f32(mut base: f32, exponent: i32) -> f32 {
    let negative = exponent < 0;
    let mut exp = exponent.unsigned_abs();
    let mut acc = 1.0f32;
    while exp != 0 {
        if exp & 1 != 0 {
            acc *= base;
        }
        base *= base;
        exp >>= 1;
    }
    if negative {
        1.0 / acc
    } else {
        acc
    }
}

#[cfg(not(feature = "std"))]
fn powi_by_squaring_f64(mut base: f64, exponent: i32) -> f64 {
    let negative = exponent < 0;
    let mut exp = exponent.unsigned_abs();
    let mut acc = 1.0f64;
    while exp != 0 {
        if exp & 1 != 0 {
            acc *= base;
        }
        base *= base;
        exp >>= 1;
    }
    if negative {
        1.0 / acc
    } else {
        acc
    }
}
