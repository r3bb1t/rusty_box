//! FEAT_SME2's predicate-as-counter and the instructions that write, read
//! or are governed by one, on a processor that implements FEAT_SME2
//! ([`Implementation::EVERY_FEATURE`]), in Streaming SVE mode at a 128-bit
//! vector length.
//!
//! Each property is the guest-visible one (R9), against its Arm pseudocode
//! (`EncodePredCount`, `CounterToPredicate`, `PredCountTest` and the
//! instruction pages): the counter's encoding, a count of every element
//! held as an inverted zero, the element size its low bits give read at
//! another size, a low nibble of zero activating nothing; a WHILE counting
//! up from the first element and one counting down from the last, signed
//! and unsigned, with their flags; the WHILE of a pair; PEXT's parts and
//! its pair wrapping from P15 to P0; CNTP's width; and the multi-vector
//! loads and stores: their registers, consecutive and strided, one
//! register after another in memory, the counter's elements across the
//! registers, the zeroed inactive elements and untouched memory, the
//! immediate offset in multiples of the whole transfer, and a fault
//! part-way. The checks: Streaming SVE mode, and UNDEFINED without
//! FEAT_SME2.

use std::vec;
use std::vec::Vec;

use super::super::super::features::{Feature, Features, Implementation, VecLen};
use super::super::super::step::{exec, exec_in, processor, Taken, DATA, UNDEFINED};
use super::super::super::Cpu;
use super::super::Pv;

/// The SME trap with SMTC 2: outside Streaming SVE mode.
const NOT_STREAMING: Taken = Taken { ec: 0x1D, iss: 2 };

/// `ptrue pn8.b` and `ptrue pn9.s`.
const PTRUE_PN8_B: u32 = 0x2520_7810;
const PTRUE_PN9_S: u32 = 0x25A0_7811;
/// `whilelt pn8.b, x0, x1, vlx2` and `whilelt pn8.b, x0, x1, vlx4`.
const WHILELT_PN8_B: u32 = 0x2521_4410;
const WHILELT_PN8_B_VLX4: u32 = 0x2521_6410;
/// `whilelo pn10.h, x2, x3, vlx4`.
const WHILELO_PN10_H: u32 = 0x2563_6C52;
/// `whilegt pn11.d, x0, x1, vlx2`.
const WHILEGT_PN11_D: u32 = 0x25E1_401B;
/// `whilehs pn12.s, x0, x1, vlx4`.
const WHILEHS_PN12_S: u32 = 0x25A1_6814;
/// `whilelt {p0.b, p1.b}, x0, x1` and `whilege {p2.s, p3.s}, x0, x1`.
const WHILELT_PAIR_B: u32 = 0x2521_5410;
const WHILEGE_PAIR_S: u32 = 0x25A1_5012;
/// `pext p0.b, pn8[0]` and `pext p1.h, pn9[0]`.
const PEXT_P0_B: u32 = 0x2520_7010;
const PEXT_P1_H: u32 = 0x2560_7031;
/// `pext {p2.s, p3.s}, pn10[1]` and `pext {p15.b, p0.b}, pn8[0]`.
const PEXT_PAIR_S: u32 = 0x25A0_7552;
const PEXT_PAIR_WRAP: u32 = 0x2520_741F;
/// `cntp x0, pn8.b, vlx2`, `cntp x1, pn9.d, vlx4` and `cntp x1, pn9.d,
/// vlx2`.
const CNTP_B: u32 = 0x2520_8300;
const CNTP_D: u32 = 0x25E0_8721;
const CNTP_D_VLX2: u32 = 0x25E0_8321;
/// `ld1b {z0.b-z1.b}, pn8/z, [x0, x1]`.
const LD1B_2: u32 = 0xA001_0000;
/// `ld1w {z4.s-z7.s}, pn9/z, [x0, #4, mul vl]`.
const LD1W_4_IMM: u32 = 0xA041_C404;
/// `ld1b {z0.b, z8.b}, pn8/z, [x0, x1]`.
const LD1B_2X8: u32 = 0xA101_0000;
/// `ld1d {z19.d, z23.d, z27.d, z31.d}, pn11/z, [x0, x1, lsl #3]`.
const LD1D_4X4: u32 = 0xA101_EC13;
/// `st1d {z0.d-z1.d}, pn8, [x0, x1, lsl #3]`.
const ST1D_2: u32 = 0xA021_6000;
/// `st1h {z16.h, z24.h}, pn8, [x0, x1, lsl #1]`.
const ST1H_2X8: u32 = 0xA121_2010;
/// `stnt1w {z18.s, z22.s, z26.s, z30.s}, pn12, [x0, #4, mul vl]`.
const STNT1W_4X4_IMM: u32 = 0xA161_D01A;
/// `st1b {z28.b-z31.b}, pn15, [x0, #-4, mul vl]`.
const ST1B_4_IMM: u32 = 0xA06F_9C1C;

