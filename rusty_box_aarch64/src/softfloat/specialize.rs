//! Arm specialization: default NaNs, NaN propagation, conversion responses,
//! tininess detection and operand flushing.
//!
//! Ported from Berkeley SoftFloat 3e `source/ARM-VFPv2/specialize.h`,
//! `s_propagateNaNF32UI.c` and `s_propagateNaNF64UI.c` (with the
//! ARM-VFPv2-defaultNaN variant folded in behind FPCR.DN), extended with the
//! ARMv8 pseudocode the VFPv2 port does not cover: FPProcessNaNs3 for fused
//! multiply-add, FPToFixed saturation, FPRoundBase tininess and flush-to-zero,
//! FPUnpackBase operand flushing, FPProcessDenorms, and FEAT_AFP's
//! alternate behaviours under FPCR.AH (the pseudocode's `altfp`). QEMU
//! configures the FPCR.AH = 0 behaviour in target/arm/tcg/vfp_helper.c
//! `arm_set_default_fp_behaviours` (`float_tininess_before_rounding`,
//! `float_ftz_before_rounding`, `float_2nan_prop_s_ab`,
//! `float_3nan_prop_s_cab`, `float_infzeronan_dnan_if_qnan`, default NaN
//! pattern 0b01000000) and the FPCR.AH = 1 one in `arm_set_ah_fp_behaviours`.
//!
//! What this module models is AArch64 with every exception untrapped.
//! Floating-point exception trapping (FPCR.IOE/DZE/OFE/UFE/IXE/IDE) is not
//! modelled: with UFE = 1 Arm signals underflow on a tiny result even when
//! it is exact, where here underflow requires inexactness, as for every
//! untrapped underflow. Implementations without FEAT_TRAPFPE leave these
//! controls RAZ/WI, as QEMU does.
//!
//! The IEEE half-precision conversions here ignore FPCR.AHP; the
//! alternative format it selects is converted by `arm_ops`, which the
//! executor calls instead when AHP is set.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Whether Arm FPRoundBase decides tininess on the unrounded result: under
/// FPCR.AH = 0, and after rounding (with an unbounded exponent) under
/// FPCR.AH = 1. Berkeley `softfloat_detectTininess`.
#[inline]
pub(crate) fn tininess_before_rounding(status: &SoftFloatStatus) -> bool {
    status.handling == Handling::Standard
}

// --- Integer responses for unrepresentable conversions ---
// Berkeley ARM-VFPv2 specialize.h `ui32_fromPosOverflow` … `i64_fromNaN`:
// Arm FPToFixed saturates and turns a NaN into zero.
pub(crate) const UI32_FROM_POS_OVERFLOW: u32 = 0xFFFF_FFFF;
pub(crate) const UI32_FROM_NEG_OVERFLOW: u32 = 0;
pub(crate) const UI32_FROM_NAN: u32 = 0;
pub(crate) const I32_FROM_POS_OVERFLOW: i32 = 0x7FFF_FFFF;
pub(crate) const I32_FROM_NEG_OVERFLOW: i32 = -0x7FFF_FFFF - 1;
pub(crate) const I32_FROM_NAN: i32 = 0;
pub(crate) const UI64_FROM_POS_OVERFLOW: u64 = 0xFFFF_FFFF_FFFF_FFFF;
pub(crate) const UI64_FROM_NEG_OVERFLOW: u64 = 0;
pub(crate) const UI64_FROM_NAN: u64 = 0;
pub(crate) const I64_FROM_POS_OVERFLOW: i64 = 0x7FFF_FFFF_FFFF_FFFF;
pub(crate) const I64_FROM_NEG_OVERFLOW: i64 = -0x7FFF_FFFF_FFFF_FFFF - 1;
pub(crate) const I64_FROM_NAN: i64 = 0;

// --- Default NaNs (Arm FPDefaultNaN: the top fraction bit set, the sign
// FPCR.AH) ---

/// Arm FPDefaultNaN{16}.
#[inline]
pub(crate) fn float16_default_nan(status: &SoftFloatStatus) -> Float16 {
    match status.handling {
        Handling::Standard => 0x7E00,
        Handling::Alternate => 0xFE00,
    }
}

/// Arm FPDefaultNaN{32}.
#[inline]
pub(crate) fn float32_default_nan(status: &SoftFloatStatus) -> Float32 {
    match status.handling {
        Handling::Standard => 0x7FC0_0000,
        Handling::Alternate => 0xFFC0_0000,
    }
}

