//! Software IEC/IEEE floating-point types.
//! Ported from Berkeley SoftFloat 3e softfloat_types.h / Bochs
//! softfloat3e/include/softfloat_types.h: the half, single and double
//! formats, held as their raw bit patterns.

/// 16-bit (half-precision) floating-point bits.
pub(crate) type Float16 = u16;
/// 32-bit (single-precision) floating-point bits.
pub(crate) type Float32 = u32;
/// 64-bit (double-precision) floating-point bits.
pub(crate) type Float64 = u64;