/// `PSTATE.NZCV` of N, Z, C and V.
const fn nzcv(n: bool, z: bool, c: bool) -> u64 {
    ((n as u64) << 31) | ((z as u64) << 30) | ((c as u64) << 29)
}

/// A synchronous external abort from the current level.
const fn external_abort(write: bool) -> Taken {
    Taken {
        ec: 0x25,
        iss: 0x10 | ((write as u64) << 6),
    }
}

/// `size` bytes counting up from `from`.
fn counting(from: u8, size: usize) -> Vec<u8> {
    (0..size).map(|i| from.wrapping_add(i as u8)).collect()
}

/// A processor with FEAT_SME2 in Streaming SVE mode at a 128-bit vector
/// length.
fn streaming() -> Cpu {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    cpu.set_pstate_sm(true);
    cpu
}

/// The low 64 bits of predicate `n`.
fn p_bits(cpu: &Cpu, n: u32) -> u64 {
    let p = cpu.p(n, cpu.current_svl());
    u64::from_le_bytes(core::array::from_fn(|i| p.byte(i)))
}

/// Predicate `n` holding `bits`, bit `i` that of vector byte `i`.
fn set_p_bits(cpu: &mut Cpu, n: u32, bits: u64) {
    let svl = cpu.current_svl();
    cpu.set_p(n, svl, &Pv::from_bytes(&bits.to_le_bytes()[..svl.predicate_bytes()]));
}

/// The bytes of Z register `n`.
fn z_bytes(cpu: &Cpu, n: u32) -> Vec<u8> {
    let svl = cpu.current_svl();
    cpu.z(n, svl).as_bytes()[..svl.bytes()].to_vec()
}

/// The bytes a test puts in Z register `z`: `16 * z` up, modulo 256.
fn pattern(z: u32) -> Vec<u8> {
    counting((16 * z) as u8, 16)
}

/// Z register `n` of `bytes`.
fn set_z_bytes(cpu: &mut Cpu, n: u32, bytes: &[u8]) {
    let svl = cpu.current_svl();
    cpu.set_z(n, svl, &super::super::Zv::from_bytes(bytes));
}

/// PTRUE counts every element, which `EncodePredCount` holds as a count
/// of zero inverted, in the low 16 bits alone.
#[test]
fn ptrue_holds_every_element_as_an_inverted_zero() {
    let mut cpu = streaming();
    set_p_bits(&mut cpu, 8, u64::MAX);
    set_p_bits(&mut cpu, 9, u64::MAX);
    assert_eq!(exec(&mut cpu, PTRUE_PN8_B), None);
    assert_eq!(exec(&mut cpu, PTRUE_PN9_S), None);
    assert_eq!((p_bits(&cpu, 8), p_bits(&cpu, 9)), (0x8001, 0x8004));
}

/// A WHILE that counts up gives the first elements while the compare
/// holds; one that counts down the last, held inverted; each of `2 << vl`
/// vectors' elements, with `PredCountTest`'s flags.
#[test]
fn a_while_counts_up_from_the_first_element_or_down_from_the_last() {
    let mut cpu = streaming();
    // 3, 4, ... 9 < 10: seven of 32 bytes.
    (cpu.x[0], cpu.x[1]) = (3, 10);
    assert_eq!(exec(&mut cpu, WHILELT_PN8_B), None);
    assert_eq!(p_bits(&cpu, 8), (7 << 1) | 1);
    assert_eq!(cpu.pstate.nzcv(), nzcv(true, false, true));
    // 0 to 39 < 40: forty of the 64 bytes of four vectors, more than two
    // vectors hold.
    (cpu.x[0], cpu.x[1]) = (0, 40);
    assert_eq!(exec(&mut cpu, WHILELT_PN8_B_VLX4), None);
    assert_eq!(p_bits(&cpu, 8), (40 << 1) | 1);
    assert_eq!(cpu.pstate.nzcv(), nzcv(true, false, true));
    // Every one of 32 halfwords: held as an inverted zero, the last
    // active.
    (cpu.x[2], cpu.x[3]) = (0, 100);
    assert_eq!(exec(&mut cpu, WHILELO_PN10_H), None);
    assert_eq!(p_bits(&cpu, 10), 0x8000 | 0b10);
    assert_eq!(cpu.pstate.nzcv(), nzcv(true, false, false));
    // Down from the last of four doublewords: 5, 4, 3 > 2, then 2 is not.
    // Three active, the last ones: one inactive, inverted. The first is
    // not active, the last is.
    (cpu.x[0], cpu.x[1]) = (5, 2);
    assert_eq!(exec(&mut cpu, WHILEGT_PN11_D), None);
    assert_eq!(p_bits(&cpu, 11), 0x8000 | (1 << 4) | 0b1000);
    assert_eq!(cpu.pstate.nzcv(), nzcv(false, false, false));
    // Unsigned: 1 is not at or above 2^64 - 1, so no element: zero, Z and
    // C set. Signed it would be all of them.
    (cpu.x[0], cpu.x[1]) = (1, u64::MAX);
    assert_eq!(exec(&mut cpu, WHILEHS_PN12_S), None);
    assert_eq!(p_bits(&cpu, 12), 0);
    assert_eq!(cpu.pstate.nzcv(), nzcv(false, true, true));
}