/// Arm FPDefaultNaN{64}.
#[inline]
pub(crate) fn float64_default_nan(status: &SoftFloatStatus) -> Float64 {
    match status.handling {
        Handling::Standard => 0x7FF8_0000_0000_0000,
        Handling::Alternate => 0xFFF8_0000_0000_0000,
    }
}

// --- Two-operand NaN propagation ---

/// Arm FPProcessNaNs (Berkeley ARM-VFPv2 `softfloat_propagateNaNF32UI`):
/// the first signaling NaN of (`a`, `b`), quietened, else the first quiet
/// NaN; under FPCR.AH = 1, when both are NaNs, `a`, quietened. Any
/// signaling input raises invalid; FPCR.DN replaces the result with the
/// default NaN. A one-operand caller passes a non-NaN `b` (FPProcessNaN).
pub(crate) fn softfloat_propagate_nan_f32(
    a: Float32,
    b: Float32,
    status: &mut SoftFloatStatus,
) -> Float32 {
    let a_is_snan = f32_is_signaling_nan(a);
    let b_is_snan = f32_is_signaling_nan(b);
    if a_is_snan || b_is_snan {
        softfloat_raise_flags(status, FLAG_INVALID);
    }
    if softfloat_default_nan_mode(status) {
        return float32_default_nan(status);
    }
    let both_nan = is_nan_f32(a) && is_nan_f32(b);
    let picked = if a_is_snan || (status.handling == Handling::Alternate && both_nan) {
        a
    } else if b_is_snan {
        b
    } else if is_nan_f32(a) {
        a
    } else {
        b
    };
    picked | 0x0040_0000
}

/// Arm FPProcessNaNs for double precision; see
/// [`softfloat_propagate_nan_f32`].
pub(crate) fn softfloat_propagate_nan_f64(
    a: Float64,
    b: Float64,
    status: &mut SoftFloatStatus,
) -> Float64 {
    let a_is_snan = f64_is_signaling_nan(a);
    let b_is_snan = f64_is_signaling_nan(b);
    if a_is_snan || b_is_snan {
        softfloat_raise_flags(status, FLAG_INVALID);
    }
    if softfloat_default_nan_mode(status) {
        return float64_default_nan(status);
    }
    let both_nan = is_nan_f64(a) && is_nan_f64(b);
    let picked = if a_is_snan || (status.handling == Handling::Alternate && both_nan) {
        a
    } else if b_is_snan {
        b
    } else if is_nan_f64(a) {
        a
    } else {
        b
    };
    picked | 0x0008_0000_0000_0000
}

// --- Three-operand NaN propagation (fused multiply-add) ---

/// NaN result of the fused `a * b + c` when at least one operand is a NaN.
/// Arm FPMulAdd(addend = `c`, op1 = `a`, op2 = `b`): FPProcessNaNs3 takes
/// the first signaling NaN in the order addend, op1, op2, else the first
/// quiet NaN in that order (QEMU `float_3nan_prop_s_cab`). Under FPCR.AH =
/// 1 two or three NaNs choose by register instead: op1 when it is one of
/// them, else op2. An infinity times a zero plus a quiet NaN is the default
/// NaN with invalid raised under FPCR.AH = 0, the quiet NaN itself with
/// nothing raised under FPCR.AH = 1; plus a signaling NaN it is that NaN,
/// quietened (QEMU `float_infzeronan_dnan_if_qnan`).
pub(crate) fn softfloat_propagate_nan3_f32(
    a: Float32,
    b: Float32,
    c: Float32,
    status: &mut SoftFloatStatus,
) -> Float32 {
    let a_is_snan = f32_is_signaling_nan(a);
    let b_is_snan = f32_is_signaling_nan(b);
    let c_is_snan = f32_is_signaling_nan(c);
    if a_is_snan || b_is_snan || c_is_snan {
        softfloat_raise_flags(status, FLAG_INVALID);
    }
    let alternate = status.handling == Handling::Alternate;
    let is_inf = |x: Float32| (x & 0x7FFF_FFFF) == 0x7F80_0000;
    let is_zero = |x: Float32| (x & 0x7FFF_FFFF) == 0;
    let inf_times_zero = (is_inf(a) && is_zero(b)) || (is_zero(a) && is_inf(b));
    if !alternate && inf_times_zero && is_nan_f32(c) && !c_is_snan {
        softfloat_raise_flags(status, FLAG_INVALID);
        return float32_default_nan(status);
    }
    if softfloat_default_nan_mode(status) {
        return float32_default_nan(status);
    }
    let picked = if alternate && is_nan_f32(a) && (is_nan_f32(b) || is_nan_f32(c)) {
        a
    } else if alternate && is_nan_f32(b) && is_nan_f32(c) {
        b
    } else if c_is_snan {
        c
    } else if a_is_snan {
        a
    } else if b_is_snan {
        b
    } else if is_nan_f32(c) {
        c
    } else if is_nan_f32(a) {
        a
    } else {
        b
    };
    picked | 0x0040_0000
}

