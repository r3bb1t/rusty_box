//! The SVE instructions in Streaming SVE mode, on a processor that
//! implements FEAT_SME and not FEAT_SVE ([`Implementation::EVERY_FEATURE`]).
//!
//! tests/vectors/qemu_sve.txt checks every class against QEMU at one vector
//! length with random operands. These are the properties random operands
//! rarely or never reach, each the guest-visible one (R9), each against
//! its Arm pseudocode: the enable checks (`CheckSVEEnabled`,
//! `CheckStreamingSVEEnabled`), the vector length (`CurrentVL`, the
//! `SVEZEROUPPER` choice), `DecodePredCount`, and the boundaries of WHILE,
//! CTERM, SDIV, EXT, DUP (indexed), SQINCB and PSEL; of the loads and
//! stores, the inactive elements, a fault part-way, the register numbers
//! wrapping, the alignment of LDR and STR, the SP check with no element
//! active, and the accesses a prefetch and an all-inactive load do not
//! make; and of floating point, the order of a reduction and the identity
//! of its inactive elements, the arithmetic immediates, FSCALE's clamp,
//! FLOGB's specials and denormals, the
//! container a conversion writes, the multiply-add negations, FCMLA's
//! rotations, the indexed segments, FPCR.AHP ignored, the exceptions of
//! the active elements alone, and `size = 00` UNDEFINED; and of SVE2's
//! integer instructions, the extremes of the doubling multiplies, the
//! shift by the whole element, the reversed and pairwise operands, the
//! mixed-sign and halving ranges, the saturation of the product before the
//! accumulate, the dot products' signedness and wrap, the complex
//! rotations, the indexed source elements, the clamp's order, the
//! elements of the long, wide and interleaved forms, PMULL, the carry
//! chain, the shift accumulates and inserts at the element size, the
//! narrowing halves, and the reserved sizes and `tsize` values. Of the
//! SVE2.1 instructions FEAT_SME2 makes legal: the two-way dot products'
//! pairs and wrap, FDOT's two roundings and the exceptions it raises,
//! BFMLSL's negation and its NaN under FPCR.AH, and FEAT_SME2 itself.

use std::vec::Vec;

use super::super::features::{Feature, Features, Implementation, VecLen};
use super::super::fp_estimate::fpsr::{DZC, IDC, IOC, IXC, OFC, UFC};
use super::super::sctlr;
use crate::softfloat::FPCR_AH;
use super::super::step::{exec, exec_in, processor, Taken, CPACR_ALL, DATA, FP_TRAP, SME_DISABLED, UNDEFINED};
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

// ----- loads, stores and prefetches -----

/// A Data Abort from the current exception level (EC 0x25) with `dfsc`,
/// of a write when `write`.
const fn data_abort(dfsc: u64, write: bool) -> Taken {
    Taken {
        ec: 0x25,
        iss: dfsc | ((write as u64) << 6),
    }
}

/// The DFSC of an alignment fault and of a synchronous external abort.
const ALIGNMENT: u64 = 0x21;
const EXTERNAL: u64 = 0x10;

/// An address the bus maps nothing at.
const UNMAPPED: u64 = 0x9000_0000;

/// `size` bytes of RAM counting up from 0.
fn counting(size: usize) -> Vec<u8> {
    (0..size).map(|i| i as u8).collect()
}

/// The elements of `es` of `Z<n>` at the current length.
fn elements(cpu: &Cpu, n: u32, es: Esize) -> Vec<u64> {
    let vl = cpu.current_svl();
    let z = cpu.z(n, vl);
    (0..es.elements(vl)).map(|e| z.get(e, es)).collect()
}

/// LD1 zeroes its inactive elements; ST1 writes its active ones alone, at
/// a multiple of the vector's bytes past the base.
#[test]
fn ld1_zeroes_inactive_elements_and_st1_writes_the_active_ones_alone() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    let mut ram = counting(64);
    cpu.x[0] = DATA;
    // P0: words 0 and 2.
    cpu.set_p(0, vl, &Pv::from_bytes(&[0x01, 0x01]));
    cpu.set_z(0, vl, &Zv::replicate(0xFFFF_FFFF, Esize::S, vl));
    // ld1w {z0.s}, p0/z, [x0].
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xA540_A000), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x0302_0100, 0, 0x0B0A_0908, 0]);
    // st1w {z1.s}, p0, [x0, #1, mul vl]: 16 bytes on.
    cpu.set_z(1, vl, &Zv::replicate(0xA1A2_A3A4, Esize::S, vl));
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE541_E001), None);
    let mut want = counting(64);
    want[16..20].copy_from_slice(&[0xA4, 0xA3, 0xA2, 0xA1]);
    want[24..28].copy_from_slice(&[0xA4, 0xA3, 0xA2, 0xA1]);
    assert_eq!(ram, want);
}

/// LD1SB sign-extends a byte to the element; ST1B stores an element's low
/// byte.
#[test]
fn ld1sb_sign_extends_and_st1b_stores_the_low_byte() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    let mut ram = counting(64);
    ram[0] = 0x80;
    ram[1] = 0x7F;
    cpu.x[0] = DATA;
    cpu.set_p(0, vl, &Pv::ones(vl));
    // ld1sb {z0.d}, p0/z, [x0].
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xA580_A000), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [0xFFFF_FFFF_FFFF_FF80, 0x7F]);
    // st1b {z0.d}, p0, [x0, #1, mul vl]: two bytes on.
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE461_E000), None);
    assert_eq!(ram[..4], [0x80, 0x7F, 0x80, 0x7F]);
}

/// A load that faults part-way has written no register; a store that does
/// has written the elements before the fault. The fault reports the
/// faulting element's address.
#[test]
fn a_fault_part_way_writes_no_register_and_keeps_the_stores_before_it() {
    let mut cpu = streaming(1);
    let vl = cpu.current_svl();
    let mut ram = counting(64);
    // Four doublewords from 16 bytes before the end of the RAM.
    cpu.x[0] = DATA + 48;
    cpu.set_p(0, vl, &Pv::ones(vl));
    cpu.set_z(0, vl, &Zv::replicate(0x5555, Esize::D, vl));
    // ld1d {z0.d}, p0/z, [x0].
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xA5E0_A000), Some(data_abort(EXTERNAL, false)));
    assert_eq!(cpu.sys.far_el1, DATA + 64);
    assert_eq!(elements(&cpu, 0, Esize::D), [0x5555; 4]);
    // st1d {z1.d}, p0, [x0].
    cpu.set_z(1, vl, &Zv::replicate(0xA5A5_A5A5_A5A5_A5A5, Esize::D, vl));
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE5E0_E001), Some(data_abort(EXTERNAL, true)));
    assert_eq!(cpu.sys.far_el1, DATA + 64);
    assert_eq!(ram[..48], counting(48)[..]);
    assert!(ram[48..].iter().all(|&b| b == 0xA5));
}

/// LDR and STR move `VL DIV 8` bytes of a vector and `PL DIV 8` of a
/// predicate, at the immediate times that length.
#[test]
fn ldr_and_str_move_the_register_length_in_bytes() {
    let mut cpu = streaming(1);
    let vl = cpu.current_svl();
    let mut ram = counting(64);
    cpu.x[0] = DATA;
    // ldr z0, [x0, #1, mul vl]: bytes 32 to 63.
    assert_eq!(exec_in(&mut cpu, &mut ram, 0x8580_4400), None);
    let z0 = cpu.z(0, vl);
    assert_eq!((0..32).map(|i| z0.byte(i)).collect::<Vec<u8>>(), counting(64)[32..]);
    // ldr p1, [x0]: four bytes.
    assert_eq!(exec_in(&mut cpu, &mut ram, 0x8580_0001), None);
    assert_eq!(cpu.p(1, vl).as_bytes()[..4], [0, 1, 2, 3]);
    // str p1, [x0, #1, mul vl]: to bytes 4 to 7.
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xE580_0401), None);
    assert_eq!(ram[..12], [0, 1, 2, 3, 0, 1, 2, 3, 8, 9, 10, 11]);
}

/// LDR's transfer is misaligned when its base is, whatever the byte: it
/// faults at its first byte when SCTLR_EL1.A enforces alignment, and on
/// Device memory (all of it, with the MMU off) every byte is refused. A
/// predicate's transfer needs two-byte alignment alone.
#[test]
fn a_misaligned_ldr_faults_when_enforced_and_on_device_memory() {
    let mut cpu = streaming(0);
    let mut ram = counting(64);
    // ldr z0, [x0, #1, mul vl] from a base one byte past alignment.
    let ldr_z = 0x8580_4400;
    cpu.x[0] = DATA + 1;
    cpu.sys.sctlr_el1 |= sctlr::A;
    assert_eq!(exec_in(&mut cpu, &mut ram, ldr_z), Some(data_abort(ALIGNMENT, false)));
    assert_eq!(cpu.sys.far_el1, DATA + 17);
    cpu.sys.sctlr_el1 &= !sctlr::A;
    cpu.sys.far_el1 = 0;
    assert_eq!(exec_in(&mut cpu, &mut ram, ldr_z), Some(data_abort(ALIGNMENT, false)));
    assert_eq!(cpu.sys.far_el1, DATA + 17);
    cpu.x[0] = DATA;
    assert_eq!(exec_in(&mut cpu, &mut ram, ldr_z), None);
    // ldr p0, [x0]: two-byte alignment.
    cpu.x[0] = DATA + 2;
    assert_eq!(exec_in(&mut cpu, &mut ram, 0x8580_0000), None);
    cpu.x[0] = DATA + 1;
    assert_eq!(exec_in(&mut cpu, &mut ram, 0x8580_0000), Some(data_abort(ALIGNMENT, false)));
}

