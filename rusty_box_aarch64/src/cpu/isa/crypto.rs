//! The Cryptographic Extension classes of Arm group `simd_dp` (Arm ARM C4.1,
//! "Data Processing -- Scalar Floating-Point and Advanced SIMD"):
//! `cryptoaes` (FEAT_AES), and `cryptosha3` and `cryptosha2` (FEAT_SHA1,
//! FEAT_SHA256). The Cortex-A57 has no FEAT_SHA512, FEAT_SHA3, FEAT_SM3 or
//! FEAT_SM4, whose classes are absent.

use super::Feature;
use crate::cpu::exec::bits;

/// Arm group `cryptoaes`: Cryptographic AES.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Cryptoaes {
    AeseB,
    AesdB,
    AesmcB,
    AesimcB,
}

/// Arm group `cryptosha3`: Cryptographic three-register SHA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Cryptosha3 {
    Sha1cQsv,
    Sha1pQsv,
    Sha1mQsv,
    Sha1su0Vvv,
    Sha256hQqv,
    Sha256h2Qqv,
    Sha256su1Vvv,
}

/// Arm group `cryptosha2`: Cryptographic two-register SHA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Cryptosha2 {
    Sha1hSs,
    Sha1su1Vv,
    Sha256su0Vv,
}

pub(super) fn cryptoaes(word: u32) -> Option<Cryptoaes> {
    let size = bits(word, 23, 22);
    let opcode = bits(word, 16, 12);
    match (size, opcode) {
        (0b00, 0b00100) => Some(Cryptoaes::AeseB),
        (0b00, 0b00101) => Some(Cryptoaes::AesdB),
        (0b00, 0b00110) => Some(Cryptoaes::AesmcB),
        (0b00, 0b00111) => Some(Cryptoaes::AesimcB),
        _ => None,
    }
}

pub(super) fn cryptosha3(word: u32) -> Option<Cryptosha3> {
    let size = bits(word, 23, 22);
    let opcode = bits(word, 14, 12);
    match (size, opcode) {
        (0b00, 0b000) => Some(Cryptosha3::Sha1cQsv),
        (0b00, 0b001) => Some(Cryptosha3::Sha1pQsv),
        (0b00, 0b010) => Some(Cryptosha3::Sha1mQsv),
        (0b00, 0b011) => Some(Cryptosha3::Sha1su0Vvv),
        (0b00, 0b100) => Some(Cryptosha3::Sha256hQqv),
        (0b00, 0b101) => Some(Cryptosha3::Sha256h2Qqv),
        (0b00, 0b110) => Some(Cryptosha3::Sha256su1Vvv),
        _ => None,
    }
}

pub(super) fn cryptosha2(word: u32) -> Option<Cryptosha2> {
    let size = bits(word, 23, 22);
    let opcode = bits(word, 16, 12);
    match (size, opcode) {
        (0b00, 0b00000) => Some(Cryptosha2::Sha1hSs),
        (0b00, 0b00001) => Some(Cryptosha2::Sha1su1Vv),
        (0b00, 0b00010) => Some(Cryptosha2::Sha256su0Vv),
        _ => None,
    }
}

impl Cryptosha3 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Sha1cQsv | Self::Sha1pQsv | Self::Sha1mQsv | Self::Sha1su0Vvv => Feature::Sha1,
            Self::Sha256hQqv | Self::Sha256h2Qqv | Self::Sha256su1Vvv => Feature::Sha256,
        }
    }
}

impl Cryptosha2 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Sha1hSs | Self::Sha1su1Vv => Feature::Sha1,
            Self::Sha256su0Vv => Feature::Sha256,
        }
    }
}
