//! The scalar floating-point groups of Arm group `simd_dp` (Arm ARM C4.1,
//! "Data Processing -- Scalar Floating-Point and Advanced SIMD", the rows
//! with `op0 = x0x1`). `ftype = 11`, half precision, is FEAT_FP16 except
//! in the precision conversions.

use super::Feature;
use crate::cpu::exec::bits;

/// Arm group `float2fix`: Conversion between floating-point and fixed-point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Float2fix {
    ScvtfS32,
    UcvtfS32,
    Fcvtzs32s,
    Fcvtzu32s,
    ScvtfD32,
    UcvtfD32,
    Fcvtzs32d,
    Fcvtzu32d,
    ScvtfH32,
    UcvtfH32,
    Fcvtzs32h,
    Fcvtzu32h,
    ScvtfS64,
    UcvtfS64,
    Fcvtzs64s,
    Fcvtzu64s,
    ScvtfD64,
    UcvtfD64,
    Fcvtzs64d,
    Fcvtzu64d,
    ScvtfH64,
    UcvtfH64,
    Fcvtzs64h,
    Fcvtzu64h,
}

impl Float2fix {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::ScvtfS32
            | Self::UcvtfS32
            | Self::Fcvtzs32s
            | Self::Fcvtzu32s
            | Self::ScvtfD32
            | Self::UcvtfD32
            | Self::Fcvtzs32d
            | Self::Fcvtzu32d
            | Self::ScvtfS64
            | Self::UcvtfS64
            | Self::Fcvtzs64s
            | Self::Fcvtzu64s
            | Self::ScvtfD64
            | Self::UcvtfD64
            | Self::Fcvtzs64d
            | Self::Fcvtzu64d => Feature::Fp,
            Self::ScvtfH32
            | Self::UcvtfH32
            | Self::Fcvtzs32h
            | Self::Fcvtzu32h
            | Self::ScvtfH64
            | Self::UcvtfH64
            | Self::Fcvtzs64h
            | Self::Fcvtzu64h => Feature::Fp16,
        }
    }
}

/// Arm group `float2int`: Conversion between floating-point and integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Float2int {
    Fcvtns32s,
    Fcvtnu32s,
    ScvtfS32,
    UcvtfS32,
    Fcvtas32s,
    Fcvtau32s,
    Fmov32s,
    FmovS32,
    Fcvtps32s,
    Fcvtpu32s,
    Fcvtms32s,
    Fcvtmu32s,
    Fcvtzs32s,
    Fcvtzu32s,
    Fcvtns32d,
    Fcvtnu32d,
    ScvtfD32,
    UcvtfD32,
    Fcvtas32d,
    Fcvtau32d,
    Fcvtps32d,
    Fcvtpu32d,
    Fcvtms32d,
    Fcvtmu32d,
    Fcvtzs32d,
    Fcvtzu32d,
    Fjcvtzs32d,
    Fcvtns32h,
    Fcvtnu32h,
    ScvtfH32,
    UcvtfH32,
    Fcvtas32h,
    Fcvtau32h,
    Fmov32h,
    FmovH32,
    Fcvtps32h,
    Fcvtpu32h,
    Fcvtms32h,
    Fcvtmu32h,
    Fcvtzs32h,
    Fcvtzu32h,
    Fcvtns64s,
    Fcvtnu64s,
    ScvtfS64,
    UcvtfS64,
    Fcvtas64s,
    Fcvtau64s,
    Fcvtps64s,
    Fcvtpu64s,
    Fcvtms64s,
    Fcvtmu64s,
    Fcvtzs64s,
    Fcvtzu64s,
    Fcvtns64d,
    Fcvtnu64d,
    ScvtfD64,
    UcvtfD64,
    Fcvtas64d,
    Fcvtau64d,
    Fmov64d,
    FmovD64,
    Fcvtps64d,
    Fcvtpu64d,
    Fcvtms64d,
    Fcvtmu64d,
    Fcvtzs64d,
    Fcvtzu64d,
    Fmov64vx,
    FmovV64i,
    Fcvtns64h,
    Fcvtnu64h,
    ScvtfH64,
    UcvtfH64,
    Fcvtas64h,
    Fcvtau64h,
    Fmov64h,
    FmovH64,
    Fcvtps64h,
    Fcvtpu64h,
    Fcvtms64h,
    Fcvtmu64h,
    Fcvtzs64h,
    Fcvtzu64h,
}

