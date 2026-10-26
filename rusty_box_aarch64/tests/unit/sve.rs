//! The SVE instructions in Streaming SVE mode, on a processor that
//! implements FEAT_SME and not FEAT_SVE ([`Implementation::EVERY_FEATURE`]).
//!
//! tests/vectors/qemu_sve.txt checks every class against QEMU at one vector
//! length with random operands. These are the properties random operands
//! rarely or never reach, each the guest-visible one (R9), each against
//! its Arm pseudocode: the enable checks (`CheckSVEEnabled`,
//! `CheckStreamingSVEEnabled`), the vector length (`CurrentVL`, the
//! `SVEZEROUPPER` choice), `DecodePredCount`, and the boundaries of WHILE,
//! CTERM, SDIV, EXT, DUP (indexed), SQINCB and PSEL.

use std::vec::Vec;

use super::super::features::{Implementation, VecLen};
use super::super::step::{exec, processor, Taken, CPACR_ALL, FP_TRAP, SME_DISABLED};
use super::super::Cpu;
use super::{Esize, Pv, Zv};

/// The SME trap with SMTC 2: an SVE instruction outside Streaming SVE mode
/// on a processor without FEAT_SVE.
const NOT_STREAMING: Taken = Taken { ec: 0x1D, iss: 2 };

/// `cntb x0` (pattern ALL, `MUL #1`).
const CNTB_X0: u32 = 0x0420_E3E0;
/// `index z0.b, #0, #1`.
const INDEX_Z0_B: u32 = 0x0421_4000;

/// A processor in Streaming SVE mode with `SMCR_EL1.LEN = len`.
fn streaming(len: u64) -> Cpu {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.sys.smcr_el1 = len;
    cpu.set_pstate_sm(true);
    cpu
}

/// `ptrue pd.<size>, pattern` (`ptrues` with `setflags`).
const fn ptrue(size: u32, pattern: u32, setflags: bool) -> u32 {
    0x2518_E000 | (size << 22) | ((setflags as u32) << 16) | (pattern << 5)
}

/// The elements of `es` of P0 that are active, at the current length.
fn active(cpu: &Cpu, es: Esize) -> Vec<bool> {
    let vl = cpu.current_svl();
    let p = cpu.p(0, vl);
    (0..es.elements(vl)).map(|e| p.active(e, es)).collect()
}

#[test]
fn outside_streaming_mode_an_sve_instruction_takes_the_sme_trap() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    assert_eq!(exec(&mut cpu, CNTB_X0), Some(NOT_STREAMING));
    assert_eq!(exec(&mut cpu, INDEX_Z0_B), Some(NOT_STREAMING));
    // CheckStreamingSVEEnabled is CheckSMEEnabled first.
    cpu.sys.cpacr_el1 = 0;
    assert_eq!(exec(&mut cpu, CNTB_X0), Some(SME_DISABLED));
    cpu.sys.cpacr_el1 = 3 << 24;
    assert_eq!(exec(&mut cpu, CNTB_X0), Some(FP_TRAP));
}

#[test]
fn in_streaming_mode_an_sve_instruction_checks_sme_then_simd_and_fp() {
    let mut cpu = streaming(0);
    cpu.sys.cpacr_el1 = 0;
    assert_eq!(exec(&mut cpu, CNTB_X0), Some(SME_DISABLED));
    cpu.sys.cpacr_el1 = 3 << 24;
    assert_eq!(exec(&mut cpu, CNTB_X0), Some(FP_TRAP));
    cpu.sys.cpacr_el1 = CPACR_ALL;
    assert_eq!(exec(&mut cpu, CNTB_X0), None);
    assert_eq!(cpu.x[0], 16);
}

/// A decode-time UNDEFINED is taken before the enable check
/// (`EndOfDecode(Decode_UNDEF)` precedes `CheckSVEEnabled`).
#[test]
fn a_reserved_encoding_is_undefined_before_the_sme_trap() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    // SDIV z0.b: `size` = 00 is reserved.
    assert_eq!(exec(&mut cpu, 0x0414_0020).map(|t| t.ec), Some(0x00));
    // ASR z0.d, p0/m, z0.d, #imm with `tszh:tszl` = 0000 is reserved.
    assert_eq!(exec(&mut cpu, 0x0400_8000).map(|t| t.ec), Some(0x00));
}

