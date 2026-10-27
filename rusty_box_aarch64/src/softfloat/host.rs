//! The host's IEEE 754 arithmetic, where it gives exactly the bits and the
//! exceptions SoftFloat gives.
//!
//! QEMU fpu/softfloat.c computes on the host ("hardfloat": `can_use_fpu`,
//! `float32_gen2`/`float64_gen2`, `f64_is_zon2`, `f64_addsubmul_post`)
//! when the rounding is to nearest even and inexact is already raised, so
//! that whether the host's result is inexact does not matter, and when the
//! operands are of a kind on which the host and SoftFloat agree. This
//! module applies that rule more narrowly:
//!
//! - both operands are normal, so that FPCR.FZ flushes neither and no NaN
//!   or infinity has a rule of its own;
//! - the result is finite, and larger in magnitude than the smallest
//!   normal number.
//!
//! The result bound means the operation did not overflow. It also means
//! the exact value was not tiny: Arm detects tininess before rounding, and
//! an exact value below the smallest normal rounds to at most the smallest
//! normal itself, which is left to SoftFloat. So FPCR.FZ has no tiny result
//! to flush, and FPCR.DN has no NaN to choose.
//!
//! IEEE 754 then defines the result as the exact value rounded to nearest
//! even, which is what both compute. The only exception either raises is
//! inexact, which is already set. Every other case is `None`, and SoftFloat
//! computes it.
//!
//! FEAT_AFP's controls change none of this. FPCR.FIZ, and FPCR.FZ under
//! FPCR.AH, flush denormal operands only (Arm `FPUnpackBase`); FPCR.AH
//! moves the flush of a result to after rounding, which a result above the
//! smallest normal never meets (`FPRoundBase`), and changes how a NaN is
//! negated and chosen (`FPNeg`, `FPProcessNaNs`), none of which a normal
//! operand or a finite result is; FPCR.NEP decides only the bits above a
//! scalar result, which the caller merges ([`crate::cpu`]'s
//! `fp_set_merging`).
//!
//! A fused multiply-add follows the same rule, with all three operands
//! normal. The Arm ARM's FPMulAdd rounds the exact `addend + n*m` once
//! (FPRound), as SoftFloat's `f64_mulAdd`/`f32_mulAdd` and IEEE 754-2019
//! fusedMultiplyAdd do, so the result bound means for it what it means
//! above; it also leaves out an exact zero, whose sign FPMulAdd chooses
//! by a rule of its own. The host's fusedMultiplyAdd is Rust's `mul_add`,
//! which needs `std`; without it every fused multiply-add is `None`.
//!
//! `mul_add` is only as exact as what it lowers to, and where that is a
//! C library's `fma`/`fmaf` some round normal results wrongly: MinGW-w64
//! on MSVCRT (rust-lang/rust#140515), and FreeBSD msun's `s_fmaf.c`, which
//! bionic uses on Android's non-Arm ABIs (JuliaMath/openlibm#357). So the
//! host fuses only where its `mul_add` is known to round once, and every
//! other host's fused multiply-add is `None`:
//!
//! - aarch64, where `mul_add` is FMADD/FMSUB of the base Armv8-A floating
//!   point, which rounds once (Arm ARM FPMulAdd);
//! - x86 and x86_64 built with the `fma` target feature, where it is
//!   VFMADD, which rounds once (Intel SDM, VFMADD132SD/VFMADD213SD/
//!   VFMADD231SD and their SS forms);
//! - x86_64 Linux with glibc, where it is a call to `fma`/`fmaf`. The Rust
//!   toolchain's compiler-builtins defines those in the program itself:
//!   its port of the `libm` crate runs VFMADD (or AMD's VFMADDSD) where
//!   the CPU has it, and otherwise `fma_round`, a port of musl's `fma.c`,
//!   which rounds once for every input, and `fma_wide_round`, a port of
//!   musl's `fmaf.c`, which rounds once wherever the result is normal.
//!   glibc's own `fma`/`fmaf` round once wherever the result is normal
//!   too, the only results the bound accepts: on x86_64
//!   (`sysdeps/x86_64/fpu/multiarch/s_fma.c`, `s_fmaf.c`) they run
//!   VFMADD213SD/SS, or FMA4's VFMADDSD/SS, where the CPU has it, and
//!   otherwise `sysdeps/ieee754/dbl-64/s_fma.c`, which forms the exact
//!   result in integers and rounds it once, and `dbl-64/s_fmaf.c`, which
//!   forms the sum in binary64 and corrects its last bit wherever rounding
//!   it again to binary32 could err.
//!
//! Rust's `f32` and `f64` arithmetic is IEEE 754 binary32 and binary64,
//! rounding to nearest even, on every target but 32-bit x86 without SSE2:
//! there x87 rounds twice, and this module answers `None`.

use super::softfloat::{FLAG_INEXACT, ROUND_NEAR_EVEN};
use super::ArmFpStatus;

#[cfg(test)]
#[path = "../../tests/unit/host_float.rs"]
mod tests;

/// Whether the host's arithmetic rounds once, as IEEE 754 binary32 and
/// binary64 do.
const HOST_ROUNDS_ONCE: bool = !cfg!(all(target_arch = "x86", not(target_feature = "sse2")));