/// The WHILE of a pair writes the first vector's elements to `Pd` and the
/// second's to `Pd + 1`, with `PredTest`'s flags over both.
#[test]
fn the_while_of_a_pair_splits_its_elements_across_two_predicates() {
    let mut cpu = streaming();
    (cpu.x[0], cpu.x[1]) = (0, 20);
    assert_eq!(exec(&mut cpu, WHILELT_PAIR_B), None);
    assert_eq!((p_bits(&cpu, 0), p_bits(&cpu, 1)), (0xFFFF, 0x000F));
    assert_eq!(cpu.pstate.nzcv(), nzcv(true, false, true));
    // Down from the last of eight words: 5 to 0 are at or above 0, -1 is
    // not; elements 2 to 7.
    (cpu.x[0], cpu.x[1]) = (5, 0);
    assert_eq!(exec(&mut cpu, WHILEGE_PAIR_S), None);
    assert_eq!((p_bits(&cpu, 2), p_bits(&cpu, 3)), (0x1100, 0x1111));
    assert_eq!(cpu.pstate.nzcv(), nzcv(false, false, false));
}

/// PEXT reads the counter's predicate at the instruction's element size,
/// whatever size the counter holds; a part is a vector's worth of those
/// elements, a pair two parts from `2 * i1`, its second predicate
/// `(Pd + 1) MOD 16`.
#[test]
fn pext_reads_a_part_of_the_counter_at_its_own_element_size() {
    let mut cpu = streaming();
    // Three halfwords: predicate bits 0, 2 and 4, as bytes.
    set_p_bits(&mut cpu, 8, (3 << 2) | 0b10);
    assert_eq!(exec(&mut cpu, PEXT_P0_B), None);
    assert_eq!(p_bits(&cpu, 0), 0b1_0101);
    // The first five bytes inactive (inverted), as halfwords: elements 0
    // to 2 (bits 0, 2, 4) inactive, 3 to 7 active.
    set_p_bits(&mut cpu, 9, 0x8000 | (5 << 1) | 1);
    assert_eq!(exec(&mut cpu, PEXT_P1_H), None);
    assert_eq!(p_bits(&cpu, 1), 0x5540);
    // Ten words: part 1 of the pair is words 8 to 15; 8 and 9 active.
    set_p_bits(&mut cpu, 10, (10 << 3) | 0b100);
    assert_eq!(exec(&mut cpu, PEXT_PAIR_S), None);
    assert_eq!((p_bits(&cpu, 2), p_bits(&cpu, 3)), (0x11, 0));
    // Twenty bytes, to P15 and P0.
    set_p_bits(&mut cpu, 8, (20 << 1) | 1);
    assert_eq!(exec(&mut cpu, PEXT_PAIR_WRAP), None);
    assert_eq!((p_bits(&cpu, 15), p_bits(&cpu, 0)), (0xFFFF, 0x000F));
    // A low nibble of zero activates nothing.
    set_p_bits(&mut cpu, 8, 0x8030);
    assert_eq!(exec(&mut cpu, PEXT_P0_B), None);
    assert_eq!(p_bits(&cpu, 0), 0);
}