/// `CurrentVL` is the Streaming SVE vector length, and a write of a Z
/// register zeroes it above that (`Unpredictable_SVEZEROUPPER` taken as
/// TRUE).
#[test]
fn the_vector_length_is_the_streaming_one() {
    for (len, bytes) in [(0, 16u64), (1, 32), (3, 64)] {
        let mut cpu = streaming(len);
        assert_eq!(exec(&mut cpu, CNTB_X0), None);
        assert_eq!(cpu.x[0], bytes, "cntb at LEN {len}");
        let vl = cpu.current_svl();
        cpu.set_z(0, VecLen::MAX, &Zv::replicate(0xFF, Esize::B, VecLen::MAX));
        assert_eq!(exec(&mut cpu, INDEX_Z0_B), None);
        let z = cpu.z(0, VecLen::MAX);
        for i in 0..VecLen::MAX.bytes() {
            let want = if i < vl.bytes() { i as u8 } else { 0 };
            assert_eq!(z.byte(i), want, "byte {i} at LEN {len}");
        }
    }
}

/// `DecodePredCount`: the fixed counts name no more elements than there
/// are, and none past them; MUL3 and MUL4 round down; PTRUES sets the
/// flags by `PredTest`.
#[test]
fn ptrue_counts_by_decode_pred_count() {
    let mut cpu = streaming(0);
    // VL7 of .D at 128 bits (2 elements): none.
    assert_eq!(exec(&mut cpu, ptrue(3, 0b00111, false)), None);
    assert_eq!(active(&cpu, Esize::D), [false, false]);
    // VL2 of .D: both.
    assert_eq!(exec(&mut cpu, ptrue(3, 0b00010, false)), None);
    assert_eq!(active(&cpu, Esize::D), [true, true]);
    // MUL3 of .B (16 elements): 15.
    assert_eq!(exec(&mut cpu, ptrue(0, 0b11110, false)), None);
    assert_eq!(active(&cpu, Esize::B).iter().filter(|&&a| a).count(), 15);
    assert!(!active(&cpu, Esize::B)[15]);
    // VL256 of .B at 128 bits: none, and PTRUES says so (Z set, C set).
    assert_eq!(exec(&mut cpu, ptrue(0, 0b01101, true)), None);
    assert!(active(&cpu, Esize::B).iter().all(|&a| !a));
    assert_eq!(cpu.pstate.nzcv(), 0x6000_0000);
    // ALL of .S with flags: N (first true), C clear (last true).
    assert_eq!(exec(&mut cpu, ptrue(2, 0b11111, true)), None);
    assert_eq!(cpu.pstate.nzcv(), 0x8000_0000);
    // An unallocated pattern names no element.
    assert_eq!(exec(&mut cpu, ptrue(0, 0b10101, false)), None);
    assert!(active(&cpu, Esize::B).iter().all(|&a| !a));
    // At 512 bits VL256 of .B is 256 of 64 elements: none; POW2 is all.
    let mut cpu = streaming(3);
    assert_eq!(exec(&mut cpu, ptrue(0, 0b01101, false)), None);
    assert!(active(&cpu, Esize::B).iter().all(|&a| !a));
    assert_eq!(exec(&mut cpu, ptrue(0, 0b00000, false)), None);
    assert!(active(&cpu, Esize::B).iter().all(|&a| a));
}

/// WHILE counts with the register's own width and wraps: WHILELE from
/// INT32_MAX - 1 to INT32_MAX stays true through the wrap to INT32_MIN,
/// which is less than INT32_MAX too. WHILELT stops at the limit.
#[test]
fn while_counts_with_the_register_width_through_the_wrap() {
    let mut cpu = streaming(0);
    // whilele p0.s, w0, w1.
    cpu.x[0] = 0x7FFF_FFFE;
    cpu.x[1] = 0x7FFF_FFFF;
    assert_eq!(exec(&mut cpu, 0x25A1_0410), None);
    assert_eq!(active(&cpu, Esize::S), [true, true, true, true]);
    // whilelt p0.s, w0, w1: only the first.
    assert_eq!(exec(&mut cpu, 0x25A1_0400), None);
    assert_eq!(active(&cpu, Esize::S), [true, false, false, false]);
    // N first active true, Z clear, C (not last) set.
    assert_eq!(cpu.pstate.nzcv(), 0xA000_0000);
    // whilelo p0.s, x0, x1 (64-bit, unsigned) from 0xFFFF_FFFF_FFFF_FFFE
    // to the same: none.
    cpu.x[0] = u64::MAX - 1;
    cpu.x[1] = u64::MAX - 1;
    assert_eq!(exec(&mut cpu, 0x25A1_1C00), None);
    assert_eq!(active(&cpu, Esize::S), [false, false, false, false]);
    assert_eq!(cpu.pstate.nzcv(), 0x6000_0000);
}

