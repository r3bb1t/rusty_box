#![allow(non_camel_case_types, dead_code)]
//! Software IEC/IEEE floating-point types.
//! Ported from Berkeley SoftFloat 3e.

/// 16-bit floating-point (stored as u16)
pub type float16 = u16;
/// Brain float 16-bit
pub type bfloat16 = u16;
/// 32-bit floating-point (stored as u32)
pub type float32 = u32;
/// 64-bit floating-point (stored as u64)
pub type float64 = u64;

/// 128-bit floating-point (stored as u128)
pub type float128_t = u128;

/// 80-bit extended precision float (little-endian layout)
#[cfg(target_endian = "little")]
#[derive(Debug, PartialEq, Eq, Default, Clone, Copy)]
pub struct extFloat80M {
    pub(crate) signif: u64,
    pub(crate) sign_exp: u16,
}

/// 80-bit extended precision float (big-endian layout)
#[cfg(target_endian = "big")]
#[derive(Debug, PartialEq, Eq, Default, Clone, Copy)]
pub struct extFloat80M {
    pub(crate) sign_exp: u16,
    pub(crate) signif: u64,
}

pub type extFloat80_t = extFloat80M;
pub type floatx80 = extFloat80M;

impl extFloat80M {
    #[inline]
    pub const fn new(sign_exp: u16, signif: u64) -> Self {
        Self { signif, sign_exp }
    }
}