impl Float2int {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Fcvtns32s
            | Self::Fcvtnu32s
            | Self::ScvtfS32
            | Self::UcvtfS32
            | Self::Fcvtas32s
            | Self::Fcvtau32s
            | Self::Fmov32s
            | Self::FmovS32
            | Self::Fcvtps32s
            | Self::Fcvtpu32s
            | Self::Fcvtms32s
            | Self::Fcvtmu32s
            | Self::Fcvtzs32s
            | Self::Fcvtzu32s
            | Self::Fcvtns32d
            | Self::Fcvtnu32d
            | Self::ScvtfD32
            | Self::UcvtfD32
            | Self::Fcvtas32d
            | Self::Fcvtau32d
            | Self::Fcvtps32d
            | Self::Fcvtpu32d
            | Self::Fcvtms32d
            | Self::Fcvtmu32d
            | Self::Fcvtzs32d
            | Self::Fcvtzu32d
            | Self::Fcvtns64s
            | Self::Fcvtnu64s
            | Self::ScvtfS64
            | Self::UcvtfS64
            | Self::Fcvtas64s
            | Self::Fcvtau64s
            | Self::Fcvtps64s
            | Self::Fcvtpu64s
            | Self::Fcvtms64s
            | Self::Fcvtmu64s
            | Self::Fcvtzs64s
            | Self::Fcvtzu64s
            | Self::Fcvtns64d
            | Self::Fcvtnu64d
            | Self::ScvtfD64
            | Self::UcvtfD64
            | Self::Fcvtas64d
            | Self::Fcvtau64d
            | Self::Fmov64d
            | Self::FmovD64
            | Self::Fcvtps64d
            | Self::Fcvtpu64d
            | Self::Fcvtms64d
            | Self::Fcvtmu64d
            | Self::Fcvtzs64d
            | Self::Fcvtzu64d
            | Self::Fmov64vx
            | Self::FmovV64i => Feature::Fp,
            Self::Fjcvtzs32d => Feature::Jscvt,
            Self::Fcvtns32h
            | Self::Fcvtnu32h
            | Self::ScvtfH32
            | Self::UcvtfH32
            | Self::Fcvtas32h
            | Self::Fcvtau32h
            | Self::Fmov32h
            | Self::FmovH32
            | Self::Fcvtps32h
            | Self::Fcvtpu32h
            | Self::Fcvtms32h
            | Self::Fcvtmu32h
            | Self::Fcvtzs32h
            | Self::Fcvtzu32h
            | Self::Fcvtns64h
            | Self::Fcvtnu64h
            | Self::ScvtfH64
            | Self::UcvtfH64
            | Self::Fcvtas64h
            | Self::Fcvtau64h
            | Self::Fmov64h
            | Self::FmovH64
            | Self::Fcvtps64h
            | Self::Fcvtpu64h
            | Self::Fcvtms64h
            | Self::Fcvtmu64h
            | Self::Fcvtzs64h
            | Self::Fcvtzu64h => Feature::Fp16,
        }
    }
}

/// Arm group `floatdp1`: Floating-point data-processing (1 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatdp1 {
    FmovS,
    FabsS,
    FnegS,
    FsqrtS,
    FcvtDs,
    FcvtHs,
    FrintnS,
    FrintpS,
    FrintmS,
    FrintzS,
    FrintaS,
    FrintxS,
    FrintiS,
    Frint32zS,
    Frint32xS,
    Frint64zS,
    Frint64xS,
    FmovD,
    FabsD,
    FnegD,
    FsqrtD,
    FcvtSd,
    BfcvtBs,
    FcvtHd,
    FrintnD,
    FrintpD,
    FrintmD,
    FrintzD,
    FrintaD,
    FrintxD,
    FrintiD,
    Frint32zD,
    Frint32xD,
    Frint64zD,
    Frint64xD,
    FmovH,
    FabsH,
    FnegH,
    FsqrtH,
    FcvtSh,
    FcvtDh,
    FrintnH,
    FrintpH,
    FrintmH,
    FrintzH,
    FrintaH,
    FrintxH,
    FrintiH,
}

impl Floatdp1 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::FmovS
            | Self::FabsS
            | Self::FnegS
            | Self::FsqrtS
            | Self::FcvtDs
            | Self::FcvtHs
            | Self::FrintnS
            | Self::FrintpS
            | Self::FrintmS
            | Self::FrintzS
            | Self::FrintaS
            | Self::FrintxS
            | Self::FrintiS
            | Self::FmovD
            | Self::FabsD
            | Self::FnegD
            | Self::FsqrtD
            | Self::FcvtSd
            | Self::FcvtHd
            | Self::FrintnD
            | Self::FrintpD
            | Self::FrintmD
            | Self::FrintzD
            | Self::FrintaD
            | Self::FrintxD
            | Self::FrintiD
            | Self::FcvtSh
            | Self::FcvtDh => Feature::Fp,
            Self::Frint32zS
            | Self::Frint32xS
            | Self::Frint64zS
            | Self::Frint64xS
            | Self::Frint32zD
            | Self::Frint32xD
            | Self::Frint64zD
            | Self::Frint64xD => Feature::Frintts,
            Self::BfcvtBs => Feature::Bf16,
            Self::FmovH
            | Self::FabsH
            | Self::FnegH
            | Self::FsqrtH
            | Self::FrintnH
            | Self::FrintpH
            | Self::FrintmH
            | Self::FrintzH
            | Self::FrintaH
            | Self::FrintxH
            | Self::FrintiH => Feature::Fp16,
        }
    }
}

