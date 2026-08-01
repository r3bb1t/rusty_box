// Ported wholesale from Berkeley SoftFloat 3e / Bochs softfloat3e: these
// modules carry the complete primitive surface, part of which no x86
// instruction reaches yet. Kept for parity with upstream rather than
// trimmed to current callers.
#![allow(dead_code)]
//! Software IEC/IEEE floating-point types.
//! Ported from Berkeley SoftFloat 3e.

/// 16-bit floating-point (stored as u16)
pub(in crate::cpu) type Float16 = u16;
/// Brain float 16-bit
pub(in crate::cpu) type BFloat16 = u16;
/// 32-bit floating-point (stored as u32)
pub(in crate::cpu) type Float32 = u32;
/// 64-bit floating-point (stored as u64)
pub(in crate::cpu) type Float64 = u64;

/// 128-bit floating-point (stored as u128)
pub(in crate::cpu) type Float128 = u128;

/// 80-bit extended precision float (little-endian layout)
#[cfg(target_endian = "little")]
#[derive(Debug, PartialEq, Eq, Default, Clone, Copy)]
pub(in crate::cpu) struct ExtFloat80 {
    pub(in crate::cpu) signif: u64,
    pub(in crate::cpu) sign_exp: u16,
}

/// 80-bit extended precision float (big-endian layout)
#[cfg(target_endian = "big")]
#[derive(Debug, PartialEq, Eq, Default, Clone, Copy)]
pub(in crate::cpu) struct ExtFloat80 {
    pub(in crate::cpu) sign_exp: u16,
    pub(in crate::cpu) signif: u64,
}

impl ExtFloat80 {
    #[inline]
    pub const fn new(sign_exp: u16, signif: u64) -> Self {
        Self { signif, sign_exp }
    }
}
