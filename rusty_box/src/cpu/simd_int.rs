//! Integer SIMD lane operations — Bochs cpu/simd_int.h and simd_compare.h.
//!
//! Each function computes `op1 = op1 OP op2` over one 128-bit lane. They are
//! the bodies Bochs's `HANDLE_SSE_2OP` and `HANDLE_AVX_2OP` templates share
//! (cpu_templates.h): a legacy SSE instruction applies one to its destination
//! and second operand, a VEX instruction to each 128-bit lane of its VEX.vvvv
//! and r/m sources. One body per operation, so the two encodings cannot drift
//! apart (R5).
//!
//! The functions read the operands they were handed and write `op1` in place
//! in the order Bochs does; every element they write is read, from `op1` or
//! `op2`, before it is overwritten, so each is a pure function of its inputs.

use super::sse::{
    saturate_dword_s_to_word_s, saturate_dword_s_to_word_u, saturate_word_s_to_byte_s,
    saturate_word_s_to_byte_u,
};
use super::xmm::BxPackedXmmRegister;

/// Bochs cpu.h `simd_xmm_2op`: one lane operation, `op1 = op1 OP op2`.
pub(super) type Xmm2Op = fn(&mut BxPackedXmmRegister, &BxPackedXmmRegister);

// ── SSSE3 horizontal add/subtract ─────────────────────────────────────────

/// Bochs simd_int.h `xmm_phaddw`.
pub(super) fn xmm_phaddw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        op1.set_xmm16u(n, op1.xmm16u(n * 2).wrapping_add(op1.xmm16u(n * 2 + 1)));
    }
    for n in 0..4 {
        op1.set_xmm16u(n + 4, op2.xmm16u(n * 2).wrapping_add(op2.xmm16u(n * 2 + 1)));
    }
}

/// Bochs simd_int.h `xmm_phaddd`.
pub(super) fn xmm_phaddd(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..2 {
        op1.set_xmm32u(n, op1.xmm32u(n * 2).wrapping_add(op1.xmm32u(n * 2 + 1)));
    }
    for n in 0..2 {
        op1.set_xmm32u(n + 2, op2.xmm32u(n * 2).wrapping_add(op2.xmm32u(n * 2 + 1)));
    }
}

/// Bochs simd_int.h `xmm_phaddsw`.
pub(super) fn xmm_phaddsw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        let sum = i32::from(op1.xmm16s(n * 2)) + i32::from(op1.xmm16s(n * 2 + 1));
        op1.set_xmm16s(n, saturate_dword_s_to_word_s(sum));
    }
    for n in 0..4 {
        let sum = i32::from(op2.xmm16s(n * 2)) + i32::from(op2.xmm16s(n * 2 + 1));
        op1.set_xmm16s(n + 4, saturate_dword_s_to_word_s(sum));
    }
}

/// Bochs simd_int.h `xmm_phsubw`.
pub(super) fn xmm_phsubw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        op1.set_xmm16u(n, op1.xmm16u(n * 2).wrapping_sub(op1.xmm16u(n * 2 + 1)));
    }
    for n in 0..4 {
        op1.set_xmm16u(n + 4, op2.xmm16u(n * 2).wrapping_sub(op2.xmm16u(n * 2 + 1)));
    }
}

/// Bochs simd_int.h `xmm_phsubd`.
pub(super) fn xmm_phsubd(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..2 {
        op1.set_xmm32u(n, op1.xmm32u(n * 2).wrapping_sub(op1.xmm32u(n * 2 + 1)));
    }
    for n in 0..2 {
        op1.set_xmm32u(n + 2, op2.xmm32u(n * 2).wrapping_sub(op2.xmm32u(n * 2 + 1)));
    }
}