/// The 64-bit forms count through the register's own wrap too: WHILELE
/// from -2 to 0 counts -2, -1, 0 and stops at 1, and SVE2's WHILEGE from 1
/// to -1 counts 1, 0, -1 and stops at -2, the register passing through
/// all ones on the way.
#[test]
fn while_counts_through_the_64_bit_wrap() {
    let mut cpu = streaming(0);
    let first_three: Vec<bool> = (0..16).map(|e| e < 3).collect();
    // whilele p0.b, x0, x1.
    cpu.x[0] = (-2i64) as u64;
    cpu.x[1] = 0;
    assert_eq!(exec(&mut cpu, 0x2521_1410), None);
    assert_eq!(active(&cpu, Esize::B), first_three);
    // whilege p0.b, x0, x1: counted from the last element down.
    cpu.x[0] = 1;
    cpu.x[1] = (-1i64) as u64;
    assert_eq!(exec(&mut cpu, 0x2521_1000), None);
    let last_three: Vec<bool> = (0..16).map(|e| e >= 13).collect();
    assert_eq!(active(&cpu, Esize::B), last_three);
}

/// CTERMEQ and CTERMNE set N and V, and leave Z and C.
#[test]
fn cterm_sets_n_and_v_alone() {
    let mut cpu = streaming(0);
    // ctermeq x0, x1; ctermne x0, x1.
    let (eq, ne) = (0x25E1_2000, 0x25E1_2010);
    cpu.x[0] = 5;
    cpu.x[1] = 5;
    cpu.pstate.set_nzcv(0x6000_0000);
    assert_eq!(exec(&mut cpu, eq), None);
    assert_eq!(cpu.pstate.nzcv(), 0xE000_0000, "terminate: N set, V clear");
    cpu.pstate.set_nzcv(0x0000_0000);
    assert_eq!(exec(&mut cpu, ne), None);
    assert_eq!(cpu.pstate.nzcv(), 0x1000_0000, "continue: N clear, V = NOT C");
}

/// SDIV: a zero divisor gives zero, and INT_MIN / -1 wraps to INT_MIN.
#[test]
fn sdiv_by_zero_is_zero_and_int_min_by_minus_one_wraps() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    let mut dividend = Zv::ZERO;
    let mut divisor = Zv::ZERO;
    for (e, (a, b)) in [(7u64, 0u64), (0x8000_0000, 0xFFFF_FFFF), (0xFFFF_FFF9, 2), (9, 0xFFFF_FFFD)].into_iter().enumerate() {
        dividend.set(e, Esize::S, a);
        divisor.set(e, Esize::S, b);
    }
    cpu.set_z(0, vl, &dividend);
    cpu.set_z(1, vl, &divisor);
    cpu.set_p(0, vl, &Pv::ones(vl));
    // sdiv z0.s, p0/m, z0.s, z1.s.
    assert_eq!(exec(&mut cpu, 0x0494_0020), None);
    let z = cpu.z(0, vl);
    let got: Vec<u64> = (0..4).map(|e| z.get(e, Esize::S)).collect();
    assert_eq!(got, [0, 0x8000_0000, 0xFFFF_FFFD, 0xFFFF_FFFD]);
}

/// The 64-bit multiplies at their extremes: SQDMULH and SQRDMULH of
/// INT64_MIN by itself saturate to INT64_MAX (the doubled product is
/// 2^127), and SMULH and UMULH give the exact high halves.
#[test]
fn the_64_bit_multiplies_are_exact_at_their_extremes() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    let mut a = Zv::ZERO;
    let mut b = Zv::ZERO;
    for (e, (x, y)) in [(0x8000_0000_0000_0000u64, 0x8000_0000_0000_0000u64), (u64::MAX, u64::MAX)].into_iter().enumerate() {
        a.set(e, Esize::D, x);
        b.set(e, Esize::D, y);
    }
    cpu.set_z(1, vl, &a);
    cpu.set_z(2, vl, &b);
    let d_lanes = |cpu: &Cpu| {
        let z = cpu.z(0, vl);
        [z.get(0, Esize::D), z.get(1, Esize::D)]
    };
    // sqdmulh z0.d, z1.d, z2.d; sqrdmulh z0.d, z1.d, z2.d.
    assert_eq!(exec(&mut cpu, 0x04E2_7020), None);
    assert_eq!(d_lanes(&cpu), [0x7FFF_FFFF_FFFF_FFFF, 0], "sqdmulh");
    assert_eq!(exec(&mut cpu, 0x04E2_7420), None);
    assert_eq!(d_lanes(&cpu), [0x7FFF_FFFF_FFFF_FFFF, 0], "sqrdmulh: -1 * -1 doubled and rounded is 0");
    // smulh z0.d, z1.d, z2.d; umulh z0.d, z1.d, z2.d.
    assert_eq!(exec(&mut cpu, 0x04E2_6820), None);
    assert_eq!(d_lanes(&cpu), [0x4000_0000_0000_0000, 0], "smulh");
    assert_eq!(exec(&mut cpu, 0x04E2_6C20), None);
    assert_eq!(d_lanes(&cpu), [0x4000_0000_0000_0000, 0xFFFF_FFFF_FFFF_FFFE], "umulh");
    // umulh z1.d, p0/m, z1.d, z2.d (predicated).
    cpu.set_p(0, vl, &Pv::ones(vl));
    assert_eq!(exec(&mut cpu, 0x04D3_0041), None);
    let z = cpu.z(1, vl);
    assert_eq!([z.get(0, Esize::D), z.get(1, Esize::D)], [0x4000_0000_0000_0000, 0xFFFF_FFFF_FFFF_FFFE]);
}