/// On Normal memory a misaligned LDR moves its bytes, unless SCTLR_EL1.A
/// enforces alignment, when it faults at its first byte.
#[test]
fn a_misaligned_ldr_on_normal_memory_faults_only_when_enforced() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    // The MMU on, its tables at the start of the RAM: the level-1 table's
    // first entry points to a level-2 table whose first entry maps the low
    // 2 MiB, code and RAM included, as Normal memory (attribute 0).
    let mut ram = counting(0x3000);
    ram[..8].copy_from_slice(&((DATA + 0x1000) | 0b11).to_le_bytes());
    const AF: u64 = 1 << 10;
    const INNER_SHAREABLE: u64 = 3 << 8;
    ram[0x1000..0x1008].copy_from_slice(&(AF | INNER_SHAREABLE | 0b01).to_le_bytes());
    cpu.sys.mair_el1 = 0xFF;
    // T0SZ = 25: walks start at level 1; TTBR1 walks disabled (EPD1).
    cpu.sys.tcr_el1 = 25 | (1 << 23);
    cpu.sys.ttbr0_el1 = DATA;
    cpu.sys.sctlr_el1 = (cpu.sys.sctlr_el1 | sctlr::M) & !sctlr::A;
    // ldr z0, [x0] from a base one byte past alignment.
    let ldr_z = 0x8580_4000;
    cpu.x[0] = DATA + 0x2001;
    assert_eq!(exec_in(&mut cpu, &mut ram, ldr_z), None);
    let z0 = cpu.z(0, vl);
    assert_eq!((0..16).map(|i| z0.byte(i)).collect::<Vec<u8>>(), ram[0x2001..0x2011]);
    cpu.sys.sctlr_el1 |= sctlr::A;
    assert_eq!(exec_in(&mut cpu, &mut ram, ldr_z), Some(data_abort(ALIGNMENT, false)));
    assert_eq!(cpu.sys.far_el1, DATA + 0x2001);
}

/// LD1RQ reads a quadword under the low 16 bits of the predicate, the
/// bits above them ignored, its inactive elements zero, into every
/// quadword of the vector.
#[test]
fn ld1rq_broadcasts_a_quadword_under_the_low_sixteen_predicate_bits() {
    let mut cpu = streaming(3);
    let vl = cpu.current_svl();
    let mut ram = counting(64);
    cpu.x[0] = DATA;
    // Words 1 and 3 of the quadword; every bit above it set.
    let mut p0 = [0xFFu8; 8];
    p0[..2].copy_from_slice(&[0x10, 0x10]);
    cpu.set_p(0, vl, &Pv::from_bytes(&p0));
    // ld1rqw {z0.s}, p0/z, [x0].
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xA500_2000), None);
    let quadword = [0, 0x0706_0504, 0, 0x0F0E_0D0C];
    let want: Vec<u64> = quadword.iter().copied().cycle().take(16).collect();
    assert_eq!(elements(&cpu, 0, Esize::S), want);
}

/// With no element active, LD1R and LD1 read nothing, so an unmapped base
/// does not fault, and the result is zero; one active element reads.
#[test]
fn a_load_with_no_element_active_reads_nothing() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    let mut ram = counting(64);
    cpu.x[0] = UNMAPPED;
    cpu.set_p(0, vl, &Pv::ZERO);
    cpu.set_z(0, vl, &Zv::replicate(0xFF, Esize::B, vl));
    // ld1rb {z0.b}, p0/z, [x0].
    assert_eq!(exec_in(&mut cpu, &mut ram, 0x8440_8000), None);
    assert_eq!(elements(&cpu, 0, Esize::B), [0; 16]);
    // ld1b {z0.b}, p0/z, [x0].
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xA400_A000), None);
    let mut p0 = Pv::ZERO;
    p0.set_bit(5, true);
    cpu.set_p(0, vl, &p0);
    assert_eq!(exec_in(&mut cpu, &mut ram, 0x8440_8000), Some(data_abort(EXTERNAL, false)));
    assert_eq!(cpu.sys.far_el1, UNMAPPED);
}

/// LD3 fills three consecutive registers, wrapping from Z31 to Z0, from
/// interleaved elements.
#[test]
fn ld3_interleaves_and_wraps_the_register_numbers() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    let mut ram = counting(64);
    cpu.x[0] = DATA;
    cpu.set_p(0, vl, &Pv::ones(vl));
    // ld3b {z30.b, z31.b, z0.b}, p0/z, [x0].
    assert_eq!(exec_in(&mut cpu, &mut ram, 0xA440_E01E), None);
    for (r, z) in [30, 31, 0].into_iter().enumerate() {
        let want: Vec<u64> = (0..16).map(|e| 3 * e + r as u64).collect();
        assert_eq!(elements(&cpu, z, Esize::B), want, "z{z}");
    }
}

/// ST1H of byte elements (`size = 00`) is UNDEFINED, which the decode
/// raises before the enable check.
#[test]
fn st1h_of_byte_elements_is_undefined_before_the_sme_trap() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    // st1h {z0.b}, p0, [x0].
    assert_eq!(exec(&mut cpu, 0xE480_E000), Some(UNDEFINED));
}

/// A prefetch reads nothing, whatever its address, and outside Streaming
/// SVE mode takes the SME trap like any other SVE instruction.
#[test]
fn a_prefetch_reads_nothing() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    cpu.x[0] = UNMAPPED;
    cpu.set_p(0, vl, &Pv::ones(vl));
    // prfb pldl1keep, p0, [x0].
    let prfb = 0x85C0_0000;
    assert_eq!(exec(&mut cpu, prfb), None);
    cpu.set_pstate_sm(false);
    assert_eq!(exec(&mut cpu, prfb), Some(NOT_STREAMING));
}

/// An SP base takes the SP alignment check even with no element active
/// (`Unpredictable_CHECKSPNONEACTIVE` taken as TRUE).
#[test]
fn an_sp_base_is_checked_with_no_element_active() {
    let mut cpu = streaming(0);
    let vl = cpu.current_svl();
    cpu.set_p(0, vl, &Pv::ZERO);
    cpu.sys.sctlr_el1 |= sctlr::SA;
    cpu.sp_el1 = DATA + 8;
    // ld1b {z0.b}, p0/z, [sp].
    assert_eq!(exec(&mut cpu, 0xA400_A3E0), Some(Taken { ec: 0x26, iss: 0 }));
}

// ----- Floating point -----

/// FPCR.FZ and FPCR.AHP.
const FPCR_FZ: u64 = 1 << 24;
const FPCR_AHP: u64 = 1 << 26;

/// `Z<n>` at the current length: the elements of `es` from `values`,
/// repeated.
fn set_elements(cpu: &mut Cpu, n: u32, es: Esize, values: &[u64]) {
    let vl = cpu.current_svl();
    let mut z = Zv::ZERO;
    for e in 0..es.elements(vl) {
        z.set(e, es, values[e % values.len()]);
    }
    cpu.set_z(n, vl, &z);
}

/// A processor in Streaming SVE mode with `SMCR_EL1.LEN = len`, P0 all
/// true, FPCR `fpcr` and FPSR clear.
fn fp_streaming(len: u64, fpcr: u64) -> Cpu {
    let mut cpu = streaming(len);
    let vl = cpu.current_svl();
    cpu.set_p(0, vl, &Pv::ones(vl));
    cpu.fpcr = fpcr;
    cpu.fpsr = 0;
    cpu
}

/// FADDV reduces by recursive halves (`FPReduce`): at 128 bits, the sum
/// of `e0 + e1` and `e2 + e3`, which for 1e8, 1, -1e8, 1 is 0 where a
/// running sum is 1; the scalar is written to Vd, the rest of Zd zeroed.
/// An inactive element is the operation's identity: FMINV's +infinity,
/// FMAXV's -infinity, FMAXNMV's default NaN, negative under FPCR.AH.
#[test]
fn faddv_reduces_by_halves_and_inactive_elements_are_the_identity() {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 1, Esize::S, &[0x4CBE_BC20, 0x3F80_0000, 0xCCBE_BC20, 0x3F80_0000]);
    set_elements(&mut cpu, 0, Esize::S, &[0xFFFF_FFFF]);
    // faddv s0, p0, z1.s.
    assert_eq!(exec(&mut cpu, 0x6580_2020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0, 0, 0, 0]);
    assert_eq!(cpu.fpsr, IXC);
    // At 512 bits nothing above the scalar survives either.
    let mut cpu = fp_streaming(3, 0);
    set_elements(&mut cpu, 0, Esize::B, &[0xFF]);
    set_elements(&mut cpu, 1, Esize::S, &[0x3F80_0000]);
    assert_eq!(exec(&mut cpu, 0x6580_2020), None);
    let mut want = [0u64; 16];
    want[0] = 0x4180_0000;
    assert_eq!(elements(&cpu, 0, Esize::S), want);

    let mut cpu = fp_streaming(0, 0);
    let vl = cpu.current_svl();
    cpu.set_p(1, vl, &Pv::ZERO);
    // fminv s2, p1, z1.s; fmaxv s3, p1, z1.s; fmaxnmv s4, p1, z1.s.
    assert_eq!(exec(&mut cpu, 0x6587_2422), None);
    assert_eq!(exec(&mut cpu, 0x6586_2423), None);
    assert_eq!(exec(&mut cpu, 0x6584_2424), None);
    assert_eq!(elements(&cpu, 2, Esize::S)[0], 0x7F80_0000);
    assert_eq!(elements(&cpu, 3, Esize::S)[0], 0xFF80_0000);
    assert_eq!(elements(&cpu, 4, Esize::S)[0], 0x7FC0_0000);
    assert_eq!(cpu.fpsr, 0);
    cpu.fpcr = FPCR_AH;
    assert_eq!(exec(&mut cpu, 0x6584_2424), None);
    assert_eq!(elements(&cpu, 4, Esize::S)[0], 0xFFC0_0000);

    // FADDV's identity is +0: of all -0 the sum is -0, but with one
    // element inactive it is -0 + +0 = +0.
    let mut cpu = fp_streaming(0, 0);
    let vl = cpu.current_svl();
    set_elements(&mut cpu, 1, Esize::S, &[0x8000_0000]);
    assert_eq!(exec(&mut cpu, 0x6580_2020), None);
    assert_eq!(elements(&cpu, 0, Esize::S)[0], 0x8000_0000);
    cpu.set_p(0, vl, &Pv::from_bytes(&[0x11, 0x01]));
    assert_eq!(exec(&mut cpu, 0x6580_2020), None);
    assert_eq!(elements(&cpu, 0, Esize::S)[0], 0);
}

