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
//! the active elements alone, and `size = 00` UNDEFINED.

use std::vec::Vec;

use super::super::features::{Implementation, VecLen};
use super::super::fp_estimate::fpsr::{DZC, IDC, IOC, IXC, OFC, UFC};
use super::super::sctlr;
use crate::softfloat::FPCR_AH;
use super::super::step::{exec, exec_in, processor, Taken, CPACR_ALL, DATA, FP_TRAP, SME_DISABLED, UNDEFINED};
use super::super::{Cpu, NoEl2};
use super::{Esize, Pv, Zv};

/// The SME trap with SMTC 2: an SVE instruction outside Streaming SVE mode
/// on a processor without FEAT_SVE.
const NOT_STREAMING: Taken = Taken { ec: 0x1D, iss: 2 };

/// `cntb x0` (pattern ALL, `MUL #1`).
const CNTB_X0: u32 = 0x0420_E3E0;
/// `index z0.b, #0, #1`.
const INDEX_Z0_B: u32 = 0x0421_4000;

/// A processor in Streaming SVE mode with `SMCR_EL1.LEN = len`.
fn streaming(len: u64) -> Cpu<NoEl2> {
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
fn active(cpu: &Cpu<NoEl2>, es: Esize) -> Vec<bool> {
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
    let d_lanes = |cpu: &Cpu<NoEl2>| {
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
fn elements(cpu: &Cpu<NoEl2>, n: u32, es: Esize) -> Vec<u64> {
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
fn set_elements(cpu: &mut Cpu<NoEl2>, n: u32, es: Esize, values: &[u64]) {
    let vl = cpu.current_svl();
    let mut z = Zv::ZERO;
    for e in 0..es.elements(vl) {
        z.set(e, es, values[e % values.len()]);
    }
    cpu.set_z(n, vl, &z);
}

/// A processor in Streaming SVE mode with `SMCR_EL1.LEN = len`, P0 all
/// true, FPCR `fpcr` and FPSR clear.
fn fp_streaming(len: u64, fpcr: u64) -> Cpu<NoEl2> {
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
    // fcvtzs z0.d, p0/m, z1.d (to 32 bits).
    assert_eq!(exec(&mut cpu, 0x65D8_A020), None);
    assert_eq!(elements(&cpu, 0, Esize::D), [u64::MAX, 0x7FFF_FFFF, 0xFFFF_FFFF_8000_0000, 2]);
    assert_eq!(cpu.fpsr, IOC | IXC);
    cpu.fpsr = 0;
    // fcvtzu z2.d, p0/m, z1.d (to 32 bits).
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
