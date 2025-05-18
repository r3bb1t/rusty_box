#![allow(non_camel_case_types)]
/*----------------------------------------------------------------------------
| Software IEC/IEEE floating-point types.
*----------------------------------------------------------------------------*/

// Software IEC/IEEE floating-point types

/// 16-bit unsigned integer
pub type float16 = u16;
/// 16-bit unsigned integer (same as Float16)
pub type bfloat16 = u16;
/// 32-bit unsigned integer
pub type float32 = u32;
/// 64-bit unsigned integer
pub type float64 = u64;

#[cfg(feature = "bx_little_endian")]
#[derive(Debug, PartialEq, Eq)]
pub struct uint128 {
    pub v0: u64,
    pub v64: u64,
}

#[cfg(feature = "bx_little_endian")]
#[derive(Debug, PartialEq, Eq)]
pub struct uint64_extra {
    pub extra: u64,
    pub v: u64,
}

#[cfg(feature = "bx_little_endian")]
#[derive(Debug, PartialEq, Eq)]
pub struct uint128_extra {
    pub extra: u64,
    pub v: uint128,
}

// Same same ... But different

#[cfg(not(feature = "bx_little_endian"))]
#[derive(Debug, PartialEq, Eq)]
pub struct uint128 {
    pub v64: u64,
    pub v0: u64,
}

#[cfg(not(feature = "bx_little_endian"))]
#[derive(Debug, PartialEq, Eq)]
pub struct uint64_extra {
    pub v: u64,
    pub extra: u64,
}

#[cfg(not(feature = "bx_little_endian"))]
#[derive(Debug, PartialEq, Eq)]
pub struct uint128_extra {
    pub v: uint128,
    pub extra: u64,
}

/*----------------------------------------------------------------------------
| Types used to pass 16-bit, 32-bit, 64-bit, and 128-bit floating-point
| arguments and results to/from functions.  These types must be exactly
| 16 bits, 32 bits, 64 bits, and 128 bits in size, respectively.
*----------------------------------------------------------------------------*/
#[derive(Debug, PartialEq, Eq)]
pub struct f16_t {
    pub v: u16,
}

#[derive(Debug, PartialEq, Eq)]
pub struct f32_t {
    pub v: u32,
}

pub type float128_t = u128;

/*----------------------------------------------------------------------------
| The format of an 80-bit extended floating-point number in memory.  This
| structure must contain a 16-bit field named 'signExp' and a 64-bit field
| named 'signif'.
*----------------------------------------------------------------------------*/

#[cfg(feature = "bx_little_endian")]
#[derive(Debug, PartialEq, Eq)]
pub struct extFloat80M {
    pub signif: u64,
    pub sign_exp: u16,
}

#[cfg(not(feature = "bx_little_endian"))]
#[derive(Debug, PartialEq, Eq)]
pub struct extFloat80M {
    pub sign_exp: u16,
    pub signif: u64,
}

/*----------------------------------------------------------------------------
| The type used to pass 80-bit extended floating-point arguments and
| results to/from functions.  This type must have size identical to
| 'struct extFloat80M'.  Type 'extFloat80_t' can be defined as an alias for
| 'struct extFloat80M'.  Alternatively, if a platform has "native" support
| for IEEE-Standard 80-bit extended floating-point, it may be possible,
| if desired, to define 'extFloat80_t' as an alias for the native type
| (presumably either 'long double' or a nonstandard compiler-intrinsic type).
| In that case, the 'signif' and 'signExp' fields of 'struct extFloat80M'
| must align exactly with the locations in memory of the sign, exponent, and
| significand of the native type.
*----------------------------------------------------------------------------*/
pub type extFloat80_t = extFloat80M;
pub type floatx80 = extFloat80M;
