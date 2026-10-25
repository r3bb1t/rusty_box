//! SoftFloat status, rounding modes, exception flags and field helpers.
//! Ported from Berkeley SoftFloat 3e softfloat.h / Bochs
//! softfloat3e/include/softfloat.h and softfloat-extra.h. The status carries
//! the Arm FPCR controls the arithmetic reads (DN, FZ, RMode) in place of the
//! x86 exception masks, which Arm has no use for: every exception here is
//! untrapped and only accumulates into the FPSR cumulative bits.

/// Software floating-point status — passed by `&mut` to every operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SoftFloatStatus {
    /// One of the `ROUND_*` constants.
    pub(crate) softfloat_rounding_mode: u8,
    /// Accumulated `FLAG_*` bits.
    pub(crate) softfloat_exception_flags: i32,
    /// FPCR.DN: every NaN result is the default NaN.
    pub(crate) default_nan_mode: bool,
    /// FPCR.FZ: a single- or double-precision result that is tiny before
    /// rounding becomes a zero of the same sign, raising underflow alone.
    /// Input flushing (FPUnpack) is applied by the callers in `mod.rs`.
    pub(crate) flush_to_zero: bool,
}

impl Default for SoftFloatStatus {
    fn default() -> Self {
        Self {
            softfloat_rounding_mode: ROUND_NEAR_EVEN,
            softfloat_exception_flags: 0,
            default_nan_mode: false,
            flush_to_zero: false,
        }
    }
}

// Rounding modes (Berkeley `softfloat_round_*`).
pub(crate) const ROUND_NEAR_EVEN: u8 = 0;
pub(crate) const ROUND_MIN: u8 = 1;
pub(crate) const ROUND_MAX: u8 = 2;
pub(crate) const ROUND_MINMAG: u8 = 3;
pub(crate) const ROUND_NEAR_MAXMAG: u8 = 4;

// Exception flags (Berkeley `softfloat_flag_*`).
pub(crate) const FLAG_INVALID: i32 = 0x01;
/// A denormal operand took part in the operation without being flushed
/// (x86 DE; QEMU `float_flag_input_denormal_used`). Arm with FPCR.AH = 0
/// does not report it, so it never reaches the FPSR.
pub(crate) const FLAG_DENORMAL: i32 = 0x02;
pub(crate) const FLAG_DIVBYZERO: i32 = 0x04;
pub(crate) const FLAG_INFINITE: i32 = FLAG_DIVBYZERO;
pub(crate) const FLAG_OVERFLOW: i32 = 0x08;
pub(crate) const FLAG_UNDERFLOW: i32 = 0x10;
pub(crate) const FLAG_INEXACT: i32 = 0x20;
/// A denormal input was flushed to zero under FPCR.FZ (Arm FPSR.IDC).
pub(crate) const FLAG_INPUT_DENORMAL: i32 = 0x40;

/// Result of an ordered/unordered comparison (Arm FPCompare).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FpOrdering {
    Less,
    Equal,
    Greater,
    Unordered,
}

impl FpOrdering {
    /// The NZCV nibble (N = bit 3 … V = bit 0) FCMP/FCMPE write for this
    /// result. Arm ARM FPCompare.
    pub(crate) fn nzcv(self) -> u8 {
        match self {
            FpOrdering::Less => 0b1000,
            FpOrdering::Equal => 0b0110,
            FpOrdering::Greater => 0b0010,
            FpOrdering::Unordered => 0b0011,
        }
    }
}

/// Floating-point class. Bochs softfloat3e/include/softfloat.h
/// `softfloat_class_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SoftFloatClass {
    Zero,
    SNaN,
    QNaN,
    NegativeInf,
    PositiveInf,
    Denormal,
    Normalized,
}

// --- Helper functions on SoftFloatStatus ---

#[inline]
pub(crate) fn softfloat_raise_flags(status: &mut SoftFloatStatus, flags: i32) {
    status.softfloat_exception_flags |= flags;
}

#[inline]
pub(crate) fn softfloat_get_rounding_mode(status: &SoftFloatStatus) -> u8 {
    status.softfloat_rounding_mode
}

#[inline]
pub(crate) fn softfloat_flush_underflow_to_zero(status: &SoftFloatStatus) -> bool {
    status.flush_to_zero
}

#[inline]
pub(crate) fn softfloat_default_nan_mode(status: &SoftFloatStatus) -> bool {
    status.default_nan_mode
}

// --- Signaling-NaN tests (Berkeley `softfloat_isSigNaN*UI`) ---

#[inline]
pub(crate) fn f16_is_signaling_nan(a: u16) -> bool {
    ((a & 0x7E00) == 0x7C00) && ((a & 0x01FF) != 0)
}

#[inline]
pub(crate) fn f32_is_signaling_nan(a: u32) -> bool {
    ((a & 0x7FC00000) == 0x7F800000) && ((a & 0x003FFFFF) != 0)
}

#[inline]
pub(crate) fn f64_is_signaling_nan(a: u64) -> bool {
    ((a & 0x7FF8000000000000) == 0x7FF0000000000000) && ((a & 0x0007FFFFFFFFFFFF) != 0)
}
