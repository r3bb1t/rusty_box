//! Packed floating-point primitives over one 128-bit lane.
//!
//! Rust analogue of Bochs cpu/simd_pfp.h. Every element goes through
//! SoftFloat 3e with a live `SoftFloatStatus`, so MXCSR.RC, the MXCSR
//! exception flags, DAZ/FTZ and x86 NaN propagation all behave. Callers
//! build the status word with
//! [`mxcsr_to_softfloat_status_word`](super::sse_fp::mxcsr_to_softfloat_status_word),
//! run one or more of these, then feed the accumulated flags to
//! `check_exceptions_sse`.
//!
//! AVX handlers apply these per 128-bit lane exactly as Bochs does in
//! `cpu_templates_pfp.h`, which is why the unit is a lane and not a
//! whole register.

use super::softfloat3e::f32_addsub::{f32_add, f32_sub};
use super::softfloat3e::f32_compare::{f32_max, f32_min};
use super::softfloat3e::f32_div::f32_div;
use super::softfloat3e::f32_mul::f32_mul;
use super::softfloat3e::f32_sqrt::f32_sqrt;
use super::softfloat3e::f64_addsub::{f64_add, f64_sub};
use super::softfloat3e::f64_compare::{f64_max, f64_min};
use super::softfloat3e::f64_div::f64_div;
use super::softfloat3e::f64_mul::f64_mul;
use super::softfloat3e::f64_sqrt::f64_sqrt;
use super::softfloat3e::softfloat::SoftFloatStatus;
use super::softfloat3e::softfloat_compare::{f32_compare_predicate, f64_compare_predicate};
use super::xmm::BxPackedXmmRegister;

/// Generates the four single-precision and two double-precision element
/// loops that make up most of Bochs simd_pfp.h.
macro_rules! packed_binop {
    ($ps:ident, $pd:ident, $f32op:path, $f64op:path, $bochs:literal) => {
        #[doc = concat!("Bochs simd_pfp.h `", $bochs, "ps`.")]
        #[inline]
        pub(super) fn $ps(
            op1: &mut BxPackedXmmRegister,
            op2: &BxPackedXmmRegister,
            status: &mut SoftFloatStatus,
        ) {
            for n in 0..4 {
                op1.set_xmm32u(n, $f32op(op1.xmm32u(n), op2.xmm32u(n), status));
            }
        }

        #[doc = concat!("Bochs simd_pfp.h `", $bochs, "pd`.")]
        #[inline]
        pub(super) fn $pd(
            op1: &mut BxPackedXmmRegister,
            op2: &BxPackedXmmRegister,
            status: &mut SoftFloatStatus,
        ) {
            for n in 0..2 {
                op1.set_xmm64u(n, $f64op(op1.xmm64u(n), op2.xmm64u(n), status));
            }
        }
    };
}

/// The `_mask` forms of the same loops: an element whose mask bit is clear
/// is zeroed rather than computed, so it also contributes no exception.
macro_rules! packed_binop_mask {
    ($ps:ident, $pd:ident, $f32op:path, $f64op:path, $bochs:literal) => {
        #[doc = concat!("Bochs simd_pfp.h `", $bochs, "ps_mask`.")]
        #[inline]
        pub(super) fn $ps(
            op1: &mut BxPackedXmmRegister,
            op2: &BxPackedXmmRegister,
            status: &mut SoftFloatStatus,
            mut mask: u32,
        ) {
            for n in 0..4 {
                if mask & 0x1 != 0 {
                    op1.set_xmm32u(n, $f32op(op1.xmm32u(n), op2.xmm32u(n), status));
                } else {
                    op1.set_xmm32u(n, 0);
                }
                mask >>= 1;
            }
        }

        #[doc = concat!("Bochs simd_pfp.h `", $bochs, "pd_mask`.")]
        #[inline]
        pub(super) fn $pd(
            op1: &mut BxPackedXmmRegister,
            op2: &BxPackedXmmRegister,
            status: &mut SoftFloatStatus,
            mut mask: u32,
        ) {
            for n in 0..2 {
                if mask & 0x1 != 0 {
                    op1.set_xmm64u(n, $f64op(op1.xmm64u(n), op2.xmm64u(n), status));
                } else {
                    op1.set_xmm64u(n, 0);
                }
                mask >>= 1;
            }
        }
    };
}

packed_binop!(xmm_addps, xmm_addpd, f32_add, f64_add, "xmm_add");
packed_binop!(xmm_subps, xmm_subpd, f32_sub, f64_sub, "xmm_sub");
packed_binop!(xmm_mulps, xmm_mulpd, f32_mul, f64_mul, "xmm_mul");
packed_binop!(xmm_divps, xmm_divpd, f32_div, f64_div, "xmm_div");
packed_binop!(xmm_minps, xmm_minpd, f32_min, f64_min, "xmm_min");
packed_binop!(xmm_maxps, xmm_maxpd, f32_max, f64_max, "xmm_max");

packed_binop_mask!(xmm_addps_mask, xmm_addpd_mask, f32_add, f64_add, "xmm_add");
packed_binop_mask!(xmm_mulps_mask, xmm_mulpd_mask, f32_mul, f64_mul, "xmm_mul");

/// Bochs simd_int.h `xmm_shufps`.
#[inline]
pub(super) fn xmm_shufps(
    r: &mut BxPackedXmmRegister,
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    order: u8,
) {
    r.set_xmm32u(0, op1.xmm32u((order & 0x3) as usize));
    r.set_xmm32u(1, op1.xmm32u(((order >> 2) & 0x3) as usize));
    r.set_xmm32u(2, op2.xmm32u(((order >> 4) & 0x3) as usize));
    r.set_xmm32u(3, op2.xmm32u(((order >> 6) & 0x3) as usize));
}