/// Bochs simd_int.h `xmm_phsubsw`.
pub(super) fn xmm_phsubsw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        let difference = i32::from(op1.xmm16s(n * 2)) - i32::from(op1.xmm16s(n * 2 + 1));
        op1.set_xmm16s(n, saturate_dword_s_to_word_s(difference));
    }
    for n in 0..4 {
        let difference = i32::from(op2.xmm16s(n * 2)) - i32::from(op2.xmm16s(n * 2 + 1));
        op1.set_xmm16s(n + 4, saturate_dword_s_to_word_s(difference));
    }
}

// ── SSSE3 sign, multiply-add; SSE2 average ────────────────────────────────

/// Bochs simd_int.h `xmm_psignb`: negate, zero or keep each byte of `op1` by
/// the sign of the matching byte of `op2`.
pub(super) fn xmm_psignb(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..16 {
        let sign = i32::from(op2.xmm_sbyte(n) > 0) - i32::from(op2.xmm_sbyte(n) < 0);
        op1.set_xmm_sbyte(n, (i32::from(op1.xmm_sbyte(n)) * sign) as i8);
    }
}

/// Bochs simd_int.h `xmm_psignw`.
pub(super) fn xmm_psignw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..8 {
        let sign = i32::from(op2.xmm16s(n) > 0) - i32::from(op2.xmm16s(n) < 0);
        op1.set_xmm16s(n, (i32::from(op1.xmm16s(n)) * sign) as i16);
    }
}

/// Bochs simd_int.h `xmm_psignd`.
pub(super) fn xmm_psignd(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        let sign = i64::from(op2.xmm32s(n) > 0) - i64::from(op2.xmm32s(n) < 0);
        op1.set_xmm32s(n, (i64::from(op1.xmm32s(n)) * sign) as i32);
    }
}

/// Bochs simd_int.h `xmm_pavgb`: `(a + b + 1) >> 1` per unsigned byte.
pub(super) fn xmm_pavgb(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..16 {
        let average = (u16::from(op1.xmmubyte(n)) + u16::from(op2.xmmubyte(n)) + 1) >> 1;
        op1.set_xmmubyte(n, average as u8);
    }
}

/// Bochs simd_int.h `xmm_pavgw`.
pub(super) fn xmm_pavgw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..8 {
        let average = (u32::from(op1.xmm16u(n)) + u32::from(op2.xmm16u(n)) + 1) >> 1;
        op1.set_xmm16u(n, average as u16);
    }
}

/// Bochs simd_int.h `xmm_pmaddwd`: each dword is the sum of two signed word
/// products. The one sum that does not fit, `0x8000` in all four words, wraps
/// to `0x80000000`, as the processor reports it.
pub(super) fn xmm_pmaddwd(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        let low = i32::from(op1.xmm16s(n * 2)) * i32::from(op2.xmm16s(n * 2));
        let high = i32::from(op1.xmm16s(n * 2 + 1)) * i32::from(op2.xmm16s(n * 2 + 1));
        op1.set_xmm32s(n, low.wrapping_add(high));
    }
}

/// Bochs simd_int.h `xmm_pmaddubsw`: unsigned bytes of `op1` times signed
/// bytes of `op2`, adjacent products summed with signed saturation.
pub(super) fn xmm_pmaddubsw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..8 {
        let sum = i32::from(op1.xmmubyte(n * 2)) * i32::from(op2.xmm_sbyte(n * 2))
            + i32::from(op1.xmmubyte(n * 2 + 1)) * i32::from(op2.xmm_sbyte(n * 2 + 1));
        op1.set_xmm16s(n, saturate_dword_s_to_word_s(sum));
    }
}

// ── SSE4.1 minimum / maximum ──────────────────────────────────────────────

/// Bochs simd_int.h `xmm_pminsb`.
pub(super) fn xmm_pminsb(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..16 {
        if op2.xmm_sbyte(n) < op1.xmm_sbyte(n) {
            op1.set_xmmubyte(n, op2.xmmubyte(n));
        }
    }
}