/// EXT with an offset at or beyond the vector length gives the first
/// source; within it, the bytes from the offset on.
#[test]
fn ext_beyond_the_vector_gives_the_first_source() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    let mut a = Zv::ZERO;
    let mut b = Zv::ZERO;
    for i in 0..16 {
        a.set_byte(i, i as u8);
        b.set_byte(i, 0x80 | i as u8);
    }
    cpu.set_z(0, vl, &a);
    cpu.set_z(1, vl, &b);
    // ext z0.b, z0.b, z1.b, #255.
    assert_eq!(exec(&mut cpu, 0x053F_1C20), None);
    assert_eq!(cpu.z(0, vl).low128(), a.low128());
    // ext z0.b, z0.b, z1.b, #3.
    assert_eq!(exec(&mut cpu, 0x0520_0C20), None);
    let z = cpu.z(0, vl);
    let got: Vec<u8> = (0..16).map(|i| z.byte(i)).collect();
    assert_eq!(got, [3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0x80, 0x81, 0x82]);
}

/// DUP (indexed): an index beyond the vector gives zeros; the 128-bit
/// form broadcasts a quadword.
#[test]
fn dup_indexed_beyond_the_vector_is_zero() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    let mut src = Zv::ZERO;
    for i in 0..16 {
        src.set_byte(i, 0xA0 | i as u8);
    }
    cpu.set_z(1, vl, &src);
    cpu.set_z(0, vl, &Zv::replicate(0xFF, Esize::B, vl));
    // dup z0.b, z1.b[63]: element 63 of 16.
    assert_eq!(exec(&mut cpu, 0x05FF_2020), None);
    assert_eq!(cpu.z(0, vl).low128(), 0);
    // dup z0.q, z1.q[0] at 512 bits: the low quadword four times.
    let mut cpu = streaming(3);
    let vl = cpu.current_svl();
    cpu.set_z(1, vl, &src);
    assert_eq!(exec(&mut cpu, 0x0530_2020), None);
    let z = cpu.z(0, vl);
    for i in 0..vl.bytes() {
        assert_eq!(z.byte(i), 0xA0 | (i % 16) as u8, "byte {i}");
    }
    // dup z0.q, z1.q[3]: the fourth, which is zero.
    assert_eq!(exec(&mut cpu, 0x05F0_2020), None);
    assert!((0..vl.bytes()).all(|i| cpu.z(0, vl).byte(i) == 0));
}

/// SQINCB (32-bit, `Xdn, Wdn`): the count added to `Wdn` as a signed
/// 32-bit value, saturated to 32 bits and sign-extended.
#[test]
fn sqincb_saturates_to_32_bits_and_sign_extends() {
    let mut cpu = streaming(3);
    // sqincb x0, w0 (ALL, MUL #1): +64 at 512 bits.
    cpu.x[0] = 0xDEAD_BEEF_7FFF_FFF0;
    assert_eq!(exec(&mut cpu, 0x0420_F3E0), None);
    assert_eq!(cpu.x[0], 0x7FFF_FFFF);
    cpu.x[0] = 0x8000_0000;
    assert_eq!(exec(&mut cpu, 0x0420_F3E0), None);
    assert_eq!(cpu.x[0], 0xFFFF_FFFF_8000_0040);
}

/// PSEL: the index register plus the immediate, modulo the elements.
#[test]
fn psel_index_wraps_modulo_the_element_count() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    let mut pn = Pv::ZERO;
    pn.set_bit(5, true);
    let mut pm = Pv::ZERO;
    pm.set_bit(3, true);
    cpu.set_p(1, vl, &pn);
    cpu.set_p(2, vl, &pm);
    // psel p0, p1, p2.b[w12, 0]: element (w12 + 0) MOD 16.
    let psel = 0x2524_4440;
    cpu.x[12] = 0xFFFF_FFFF_0000_0013;
    assert_eq!(exec(&mut cpu, psel), None);
    assert_eq!(cpu.p(0, vl).as_bytes()[..2], [0x20, 0x00], "w12 = 19: element 3, active");
    cpu.x[12] = 4;
    assert_eq!(exec(&mut cpu, psel), None);
    assert_eq!(cpu.p(0, vl).as_bytes()[..2], [0x00, 0x00], "element 4, inactive");
}