/// Arm group `floatcmp`: Floating-point compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatcmp {
    FcmpS,
    FcmpSz,
    FcmpeS,
    FcmpeSz,
    FcmpD,
    FcmpDz,
    FcmpeD,
    FcmpeDz,
    FcmpH,
    FcmpHz,
    FcmpeH,
    FcmpeHz,
}

impl Floatcmp {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::FcmpS
            | Self::FcmpSz
            | Self::FcmpeS
            | Self::FcmpeSz
            | Self::FcmpD
            | Self::FcmpDz
            | Self::FcmpeD
            | Self::FcmpeDz => Feature::Fp,
            Self::FcmpH | Self::FcmpHz | Self::FcmpeH | Self::FcmpeHz => Feature::Fp16,
        }
    }
}

/// Arm group `floatimm`: Floating-point immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatimm {
    FmovS,
    FmovD,
    FmovH,
}

impl Floatimm {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::FmovS | Self::FmovD => Feature::Fp,
            Self::FmovH => Feature::Fp16,
        }
    }
}

/// Arm group `floatccmp`: Floating-point conditional compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatccmp {
    FccmpS,
    FccmpeS,
    FccmpD,
    FccmpeD,
    FccmpH,
    FccmpeH,
}

impl Floatccmp {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::FccmpS | Self::FccmpeS | Self::FccmpD | Self::FccmpeD => Feature::Fp,
            Self::FccmpH | Self::FccmpeH => Feature::Fp16,
        }
    }
}

/// Arm group `floatdp2`: Floating-point data-processing (2 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatdp2 {
    FmulS,
    FdivS,
    FaddS,
    FsubS,
    FmaxS,
    FminS,
    FmaxnmS,
    FminnmS,
    FnmulS,
    FmulD,
    FdivD,
    FaddD,
    FsubD,
    FmaxD,
    FminD,
    FmaxnmD,
    FminnmD,
    FnmulD,
    FmulH,
    FdivH,
    FaddH,
    FsubH,
    FmaxH,
    FminH,
    FmaxnmH,
    FminnmH,
    FnmulH,
}

impl Floatdp2 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::FmulS
            | Self::FdivS
            | Self::FaddS
            | Self::FsubS
            | Self::FmaxS
            | Self::FminS
            | Self::FmaxnmS
            | Self::FminnmS
            | Self::FnmulS
            | Self::FmulD
            | Self::FdivD
            | Self::FaddD
            | Self::FsubD
            | Self::FmaxD
            | Self::FminD
            | Self::FmaxnmD
            | Self::FminnmD
            | Self::FnmulD => Feature::Fp,
            Self::FmulH
            | Self::FdivH
            | Self::FaddH
            | Self::FsubH
            | Self::FmaxH
            | Self::FminH
            | Self::FmaxnmH
            | Self::FminnmH
            | Self::FnmulH => Feature::Fp16,
        }
    }
}

/// Arm group `floatsel`: Floating-point conditional select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatsel {
    FcselS,
    FcselD,
    FcselH,
}

impl Floatsel {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::FcselS | Self::FcselD => Feature::Fp,
            Self::FcselH => Feature::Fp16,
        }
    }
}

/// Arm group `floatdp3`: Floating-point data-processing (3 source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Floatdp3 {
    FmaddS,
    FmsubS,
    FnmaddS,
    FnmsubS,
    FmaddD,
    FmsubD,
    FnmaddD,
    FnmsubD,
    FmaddH,
    FmsubH,
    FnmaddH,
    FnmsubH,
}

impl Floatdp3 {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::FmaddS
            | Self::FmsubS
            | Self::FnmaddS
            | Self::FnmsubS
            | Self::FmaddD
            | Self::FmsubD
            | Self::FnmaddD
            | Self::FnmsubD => Feature::Fp,
            Self::FmaddH | Self::FmsubH | Self::FnmaddH | Self::FnmsubH => Feature::Fp16,
        }
    }
}