/// The predicated arithmetic immediates are Arm's two per operation
/// (`i1`): FADD, FSUB, FSUBR 0.5 and 1.0, FMUL 0.5 and 2.0, FMAXNM,
/// FMINNM, FMAX, FMIN 0.0 and 1.0.
#[test]
fn the_arithmetic_immediates_are_each_operations_pair() {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::S, &[0x4040_0000]);
    // fmul z0.s, p0/m, z0.s, #2.0; then #0.5.
    assert_eq!(exec(&mut cpu, 0x659A_8020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x40C0_0000; 4]);
    assert_eq!(exec(&mut cpu, 0x659A_8000), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x4040_0000; 4]);
    // fadd z0.s, p0/m, z0.s, #0.5; then #1.0.
    assert_eq!(exec(&mut cpu, 0x6598_8000), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x4060_0000; 4]);
    assert_eq!(exec(&mut cpu, 0x6598_8020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x4090_0000; 4]);
    // fmax z0.s, p0/m, z0.s, #0.0 of -1.0; then #1.0.
    set_elements(&mut cpu, 0, Esize::S, &[0xBF80_0000]);
    assert_eq!(exec(&mut cpu, 0x659E_8000), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0; 4]);
    assert_eq!(exec(&mut cpu, 0x659E_8020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x3F80_0000; 4]);
}

/// FSCALE multiplies by 2^SInt(Zm element), the scale clamped first
/// (`FPClampScale`) and the result rounded once (`FPRound`): 2^-1 halves,
/// 2^(2^31 - 1) overflows to infinity, 3.0 x 2^-150 rounds to the even
/// denormal 2, and 2^-2^31 underflows to zero.
#[test]
fn fscale_clamps_the_scale_and_rounds_once() {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::S, &[0x3F80_0000, 0x3F80_0000, 0x4040_0000, 0x3F80_0000]);
    set_elements(&mut cpu, 1, Esize::S, &[0xFFFF_FFFF, 0x7FFF_FFFF, 0xFFFF_FF6A, 0x8000_0000]);
    // fscale z0.s, p0/m, z0.s, z1.s.
    assert_eq!(exec(&mut cpu, 0x6589_8020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x3F00_0000, 0x7F80_0000, 0x0000_0002, 0]);
    assert_eq!(cpu.fpsr, OFC | UFC | IXC);

    // Double precision clamps by its own exponent range: 1.0 x 2^1 is 2.0,
    // and 1.0 x 2^-1074 the smallest denormal, exactly.
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::D, &[0x3FF0_0000_0000_0000]);
    set_elements(&mut cpu, 1, Esize::D, &[1, 0xFFFF_FFFF_FFFF_FBCE]);
    // fscale z0.d, p0/m, z0.d, z1.d.
    assert_eq!(exec(&mut cpu, 0x65C9_8020), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [0x4000_0000_0000_0000, 1]);
    assert_eq!(cpu.fpsr, 0);

    // Half precision: 2.0; 3.0 x 2^-25 rounds to the even denormal 2;
    // 2^32767 overflows and 2^-32768 underflows.
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::H, &[0x3C00, 0x4200, 0x3C00, 0x3C00]);
    set_elements(&mut cpu, 1, Esize::H, &[0x0001, 0xFFE7, 0x7FFF, 0x8000]);
    // fscale z0.h, p0/m, z0.h, z1.h.
    assert_eq!(exec(&mut cpu, 0x6549_8020), None);
    assert_eq!(elements(&cpu, 0, Esize::H), [0x4000, 0x0002, 0x7C00, 0, 0x4000, 0x0002, 0x7C00, 0]);
    assert_eq!(cpu.fpsr, OFC | UFC | IXC);
}

/// FLOGB gives the unbiased exponent (`FPLogB`), a denormal's from its
/// leading bit; MaxInt for an infinity; MinInt with Invalid Operation for
/// a zero, and for a denormal FPCR.FZ flushes, with Input Denormal too.
#[test]
fn flogb_gives_the_exponent_and_the_extremes_for_the_specials() {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 1, Esize::S, &[0x4100_0000, 0x0000_0001, 0x7F80_0000, 0]);
    // flogb z0.s, p0/m, z1.s.
    assert_eq!(exec(&mut cpu, 0x651C_A020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [3, 0xFFFF_FF6B, 0x7FFF_FFFF, 0x8000_0000]);
    assert_eq!(cpu.fpsr, IOC);
    let mut cpu = fp_streaming(0, FPCR_FZ);
    set_elements(&mut cpu, 1, Esize::S, &[0x0000_0001]);
    assert_eq!(exec(&mut cpu, 0x651C_A020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x8000_0000; 4]);
    assert_eq!(cpu.fpsr, IOC | IDC);

    // Half precision: 2^-24 and 2^-15, both denormal, give -24 and -15.
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 1, Esize::H, &[0x0001, 0x0200, 0x4800, 0x7C00]);
    // flogb z0.h, p0/m, z1.h.
    assert_eq!(exec(&mut cpu, 0x651A_A020), None);
    assert_eq!(elements(&cpu, 0, Esize::H), [0xFFE8, 0xFFF1, 3, 0x7FFF, 0xFFE8, 0xFFF1, 3, 0x7FFF]);
    // Double precision: 2^-1074 gives -1074.
    set_elements(&mut cpu, 1, Esize::D, &[1, 0x4020_0000_0000_0000]);
    // flogb z0.d, p0/m, z1.d.
    assert_eq!(exec(&mut cpu, 0x651E_A020), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [0xFFFF_FFFF_FFFF_FBCE, 3]);
    assert_eq!(cpu.fpsr, 0);
}

/// FCVTZS to 32 bits of a double element rounds toward zero, saturates and
/// sign-extends into the 64-bit container (`Extend(res, unsigned)`);
/// FCVTZU zero-extends, a negative value saturating to 0 with Invalid
/// Operation alone.
#[test]
fn fcvtzs_to_32_bits_saturates_and_sign_extends_into_the_container() {
    let mut cpu = fp_streaming(1, 0);
    set_elements(
        &mut cpu,
        1,
        Esize::D,
        &[0xBFF8_0000_0000_0000, 0x41E6_5A0B_C000_0000, 0xC1E6_5A0B_C000_0000, 0x4000_0000_0000_0000],
    );
    // fcvtzs z0.s, p0/m, z1.d.
    assert_eq!(exec(&mut cpu, 0x65D8_A020), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [u64::MAX, 0x7FFF_FFFF, 0xFFFF_FFFF_8000_0000, 2]);
    assert_eq!(cpu.fpsr, IOC | IXC);
    cpu.fpsr = 0;
    // fcvtzu z2.s, p0/m, z1.d.
    assert_eq!(exec(&mut cpu, 0x65D9_A022), None);
    assert_eq!(elements(&cpu, 2, Esize::D), [0, 0xB2D0_5E00, 0, 2]);
    assert_eq!(cpu.fpsr, IOC);
}

/// The predicated multiply-adds negate the multiplicand (`op1_neg`) and
/// the addend (`op3_neg`) by `opc`: FMLA, FMLS, FNMLA, FNMLS of 1 + 2 x 3
/// are 7, -5, -7, 5. FMAD and its siblings multiply Zdn by Zm and add Za:
/// of 1 + 4 x 2 they are 9, -7, -9, 7.
#[test]
fn the_multiply_adds_negate_by_opc_and_take_their_operands_by_form() {
    for (opc, want) in [(0, 0x40E0_0000), (1, 0xC0A0_0000), (2, 0xC0E0_0000), (3, 0x40A0_0000)] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::S, &[0x3F80_0000]);
        set_elements(&mut cpu, 1, Esize::S, &[0x4000_0000]);
        set_elements(&mut cpu, 2, Esize::S, &[0x4040_0000]);
        // fmla z0.s, p0/m, z1.s, z2.s, and the others by `opc`.
        assert_eq!(exec(&mut cpu, 0x65A2_0020 | (opc << 13)), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [want; 4], "opc {opc}");
    }
    for (opc, want) in [(0, 0x4110_0000), (1, 0xC0E0_0000), (2, 0xC110_0000), (3, 0x40E0_0000)] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::S, &[0x4080_0000]);
        set_elements(&mut cpu, 1, Esize::S, &[0x4000_0000]);
        set_elements(&mut cpu, 2, Esize::S, &[0x3F80_0000]);
        // fmad z0.s, p0/m, z1.s, z2.s, and the others by `opc`.
        assert_eq!(exec(&mut cpu, 0x65A2_8020 | (opc << 13)), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [want; 4], "opc {opc}");
    }
}