/// Double-precision form of [`softfloat_propagate_nan3_f32`].
pub(crate) fn softfloat_propagate_nan3_f64(
    a: Float64,
    b: Float64,
    c: Float64,
    status: &mut SoftFloatStatus,
) -> Float64 {
    let a_is_snan = f64_is_signaling_nan(a);
    let b_is_snan = f64_is_signaling_nan(b);
    let c_is_snan = f64_is_signaling_nan(c);
    if a_is_snan || b_is_snan || c_is_snan {
        softfloat_raise_flags(status, FLAG_INVALID);
    }
    let alternate = status.handling == Handling::Alternate;
    let is_inf = |x: Float64| (x & 0x7FFF_FFFF_FFFF_FFFF) == 0x7FF0_0000_0000_0000;
    let is_zero = |x: Float64| (x & 0x7FFF_FFFF_FFFF_FFFF) == 0;
    let inf_times_zero = (is_inf(a) && is_zero(b)) || (is_zero(a) && is_inf(b));
    if !alternate && inf_times_zero && is_nan_f64(c) && !c_is_snan {
        softfloat_raise_flags(status, FLAG_INVALID);
        return float64_default_nan(status);
    }
    if softfloat_default_nan_mode(status) {
        return float64_default_nan(status);
    }
    let picked = if alternate && is_nan_f64(a) && (is_nan_f64(b) || is_nan_f64(c)) {
        a
    } else if alternate && is_nan_f64(b) && is_nan_f64(c) {
        b
    } else if c_is_snan {
        c
    } else if a_is_snan {
        a
    } else if b_is_snan {
        b
    } else if is_nan_f64(c) {
        c
    } else if is_nan_f64(a) {
        a
    } else {
        b
    };
    picked | 0x0008_0000_0000_0000
}

// --- Operand flushing (Arm FPUnpackBase) and used denormals
// (FPProcessDenorms) ---

/// Arm FPUnpackBase for single precision: a denormal operand reads as a
/// zero of the same sign under FPCR.FZ with FPCR.AH = 0, raising Input
/// Denormal (IDC), and under FPCR.FIZ, raising nothing.
#[inline]
pub(crate) fn f32_flush_input(a: Float32, status: &mut SoftFloatStatus) -> Float32 {
    if status.input_flush == InputFlush::Off || exp_f32(a) != 0 || frac_f32(a) == 0 {
        return a;
    }
    if status.input_flush == InputFlush::Reported {
        softfloat_raise_flags(status, FLAG_INPUT_DENORMAL);
    }
    a & 0x8000_0000
}

/// Arm FPUnpackBase for double precision; see [`f32_flush_input`].
#[inline]
pub(crate) fn f64_flush_input(a: Float64, status: &mut SoftFloatStatus) -> Float64 {
    if status.input_flush == InputFlush::Off || exp_f64(a) != 0 || frac_f64(a) == 0 {
        return a;
    }
    if status.input_flush == InputFlush::Reported {
        softfloat_raise_flags(status, FLAG_INPUT_DENORMAL);
    }
    a & 0x8000_0000_0000_0000
}

/// Arm FPProcessDenorms for single precision: under FPCR.AH = 1 an
/// operation that uses a denormal operand — one [`f32_flush_input`] left
/// as it is — raises Input Denormal. The caller passes the flushed
/// operands, in the cases its operation's pseudocode calls FPProcessDenorms
/// (for most, when no operand is a NaN).
#[inline]
pub(crate) fn f32_report_denormals(operands: &[Float32], status: &mut SoftFloatStatus) {
    if status.handling == Handling::Alternate
        && operands
            .iter()
            .any(|&x| exp_f32(x) == 0 && frac_f32(x) != 0)
    {
        softfloat_raise_flags(status, FLAG_INPUT_DENORMAL);
    }
}

/// Arm FPProcessDenorms for double precision; see
/// [`f32_report_denormals`].
#[inline]
pub(crate) fn f64_report_denormals(operands: &[Float64], status: &mut SoftFloatStatus) {
    if status.handling == Handling::Alternate
        && operands
            .iter()
            .any(|&x| exp_f64(x) == 0 && frac_f64(x) != 0)
    {
        softfloat_raise_flags(status, FLAG_INPUT_DENORMAL);
    }
}
