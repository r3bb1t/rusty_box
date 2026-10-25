//! Arm specialization: default NaNs, NaN propagation, conversion responses,
//! tininess detection and FPCR.FZ input flushing.
//!
//! Ported from Berkeley SoftFloat 3e `source/ARM-VFPv2/specialize.h`,
//! `s_propagateNaNF32UI.c` and `s_propagateNaNF64UI.c` (with the
//! ARM-VFPv2-defaultNaN variant folded in behind FPCR.DN), extended with the
//! ARMv8 pseudocode the VFPv2 port does not cover: FPProcessNaNs3 for fused
//! multiply-add, FPToFixed saturation, FPRoundBase tininess and flush-to-zero,
//! and FPUnpackBase input flushing. QEMU configures the same behaviour in
//! target/arm/tcg/vfp_helper.c `arm_set_default_fp_behaviours`
//! (`float_tininess_before_rounding`, `float_ftz_before_rounding`,
//! `float_2nan_prop_s_ab`, `float_3nan_prop_s_cab`,
//! `float_infzeronan_dnan_if_qnan`, default NaN pattern 0b01000000).
//!
//! What this module models is AArch64 with FPCR.AH = 0, FPCR.FIZ = 0 and
//! every exception untrapped. Behaviour of real Arm hardware it does not
//! model:
//! - Floating-point exception trapping (FPCR.IOE/DZE/OFE/UFE/IXE/IDE). With
//!   UFE = 1 Arm signals underflow on a tiny result even when it is exact;
//!   here underflow requires inexactness, as for every untrapped underflow.
//!   ARMv8.0 implementations may leave these controls RAZ/WI, as QEMU does.
//! - FEAT_AFP (FPCR.AH, FIZ, NEP) and FEAT_FP16 half-precision arithmetic.
//!   Half precision appears only as a conversion source or destination, where
//!   Arm FPUnpackCV/FPRoundCV force FZ16 off, so no half-precision value is
//!   ever flushed — that much is faithful.
//!
//! The IEEE half-precision conversions here ignore FPCR.AHP; the
//! alternative format it selects is converted by `arm_ops`, which the
//! executor calls instead when AHP is set.

use super::internals::*;
use super::softfloat::*;
use super::softfloat_types::*;

/// Arm FPRoundBase decides tininess on the unrounded result (the AH = 0
/// rule). Berkeley `softfloat_detectTininess == softfloat_tininess_beforeRounding`.
pub(crate) const TININESS_BEFORE_ROUNDING: bool = true;

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

// --- Default NaNs (Arm FPDefaultNaN: sign clear, top fraction bit set) ---
pub(crate) const FLOAT16_DEFAULT_NAN: Float16 = 0x7E00;
pub(crate) const FLOAT32_DEFAULT_NAN: Float32 = 0x7FC0_0000;
pub(crate) const FLOAT64_DEFAULT_NAN: Float64 = 0x7FF8_0000_0000_0000;

// --- Two-operand NaN propagation ---

/// Arm FPProcessNaNs (Berkeley ARM-VFPv2 `softfloat_propagateNaNF32UI`):
/// the first signaling NaN of (`a`, `b`), quietened, else the first quiet
/// NaN. Any signaling input raises invalid; FPCR.DN replaces the result with
/// the default NaN. A one-operand caller passes a non-NaN `b`
/// (FPProcessNaN).
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
        return FLOAT32_DEFAULT_NAN;
    }
    let picked = if a_is_snan {
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
        return FLOAT64_DEFAULT_NAN;
    }
    let picked = if a_is_snan {
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
/// quiet NaN in that order (QEMU `float_3nan_prop_s_cab`). An infinity times
/// a zero plus a quiet NaN is the default NaN with invalid raised; plus a
/// signaling NaN it is that NaN, quietened (QEMU
/// `float_infzeronan_dnan_if_qnan`).
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
    let is_inf = |x: Float32| (x & 0x7FFF_FFFF) == 0x7F80_0000;
    let is_zero = |x: Float32| (x & 0x7FFF_FFFF) == 0;
    let inf_times_zero = (is_inf(a) && is_zero(b)) || (is_zero(a) && is_inf(b));
    if inf_times_zero && is_nan_f32(c) && !c_is_snan {
        softfloat_raise_flags(status, FLAG_INVALID);
        return FLOAT32_DEFAULT_NAN;
    }
    if softfloat_default_nan_mode(status) {
        return FLOAT32_DEFAULT_NAN;
    }
    let picked = if c_is_snan {
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
    let is_inf = |x: Float64| (x & 0x7FFF_FFFF_FFFF_FFFF) == 0x7FF0_0000_0000_0000;
    let is_zero = |x: Float64| (x & 0x7FFF_FFFF_FFFF_FFFF) == 0;
    let inf_times_zero = (is_inf(a) && is_zero(b)) || (is_zero(a) && is_inf(b));
    if inf_times_zero && is_nan_f64(c) && !c_is_snan {
        softfloat_raise_flags(status, FLAG_INVALID);
        return FLOAT64_DEFAULT_NAN;
    }
    if softfloat_default_nan_mode(status) {
        return FLOAT64_DEFAULT_NAN;
    }
    let picked = if c_is_snan {
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

// --- FPCR.FZ input flushing (Arm FPUnpackBase) ---

/// Arm FPUnpackBase for single precision: under FPCR.FZ a denormal operand
/// reads as a zero of the same sign and raises the input-denormal flag (IDC).
pub(crate) fn f32_flush_input(a: Float32, status: &mut SoftFloatStatus) -> Float32 {
    if status.flush_to_zero && exp_f32(a) == 0 && frac_f32(a) != 0 {
        softfloat_raise_flags(status, FLAG_INPUT_DENORMAL);
        return a & 0x8000_0000;
    }
    a
}

/// Arm FPUnpackBase for double precision; see [`f32_flush_input`].
pub(crate) fn f64_flush_input(a: Float64, status: &mut SoftFloatStatus) -> Float64 {
    if status.flush_to_zero && exp_f64(a) == 0 && frac_f64(a) != 0 {
        softfloat_raise_flags(status, FLAG_INPUT_DENORMAL);
        return a & 0x8000_0000_0000_0000;
    }
    a
}