/// FCMLA rotates Zm's pair: of (1 + 2i)(3 + 4i) terms, #0 adds (3, 4), #90
/// (-8, 6), #180 (-3, -4), #270 (8, -6). Each half of a pair is governed
/// by its own element: with the imaginary one alone active, the real half
/// keeps Zda's value.
#[test]
fn fcmla_rotates_and_governs_each_half_by_its_own_element() {
    let rotations = [
        (0, [0x4040_0000, 0x4080_0000]),
        (1, [0xC100_0000, 0x40C0_0000]),
        (2, [0xC040_0000, 0xC080_0000]),
        (3, [0x4100_0000, 0xC0C0_0000]),
    ];
    for (rot, [real, imag]) in rotations {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 1, Esize::S, &[0x3F80_0000, 0x4000_0000]);
        set_elements(&mut cpu, 2, Esize::S, &[0x4040_0000, 0x4080_0000]);
        // fcmla z0.s, p0/m, z1.s, z2.s, #(90 * rot).
        assert_eq!(exec(&mut cpu, 0x6482_0020 | (rot << 13)), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [real, imag, real, imag], "rot {rot}");
    }
    let mut cpu = fp_streaming(0, 0);
    let vl = cpu.current_svl();
    cpu.set_p(0, vl, &Pv::from_bytes(&[0x10, 0x00]));
    set_elements(&mut cpu, 0, Esize::S, &[0x42C8_0000]);
    set_elements(&mut cpu, 1, Esize::S, &[0x3F80_0000, 0x4000_0000]);
    set_elements(&mut cpu, 2, Esize::S, &[0x4040_0000, 0x4080_0000]);
    assert_eq!(exec(&mut cpu, 0x6482_0020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x42C8_0000, 0x42D0_0000, 0x42C8_0000, 0x42C8_0000]);
}

/// The indexed forms take Zm's element (or pair) `index` of each 128-bit
/// segment: at 256 bits FMLA by element 3 multiplies the first four
/// elements by Zm's 4.0 and the next four by its 8.0, and FCMLA by pair 1
/// takes pair 1 and pair 3.
#[test]
fn the_indexed_forms_take_their_element_from_each_segment() {
    let counting: [u64; 8] = [
        0x3F80_0000, 0x4000_0000, 0x4040_0000, 0x4080_0000, 0x40A0_0000, 0x40C0_0000, 0x40E0_0000, 0x4100_0000,
    ];
    let mut cpu = fp_streaming(1, 0);
    set_elements(&mut cpu, 0, Esize::S, &[0]);
    set_elements(&mut cpu, 1, Esize::S, &[0x3F80_0000]);
    set_elements(&mut cpu, 2, Esize::S, &counting);
    // fmla z0.s, z1.s, z2.s[3].
    assert_eq!(exec(&mut cpu, 0x64BA_0020), None);
    let (four, eight) = (0x4080_0000, 0x4100_0000);
    assert_eq!(elements(&cpu, 0, Esize::S), [four, four, four, four, eight, eight, eight, eight]);

    let mut cpu = fp_streaming(1, 0);
    set_elements(&mut cpu, 0, Esize::S, &[0]);
    set_elements(&mut cpu, 1, Esize::S, &[0x3F80_0000, 0]);
    set_elements(&mut cpu, 2, Esize::S, &counting);
    // fcmla z0.s, z1.s, z2.s[1], #0.
    assert_eq!(exec(&mut cpu, 0x64F2_1020), None);
    let (c1, d1, c3, d3) = (counting[2], counting[3], counting[6], counting[7]);
    assert_eq!(elements(&cpu, 0, Esize::S), [c1, d1, c1, d1, c3, d3, c3, d3]);
}

/// BFCVT writes each active container's low half and zeroes its upper
/// half; BFCVTNT writes the upper half alone. Inactive containers keep
/// their value.
#[test]
fn bfcvt_zeroes_the_upper_half_and_bfcvtnt_writes_it_alone() {
    let mut cpu = fp_streaming(0, 0);
    let vl = cpu.current_svl();
    cpu.set_p(0, vl, &Pv::from_bytes(&[0x01, 0x01]));
    set_elements(&mut cpu, 1, Esize::S, &[0x3F80_0000, 0x4000_0000, 0x4040_0000, 0x4080_0000]);
    set_elements(&mut cpu, 0, Esize::S, &[0xFFFF_FFFF]);
    set_elements(&mut cpu, 3, Esize::S, &[0xFFFF_FFFF]);
    // bfcvt z0.h, p0/m, z1.s; bfcvtnt z3.h, p0/m, z1.s.
    assert_eq!(exec(&mut cpu, 0x658A_A020), None);
    assert_eq!(exec(&mut cpu, 0x648A_A023), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x0000_3F80, 0xFFFF_FFFF, 0x0000_4040, 0xFFFF_FFFF]);
    assert_eq!(elements(&cpu, 3, Esize::S), [0x3F80_FFFF, 0xFFFF_FFFF, 0x4040_FFFF, 0xFFFF_FFFF]);
}

/// The SVE precision conversions read FPCR.AHP as 0 (`FPConvertSVE`): an
/// infinity converts to the IEEE half-precision infinity, without the
/// alternative format's Invalid Operation, and back.
#[test]
fn fcvt_ignores_fpcr_ahp() {
    let mut cpu = fp_streaming(0, FPCR_AHP);
    set_elements(&mut cpu, 1, Esize::S, &[0x7F80_0000]);
    // fcvt z0.h, p0/m, z1.s; fcvt z2.s, p0/m, z0.h.
    assert_eq!(exec(&mut cpu, 0x6588_A020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x7C00; 4]);
    assert_eq!(exec(&mut cpu, 0x6589_A002), None);
    assert_eq!(elements(&cpu, 2, Esize::S), [0x7F80_0000; 4]);
    assert_eq!(cpu.fpsr, 0);
}

/// Only the active elements are computed, so only theirs raise
/// exceptions: FDIV by zero with no element active changes nothing and
/// raises nothing; with one active, that element alone divides.
#[test]
fn only_the_active_elements_raise_exceptions() {
    let mut cpu = fp_streaming(0, 0);
    let vl = cpu.current_svl();
    cpu.set_p(1, vl, &Pv::ZERO);
    set_elements(&mut cpu, 0, Esize::S, &[0x3F80_0000]);
    set_elements(&mut cpu, 1, Esize::S, &[0]);
    // fdiv z0.s, p1/m, z0.s, z1.s.
    assert_eq!(exec(&mut cpu, 0x658D_8420), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x3F80_0000; 4]);
    assert_eq!(cpu.fpsr, 0);
    cpu.set_p(1, vl, &Pv::from_bytes(&[0x10, 0x00]));
    assert_eq!(exec(&mut cpu, 0x658D_8420), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x3F80_0000, 0x7F80_0000, 0x3F80_0000, 0x3F80_0000]);
    assert_eq!(cpu.fpsr, DZC);
}

/// `size = 00` in a floating-point instruction whose encoding is not
/// FEAT_SVE_B16B16's is UNDEFINED in its decode, before the enable check:
/// FSUBR, FMAD, FADDV, FCMLA and FLOGB here. FADD's is FEAT_SVE_B16B16's,
/// which this core lacks.
#[test]
fn a_floating_point_size_of_00_is_undefined_before_the_sme_trap() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in [0x6503_8020, 0x6522_8020, 0x6500_2020, 0x6402_0020, 0x6518_A020, 0x6500_8020] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    // fadd z0.h, p0/m, z0.h, z1.h.
    assert_eq!(exec(&mut cpu, 0x6540_8020), Some(NOT_STREAMING));
}

// ----- SVE2 integer -----

const I64_MIN: u64 = 0x8000_0000_0000_0000;
const I64_MAX: u64 = 0x7FFF_FFFF_FFFF_FFFF;

/// SQRDMLAH and SQRDMLSH of doublewords are exact at the extremes, the
/// accumulator inside the saturation: `SignedSat(((acc << 64) +/- 2ab +
/// 2^63) >> 64)`. MIN x MIN onto 0 saturates to MAX and onto MIN is 0;
/// subtracted from 0 it is MIN and from MAX -1. No saturation sets
/// FPSR.QC.
#[test]
fn sqrdmlah_of_doublewords_is_exact_at_the_extremes() {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 1, Esize::D, &[I64_MIN]);
    set_elements(&mut cpu, 2, Esize::D, &[I64_MIN]);
    set_elements(&mut cpu, 0, Esize::D, &[0, I64_MIN]);
    // sqrdmlah z0.d, z1.d, z2.d.
    assert_eq!(exec(&mut cpu, 0x44C2_7020), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [I64_MAX, 0]);
    set_elements(&mut cpu, 0, Esize::D, &[0, I64_MAX]);
    // sqrdmlsh z0.d, z1.d, z2.d.
    assert_eq!(exec(&mut cpu, 0x44C2_7420), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [I64_MIN, u64::MAX]);
    assert_eq!(cpu.fpsr, 0);
}

/// SQDMULH and SQRDMULH by element take Zm's element `index` of each
/// 128-bit segment, and round by `2^63` alone in SQRDMULH: at 256 bits,
/// by 2^62 then by MIN, 1 x 2^62 doubled is 2^63, 1 rounded and 0
/// truncated; MIN x MIN saturates in both.
#[test]
fn the_doubling_multiplies_high_round_saturate_and_index_by_segment() {
    let want = [
        (0x44F2_F420, [1, 0xC000_0000_0000_0000, I64_MAX, u64::MAX]),
        (0x44F2_F020, [0, 0xC000_0000_0000_0000, I64_MAX, u64::MAX]),
    ];
    for (insn, want) in want {
        let mut cpu = fp_streaming(1, 0);
        set_elements(&mut cpu, 1, Esize::D, &[1, I64_MIN, I64_MIN, 1]);
        set_elements(&mut cpu, 2, Esize::D, &[0x1234, 1 << 62, 0x5678, I64_MIN]);
        // sqrdmulh, sqdmulh z0.d, z1.d, z2.d[1].
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::D), want, "{insn:#010x}");
    }
}