/// CNTP counts the active elements of the first `2 << vl` vectors at its
/// own element size.
#[test]
fn cntp_counts_the_active_elements_of_two_or_four_vectors() {
    let mut cpu = streaming();
    // Bytes 5 to 63 active; 27 of the first 32.
    set_p_bits(&mut cpu, 8, 0x8000 | (5 << 1) | 1);
    assert_eq!(exec(&mut cpu, CNTP_B), None);
    assert_eq!(cpu.x[0], 27);
    // Three halfwords, read as doublewords: bits 0, 2 and 4 set, of which
    // doubleword 0's bit 0 alone is a doubleword's first.
    set_p_bits(&mut cpu, 9, (3 << 2) | 0b10);
    assert_eq!(exec(&mut cpu, CNTP_D), None);
    assert_eq!(cpu.x[1], 1);
    // Every byte, as doublewords: two each of four vectors, or of two.
    set_p_bits(&mut cpu, 9, 0x8001);
    assert_eq!(exec(&mut cpu, CNTP_D), None);
    assert_eq!(cpu.x[1], 8);
    assert_eq!(exec(&mut cpu, CNTP_D_VLX2), None);
    assert_eq!(cpu.x[1], 4);
}

/// A multi-vector load fills one register after another from memory, the
/// counter's element `r * elements + e` governing element `e` of register
/// `r`, the inactive ones zero; the immediate offset counts whole
/// transfers.
#[test]
fn a_multi_vector_load_fills_one_register_after_another() {
    let mut cpu = streaming();
    let mut ram = counting(0, 192);
    // Twenty bytes from byte 4: all of Z0, four of Z1.
    set_p_bits(&mut cpu, 8, (20 << 1) | 1);
    (cpu.x[0], cpu.x[1]) = (DATA, 4);
    set_z_bytes(&mut cpu, 1, &[0xEE; 16]);
    assert_eq!(exec_in(&mut cpu, &mut ram, LD1B_2), None);
    assert_eq!(z_bytes(&cpu, 0), counting(4, 16));
    let mut z1 = counting(20, 4);
    z1.resize(16, 0);
    assert_eq!(z_bytes(&cpu, 1), z1);
    // Every word of four registers, 64 bytes on (#4, MUL VL: one transfer).
    set_p_bits(&mut cpu, 9, 0x8004);
    assert_eq!(exec_in(&mut cpu, &mut ram, LD1W_4_IMM), None);
    for (r, z) in (4..8).enumerate() {
        assert_eq!(z_bytes(&cpu, z), counting(64 + 16 * r as u8, 16), "z{z}");
    }
}

/// The strided forms transfer `Zt` and the registers 8 apart (two) or 4
/// apart (four), leaving the registers between them alone.
#[test]
fn the_strided_forms_skip_the_registers_between() {
    let mut cpu = streaming();
    let mut ram = counting(0, 128);
    for z in 0..32 {
        set_z_bytes(&mut cpu, z, &[0xEE; 16]);
    }
    set_p_bits(&mut cpu, 8, 0x8001);
    (cpu.x[0], cpu.x[1]) = (DATA, 0);
    assert_eq!(exec_in(&mut cpu, &mut ram, LD1B_2X8), None);
    assert_eq!((z_bytes(&cpu, 0), z_bytes(&cpu, 8)), (counting(0, 16), counting(16, 16)));
    assert!((1..8).all(|z| z_bytes(&cpu, z) == [0xEE; 16]));
    // The last three of eight doublewords active (five inactive,
    // inverted): element 1 of Z27 and both of Z31, from byte 8 on.
    set_p_bits(&mut cpu, 11, 0x8000 | (5 << 4) | 0b1000);
    cpu.x[1] = 1;
    assert_eq!(exec_in(&mut cpu, &mut ram, LD1D_4X4), None);
    assert_eq!((z_bytes(&cpu, 19), z_bytes(&cpu, 23)), (vec![0; 16], vec![0; 16]));
    let mut z27 = vec![0; 8];
    z27.extend(counting(48, 8));
    assert_eq!(z_bytes(&cpu, 27), z27);
    assert_eq!(z_bytes(&cpu, 31), counting(56, 16));
    assert!([20, 21, 22, 24, 28, 30].iter().all(|&z| z_bytes(&cpu, z) == [0xEE; 16]));
}