pub(super) fn float2fix(word: u32) -> Option<Float2fix> {
    let sf = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let rmode = bits(word, 20, 19);
    let opcode = bits(word, 18, 16);
    match (sf, s, ftype, rmode, opcode) {
        (0b0, 0b0, 0b00, 0b00, 0b010) => Some(Float2fix::ScvtfS32),
        (0b0, 0b0, 0b00, 0b00, 0b011) => Some(Float2fix::UcvtfS32),
        (0b0, 0b0, 0b00, 0b11, 0b000) => Some(Float2fix::Fcvtzs32s),
        (0b0, 0b0, 0b00, 0b11, 0b001) => Some(Float2fix::Fcvtzu32s),
        (0b0, 0b0, 0b01, 0b00, 0b010) => Some(Float2fix::ScvtfD32),
        (0b0, 0b0, 0b01, 0b00, 0b011) => Some(Float2fix::UcvtfD32),
        (0b0, 0b0, 0b01, 0b11, 0b000) => Some(Float2fix::Fcvtzs32d),
        (0b0, 0b0, 0b01, 0b11, 0b001) => Some(Float2fix::Fcvtzu32d),
        (0b0, 0b0, 0b11, 0b00, 0b010) => Some(Float2fix::ScvtfH32),
        (0b0, 0b0, 0b11, 0b00, 0b011) => Some(Float2fix::UcvtfH32),
        (0b0, 0b0, 0b11, 0b11, 0b000) => Some(Float2fix::Fcvtzs32h),
        (0b0, 0b0, 0b11, 0b11, 0b001) => Some(Float2fix::Fcvtzu32h),
        (0b1, 0b0, 0b00, 0b00, 0b010) => Some(Float2fix::ScvtfS64),
        (0b1, 0b0, 0b00, 0b00, 0b011) => Some(Float2fix::UcvtfS64),
        (0b1, 0b0, 0b00, 0b11, 0b000) => Some(Float2fix::Fcvtzs64s),
        (0b1, 0b0, 0b00, 0b11, 0b001) => Some(Float2fix::Fcvtzu64s),
        (0b1, 0b0, 0b01, 0b00, 0b010) => Some(Float2fix::ScvtfD64),
        (0b1, 0b0, 0b01, 0b00, 0b011) => Some(Float2fix::UcvtfD64),
        (0b1, 0b0, 0b01, 0b11, 0b000) => Some(Float2fix::Fcvtzs64d),
        (0b1, 0b0, 0b01, 0b11, 0b001) => Some(Float2fix::Fcvtzu64d),
        (0b1, 0b0, 0b11, 0b00, 0b010) => Some(Float2fix::ScvtfH64),
        (0b1, 0b0, 0b11, 0b00, 0b011) => Some(Float2fix::UcvtfH64),
        (0b1, 0b0, 0b11, 0b11, 0b000) => Some(Float2fix::Fcvtzs64h),
        (0b1, 0b0, 0b11, 0b11, 0b001) => Some(Float2fix::Fcvtzu64h),
        _ => None,
    }
}