/// The shifts by vector shift by the whole element, `ShiftSat(SInt(Zm),
/// esize)`, not by its low byte: 256 shifts a halfword left by 17. A
/// negative shift is to the right, rounding in the `R` forms (by -16,
/// unsigned 0x8000 rounds up to 1); the result is truncated, or saturated
/// in the `Q` forms.
#[test]
fn the_shifts_by_vector_take_the_whole_element_and_round_and_saturate() {
    let values = [0x0001, 0x0001, 0x8000, 0x7FFF, 0x0003, 0xFFFD, 0x8000, 0x1234];
    let shifts = [0x0100, 0x000F, 0xFFF0, 0xFFF0, 0xFFFF, 0xFFFF, 0xFF00, 0x0000];
    let forms = [
        // srshl, sqrshl, urshl, uqrshl, sqshl z0.h, p0/m, z0.h, z1.h.
        (0x4442_8020, [0, 0x8000, 0, 0, 2, 0xFFFF, 0, 0x1234]),
        (0x444A_8020, [0x7FFF, 0x7FFF, 0, 0, 2, 0xFFFF, 0, 0x1234]),
        (0x4443_8020, [0, 0x8000, 1, 0, 2, 0x7FFF, 0, 0x1234]),
        (0x444B_8020, [0xFFFF, 0x8000, 1, 0, 2, 0x7FFF, 0, 0x1234]),
        (0x4448_8020, [0x7FFF, 0x7FFF, 0xFFFF, 0, 1, 0xFFFE, 0xFFFF, 0x1234]),
    ];
    for (insn, want) in forms {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::H, &values);
        set_elements(&mut cpu, 1, Esize::H, &shifts);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::H), want, "{insn:#010x}");
        assert_eq!(cpu.fpsr, 0, "{insn:#010x}");
    }
}

/// URSHL of doublewords by -64 rounds: `(element + 2^63) >> 64` is 1 for
/// an element of 2^63 or more; by -65, and by -1000 (`ShiftSat` makes it
/// -65), it is 0.
#[test]
fn urshl_of_doublewords_by_64_rounds_up_and_by_more_is_zero() {
    let mut cpu = fp_streaming(1, 0);
    set_elements(&mut cpu, 0, Esize::D, &[I64_MIN, u64::MAX, u64::MAX, u64::MAX]);
    set_elements(&mut cpu, 1, Esize::D, &[(-64i64) as u64, (-64i64) as u64, (-65i64) as u64, (-1000i64) as u64]);
    // urshl z0.d, p0/m, z0.d, z1.d.
    assert_eq!(exec(&mut cpu, 0x44C3_8020), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [1, 1, 0, 0]);
}

/// The reversed forms swap the operands and still write Zdn, its inactive
/// elements kept: SRSHLR shifts Zm's element by Zdn's, SHSUBR and SQSUBR
/// subtract Zdn from Zm.
#[test]
fn the_reversed_forms_take_zm_first_and_merge_into_zdn() {
    let mut cpu = fp_streaming(0, 0);
    let vl = cpu.current_svl();
    // Halfwords 0 and 2 active.
    cpu.set_p(1, vl, &Pv::from_bytes(&[0x11, 0x00]));
    set_elements(&mut cpu, 0, Esize::H, &[0xFFFF, 0x0003, 0x0001, 0x0005]);
    set_elements(&mut cpu, 1, Esize::H, &[0x0003, 0x0010, 0x0040, 0x0007]);
    // srshlr z0.h, p1/m, z0.h, z1.h.
    assert_eq!(exec(&mut cpu, 0x4446_8420), None);
    assert_eq!(elements(&cpu, 0, Esize::H), [2, 3, 0x80, 5, 0xFFFF, 3, 1, 5]);

    // Bytes 0 and 4 active: (-128 - 1) >> 1 and SignedSat(-128 - 1).
    for (insn, result) in [(0x4416_8420, 0xBF), (0x441E_8420, 0x80)] {
        let mut cpu = fp_streaming(0, 0);
        cpu.set_p(1, vl, &Pv::from_bytes(&[0x11, 0x00]));
        set_elements(&mut cpu, 0, Esize::B, &[0x01]);
        set_elements(&mut cpu, 1, Esize::B, &[0x80]);
        // shsubr, sqsubr z0.b, p1/m, z0.b, z1.b.
        assert_eq!(exec(&mut cpu, insn), None);
        let mut want = [1u64; 16];
        want[0] = result;
        want[4] = result;
        assert_eq!(elements(&cpu, 0, Esize::B), want, "{insn:#010x}");
    }
}

/// A pairwise operation's even element is of the pair of Zdn it starts,
/// its odd one of the pair of Zm it ends; inactive elements keep Zdn's.
/// SMAXP compares signed and UMAXP unsigned.
#[test]
fn the_pairwise_operations_take_even_elements_from_zdn_and_odd_from_zm() {
    let mut cpu = fp_streaming(0, 0);
    let vl = cpu.current_svl();
    // Bytes 0 to 3 active.
    cpu.set_p(1, vl, &Pv::from_bytes(&[0x0F, 0x00]));
    set_elements(&mut cpu, 0, Esize::B, &[1, 2, 3, 4]);
    set_elements(&mut cpu, 1, Esize::B, &[0x10, 0x20, 0x30, 0x40]);
    // addp z0.b, p1/m, z0.b, z1.b.
    assert_eq!(exec(&mut cpu, 0x4411_A420), None);
    assert_eq!(elements(&cpu, 0, Esize::B), [3, 0x30, 7, 0x70, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4]);

    // smaxp, umaxp z0.b, p1/m, z0.b, z1.b.
    for (insn, want) in [(0x4414_A420, [0x7F, 0x01]), (0x4415_A420, [0x80, 0xFF])] {
        let mut cpu = fp_streaming(0, 0);
        cpu.set_p(1, vl, &Pv::from_bytes(&[0x03, 0x00]));
        set_elements(&mut cpu, 0, Esize::B, &[0x80, 0x7F]);
        set_elements(&mut cpu, 1, Esize::B, &[0xFF, 0x01]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::B)[..3], [want[0], want[1], 0x80], "{insn:#010x}");
    }
}

/// SUQADD adds Zm's element unsigned to Zdn's signed, saturating signed;
/// USQADD adds it signed to Zdn's unsigned, saturating unsigned. The
/// halving operations are exact before the shift: SHSUB of -128 - 127 is
/// -128, URHADD of 255 + 255 + 1 is 255.
#[test]
fn the_mixed_and_halving_operations_compute_in_the_wider_range() {
    let cases = [
        // suqadd, usqadd, shsub, urhadd, srhadd z0.b, p0/m, z0.b, z1.b.
        (0x441C_8020, [0x7F, 0x80, 0x80, 0xF0], [0xFF, 0xFF, 0x01, 0x20], [0x7F, 0x7F, 0x81, 0x10]),
        (0x441D_8020, [0x01, 0x00, 0xFF, 0x80], [0xFF, 0x80, 0x01, 0x7F], [0x00, 0x00, 0xFF, 0xFF]),
        (0x4412_8020, [0x80, 0xFF, 0xFF, 0x7F], [0x7F, 0xFF, 0x00, 0x7F], [0x80, 0x00, 0xFF, 0x00]),
        (0x4415_8020, [0x80, 0xFF, 0xFF, 0x7F], [0x7F, 0xFF, 0x00, 0x7F], [0x80, 0xFF, 0x80, 0x7F]),
        (0x4414_8020, [0x80, 0xFF, 0xFF, 0x7F], [0x7F, 0xFF, 0x00, 0x7F], [0x00, 0xFF, 0x00, 0x7F]),
    ];
    for (insn, a, b, want) in cases {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::B, &a);
        set_elements(&mut cpu, 1, Esize::B, &b);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::B)[..4], want, "{insn:#010x}");
        assert_eq!(cpu.fpsr, 0, "{insn:#010x}");
    }
}

/// SQABS and SQNEG of the smallest value saturate to the largest. SADALP
/// and UADALP add each pair of Zn's bytes, signed or unsigned, to the
/// halfword of Zda, wrapping; inactive halfwords keep Zda's value.
#[test]
fn the_unary_operations_saturate_and_sadalp_adds_pairs_into_the_accumulator() {
    // sqabs, sqneg z0.b, p0/m, z1.b.
    for (insn, want) in [(0x4408_A020, [0x7F, 0x7F, 0x01, 0x00]), (0x4409_A020, [0x7F, 0x81, 0x01, 0x00])] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 1, Esize::B, &[0x80, 0x7F, 0xFF, 0x00]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::B)[..4], want, "{insn:#010x}");
    }
    // sadalp, uadalp z0.h, p1/m, z1.b, halfwords 0 and 1 active.
    for (insn, sums) in [(0x4444_A420, [0x7EFF, 0x0010]), (0x4445_A420, [0x80FF, 0x0110])] {
        let mut cpu = fp_streaming(0, 0);
        let vl = cpu.current_svl();
        cpu.set_p(1, vl, &Pv::from_bytes(&[0x05, 0x00]));
        set_elements(&mut cpu, 0, Esize::H, &[0x7FFF, 0x0010, 0x1111, 0x2222]);
        set_elements(&mut cpu, 1, Esize::B, &[0x80, 0x80, 0xFF, 0x01]);
        assert_eq!(exec(&mut cpu, insn), None);
        let want = [sums[0], sums[1], 0x1111, 0x2222, 0x7FFF, 0x0010, 0x1111, 0x2222];
        assert_eq!(elements(&cpu, 0, Esize::H), want, "{insn:#010x}");
    }
}