/// Whether the host's `mul_add` is known to round once, as IEEE 754
/// fusedMultiplyAdd does, on every result the bound accepts: the hosts
/// the module doc lists. `mul_add` needs `std`.
#[cfg(feature = "std")]
const HOST_FUSES_ONCE: bool = cfg!(any(
    target_arch = "aarch64",
    all(
        any(target_arch = "x86", target_arch = "x86_64"),
        target_feature = "fma"
    ),
    all(
        target_arch = "x86_64",
        target_os = "linux",
        target_env = "gnu"
    ),
));

/// The operations the host computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Arith {
    Add,
    Sub,
    Mul,
    Div,
}

/// Whether the host may compute under `fp`: rounding to nearest even,
/// with inexact already raised.
#[inline(always)]
fn host_may_round(fp: &ArmFpStatus) -> bool {
    HOST_ROUNDS_ONCE
        && fp.status.softfloat_rounding_mode == ROUND_NEAR_EVEN
        && fp.status.softfloat_exception_flags & FLAG_INEXACT != 0
}

/// Whether the binary64 `bits` are a normal number.
#[inline(always)]
const fn f64_normal(bits: u64) -> bool {
    let exp = (bits >> 52) & 0x7FF;
    exp != 0 && exp != 0x7FF
}

/// Whether the binary32 `bits` are a normal number.
#[inline(always)]
const fn f32_normal(bits: u32) -> bool {
    let exp = (bits >> 23) & 0xFF;
    exp != 0 && exp != 0xFF
}

/// `a op b` in binary64, if the host computes it as SoftFloat does.
#[inline(always)]
pub(super) fn f64_arith(op: Arith, a: u64, b: u64, fp: &ArmFpStatus) -> Option<u64> {
    if !(host_may_round(fp) && f64_normal(a) && f64_normal(b)) {
        return None;
    }
    let (x, y) = (f64::from_bits(a), f64::from_bits(b));
    let r = match op {
        Arith::Add => x + y,
        Arith::Sub => x - y,
        Arith::Mul => x * y,
        Arith::Div => x / y,
    }
    .to_bits();
    f64_bounded(r)
}

/// The binary64 result `r`, if its magnitude is above the smallest normal
/// and below infinity (a NaN's magnitude is above infinity's).
#[inline(always)]
fn f64_bounded(r: u64) -> Option<u64> {
    let magnitude = r & !(1 << 63);
    (magnitude > f64::MIN_POSITIVE.to_bits() && magnitude < f64::INFINITY.to_bits()).then_some(r)
}

/// The binary32 result `r`, if its magnitude is above the smallest normal
/// and below infinity.
#[inline(always)]
fn f32_bounded(r: u32) -> Option<u32> {
    let magnitude = r & !(1 << 31);
    (magnitude > f32::MIN_POSITIVE.to_bits() && magnitude < f32::INFINITY.to_bits()).then_some(r)
}

/// `a op b` in binary32, if the host computes it as SoftFloat does.
#[inline(always)]
pub(super) fn f32_arith(op: Arith, a: u32, b: u32, fp: &ArmFpStatus) -> Option<u32> {
    if !(host_may_round(fp) && f32_normal(a) && f32_normal(b)) {
        return None;
    }
    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
    let r = match op {
        Arith::Add => x + y,
        Arith::Sub => x - y,
        Arith::Mul => x * y,
        Arith::Div => x / y,
    }
    .to_bits();
    f32_bounded(r)
}

/// `addend + n * m` in binary64, rounded once, if the host computes it as
/// SoftFloat does (Arm ARM FPMulAdd; IEEE 754-2019 fusedMultiplyAdd).
#[cfg(feature = "std")]
#[inline(always)]
pub(super) fn f64_fused(addend: u64, n: u64, m: u64, fp: &ArmFpStatus) -> Option<u64> {
    if !(HOST_FUSES_ONCE
        && host_may_round(fp)
        && f64_normal(addend)
        && f64_normal(n)
        && f64_normal(m))
    {
        return None;
    }
    f64_bounded(
        f64::from_bits(n)
            .mul_add(f64::from_bits(m), f64::from_bits(addend))
            .to_bits(),
    )
}

/// Without `std` the host has no fused multiply-add: SoftFloat computes
/// every one.
#[cfg(not(feature = "std"))]
#[inline(always)]
pub(super) fn f64_fused(_addend: u64, _n: u64, _m: u64, _fp: &ArmFpStatus) -> Option<u64> {
    None
}

/// `addend + n * m` in binary32, rounded once, if the host computes it as
/// SoftFloat does (Arm ARM FPMulAdd; IEEE 754-2019 fusedMultiplyAdd).
#[cfg(feature = "std")]
#[inline(always)]
pub(super) fn f32_fused(addend: u32, n: u32, m: u32, fp: &ArmFpStatus) -> Option<u32> {
    if !(HOST_FUSES_ONCE
        && host_may_round(fp)
        && f32_normal(addend)
        && f32_normal(n)
        && f32_normal(m))
    {
        return None;
    }
    f32_bounded(
        f32::from_bits(n)
            .mul_add(f32::from_bits(m), f32::from_bits(addend))
            .to_bits(),
    )
}

/// Without `std` the host has no fused multiply-add: SoftFloat computes
/// every one.
#[cfg(not(feature = "std"))]
#[inline(always)]
pub(super) fn f32_fused(_addend: u32, _n: u32, _m: u32, _fp: &ArmFpStatus) -> Option<u32> {
    None
}