/// Bochs simd_int.h `xmm_pminuw`.
pub(super) fn xmm_pminuw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..8 {
        if op2.xmm16u(n) < op1.xmm16u(n) {
            op1.set_xmm16u(n, op2.xmm16u(n));
        }
    }
}

/// Bochs simd_int.h `xmm_pminsd`.
pub(super) fn xmm_pminsd(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        if op2.xmm32s(n) < op1.xmm32s(n) {
            op1.set_xmm32u(n, op2.xmm32u(n));
        }
    }
}

/// Bochs simd_int.h `xmm_pmaxsb`.
pub(super) fn xmm_pmaxsb(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..16 {
        if op2.xmm_sbyte(n) > op1.xmm_sbyte(n) {
            op1.set_xmmubyte(n, op2.xmmubyte(n));
        }
    }
}

/// Bochs simd_int.h `xmm_pmaxuw`.
pub(super) fn xmm_pmaxuw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..8 {
        if op2.xmm16u(n) > op1.xmm16u(n) {
            op1.set_xmm16u(n, op2.xmm16u(n));
        }
    }
}

/// Bochs simd_int.h `xmm_pmaxsd`.
pub(super) fn xmm_pmaxsd(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        if op2.xmm32s(n) > op1.xmm32s(n) {
            op1.set_xmm32u(n, op2.xmm32u(n));
        }
    }
}

// ── SSE4.1 / SSE4.2 quadword compares ─────────────────────────────────────

/// Bochs simd_compare.h `xmm_pcmpeqq`.
pub(super) fn xmm_pcmpeqq(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..2 {
        op1.set_xmm64u(n, if op1.xmm64u(n) == op2.xmm64u(n) { u64::MAX } else { 0 });
    }
}

/// Bochs simd_compare.h `xmm_pcmpgtq`: signed greater-than.
pub(super) fn xmm_pcmpgtq(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..2 {
        op1.set_xmm64u(n, if op1.xmm64s(n) > op2.xmm64s(n) { u64::MAX } else { 0 });
    }
}

// ── Packs ─────────────────────────────────────────────────────────────────

/// Bochs simd_int.h `xmm_packsswb`: signed words to signed bytes, `op1`'s
/// eight in the low half and `op2`'s in the high.
pub(super) fn xmm_packsswb(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..8 {
        op1.set_xmm_sbyte(n, saturate_word_s_to_byte_s(op1.xmm16s(n)));
    }
    for n in 0..8 {
        op1.set_xmm_sbyte(n + 8, saturate_word_s_to_byte_s(op2.xmm16s(n)));
    }
}

/// Bochs simd_int.h `xmm_packuswb`: signed words to unsigned bytes.
pub(super) fn xmm_packuswb(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..8 {
        op1.set_xmmubyte(n, saturate_word_s_to_byte_u(op1.xmm16s(n)));
    }
    for n in 0..8 {
        op1.set_xmmubyte(n + 8, saturate_word_s_to_byte_u(op2.xmm16s(n)));
    }
}

/// Bochs simd_int.h `xmm_packssdw`: signed dwords to signed words.
pub(super) fn xmm_packssdw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        op1.set_xmm16s(n, saturate_dword_s_to_word_s(op1.xmm32s(n)));
    }
    for n in 0..4 {
        op1.set_xmm16s(n + 4, saturate_dword_s_to_word_s(op2.xmm32s(n)));
    }
}

/// Bochs simd_int.h `xmm_packusdw`: signed dwords to unsigned words.
pub(super) fn xmm_packusdw(op1: &mut BxPackedXmmRegister, op2: &BxPackedXmmRegister) {
    for n in 0..4 {
        op1.set_xmm16u(n, saturate_dword_s_to_word_u(op1.xmm32s(n)));
    }
    for n in 0..4 {
        op1.set_xmm16u(n + 4, saturate_dword_s_to_word_u(op2.xmm32s(n)));
    }
}