/// The dot products accumulate four products into each element, wrapping:
/// SDOT of -128 x -128 and UDOT of 255 x 255 past 2^31, UDOT of halfwords
/// past 2^64. USDOT takes Zn unsigned and Zm signed, SUDOT the reverse:
/// 0x80 x 0xFF is 128 x -1 in one and -128 x 255 in the other. By element,
/// Zm's word `index` of each segment.
#[test]
fn the_dot_products_wrap_and_take_each_operands_signedness() {
    // (word, by element, accumulator, Zn's bytes, Zm's bytes, result).
    let cases = [
        // sdot, udot, usdot z0.s, z1.b, z2.b.
        (0x4482_0020, false, 0x7FFF_FFFF, 0x80, 0x80, 0x8000_FFFF),
        (0x4482_0420, false, 0x7FFF_FFFF, 0xFF, 0xFF, 0x8003_F803),
        (0x4482_7820, false, 0, 0x80, 0xFF, 0xFFFF_FE00),
        // usdot, sudot z0.s, z1.b, z2.b[0].
        (0x44A2_1820, true, 0, 0x80, 0xFF, 0xFFFF_FE00),
        (0x44A2_1C20, true, 0, 0x80, 0xFF, 0xFFFE_0200),
    ];
    for (insn, indexed, acc, n, m, want) in cases {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::S, &[acc]);
        set_elements(&mut cpu, 1, Esize::B, &[n]);
        if indexed {
            // Word 0 of Zm is `m`'s bytes, the others 1s, which another
            // index would take.
            set_elements(&mut cpu, 2, Esize::S, &[m * 0x0101_0101, 0x0101_0101, 0x0101_0101, 0x0101_0101]);
        } else {
            set_elements(&mut cpu, 2, Esize::B, &[m]);
        }
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [want; 4], "{insn:#010x}");
    }
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::D, &[u64::MAX]);
    set_elements(&mut cpu, 1, Esize::H, &[0xFFFF]);
    set_elements(&mut cpu, 2, Esize::H, &[0xFFFF]);
    // udot z0.d, z1.h, z2.h.
    assert_eq!(exec(&mut cpu, 0x44C2_0420), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [0x3_FFF8_0003; 2]);
}

/// SVE2.1's two-way SDOT and UDOT accumulate into each word the two
/// products of its halfwords, wrapping: 0x8000_0001 plus twice (-32768)^2
/// is 1, and 1 plus twice 65535^2 is 0xFFFC_0003 modulo 2^32. By element,
/// the word `i2` of each 128-bit segment of Zm.
#[test]
fn the_two_way_dot_products_sum_pairs_of_halfwords() {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::S, &[0x8000_0001]);
    set_elements(&mut cpu, 1, Esize::H, &[0x8000]);
    set_elements(&mut cpu, 2, Esize::H, &[0x8000]);
    // sdot z0.s, z1.h, z2.h.
    assert_eq!(exec(&mut cpu, 0x4402_C820), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [1; 4]);
    set_elements(&mut cpu, 1, Esize::H, &[0xFFFF]);
    set_elements(&mut cpu, 2, Esize::H, &[0xFFFF]);
    // udot z0.s, z1.h, z2.h.
    assert_eq!(exec(&mut cpu, 0x4402_CC20), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0xFFFC_0003; 4]);

    // sdot z0.s, z1.h, z2.h[1] at 256 bits: word 1 of each segment, (2, 3)
    // then (4, 5), by (1, 1).
    let mut cpu = fp_streaming(1, 0);
    set_elements(&mut cpu, 1, Esize::H, &[1]);
    let m = [0x0001_0001, 0x0003_0002, 0x0001_0001, 0x0001_0001, 0x0001_0001, 0x0005_0004, 0x0001_0001, 0x0001_0001];
    set_elements(&mut cpu, 2, Esize::S, &m);
    assert_eq!(exec(&mut cpu, 0x448A_C820), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [5, 5, 5, 5, 9, 9, 9, 9]);
    // udot z0.s, z1.h, z7.h[3]: word 3, both halfwords 0xFFFF, unsigned.
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 1, Esize::H, &[0xFFFF]);
    set_elements(&mut cpu, 7, Esize::S, &[1, 1, 1, 0xFFFF_FFFF]);
    assert_eq!(exec(&mut cpu, 0x449F_CC20), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0xFFFC_0002; 4]);
}

/// SVE2.1's FDOT of half-precision pairs is `FPDotAdd`: the products
/// summed with one rounding, then added with a second, so `1 + 2^-28` is 1
/// before -1 is added, and the Inexact reaches FPSR, as it does not from
/// FEAT_SME2's FDOT into ZA. By element, the pair `i2` of each 128-bit
/// segment of Zm.
#[test]
fn fdot_rounds_the_pair_then_the_sum_and_raises_its_exceptions() {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::S, &[0xBF80_0000]);
    set_elements(&mut cpu, 1, Esize::H, &[0x3C00, 0x0400]);
    set_elements(&mut cpu, 2, Esize::H, &[0x3C00, 0x0400]);
    // fdot z0.s, z1.h, z2.h.
    assert_eq!(exec(&mut cpu, 0x6422_8020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0; 4]);
    assert_eq!(cpu.fpsr, IXC);

    // fdot z0.s, z1.h, z2.h[3]: 0.5 + 1.0 x 1.0 + 1.0 x 2.0.
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::S, &[0x3F00_0000]);
    set_elements(&mut cpu, 1, Esize::H, &[0x3C00]);
    set_elements(&mut cpu, 2, Esize::S, &[0, 0, 0, 0x4000_3C00]);
    assert_eq!(exec(&mut cpu, 0x643A_4020), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x4060_0000; 4]);
    assert_eq!(cpu.fpsr, 0);
}

/// SVE2.1's BFMLSLB and BFMLSLT negate Zn's even or odd element (BFNeg)
/// before BFMulAddH: 0 - 1.0 x 3.0 and 0 - 2.0 x 4.0, and by element Zm's
/// halfword `i3h:i3l` of the segment, 0 - 1.0 x 5.0. Under FPCR.AH = 1
/// BFNeg leaves a NaN's sign alone, so the NaN the product propagates is
/// positive; under AH = 0 it is negated.
#[test]
fn bfmlsl_negates_zns_element_but_not_a_nan_under_ah() {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 1, Esize::H, &[0x3F80, 0x4000]);
    set_elements(&mut cpu, 2, Esize::H, &[0x4040, 0x4080]);
    // bfmlslb, bfmlslt z0.s, z1.h, z2.h.
    for (insn, want) in [(0x64E2_A020, 0xC040_0000), (0x64E2_A420, 0xC100_0000)] {
        set_elements(&mut cpu, 0, Esize::S, &[0]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [want; 4], "{insn:#010x}");
    }
    // bfmlslb z0.s, z1.h, z2.h[7].
    set_elements(&mut cpu, 2, Esize::H, &[0, 0, 0, 0, 0, 0, 0, 0x40A0]);
    set_elements(&mut cpu, 0, Esize::S, &[0]);
    assert_eq!(exec(&mut cpu, 0x64FA_6820), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0xC0A0_0000; 4]);

    // bfmlslt z0.s, z1.h, z2.h of a quiet NaN.
    for (fpcr, want) in [(FPCR_AH, 0x7FC1_0000), (0, 0xFFC1_0000)] {
        let mut cpu = fp_streaming(0, fpcr);
        set_elements(&mut cpu, 1, Esize::H, &[0x3F80, 0x7FC1]);
        set_elements(&mut cpu, 2, Esize::H, &[0x3F80]);
        assert_eq!(exec(&mut cpu, 0x64E2_A420), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [want; 4], "FPCR {fpcr:#x}");
    }
}