/// A multi-vector store writes each register's active elements in turn and
/// leaves the memory of the inactive ones alone.
#[test]
fn a_multi_vector_store_writes_the_active_elements_alone() {
    let mut cpu = streaming();
    let mut ram = vec![0xEE; 192];
    for z in 0..32 {
        set_z_bytes(&mut cpu, z, &pattern(z));
    }
    // Three doublewords: Z0's two and Z1's first, from byte 8.
    set_p_bits(&mut cpu, 8, (3 << 4) | 0b1000);
    (cpu.x[0], cpu.x[1]) = (DATA, 1);
    assert_eq!(exec_in(&mut cpu, &mut ram, ST1D_2), None);
    assert_eq!(ram[8..24], pattern(0)[..]);
    assert_eq!(ram[24..32], pattern(1)[..8]);
    assert!(ram[..8].iter().chain(&ram[32..]).all(|&b| b == 0xEE));
    // Halfwords of Z16 and Z24 under every element but the first 9 bytes'
    // (inverted), read as halfwords: from halfword 5 of Z16 on.
    let mut ram = vec![0xEE; 64];
    set_p_bits(&mut cpu, 8, 0x8000 | (9 << 1) | 1);
    cpu.x[1] = 0;
    assert_eq!(exec_in(&mut cpu, &mut ram, ST1H_2X8), None);
    assert!(ram[..10].iter().all(|&b| b == 0xEE));
    assert_eq!(ram[10..16], pattern(16)[10..]);
    assert_eq!(ram[16..32], pattern(24)[..]);
    // Every word of Z18, Z22, Z26, Z30 at 64 bytes on.
    let mut ram = vec![0xEE; 128];
    set_p_bits(&mut cpu, 12, 0x8004);
    assert_eq!(exec_in(&mut cpu, &mut ram, STNT1W_4X4_IMM), None);
    for (r, z) in [18, 22, 26, 30].into_iter().enumerate() {
        assert_eq!(ram[64 + 16 * r..80 + 16 * r], pattern(z)[..], "z{z}");
    }
}

/// A load that faults part-way writes no register; a store keeps the
/// elements it stored before the faulting one. A negative immediate
/// counts whole transfers back.
#[test]
fn a_fault_part_way_writes_no_register_and_keeps_the_stores_before_it() {
    let mut cpu = streaming();
    let mut ram = counting(0, 64);
    set_z_bytes(&mut cpu, 0, &[0xEE; 16]);
    set_p_bits(&mut cpu, 8, 0x8001);
    // 32 bytes from 16 before the end of the RAM.
    (cpu.x[0], cpu.x[1]) = (DATA + 48, 0);
    assert_eq!(exec_in(&mut cpu, &mut ram, LD1B_2), Some(external_abort(false)));
    assert_eq!(cpu.sys.far_el1, DATA + 64);
    assert_eq!(z_bytes(&cpu, 0), [0xEE; 16]);
    // Four registers 64 bytes back from 112: bytes 48 to 63, then a fault.
    for z in 28..32 {
        set_z_bytes(&mut cpu, z, &[z as u8; 16]);
    }
    set_p_bits(&mut cpu, 15, 0x8001);
    cpu.x[0] = DATA + 112;
    assert_eq!(exec_in(&mut cpu, &mut ram, ST1B_4_IMM), Some(external_abort(true)));
    assert_eq!(cpu.sys.far_el1, DATA + 64);
    assert_eq!(ram[48..], [28; 16]);
    assert_eq!(ram[..48], counting(0, 48)[..]);
}

/// The predicate-as-counter instructions and the multi-vector accesses
/// take the trap outside Streaming SVE mode, and are UNDEFINED without
/// FEAT_SME2.
#[test]
fn they_need_streaming_mode_and_sme2() {
    let mut cpu = processor(Implementation::EVERY_FEATURE);
    for insn in [PTRUE_PN8_B, WHILELT_PN8_B, WHILELT_PAIR_B, PEXT_P0_B, CNTP_B, LD1B_2, ST1H_2X8] {
        assert_eq!(exec(&mut cpu, insn), Some(NOT_STREAMING), "{insn:#010x}");
    }
    let features: Vec<Feature> = Feature::ALL.into_iter().filter(|&f| f != Feature::Sme2).collect();
    let mut cpu = processor(Implementation {
        features: Features::of(&features),
        max_svl: VecLen::MAX,
    });
    cpu.set_pstate_sm(true);
    for insn in [PTRUE_PN8_B, WHILELT_PN8_B, WHILELT_PAIR_B, PEXT_P0_B, CNTP_B, LD1B_2, ST1H_2X8] {
        assert_eq!(exec(&mut cpu, insn), Some(UNDEFINED), "{insn:#010x}");
    }
}