/// Bochs simd_int.h `xmm_shufpd`.
#[inline]
pub(super) fn xmm_shufpd(
    r: &mut BxPackedXmmRegister,
    op1: &BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    order: u8,
) {
    r.set_xmm64u(0, op1.xmm64u((order & 0x1) as usize));
    r.set_xmm64u(1, op2.xmm64u(((order >> 1) & 0x1) as usize));
}

/// Bochs simd_pfp.h `xmm_sqrtps`.
#[inline]
pub(super) fn xmm_sqrtps(op: &mut BxPackedXmmRegister, status: &mut SoftFloatStatus) {
    for n in 0..4 {
        op.set_xmm32u(n, f32_sqrt(op.xmm32u(n), status));
    }
}

/// Bochs simd_pfp.h `xmm_sqrtpd`.
#[inline]
pub(super) fn xmm_sqrtpd(op: &mut BxPackedXmmRegister, status: &mut SoftFloatStatus) {
    for n in 0..2 {
        op.set_xmm64u(n, f64_sqrt(op.xmm64u(n), status));
    }
}

/// Bochs simd_pfp.h `xmm_addsubps` — subtract in the even lanes, add in
/// the odd ones.
#[inline]
pub(super) fn xmm_addsubps(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    status: &mut SoftFloatStatus,
) {
    op1.set_xmm32u(0, f32_sub(op1.xmm32u(0), op2.xmm32u(0), status));
    op1.set_xmm32u(1, f32_add(op1.xmm32u(1), op2.xmm32u(1), status));
    op1.set_xmm32u(2, f32_sub(op1.xmm32u(2), op2.xmm32u(2), status));
    op1.set_xmm32u(3, f32_add(op1.xmm32u(3), op2.xmm32u(3), status));
}

/// Bochs simd_pfp.h `xmm_addsubpd`.
#[inline]
pub(super) fn xmm_addsubpd(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    status: &mut SoftFloatStatus,
) {
    op1.set_xmm64u(0, f64_sub(op1.xmm64u(0), op2.xmm64u(0), status));
    op1.set_xmm64u(1, f64_add(op1.xmm64u(1), op2.xmm64u(1), status));
}

/// Bochs simd_pfp.h `xmm_haddps`.
#[inline]
pub(super) fn xmm_haddps(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    status: &mut SoftFloatStatus,
) {
    let r0 = f32_add(op1.xmm32u(0), op1.xmm32u(1), status);
    let r1 = f32_add(op1.xmm32u(2), op1.xmm32u(3), status);
    let r2 = f32_add(op2.xmm32u(0), op2.xmm32u(1), status);
    let r3 = f32_add(op2.xmm32u(2), op2.xmm32u(3), status);
    op1.set_xmm32u(0, r0);
    op1.set_xmm32u(1, r1);
    op1.set_xmm32u(2, r2);
    op1.set_xmm32u(3, r3);
}

/// Bochs simd_pfp.h `xmm_haddpd`.
#[inline]
pub(super) fn xmm_haddpd(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    status: &mut SoftFloatStatus,
) {
    let r0 = f64_add(op1.xmm64u(0), op1.xmm64u(1), status);
    let r1 = f64_add(op2.xmm64u(0), op2.xmm64u(1), status);
    op1.set_xmm64u(0, r0);
    op1.set_xmm64u(1, r1);
}

/// Bochs simd_pfp.h `xmm_hsubps`.
#[inline]
pub(super) fn xmm_hsubps(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    status: &mut SoftFloatStatus,
) {
    let r0 = f32_sub(op1.xmm32u(0), op1.xmm32u(1), status);
    let r1 = f32_sub(op1.xmm32u(2), op1.xmm32u(3), status);
    let r2 = f32_sub(op2.xmm32u(0), op2.xmm32u(1), status);
    let r3 = f32_sub(op2.xmm32u(2), op2.xmm32u(3), status);
    op1.set_xmm32u(0, r0);
    op1.set_xmm32u(1, r1);
    op1.set_xmm32u(2, r2);
    op1.set_xmm32u(3, r3);
}

/// Bochs simd_pfp.h `xmm_hsubpd`.
#[inline]
pub(super) fn xmm_hsubpd(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    status: &mut SoftFloatStatus,
) {
    let r0 = f64_sub(op1.xmm64u(0), op1.xmm64u(1), status);
    let r1 = f64_sub(op2.xmm64u(0), op2.xmm64u(1), status);
    op1.set_xmm64u(0, r0);
    op1.set_xmm64u(1, r1);
}

/// One 128-bit lane of CMPPS / VCMPPS: each element becomes an all-ones or
/// all-zeroes mask. Bochs sse_pfp.cc CMPPS_VpsWpsIbR and avx_pfp.cc
/// VCMPPS_VpsHpsWpsIbR both walk `compare32[ib]` this way.
#[inline]
pub(super) fn xmm_cmpps(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    predicate: u8,
    status: &mut SoftFloatStatus,
) {
    for n in 0..4 {
        let hit = f32_compare_predicate(predicate, op1.xmm32u(n), op2.xmm32u(n), status);
        op1.set_xmm32u(n, if hit { 0xFFFF_FFFF } else { 0 });
    }
}

/// One 128-bit lane of CMPPD / VCMPPD.
#[inline]
pub(super) fn xmm_cmppd(
    op1: &mut BxPackedXmmRegister,
    op2: &BxPackedXmmRegister,
    predicate: u8,
    status: &mut SoftFloatStatus,
) {
    for n in 0..2 {
        let hit = f64_compare_predicate(predicate, op1.xmm64u(n), op2.xmm64u(n), status);
        op1.set_xmm64u(n, if hit { 0xFFFF_FFFF_FFFF_FFFF } else { 0 });
    }
}