/// The SVE2.1 instructions FEAT_SME2 makes legal take the SME trap outside
/// Streaming SVE mode, and are UNDEFINED without FEAT_SME2: two-way SDOT
/// and UDOT, by vector and by element, FDOT and BFMLSLB and BFMLSLT.
#[test]
fn the_sve2p1_instructions_need_streaming_mode_and_sme2() {
    let all = [0x4402_C820, 0x449F_CC20, 0x6422_8020, 0x643A_4020, 0x64E2_A020, 0x64FA_6820];
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in all {
        assert_eq!(exec(&mut cpu, insn), Some(NOT_STREAMING), "{insn:#010x}");
    }
    let features: Vec<Feature> = Feature::ALL.into_iter().filter(|&f| f != Feature::Sme2).collect();
    let mut cpu = processor(Implementation {
        features: Features::of(&features),
        max_svl: VecLen::MAX,
    });
    cpu.set_pstate_sm(true);
    for insn in all {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}

/// CDOT rotates Zm's pairs by `rot`: of (1 + 2i) and (3 + 4i), twice in
/// each word, #0 adds 2 x (1 x 3 - 2 x 4), #90 2 x (1 x 4 + 2 x 3), #180
/// 2 x (1 x 3 + 2 x 4) and #270 2 x (1 x 4 - 2 x 3). CMLA rotates as
/// FCMLA does. SQRDCMLAH doubles and rounds: MIN x MIN saturates, and
/// 2 x 16384 x 1 + 2^15 is 1 after the shift.
#[test]
fn the_complex_integer_operations_rotate_and_round() {
    for (rot, want) in [(0, 0xFFFF_FFF6u64), (1, 20), (2, 22), (3, 0xFFFF_FFFC)] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::S, &[0]);
        set_elements(&mut cpu, 1, Esize::B, &[1, 2]);
        set_elements(&mut cpu, 2, Esize::B, &[3, 4]);
        // cdot z0.s, z1.b, z2.b, #(90 * rot).
        assert_eq!(exec(&mut cpu, 0x4482_1020 | (rot << 10)), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [want; 4], "rot {rot}");
    }
    let rotations = [(0, [3, 4]), (1, [0xFFFF_FFF8, 6]), (2, [0xFFFF_FFFD, 0xFFFF_FFFC]), (3, [8, 0xFFFF_FFFA])];
    for (rot, [real, imag]) in rotations {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::S, &[0]);
        set_elements(&mut cpu, 1, Esize::S, &[1, 2]);
        set_elements(&mut cpu, 2, Esize::S, &[3, 4]);
        // cmla z0.s, z1.s, z2.s, #(90 * rot).
        assert_eq!(exec(&mut cpu, 0x4482_2020 | (rot << 10)), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [real, imag, real, imag], "rot {rot}");
    }
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::H, &[0]);
    set_elements(&mut cpu, 1, Esize::H, &[0x8000, 0x8000, 0x4000, 0x1234]);
    set_elements(&mut cpu, 2, Esize::H, &[0x8000, 0x8000, 0x0001, 0x0003]);
    // sqrdcmlah z0.h, z1.h, z2.h, #0.
    assert_eq!(exec(&mut cpu, 0x4442_3020), None);
    assert_eq!(elements(&cpu, 0, Esize::H), [0x7FFF, 0x7FFF, 1, 2, 0x7FFF, 0x7FFF, 1, 2]);
    assert_eq!(cpu.fpsr, 0);
}

/// SQDMLALB saturates the doubled product before it accumulates: -32768 x
/// -32768 doubled is 2^31, saturated to 2^31 - 1, onto -1 is 2^31 - 2;
/// SQDMLSLB of it from 0 is -(2^31 - 1). The `T` form takes the odd
/// elements, and SQDMLALBT Zn's even by Zm's odd.
#[test]
fn the_saturating_long_multiply_adds_saturate_the_product_first() {
    let forms = [
        // sqdmlalb, sqdmlslb, sqdmlalt, sqdmlalbt z0.s, z1.h, z2.h.
        (0x4482_6020, [0x7FFF_FFFE, 0x7FFF_FFFF]),
        (0x4482_6820, [0x8000_0000, 0x8000_0001]),
        (0x4482_6420, [0x13, 0x14]),
        (0x4482_0820, [0xFFFA_FFFF, 0xFFFB_0000]),
    ];
    for (insn, want) in forms {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::S, &[0xFFFF_FFFF, 0]);
        set_elements(&mut cpu, 1, Esize::H, &[0x8000, 0x0002]);
        set_elements(&mut cpu, 2, Esize::H, &[0x8000, 0x0005]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [want[0], want[1], want[0], want[1]], "{insn:#010x}");
        assert_eq!(cpu.fpsr, 0, "{insn:#010x}");
    }
}

/// A long instruction by element takes the source element `index` of each
/// 128-bit segment of Zm: at 256 bits SMLALB by halfword 7 takes 8 then
/// 16 from Zm counting up from 1, and UMULLT by word 3 takes 4 then 8,
/// each times Zn's odd word.
#[test]
fn the_long_indexed_forms_take_their_source_element_from_each_segment() {
    let mut cpu = fp_streaming(1, 0);
    set_elements(&mut cpu, 0, Esize::S, &[0]);
    set_elements(&mut cpu, 1, Esize::H, &[1]);
    set_elements(&mut cpu, 2, Esize::H, &(1..=16).collect::<Vec<u64>>());
    // smlalb z0.s, z1.h, z2.h[7].
    assert_eq!(exec(&mut cpu, 0x44BA_8820), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [8, 8, 8, 8, 16, 16, 16, 16]);

    let mut cpu = fp_streaming(1, 0);
    set_elements(&mut cpu, 1, Esize::S, &[0, 0xFFFF_FFFF]);
    set_elements(&mut cpu, 2, Esize::S, &(1..=8).collect::<Vec<u64>>());
    // umullt z0.d, z1.s, z2.s[3].
    assert_eq!(exec(&mut cpu, 0x44F2_DC20), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [0x3_FFFF_FFFC, 0x3_FFFF_FFFC, 0x7_FFFF_FFF8, 0x7_FFFF_FFF8]);
}

/// SCLAMP and UCLAMP are `Min(Max(Zn, Zd), Zm)`: with Zn above Zm the
/// result is Zm's; 0xFF is -1 to SCLAMP and 255 to UCLAMP.
#[test]
fn clamp_takes_the_maximum_first_and_compares_by_signedness() {
    // sclamp, uclamp z0.b, z1.b, z2.b.
    for (insn, want) in [(0x4402_C020, [0x03, 0x00, 0x90, 0x20]), (0x4402_C420, [0x03, 0x10, 0x7F, 0x20])] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::B, &[0x04, 0xFF, 0x80, 0x20]);
        set_elements(&mut cpu, 1, Esize::B, &[0x05, 0x00, 0x90, 0x10]);
        set_elements(&mut cpu, 2, Esize::B, &[0x03, 0x10, 0x7F, 0x30]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::B)[..4], want, "{insn:#010x}");
    }
}

/// The long arithmetic takes its elements by form: SADDLBT Zn's even
/// halfword and Zm's odd (2 + 7), SSUBLBT the same (2 - 7), SSUBLTB Zn's
/// odd and Zm's even (3 - 5). SABDLB of 0x00 and 0xFF is 1, UABDLB's 255;
/// SADDWT adds Zm's odd halfword to Zn's word, wrapping.
#[test]
fn the_long_arithmetic_takes_its_elements_by_form() {
    // saddlbt, ssublbt, ssubltb z0.s, z1.h, z2.h.
    for (insn, want) in [(0x4582_8020, 9), (0x4582_8820, 0xFFFF_FFFB), (0x4582_8C20, 0xFFFF_FFFE)] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 1, Esize::H, &[2, 3]);
        set_elements(&mut cpu, 2, Esize::H, &[5, 7]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::S), [want; 4], "{insn:#010x}");
    }
    // sabdlb, uabdlb z0.h, z1.b, z2.b.
    for (insn, want) in [(0x4542_3020, 1), (0x4542_3820, 0xFF)] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 1, Esize::B, &[0x00, 0x11]);
        set_elements(&mut cpu, 2, Esize::B, &[0xFF, 0x22]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::H), [want; 8], "{insn:#010x}");
    }
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 1, Esize::S, &[0x7FFF_FFFF]);
    set_elements(&mut cpu, 2, Esize::H, &[0x0000, 0x0001]);
    // saddwt z0.s, z1.s, z2.h.
    assert_eq!(exec(&mut cpu, 0x4582_4420), None);
    assert_eq!(elements(&cpu, 0, Esize::S), [0x8000_0000; 4]);
}

/// PMULLB and PMULLT multiply polynomials over GF(2): 3 x 3 is 5, 5 x 7 is
/// 0x1B, 0x80 x 0x80 is 0x4000 and 0xFF x 0xFF 0x5555. SQDMULLB of
/// -128 x -128 doubled saturates to 0x7FFF.
#[test]
fn pmull_multiplies_polynomials_and_sqdmull_saturates() {
    let cases = [
        // pmullb, pmullt, sqdmullb z0.h, z1.b, z2.b.
        (0x4542_6820, [0x0005, 0x4000]),
        (0x4542_6C20, [0x001B, 0x5555]),
        (0x4542_6020, [0x0012, 0x7FFF]),
    ];
    for (insn, want) in cases {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 1, Esize::B, &[0x03, 0x05, 0x80, 0xFF]);
        set_elements(&mut cpu, 2, Esize::B, &[0x03, 0x07, 0x80, 0xFF]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::H)[..2], want, "{insn:#010x}");
    }
}

/// SSHLLT and USHLLT shift the odd byte, extended by signedness, left by
/// up to 7: 0x80 is 0xC000 and 0x4000. EORBT writes the even bytes of Zd,
/// Zn's even exclusive-OR Zm's odd, and keeps the odd ones; SABALB
/// accumulates |0 - -128| into 0xFFFF, wrapping to 0x7F.
#[test]
fn the_shifts_long_eorbt_and_sabalb_take_their_halves() {
    // sshllt, ushllt z0.h, z1.b, #7.
    for (insn, want) in [(0x450F_A420, 0xC000), (0x450F_AC20, 0x4000)] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 1, Esize::B, &[0x11, 0x80]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::H), [want; 8], "{insn:#010x}");
    }
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::B, &[0xAA, 0xBB]);
    set_elements(&mut cpu, 1, Esize::B, &[0x0F, 0x1F]);
    set_elements(&mut cpu, 2, Esize::B, &[0xF0, 0x33]);
    // eorbt z0.b, z1.b, z2.b.
    assert_eq!(exec(&mut cpu, 0x4502_9020), None);
    assert_eq!(elements(&cpu, 0, Esize::B)[..2], [0x3C, 0xBB]);

    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::H, &[0xFFFF]);
    set_elements(&mut cpu, 1, Esize::B, &[0x00, 0x11]);
    set_elements(&mut cpu, 2, Esize::B, &[0x80, 0x22]);
    // sabalb z0.h, z1.b, z2.b.
    assert_eq!(exec(&mut cpu, 0x4542_C020), None);
    assert_eq!(elements(&cpu, 0, Esize::H), [0x7F; 8]);
}