pub(super) fn float2int(word: u32) -> Option<Float2int> {
    let sf = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let rmode = bits(word, 20, 19);
    let opcode = bits(word, 18, 16);
    match (sf, s, ftype, rmode, opcode) {
        (0b0, 0b0, 0b00, 0b00, 0b000) => Some(Float2int::Fcvtns32s),
        (0b0, 0b0, 0b00, 0b00, 0b001) => Some(Float2int::Fcvtnu32s),
        (0b0, 0b0, 0b00, 0b00, 0b010) => Some(Float2int::ScvtfS32),
        (0b0, 0b0, 0b00, 0b00, 0b011) => Some(Float2int::UcvtfS32),
        (0b0, 0b0, 0b00, 0b00, 0b100) => Some(Float2int::Fcvtas32s),
        (0b0, 0b0, 0b00, 0b00, 0b101) => Some(Float2int::Fcvtau32s),
        (0b0, 0b0, 0b00, 0b00, 0b110) => Some(Float2int::Fmov32s),
        (0b0, 0b0, 0b00, 0b00, 0b111) => Some(Float2int::FmovS32),
        (0b0, 0b0, 0b00, 0b01, 0b000) => Some(Float2int::Fcvtps32s),
        (0b0, 0b0, 0b00, 0b01, 0b001) => Some(Float2int::Fcvtpu32s),
        (0b0, 0b0, 0b00, 0b10, 0b000) => Some(Float2int::Fcvtms32s),
        (0b0, 0b0, 0b00, 0b10, 0b001) => Some(Float2int::Fcvtmu32s),
        (0b0, 0b0, 0b00, 0b11, 0b000) => Some(Float2int::Fcvtzs32s),
        (0b0, 0b0, 0b00, 0b11, 0b001) => Some(Float2int::Fcvtzu32s),
        (0b0, 0b0, 0b01, 0b00, 0b000) => Some(Float2int::Fcvtns32d),
        (0b0, 0b0, 0b01, 0b00, 0b001) => Some(Float2int::Fcvtnu32d),
        (0b0, 0b0, 0b01, 0b00, 0b010) => Some(Float2int::ScvtfD32),
        (0b0, 0b0, 0b01, 0b00, 0b011) => Some(Float2int::UcvtfD32),
        (0b0, 0b0, 0b01, 0b00, 0b100) => Some(Float2int::Fcvtas32d),
        (0b0, 0b0, 0b01, 0b00, 0b101) => Some(Float2int::Fcvtau32d),
        (0b0, 0b0, 0b01, 0b01, 0b000) => Some(Float2int::Fcvtps32d),
        (0b0, 0b0, 0b01, 0b01, 0b001) => Some(Float2int::Fcvtpu32d),
        (0b0, 0b0, 0b01, 0b10, 0b000) => Some(Float2int::Fcvtms32d),
        (0b0, 0b0, 0b01, 0b10, 0b001) => Some(Float2int::Fcvtmu32d),
        (0b0, 0b0, 0b01, 0b11, 0b000) => Some(Float2int::Fcvtzs32d),
        (0b0, 0b0, 0b01, 0b11, 0b001) => Some(Float2int::Fcvtzu32d),
        (0b0, 0b0, 0b01, 0b11, 0b110) => Some(Float2int::Fjcvtzs32d),
        (0b0, 0b0, 0b11, 0b00, 0b000) => Some(Float2int::Fcvtns32h),
        (0b0, 0b0, 0b11, 0b00, 0b001) => Some(Float2int::Fcvtnu32h),
        (0b0, 0b0, 0b11, 0b00, 0b010) => Some(Float2int::ScvtfH32),
        (0b0, 0b0, 0b11, 0b00, 0b011) => Some(Float2int::UcvtfH32),
        (0b0, 0b0, 0b11, 0b00, 0b100) => Some(Float2int::Fcvtas32h),
        (0b0, 0b0, 0b11, 0b00, 0b101) => Some(Float2int::Fcvtau32h),
        (0b0, 0b0, 0b11, 0b00, 0b110) => Some(Float2int::Fmov32h),
        (0b0, 0b0, 0b11, 0b00, 0b111) => Some(Float2int::FmovH32),
        (0b0, 0b0, 0b11, 0b01, 0b000) => Some(Float2int::Fcvtps32h),
        (0b0, 0b0, 0b11, 0b01, 0b001) => Some(Float2int::Fcvtpu32h),
        (0b0, 0b0, 0b11, 0b10, 0b000) => Some(Float2int::Fcvtms32h),
        (0b0, 0b0, 0b11, 0b10, 0b001) => Some(Float2int::Fcvtmu32h),
        (0b0, 0b0, 0b11, 0b11, 0b000) => Some(Float2int::Fcvtzs32h),
        (0b0, 0b0, 0b11, 0b11, 0b001) => Some(Float2int::Fcvtzu32h),
        (0b1, 0b0, 0b00, 0b00, 0b000) => Some(Float2int::Fcvtns64s),
        (0b1, 0b0, 0b00, 0b00, 0b001) => Some(Float2int::Fcvtnu64s),
        (0b1, 0b0, 0b00, 0b00, 0b010) => Some(Float2int::ScvtfS64),
        (0b1, 0b0, 0b00, 0b00, 0b011) => Some(Float2int::UcvtfS64),
        (0b1, 0b0, 0b00, 0b00, 0b100) => Some(Float2int::Fcvtas64s),
        (0b1, 0b0, 0b00, 0b00, 0b101) => Some(Float2int::Fcvtau64s),
        (0b1, 0b0, 0b00, 0b01, 0b000) => Some(Float2int::Fcvtps64s),
        (0b1, 0b0, 0b00, 0b01, 0b001) => Some(Float2int::Fcvtpu64s),
        (0b1, 0b0, 0b00, 0b10, 0b000) => Some(Float2int::Fcvtms64s),
        (0b1, 0b0, 0b00, 0b10, 0b001) => Some(Float2int::Fcvtmu64s),
        (0b1, 0b0, 0b00, 0b11, 0b000) => Some(Float2int::Fcvtzs64s),
        (0b1, 0b0, 0b00, 0b11, 0b001) => Some(Float2int::Fcvtzu64s),
        (0b1, 0b0, 0b01, 0b00, 0b000) => Some(Float2int::Fcvtns64d),
        (0b1, 0b0, 0b01, 0b00, 0b001) => Some(Float2int::Fcvtnu64d),
        (0b1, 0b0, 0b01, 0b00, 0b010) => Some(Float2int::ScvtfD64),
        (0b1, 0b0, 0b01, 0b00, 0b011) => Some(Float2int::UcvtfD64),
        (0b1, 0b0, 0b01, 0b00, 0b100) => Some(Float2int::Fcvtas64d),
        (0b1, 0b0, 0b01, 0b00, 0b101) => Some(Float2int::Fcvtau64d),
        (0b1, 0b0, 0b01, 0b00, 0b110) => Some(Float2int::Fmov64d),
        (0b1, 0b0, 0b01, 0b00, 0b111) => Some(Float2int::FmovD64),
        (0b1, 0b0, 0b01, 0b01, 0b000) => Some(Float2int::Fcvtps64d),
        (0b1, 0b0, 0b01, 0b01, 0b001) => Some(Float2int::Fcvtpu64d),
        (0b1, 0b0, 0b01, 0b10, 0b000) => Some(Float2int::Fcvtms64d),
        (0b1, 0b0, 0b01, 0b10, 0b001) => Some(Float2int::Fcvtmu64d),
        (0b1, 0b0, 0b01, 0b11, 0b000) => Some(Float2int::Fcvtzs64d),
        (0b1, 0b0, 0b01, 0b11, 0b001) => Some(Float2int::Fcvtzu64d),
        (0b1, 0b0, 0b10, 0b01, 0b110) => Some(Float2int::Fmov64vx),
        (0b1, 0b0, 0b10, 0b01, 0b111) => Some(Float2int::FmovV64i),
        (0b1, 0b0, 0b11, 0b00, 0b000) => Some(Float2int::Fcvtns64h),
        (0b1, 0b0, 0b11, 0b00, 0b001) => Some(Float2int::Fcvtnu64h),
        (0b1, 0b0, 0b11, 0b00, 0b010) => Some(Float2int::ScvtfH64),
        (0b1, 0b0, 0b11, 0b00, 0b011) => Some(Float2int::UcvtfH64),
        (0b1, 0b0, 0b11, 0b00, 0b100) => Some(Float2int::Fcvtas64h),
        (0b1, 0b0, 0b11, 0b00, 0b101) => Some(Float2int::Fcvtau64h),
        (0b1, 0b0, 0b11, 0b00, 0b110) => Some(Float2int::Fmov64h),
        (0b1, 0b0, 0b11, 0b00, 0b111) => Some(Float2int::FmovH64),
        (0b1, 0b0, 0b11, 0b01, 0b000) => Some(Float2int::Fcvtps64h),
        (0b1, 0b0, 0b11, 0b01, 0b001) => Some(Float2int::Fcvtpu64h),
        (0b1, 0b0, 0b11, 0b10, 0b000) => Some(Float2int::Fcvtms64h),
        (0b1, 0b0, 0b11, 0b10, 0b001) => Some(Float2int::Fcvtmu64h),
        (0b1, 0b0, 0b11, 0b11, 0b000) => Some(Float2int::Fcvtzs64h),
        (0b1, 0b0, 0b11, 0b11, 0b001) => Some(Float2int::Fcvtzu64h),
        _ => None,
    }
}

