//! Arm group `simd_dp` (Arm ARM C4.1, "Data Processing -- Scalar
//! Floating-Point and Advanced SIMD"): the A64 space with `op1 = x111`.
//! Its classes are in `asimd` (vector), `asisd` (scalar) and `float`.

use super::asimd;
use super::asisd;
use super::crypto;
use super::float;
use super::Encoding;
use crate::cpu::exec::bits;

pub(super) fn simd_dp(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 31, 28);
    match op0 {
        0b0000 | 0b0010 | 0b0100 | 0b0110 => vector(word),
        0b0101 | 0b0111 => scalar(word),
        0b0001 | 0b0011 | 0b1001 | 0b1011 => floating_point(word),
        // 1100 holds crypto3_imm2, cryptosha512_3, crypto4, crypto3_imm6 and
        // cryptosha512_2 (FEAT_SHA512, FEAT_SHA3, FEAT_SM3, FEAT_SM4).
        _ => None,
    }
}

/// `op0 = 0xx0`: Advanced SIMD vector, and AES.
fn vector(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 31, 28);
    let op1 = bits(word, 24, 23);
    let op2 = bits(word, 22, 19);
    // op1<1>, then op3<0>.
    match (bits(word, 24, 24), bits(word, 10, 10)) {
        (0b1, 0b0) => asimd::asimdelem(word).map(Encoding::Asimdelem),
        (0b1, 0b1) => match (op1, op2) {
            (0b10, 0b0000) => asimd::asimdimm(word).map(Encoding::Asimdimm),
            (0b10, _) => asimd::asimdshf(word).map(Encoding::Asimdshf),
            _ => None,
        },
        // op1 = 0x, by op2<2>.
        (0b0, _) => match bits(word, 21, 21) {
            // op3<1:0>.
            0b1 => match bits(word, 11, 10) {
                0b01 | 0b11 => asimd::asimdsame(word).map(Encoding::Asimdsame),
                0b00 => asimd::asimddiff(word).map(Encoding::Asimddiff),
                // op2<1:0> and op3<8:7>; op2 = 1111 is asimdmiscfp16.
                0b10 => match (bits(word, 20, 19), bits(word, 18, 17)) {
                    (0b00, 0b00) => asimd::asimdmisc(word).map(Encoding::Asimdmisc),
                    (0b10, 0b00) => asimd::asimdall(word).map(Encoding::Asimdall),
                    (0b11, 0b00) => match bits(word, 22, 22) {
                        0b1 => asimd::asimdmiscfp16(word).map(Encoding::Asimdmiscfp16),
                        _ => None,
                    },
                    (0b01, 0b00) => match op0 {
                        0b0100 => crypto::cryptoaes(word).map(Encoding::Cryptoaes),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            },
            // op3<0> and op3<5>; op2 = 10xx with op3<0> = 1 and op3<5> = 0
            // is asimdsamefp16.
            0b0 => match (bits(word, 10, 10), bits(word, 15, 15)) {
                (0b1, 0b1) => asimd::asimdsame2(word).map(Encoding::Asimdsame2),
                (0b1, 0b0) => match (op1, bits(word, 22, 22)) {
                    (0b00, 0b0) => asimd::asimdins(word).map(Encoding::Asimdins),
                    (_, 0b1) => match bits(word, 14, 14) {
                        0b0 => asimd::asimdsamefp16(word).map(Encoding::Asimdsamefp16),
                        _ => None,
                    },
                    _ => None,
                },
                // op0<1>, then op3<1>.
                (0b0, 0b0) => match (bits(word, 29, 29), bits(word, 11, 11)) {
                    (0b0, 0b0) => asimd::asimdtbl(word).map(Encoding::Asimdtbl),
                    (0b0, 0b1) => asimd::asimdperm(word).map(Encoding::Asimdperm),
                    (0b1, _) => asimd::asimdext(word).map(Encoding::Asimdext),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// `op0 = 01x1`: Advanced SIMD scalar, and SHA.
fn scalar(word: u32) -> Option<Encoding> {
    let op0 = bits(word, 31, 28);
    let op1 = bits(word, 24, 23);
    // op1<1>, then op3<0>.
    match (bits(word, 24, 24), bits(word, 10, 10)) {
        (0b1, 0b0) => asisd::asisdelem(word).map(Encoding::Asisdelem),
        (0b1, 0b1) => match op1 {
            0b10 => asisd::asisdshf(word).map(Encoding::Asisdshf),
            _ => None,
        },
        // op1 = 0x, by op2<2>.
        (0b0, _) => match bits(word, 21, 21) {
            // op3<1:0>.
            0b1 => match bits(word, 11, 10) {
                0b01 | 0b11 => asisd::asisdsame(word).map(Encoding::Asisdsame),
                0b00 => asisd::asisddiff(word).map(Encoding::Asisddiff),
                // op2<1:0> and op3<8:7>; op2 = 1111 is asisdmiscfp16.
                0b10 => match (bits(word, 20, 19), bits(word, 18, 17)) {
                    (0b00, 0b00) => asisd::asisdmisc(word).map(Encoding::Asisdmisc),
                    (0b10, 0b00) => asisd::asisdpair(word).map(Encoding::Asisdpair),
                    (0b11, 0b00) => match bits(word, 22, 22) {
                        0b1 => asisd::asisdmiscfp16(word).map(Encoding::Asisdmiscfp16),
                        _ => None,
                    },
                    (0b01, 0b00) => match op0 {
                        0b0101 => crypto::cryptosha2(word).map(Encoding::Cryptosha2),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            },
            // op3<0> and op3<5>; op2 = 10xx with op3<0> = 1 and op3<5> = 0
            // is asisdsamefp16.
            0b0 => match (bits(word, 10, 10), bits(word, 15, 15)) {
                (0b1, 0b1) => asisd::asisdsame2(word).map(Encoding::Asisdsame2),
                (0b1, 0b0) => match (op1, bits(word, 22, 22)) {
                    (0b00, 0b0) => asisd::asisdone(word).map(Encoding::Asisdone),
                    (_, 0b1) => match bits(word, 14, 14) {
                        0b0 => asisd::asisdsamefp16(word).map(Encoding::Asisdsamefp16),
                        _ => None,
                    },
                    _ => None,
                },
                // op0, then op3<1>.
                (0b0, 0b0) => match (op0, bits(word, 11, 11)) {
                    (0b0101, 0b0) => crypto::cryptosha3(word).map(Encoding::Cryptosha3),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// `op0 = x0x1`: scalar floating point.
fn floating_point(word: u32) -> Option<Encoding> {
    // op1<1>, then op2<2>.
    match (bits(word, 24, 24), bits(word, 21, 21)) {
        (0b1, _) => float::floatdp3(word).map(Encoding::Floatdp3),
        (0b0, 0b0) => float::float2fix(word).map(Encoding::Float2fix),
        // op3<1:0>, then op3<5:2>.
        (0b0, 0b1) => match bits(word, 11, 10) {
            0b00 => match (bits(word, 15, 15), bits(word, 14, 14), bits(word, 13, 13), bits(word, 12, 12)) {
                (_, _, _, 0b1) => float::floatimm(word).map(Encoding::Floatimm),
                (_, _, 0b1, 0b0) => float::floatcmp(word).map(Encoding::Floatcmp),
                (_, 0b1, 0b0, 0b0) => float::floatdp1(word).map(Encoding::Floatdp1),
                (0b0, 0b0, 0b0, 0b0) => float::float2int(word).map(Encoding::Float2int),
                _ => None,
            },
            0b01 => float::floatccmp(word).map(Encoding::Floatccmp),
            0b10 => float::floatdp2(word).map(Encoding::Floatdp2),
            0b11 => float::floatsel(word).map(Encoding::Floatsel),
            _ => None,
        },
        _ => None,
    }
}