/// ADCLB, ADCLT and SBCLB add with the carry in the low bit of Zm's odd
/// word (the even one is ignored), writing the sum to the even word and
/// the carry out to the odd: 0xFFFFFFFF + 0 + 1 carries; SBCLB inverts
/// Zn's word, so 5 - 6 - 1 borrows.
#[test]
fn the_long_adds_with_carry_chain_through_the_odd_elements() {
    let cases = [
        // adclb, adclt, sbclb z0.s, z1.s, z2.s.
        (0x4502_D020, [0, 1, 0xB, 0]),
        (0x4502_D420, [1, 1, 7, 0]),
        (0x4582_D020, [0xFFFF_FFFF, 1, 0xFFFF_FFFE, 0]),
    ];
    for (insn, want) in cases {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::S, &[0xFFFF_FFFF, 0x1234, 5, 0x5678]);
        set_elements(&mut cpu, 1, Esize::S, &[0, 1, 6, 2]);
        set_elements(&mut cpu, 2, Esize::S, &[0, 1, 1, 0]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::S), want, "{insn:#010x}");
    }
}

/// CADD adds Zm's pair rotated: of (1 + 2i) and (3 + 4i), #90 gives (-3,
/// 5) and #270 (5, -1). SQCADD saturates: 127 - -128 is 127.
#[test]
fn cadd_rotates_and_sqcadd_saturates() {
    // cadd z0.h, z0.h, z1.h, #90 and #270.
    for (insn, want) in [(0x4540_D820, [0xFFFD, 5]), (0x4540_DC20, [5, 0xFFFF])] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::H, &[1, 2]);
        set_elements(&mut cpu, 1, Esize::H, &[3, 4]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::H)[..2], want, "{insn:#010x}");
    }
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::B, &[0x7F, 0x80]);
    set_elements(&mut cpu, 1, Esize::B, &[0x00, 0x80]);
    // sqcadd z0.b, z0.b, z1.b, #90.
    assert_eq!(exec(&mut cpu, 0x4501_D820), None);
    assert_eq!(elements(&cpu, 0, Esize::B)[..2], [0x7F, 0x80]);
}

/// URSRA of doublewords by 64 rounds `(element + 2^63) >> 64` up to 1
/// before it accumulates. SRI by the element size keeps Zd, of bytes and
/// of doublewords, SLI by 0 replaces it, SRI by 4 keeps Zd's top four
/// bits, and SLI of doublewords by 63 replaces bit 63 alone.
#[test]
fn the_shift_accumulates_and_inserts_reach_the_element_size() {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::D, &[10, 20]);
    set_elements(&mut cpu, 1, Esize::D, &[u64::MAX, I64_MIN]);
    // ursra z0.d, z1.d, #64.
    assert_eq!(exec(&mut cpu, 0x4580_EC20), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [11, 21]);

    // sri z0.b, z1.b, #8; sli z0.b, z1.b, #0.
    for (insn, want) in [(0x4508_F020, 0xAB), (0x4508_F420, 0xFF)] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::B, &[0xAB]);
        set_elements(&mut cpu, 1, Esize::B, &[0xFF]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::B), [want; 16], "{insn:#010x}");
    }
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::H, &[0xABCD]);
    set_elements(&mut cpu, 1, Esize::H, &[0x1234]);
    // sri z0.h, z1.h, #4.
    assert_eq!(exec(&mut cpu, 0x451C_F020), None);
    assert_eq!(elements(&cpu, 0, Esize::H), [0xA123; 8]);

    // Doublewords: SRI by 64 keeps Zd; SLI by 63 moves Zn's bit 0 to bit 63.
    for (insn, want) in [(0x4580_F020, 0x0123_4567_89AB_CDEF), (0x45DF_F420, 0x8123_4567_89AB_CDEF)] {
        let mut cpu = fp_streaming(0, 0);
        set_elements(&mut cpu, 0, Esize::D, &[0x0123_4567_89AB_CDEF]);
        set_elements(&mut cpu, 1, Esize::D, &[0xFFFF_FFFF_FFFF_FFFF]);
        assert_eq!(exec(&mut cpu, insn), None);
        assert_eq!(elements(&cpu, 0, Esize::D), [want; 2], "{insn:#010x}");
    }
}

/// The narrowing instructions: a `B` form writes the even bytes and zeroes
/// the odd, a `T` form writes the odd and keeps the even. SQRSHRUN
/// saturates a negative value to 0 and 256 to 0xFF; RSHRNB rounds 0x17F
/// up to 1 and 0xFF80 to 0x100, truncated to 0; the extracts saturate by
/// their signedness; ADDHNB keeps the high byte of the sum, RADDHNB rounds
/// it, SUBHNB of 0 - 1 is 0xFF.
#[test]
fn the_narrowing_instructions_write_their_half_and_saturate() {
    // sqrshrunb, sqrshrunt z0.b, z1.h, #1.
    let shifted = [0x8000, 0x0003, 0x01FF, 0x0000];
    assert_eq!(narrowed(0x452F_0820, &shifted, &[0]), [0, 0, 2, 0, 0xFF, 0, 0, 0]);
    assert_eq!(narrowed(0x452F_0C20, &shifted, &[0]), [0xEE, 0, 0xEE, 2, 0xEE, 0xFF, 0xEE, 0]);
    // rshrnb, shrnb z0.b, z1.h, #8.
    let rounded = [0x0180, 0x017F, 0xFF80, 0x0000];
    assert_eq!(narrowed(0x4528_1820, &rounded, &[0]), [2, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(narrowed(0x4528_1020, &rounded, &[0]), [1, 0, 1, 0, 0xFF, 0, 0, 0]);
    // sqxtnb, uqxtnb, sqxtunb, sqxtnt z0.b, z1.h.
    let extracted = [0x0080, 0xFF7F, 0x00FF, 0xFFFF];
    assert_eq!(narrowed(0x4528_4020, &extracted, &[0]), [0x7F, 0, 0x80, 0, 0x7F, 0, 0xFF, 0]);
    assert_eq!(narrowed(0x4528_4820, &extracted, &[0]), [0x80, 0, 0xFF, 0, 0xFF, 0, 0xFF, 0]);
    assert_eq!(narrowed(0x4528_5020, &extracted, &[0]), [0x80, 0, 0, 0, 0xFF, 0, 0, 0]);
    assert_eq!(narrowed(0x4528_4420, &extracted, &[0]), [0xEE, 0x7F, 0xEE, 0x80, 0xEE, 0x7F, 0xEE, 0xFF]);
    // addhnb, raddhnb, subhnb, rsubhnt z0.b, z1.h, z2.h.
    let (n, m) = ([0x00FF, 0x007F, 0x0000, 0x8000], [1, 1, 1, 0x8000]);
    assert_eq!(narrowed(0x4562_6020, &n, &m), [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(narrowed(0x4562_6820, &n, &m), [1, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!(narrowed(0x4562_7020, &n, &m), [0, 0, 0, 0, 0xFF, 0, 0, 0]);
    assert_eq!(narrowed(0x4562_7C20, &n, &m), [0xEE, 1, 0xEE, 0, 0xEE, 0, 0xEE, 0]);
}

/// Zd's first eight bytes after `insn`, from Zd's bytes all 0xEE and Zn's
/// and Zm's halfwords `n` and `m`, repeated; no saturation sets FPSR.QC.
fn narrowed(insn: u32, n: &[u64], m: &[u64]) -> Vec<u64> {
    let mut cpu = fp_streaming(0, 0);
    set_elements(&mut cpu, 0, Esize::B, &[0xEE]);
    set_elements(&mut cpu, 1, Esize::H, n);
    set_elements(&mut cpu, 2, Esize::H, m);
    assert_eq!(exec(&mut cpu, insn), None, "{insn:#010x}");
    assert_eq!(cpu.fpsr, 0, "{insn:#010x}");
    elements(&cpu, 0, Esize::B)[..8].to_vec()
}

/// The sizes an SVE2 integer page's decode makes UNDEFINED, before the
/// enable check: URECPE of halfwords and doublewords, SADALP, SMLALB,
/// SQDMLALB and SQDMLALBT of bytes, CDOT of `size = 0x`. SQRDMLAH of
/// bytes is legal.
#[test]
fn the_reserved_sve2_integer_sizes_are_undefined_before_the_sme_trap() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in [0x4440_A020, 0x44C0_A020, 0x4404_A420, 0x4402_4020, 0x4402_6020, 0x4402_0820, 0x4442_1020, 0x4402_1020] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    // sqrdmlah z0.b, z1.b, z2.b.
    assert_eq!(exec(&mut cpu, 0x4402_7020), Some(NOT_STREAMING));
}

/// The widening, narrowing and accumulating instructions' reserved
/// encodings are UNDEFINED before the enable check: SADDLB and SABALB of
/// bytes, PMULLB of words (`size = 10`) and of `size = 00` (FEAT_SVE_
/// PMULL128's), SSHLLB, SRSRA and SHRNB with no `tsize` bit set, SQXTNB
/// with `tsize = 011`, ADDHNB of bytes. EORBT of bytes is legal.
#[test]
fn the_reserved_sve2_widening_and_narrowing_encodings_are_undefined_before_the_sme_trap() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    let reserved = [
        0x4502_0020, 0x4502_C020, 0x4582_6820, 0x4502_6820, 0x4507_A020, 0x4500_E820, 0x4520_1020, 0x4538_4020,
        0x4522_6020,
    ];
    for insn in reserved {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
    // eorbt z0.b, z1.b, z2.b.
    assert_eq!(exec(&mut cpu, 0x4502_9020), Some(NOT_STREAMING));
}