pub(super) fn floatdp1(word: u32) -> Option<Floatdp1> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let opcode = bits(word, 20, 15);
    match (m, s, ftype, opcode) {
        (0b0, 0b0, 0b00, 0b000000) => Some(Floatdp1::FmovS),
        (0b0, 0b0, 0b00, 0b000001) => Some(Floatdp1::FabsS),
        (0b0, 0b0, 0b00, 0b000010) => Some(Floatdp1::FnegS),
        (0b0, 0b0, 0b00, 0b000011) => Some(Floatdp1::FsqrtS),
        (0b0, 0b0, 0b00, 0b000101) => Some(Floatdp1::FcvtDs),
        (0b0, 0b0, 0b00, 0b000111) => Some(Floatdp1::FcvtHs),
        (0b0, 0b0, 0b00, 0b001000) => Some(Floatdp1::FrintnS),
        (0b0, 0b0, 0b00, 0b001001) => Some(Floatdp1::FrintpS),
        (0b0, 0b0, 0b00, 0b001010) => Some(Floatdp1::FrintmS),
        (0b0, 0b0, 0b00, 0b001011) => Some(Floatdp1::FrintzS),
        (0b0, 0b0, 0b00, 0b001100) => Some(Floatdp1::FrintaS),
        (0b0, 0b0, 0b00, 0b001110) => Some(Floatdp1::FrintxS),
        (0b0, 0b0, 0b00, 0b001111) => Some(Floatdp1::FrintiS),
        (0b0, 0b0, 0b00, 0b010000) => Some(Floatdp1::Frint32zS),
        (0b0, 0b0, 0b00, 0b010001) => Some(Floatdp1::Frint32xS),
        (0b0, 0b0, 0b00, 0b010010) => Some(Floatdp1::Frint64zS),
        (0b0, 0b0, 0b00, 0b010011) => Some(Floatdp1::Frint64xS),
        (0b0, 0b0, 0b01, 0b000000) => Some(Floatdp1::FmovD),
        (0b0, 0b0, 0b01, 0b000001) => Some(Floatdp1::FabsD),
        (0b0, 0b0, 0b01, 0b000010) => Some(Floatdp1::FnegD),
        (0b0, 0b0, 0b01, 0b000011) => Some(Floatdp1::FsqrtD),
        (0b0, 0b0, 0b01, 0b000100) => Some(Floatdp1::FcvtSd),
        (0b0, 0b0, 0b01, 0b000110) => Some(Floatdp1::BfcvtBs),
        (0b0, 0b0, 0b01, 0b000111) => Some(Floatdp1::FcvtHd),
        (0b0, 0b0, 0b01, 0b001000) => Some(Floatdp1::FrintnD),
        (0b0, 0b0, 0b01, 0b001001) => Some(Floatdp1::FrintpD),
        (0b0, 0b0, 0b01, 0b001010) => Some(Floatdp1::FrintmD),
        (0b0, 0b0, 0b01, 0b001011) => Some(Floatdp1::FrintzD),
        (0b0, 0b0, 0b01, 0b001100) => Some(Floatdp1::FrintaD),
        (0b0, 0b0, 0b01, 0b001110) => Some(Floatdp1::FrintxD),
        (0b0, 0b0, 0b01, 0b001111) => Some(Floatdp1::FrintiD),
        (0b0, 0b0, 0b01, 0b010000) => Some(Floatdp1::Frint32zD),
        (0b0, 0b0, 0b01, 0b010001) => Some(Floatdp1::Frint32xD),
        (0b0, 0b0, 0b01, 0b010010) => Some(Floatdp1::Frint64zD),
        (0b0, 0b0, 0b01, 0b010011) => Some(Floatdp1::Frint64xD),
        (0b0, 0b0, 0b11, 0b000000) => Some(Floatdp1::FmovH),
        (0b0, 0b0, 0b11, 0b000001) => Some(Floatdp1::FabsH),
        (0b0, 0b0, 0b11, 0b000010) => Some(Floatdp1::FnegH),
        (0b0, 0b0, 0b11, 0b000011) => Some(Floatdp1::FsqrtH),
        (0b0, 0b0, 0b11, 0b000100) => Some(Floatdp1::FcvtSh),
        (0b0, 0b0, 0b11, 0b000101) => Some(Floatdp1::FcvtDh),
        (0b0, 0b0, 0b11, 0b001000) => Some(Floatdp1::FrintnH),
        (0b0, 0b0, 0b11, 0b001001) => Some(Floatdp1::FrintpH),
        (0b0, 0b0, 0b11, 0b001010) => Some(Floatdp1::FrintmH),
        (0b0, 0b0, 0b11, 0b001011) => Some(Floatdp1::FrintzH),
        (0b0, 0b0, 0b11, 0b001100) => Some(Floatdp1::FrintaH),
        (0b0, 0b0, 0b11, 0b001110) => Some(Floatdp1::FrintxH),
        (0b0, 0b0, 0b11, 0b001111) => Some(Floatdp1::FrintiH),
        _ => None,
    }
}

pub(super) fn floatcmp(word: u32) -> Option<Floatcmp> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let rm = bits(word, 20, 16);
    let op = bits(word, 15, 14);
    let opcode2 = bits(word, 4, 0);
    match (m, s, ftype, op, opcode2) {
        (0b0, 0b0, 0b00, 0b00, 0b00000) => Some(Floatcmp::FcmpS),
        (0b0, 0b0, 0b00, 0b00, 0b01000) => match rm {
            0b00000 => Some(Floatcmp::FcmpSz),
            _ => None,
        },
        (0b0, 0b0, 0b00, 0b00, 0b10000) => Some(Floatcmp::FcmpeS),
        (0b0, 0b0, 0b00, 0b00, 0b11000) => match rm {
            0b00000 => Some(Floatcmp::FcmpeSz),
            _ => None,
        },
        (0b0, 0b0, 0b01, 0b00, 0b00000) => Some(Floatcmp::FcmpD),
        (0b0, 0b0, 0b01, 0b00, 0b01000) => match rm {
            0b00000 => Some(Floatcmp::FcmpDz),
            _ => None,
        },
        (0b0, 0b0, 0b01, 0b00, 0b10000) => Some(Floatcmp::FcmpeD),
        (0b0, 0b0, 0b01, 0b00, 0b11000) => match rm {
            0b00000 => Some(Floatcmp::FcmpeDz),
            _ => None,
        },
        (0b0, 0b0, 0b11, 0b00, 0b00000) => Some(Floatcmp::FcmpH),
        (0b0, 0b0, 0b11, 0b00, 0b01000) => match rm {
            0b00000 => Some(Floatcmp::FcmpHz),
            _ => None,
        },
        (0b0, 0b0, 0b11, 0b00, 0b10000) => Some(Floatcmp::FcmpeH),
        (0b0, 0b0, 0b11, 0b00, 0b11000) => match rm {
            0b00000 => Some(Floatcmp::FcmpeHz),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn floatimm(word: u32) -> Option<Floatimm> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let imm5 = bits(word, 9, 5);
    match (m, s, ftype, imm5) {
        (0b0, 0b0, 0b00, 0b00000) => Some(Floatimm::FmovS),
        (0b0, 0b0, 0b01, 0b00000) => Some(Floatimm::FmovD),
        (0b0, 0b0, 0b11, 0b00000) => Some(Floatimm::FmovH),
        _ => None,
    }
}

pub(super) fn floatccmp(word: u32) -> Option<Floatccmp> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let op = bits(word, 4, 4);
    match (m, s, ftype, op) {
        (0b0, 0b0, 0b00, 0b0) => Some(Floatccmp::FccmpS),
        (0b0, 0b0, 0b00, 0b1) => Some(Floatccmp::FccmpeS),
        (0b0, 0b0, 0b01, 0b0) => Some(Floatccmp::FccmpD),
        (0b0, 0b0, 0b01, 0b1) => Some(Floatccmp::FccmpeD),
        (0b0, 0b0, 0b11, 0b0) => Some(Floatccmp::FccmpH),
        (0b0, 0b0, 0b11, 0b1) => Some(Floatccmp::FccmpeH),
        _ => None,
    }
}

pub(super) fn floatdp2(word: u32) -> Option<Floatdp2> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let opcode = bits(word, 15, 12);
    match (m, s, ftype, opcode) {
        (0b0, 0b0, 0b00, 0b0000) => Some(Floatdp2::FmulS),
        (0b0, 0b0, 0b00, 0b0001) => Some(Floatdp2::FdivS),
        (0b0, 0b0, 0b00, 0b0010) => Some(Floatdp2::FaddS),
        (0b0, 0b0, 0b00, 0b0011) => Some(Floatdp2::FsubS),
        (0b0, 0b0, 0b00, 0b0100) => Some(Floatdp2::FmaxS),
        (0b0, 0b0, 0b00, 0b0101) => Some(Floatdp2::FminS),
        (0b0, 0b0, 0b00, 0b0110) => Some(Floatdp2::FmaxnmS),
        (0b0, 0b0, 0b00, 0b0111) => Some(Floatdp2::FminnmS),
        (0b0, 0b0, 0b00, 0b1000) => Some(Floatdp2::FnmulS),
        (0b0, 0b0, 0b01, 0b0000) => Some(Floatdp2::FmulD),
        (0b0, 0b0, 0b01, 0b0001) => Some(Floatdp2::FdivD),
        (0b0, 0b0, 0b01, 0b0010) => Some(Floatdp2::FaddD),
        (0b0, 0b0, 0b01, 0b0011) => Some(Floatdp2::FsubD),
        (0b0, 0b0, 0b01, 0b0100) => Some(Floatdp2::FmaxD),
        (0b0, 0b0, 0b01, 0b0101) => Some(Floatdp2::FminD),
        (0b0, 0b0, 0b01, 0b0110) => Some(Floatdp2::FmaxnmD),
        (0b0, 0b0, 0b01, 0b0111) => Some(Floatdp2::FminnmD),
        (0b0, 0b0, 0b01, 0b1000) => Some(Floatdp2::FnmulD),
        (0b0, 0b0, 0b11, 0b0000) => Some(Floatdp2::FmulH),
        (0b0, 0b0, 0b11, 0b0001) => Some(Floatdp2::FdivH),
        (0b0, 0b0, 0b11, 0b0010) => Some(Floatdp2::FaddH),
        (0b0, 0b0, 0b11, 0b0011) => Some(Floatdp2::FsubH),
        (0b0, 0b0, 0b11, 0b0100) => Some(Floatdp2::FmaxH),
        (0b0, 0b0, 0b11, 0b0101) => Some(Floatdp2::FminH),
        (0b0, 0b0, 0b11, 0b0110) => Some(Floatdp2::FmaxnmH),
        (0b0, 0b0, 0b11, 0b0111) => Some(Floatdp2::FminnmH),
        (0b0, 0b0, 0b11, 0b1000) => Some(Floatdp2::FnmulH),
        _ => None,
    }
}

pub(super) fn floatsel(word: u32) -> Option<Floatsel> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    match (m, s, ftype) {
        (0b0, 0b0, 0b00) => Some(Floatsel::FcselS),
        (0b0, 0b0, 0b01) => Some(Floatsel::FcselD),
        (0b0, 0b0, 0b11) => Some(Floatsel::FcselH),
        _ => None,
    }
}

pub(super) fn floatdp3(word: u32) -> Option<Floatdp3> {
    let m = bits(word, 31, 31);
    let s = bits(word, 29, 29);
    let ftype = bits(word, 23, 22);
    let o1 = bits(word, 21, 21);
    let o0 = bits(word, 15, 15);
    match (m, s, ftype, o1, o0) {
        (0b0, 0b0, 0b00, 0b0, 0b0) => Some(Floatdp3::FmaddS),
        (0b0, 0b0, 0b00, 0b0, 0b1) => Some(Floatdp3::FmsubS),
        (0b0, 0b0, 0b00, 0b1, 0b0) => Some(Floatdp3::FnmaddS),
        (0b0, 0b0, 0b00, 0b1, 0b1) => Some(Floatdp3::FnmsubS),
        (0b0, 0b0, 0b01, 0b0, 0b0) => Some(Floatdp3::FmaddD),
        (0b0, 0b0, 0b01, 0b0, 0b1) => Some(Floatdp3::FmsubD),
        (0b0, 0b0, 0b01, 0b1, 0b0) => Some(Floatdp3::FnmaddD),
        (0b0, 0b0, 0b01, 0b1, 0b1) => Some(Floatdp3::FnmsubD),
        (0b0, 0b0, 0b11, 0b0, 0b0) => Some(Floatdp3::FmaddH),
        (0b0, 0b0, 0b11, 0b0, 0b1) => Some(Floatdp3::FmsubH),
        (0b0, 0b0, 0b11, 0b1, 0b0) => Some(Floatdp3::FnmaddH),
        (0b0, 0b0, 0b11, 0b1, 0b1) => Some(Floatdp3::FnmsubH),
        _ => None,
    }
}
