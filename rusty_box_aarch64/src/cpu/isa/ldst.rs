//! Arm group `ldst` (Arm ARM C4.1, "Loads and Stores"): the A64 space with
//! `op1 = x1x0`.

#[cfg(test)]
use super::Feature;
use super::Encoding;
use crate::cpu::exec::bits;

/// Arm group `asisdlse`: Advanced SIMD load/store multiple structures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdlse {
    St4R4,
    St1R44v,
    St3R3,
    St1R33v,
    St1R11v,
    St2R2,
    St1R22v,
    Ld4R4,
    Ld1R44v,
    Ld3R3,
    Ld1R33v,
    Ld1R11v,
    Ld2R2,
    Ld1R22v,
}

/// Arm group `asisdlsep`: Advanced SIMD load/store multiple structures
/// (post-indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdlsep {
    St4R4R,
    St1R4R4,
    St3R3R,
    St1R3R3,
    St1R1R1,
    St2R2R,
    St1R2R2,
    St4I4I,
    St1I4I4,
    St3I3I,
    St1I3I3,
    St1I1I1,
    St2I2I,
    St1I2I2,
    Ld4R4R,
    Ld1R4R4,
    Ld3R3R,
    Ld1R3R3,
    Ld1R1R1,
    Ld2R2R,
    Ld1R2R2,
    Ld4I4I,
    Ld1I4I4,
    Ld3I3I,
    Ld1I3I3,
    Ld1I1I1,
    Ld2I2I,
    Ld1I2I2,
}

/// Arm group `asisdlso`: Advanced SIMD load/store single structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdlso {
    St1B11b,
    St3B33b,
    St1H11h,
    St3H33h,
    St1S11s,
    St1D11d,
    St3S33s,
    St3D33d,
    St2B22b,
    St4B44b,
    St2H22h,
    St4H44h,
    St2S22s,
    St2D22d,
    St4S44s,
    St4D44d,
    Ld1B11b,
    Ld3B33b,
    Ld1H11h,
    Ld3H33h,
    Ld1S11s,
    Ld1D11d,
    Ld3S33s,
    Ld3D33d,
    Ld1rR1,
    Ld3rR3,
    Ld2B22b,
    Ld4B44b,
    Ld2H22h,
    Ld4H44h,
    Ld2S22s,
    Ld2D22d,
    Ld4S44s,
    Ld4D44d,
    Ld2rR2,
    Ld4rR4,
}

/// Arm group `asisdlsop`: Advanced SIMD load/store single structure
/// (post-indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Asisdlsop {
    St1Bx1R1b,
    St3Bx3R3b,
    St1Hx1R1h,
    St3Hx3R3h,
    St1Sx1R1s,
    St1Dx1R1d,
    St3Sx3R3s,
    St3Dx3R3d,
    St1B1I1b,
    St3B3I3b,
    St1H1I1h,
    St3H3I3h,
    St1S1I1s,
    St1D1I1d,
    St3S3I3s,
    St3D3I3d,
    St2Bx2R2b,
    St4Bx4R4b,
    St2Hx2R2h,
    St4Hx4R4h,
    St2Sx2R2s,
    St2Dx2R2d,
    St4Sx4R4s,
    St4Dx4R4d,
    St2B2I2b,
    St4B4I4b,
    St2H2I2h,
    St4H4I4h,
    St2S2I2s,
    St2D2I2d,
    St4S4I4s,
    St4D4I4d,
    Ld1Bx1R1b,
    Ld3Bx3R3b,
    Ld1Hx1R1h,
    Ld3Hx3R3h,
    Ld1Sx1R1s,
    Ld1Dx1R1d,
    Ld3Sx3R3s,
    Ld3Dx3R3d,
    Ld1rRx1R,
    Ld3rRx3R,
    Ld1B1I1b,
    Ld3B3I3b,
    Ld1H1I1h,
    Ld3H3I3h,
    Ld1S1I1s,
    Ld1D1I1d,
    Ld3S3I3s,
    Ld3D3I3d,
    Ld1rR1I,
    Ld3rR3I,
    Ld2Bx2R2b,
    Ld4Bx4R4b,
    Ld2Hx2R2h,
    Ld4Hx4R4h,
    Ld2Sx2R2s,
    Ld2Dx2R2d,
    Ld4Sx4R4s,
    Ld4Dx4R4d,
    Ld2rRx2R,
    Ld4rRx4R,
    Ld2B2I2b,
    Ld4B4I4b,
    Ld2H2I2h,
    Ld4H4I4h,
    Ld2S2I2s,
    Ld2D2I2d,
    Ld4S4I4s,
    Ld4D4I4d,
    Ld2rR2I,
    Ld4rR4I,
}

/// Arm group `ldstexclp`: Load/store exclusive pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Ldstexclp {
    StxpSp32,
    StlxpSp32,
    LdxpLp32,
    LdaxpLp32,
    StxpSp64,
    StlxpSp64,
    LdxpLp64,
    LdaxpLp64,
}

/// Arm group `ldstexclr`: Load/store exclusive register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Ldstexclr {
    StxrbSr32,
    StlxrbSr32,
    LdxrbLr32,
    LdaxrbLr32,
    StxrhSr32,
    StlxrhSr32,
    LdxrhLr32,
    LdaxrhLr32,
    StxrSr32,
    StlxrSr32,
    LdxrLr32,
    LdaxrLr32,
    StxrSr64,
    StlxrSr64,
    LdxrLr64,
    LdaxrLr64,
}

/// Arm group `ldstord`: Load/store ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Ldstord {
    StlrbSl32,
    LdarbLr32,
    StlrhSl32,
    LdarhLr32,
    StlrSl32,
    LdarLr32,
    StlrSl64,
    LdarLr64,
}

/// Arm group `loadlit`: Load register (literal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Loadlit {
    Ldr32,
    LdrS,
    Ldr64,
    LdrD,
    Ldrsw64,
    LdrQ,
    PrfmP,
}

/// Arm group `ldstnapair_offs`: Load/store no-allocate pair (offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstnapairOffs {
    Stnp32,
    Ldnp32,
    StnpS,
    LdnpS,
    StnpD,
    LdnpD,
    Stnp64,
    Ldnp64,
    StnpQ,
    LdnpQ,
}

/// Arm group `ldstpair_post`: Load/store register pair (post-indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstpairPost {
    Stp32,
    Ldp32,
    StpS,
    LdpS,
    Ldpsw64,
    StpD,
    LdpD,
    Stp64,
    Ldp64,
    StpQ,
    LdpQ,
}

/// Arm group `ldstpair_off`: Load/store register pair (offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstpairOff {
    Stp32,
    Ldp32,
    StpS,
    LdpS,
    Ldpsw64,
    StpD,
    LdpD,
    Stp64,
    Ldp64,
    StpQ,
    LdpQ,
}

/// Arm group `ldstpair_pre`: Load/store register pair (pre-indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstpairPre {
    Stp32,
    Ldp32,
    StpS,
    LdpS,
    Ldpsw64,
    StpD,
    LdpD,
    Stp64,
    Ldp64,
    StpQ,
    LdpQ,
}

/// Arm group `ldst_unscaled`: Load/store register (unscaled immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstUnscaled {
    Sturb32,
    Ldurb32,
    Ldursb64,
    Ldursb32,
    SturB,
    LdurB,
    SturQ,
    LdurQ,
    Sturh32,
    Ldurh32,
    Ldursh64,
    Ldursh32,
    SturH,
    LdurH,
    Stur32,
    Ldur32,
    Ldursw64,
    SturS,
    LdurS,
    Stur64,
    Ldur64,
    PrfumP,
    SturD,
    LdurD,
}

/// Arm group `ldst_immpost`: Load/store register (immediate post-indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstImmpost {
    Strb32,
    Ldrb32,
    Ldrsb64,
    Ldrsb32,
    StrB,
    LdrB,
    StrQ,
    LdrQ,
    Strh32,
    Ldrh32,
    Ldrsh64,
    Ldrsh32,
    StrH,
    LdrH,
    Str32,
    Ldr32,
    Ldrsw64,
    StrS,
    LdrS,
    Str64,
    Ldr64,
    StrD,
    LdrD,
}

/// Arm group `ldst_unpriv`: Load/store register (unprivileged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstUnpriv {
    Sttrb32,
    Ldtrb32,
    Ldtrsb64,
    Ldtrsb32,
    Sttrh32,
    Ldtrh32,
    Ldtrsh64,
    Ldtrsh32,
    Sttr32,
    Ldtr32,
    Ldtrsw64,
    Sttr64,
    Ldtr64,
}

/// Arm group `ldst_immpre`: Load/store register (immediate pre-indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstImmpre {
    Strb32,
    Ldrb32,
    Ldrsb64,
    Ldrsb32,
    StrB,
    LdrB,
    StrQ,
    LdrQ,
    Strh32,
    Ldrh32,
    Ldrsh64,
    Ldrsh32,
    StrH,
    LdrH,
    Str32,
    Ldr32,
    Ldrsw64,
    StrS,
    LdrS,
    Str64,
    Ldr64,
    StrD,
    LdrD,
}

/// Arm group `ldst_regoff`: Load/store register (register offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstRegoff {
    Strb32b,
    Strb32bl,
    Ldrb32b,
    Ldrb32bl,
    Ldrsb64b,
    Ldrsb64bl,
    Ldrsb32b,
    Ldrsb32bl,
    StrB,
    StrBl,
    LdrB,
    LdrBl,
    StrQ,
    LdrQ,
    Strh32,
    Ldrh32,
    Ldrsh64,
    Ldrsh32,
    StrH,
    LdrH,
    Str32,
    Ldr32,
    Ldrsw64,
    StrS,
    LdrS,
    Str64,
    Ldr64,
    PrfmP,
    StrD,
    LdrD,
}

/// Arm group `ldst_pos`: Load/store register (unsigned immediate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LdstPos {
    Strb32,
    Ldrb32,
    Ldrsb64,
    Ldrsb32,
    StrB,
    LdrB,
    StrQ,
    LdrQ,
    Strh32,
    Ldrh32,
    Ldrsh64,
    Ldrsh32,
    StrH,
    LdrH,
    Str32,
    Ldr32,
    Ldrsw64,
    StrS,
    LdrS,
    Str64,
    Ldr64,
    PrfmP,
    StrD,
    LdrD,
}

pub(super) fn ldst(word: u32) -> Option<Encoding> {
    let op1 = bits(word, 26, 26);
    // op0<1:0>, then op2<14:13>.
    match bits(word, 29, 28) {
        0b00 => match (op1, bits(word, 24, 23)) {
            (0b1, 0b00) => match (bits(word, 31, 31), bits(word, 21, 16)) {
                (0b0, 0b000000) => asisdlse(word).map(Encoding::Asisdlse),
                _ => None,
            },
            (0b1, 0b01) => match (bits(word, 31, 31), bits(word, 21, 21)) {
                (0b0, 0b0) => asisdlsep(word).map(Encoding::Asisdlsep),
                _ => None,
            },
            (0b1, 0b10) => match (bits(word, 31, 31), bits(word, 20, 17)) {
                (0b0, 0b0000) => asisdlso(word).map(Encoding::Asisdlso),
                _ => None,
            },
            (0b1, 0b11) => match bits(word, 31, 31) {
                0b0 => asisdlsop(word).map(Encoding::Asisdlsop),
                _ => None,
            },
            // op2<11> = 1 with op0<3> = 0 is comswappr (FEAT_LSE).
            (0b0, 0b00) => match (bits(word, 31, 31), bits(word, 21, 21)) {
                (_, 0b0) => ldstexclr(word).map(Encoding::Ldstexclr),
                (0b1, 0b1) => ldstexclp(word).map(Encoding::Ldstexclp),
                _ => None,
            },
            // comswap (FEAT_LSE) at op2<11> = 1.
            (0b0, 0b01) => match bits(word, 21, 21) {
                0b0 => ldstord(word).map(Encoding::Ldstord),
                _ => None,
            },
            // The unprivileged exclusive and compare-and-swap forms
            // (FEAT_LSUI).
            _ => None,
        },
        // op2<14> = 1 holds the load/store groups of later features
        // (FEAT_MOPS, FEAT_LRCPC2, FEAT_LRCPC3, FEAT_LSE128, FEAT_THE,
        // FEAT_LSUI, FEAT_MTE, FEAT_GCS).
        0b01 => match bits(word, 24, 24) {
            0b0 => loadlit(word).map(Encoding::Loadlit),
            _ => None,
        },
        0b10 => match bits(word, 24, 23) {
            0b00 => ldstnapair_offs(word).map(Encoding::LdstnapairOffs),
            0b01 => ldstpair_post(word).map(Encoding::LdstpairPost),
            0b10 => ldstpair_off(word).map(Encoding::LdstpairOff),
            0b11 => ldstpair_pre(word).map(Encoding::LdstpairPre),
            _ => None,
        },
        0b11 => match bits(word, 24, 24) {
            // op2<11>, then op2<1:0>. op2<11> = 1 with op2<1:0> = 00 is
            // memop (FEAT_LSE), and with op2<0> = 1 ldst_pac (FEAT_PAuth).
            0b0 => match (bits(word, 21, 21), bits(word, 11, 10)) {
                (0b0, 0b00) => ldst_unscaled(word).map(Encoding::LdstUnscaled),
                (0b0, 0b01) => ldst_immpost(word).map(Encoding::LdstImmpost),
                (0b0, 0b10) => ldst_unpriv(word).map(Encoding::LdstUnpriv),
                (0b0, 0b11) => ldst_immpre(word).map(Encoding::LdstImmpre),
                (0b1, 0b10) => ldst_regoff(word).map(Encoding::LdstRegoff),
                _ => None,
            },
            0b1 => ldst_pos(word).map(Encoding::LdstPos),
            _ => None,
        },
        _ => None,
    }
}

fn asisdlse(word: u32) -> Option<Asisdlse> {
    let l = bits(word, 22, 22);
    let opcode = bits(word, 15, 12);
    match (l, opcode) {
        (0b0, 0b0000) => Some(Asisdlse::St4R4),
        (0b0, 0b0010) => Some(Asisdlse::St1R44v),
        (0b0, 0b0100) => Some(Asisdlse::St3R3),
        (0b0, 0b0110) => Some(Asisdlse::St1R33v),
        (0b0, 0b0111) => Some(Asisdlse::St1R11v),
        (0b0, 0b1000) => Some(Asisdlse::St2R2),
        (0b0, 0b1010) => Some(Asisdlse::St1R22v),
        (0b1, 0b0000) => Some(Asisdlse::Ld4R4),
        (0b1, 0b0010) => Some(Asisdlse::Ld1R44v),
        (0b1, 0b0100) => Some(Asisdlse::Ld3R3),
        (0b1, 0b0110) => Some(Asisdlse::Ld1R33v),
        (0b1, 0b0111) => Some(Asisdlse::Ld1R11v),
        (0b1, 0b1000) => Some(Asisdlse::Ld2R2),
        (0b1, 0b1010) => Some(Asisdlse::Ld1R22v),
        _ => None,
    }
}

fn asisdlsep(word: u32) -> Option<Asisdlsep> {
    let l = bits(word, 22, 22);
    let rm = bits(word, 20, 16);
    let opcode = bits(word, 15, 12);
    // Rm = 11111 is the immediate post-index form.
    match (l, rm, opcode) {
        (0b0, 0b11111, 0b0000) => Some(Asisdlsep::St4I4I),
        (0b0, 0b11111, 0b0010) => Some(Asisdlsep::St1I4I4),
        (0b0, 0b11111, 0b0100) => Some(Asisdlsep::St3I3I),
        (0b0, 0b11111, 0b0110) => Some(Asisdlsep::St1I3I3),
        (0b0, 0b11111, 0b0111) => Some(Asisdlsep::St1I1I1),
        (0b0, 0b11111, 0b1000) => Some(Asisdlsep::St2I2I),
        (0b0, 0b11111, 0b1010) => Some(Asisdlsep::St1I2I2),
        (0b0, _, 0b0000) => Some(Asisdlsep::St4R4R),
        (0b0, _, 0b0010) => Some(Asisdlsep::St1R4R4),
        (0b0, _, 0b0100) => Some(Asisdlsep::St3R3R),
        (0b0, _, 0b0110) => Some(Asisdlsep::St1R3R3),
        (0b0, _, 0b0111) => Some(Asisdlsep::St1R1R1),
        (0b0, _, 0b1000) => Some(Asisdlsep::St2R2R),
        (0b0, _, 0b1010) => Some(Asisdlsep::St1R2R2),
        (0b1, 0b11111, 0b0000) => Some(Asisdlsep::Ld4I4I),
        (0b1, 0b11111, 0b0010) => Some(Asisdlsep::Ld1I4I4),
        (0b1, 0b11111, 0b0100) => Some(Asisdlsep::Ld3I3I),
        (0b1, 0b11111, 0b0110) => Some(Asisdlsep::Ld1I3I3),
        (0b1, 0b11111, 0b0111) => Some(Asisdlsep::Ld1I1I1),
        (0b1, 0b11111, 0b1000) => Some(Asisdlsep::Ld2I2I),
        (0b1, 0b11111, 0b1010) => Some(Asisdlsep::Ld1I2I2),
        (0b1, _, 0b0000) => Some(Asisdlsep::Ld4R4R),
        (0b1, _, 0b0010) => Some(Asisdlsep::Ld1R4R4),
        (0b1, _, 0b0100) => Some(Asisdlsep::Ld3R3R),
        (0b1, _, 0b0110) => Some(Asisdlsep::Ld1R3R3),
        (0b1, _, 0b0111) => Some(Asisdlsep::Ld1R1R1),
        (0b1, _, 0b1000) => Some(Asisdlsep::Ld2R2R),
        (0b1, _, 0b1010) => Some(Asisdlsep::Ld1R2R2),
        _ => None,
    }
}

fn asisdlso(word: u32) -> Option<Asisdlso> {
    let l = bits(word, 22, 22);
    let r = bits(word, 21, 21);
    let o2 = bits(word, 16, 16);
    let opcode = bits(word, 15, 13);
    let s = bits(word, 12, 12);
    let size = bits(word, 11, 10);
    // o2 = 1 is the RCpc single-lane forms (FEAT_LRCPC3).
    match (l, r, o2, opcode, s, size) {
        (0b0, 0b0, 0b0, 0b000, _, _) => Some(Asisdlso::St1B11b),
        (0b0, 0b0, 0b0, 0b001, _, _) => Some(Asisdlso::St3B33b),
        (0b0, 0b0, 0b0, 0b010, _, 0b00 | 0b10) => Some(Asisdlso::St1H11h),
        (0b0, 0b0, 0b0, 0b011, _, 0b00 | 0b10) => Some(Asisdlso::St3H33h),
        (0b0, 0b0, 0b0, 0b100, _, 0b00) => Some(Asisdlso::St1S11s),
        (0b0, 0b0, 0b0, 0b100, 0b0, 0b01) => Some(Asisdlso::St1D11d),
        (0b0, 0b0, 0b0, 0b101, _, 0b00) => Some(Asisdlso::St3S33s),
        (0b0, 0b0, 0b0, 0b101, 0b0, 0b01) => Some(Asisdlso::St3D33d),
        (0b0, 0b1, 0b0, 0b000, _, _) => Some(Asisdlso::St2B22b),
        (0b0, 0b1, 0b0, 0b001, _, _) => Some(Asisdlso::St4B44b),
        (0b0, 0b1, 0b0, 0b010, _, 0b00 | 0b10) => Some(Asisdlso::St2H22h),
        (0b0, 0b1, 0b0, 0b011, _, 0b00 | 0b10) => Some(Asisdlso::St4H44h),
        (0b0, 0b1, 0b0, 0b100, _, 0b00) => Some(Asisdlso::St2S22s),
        (0b0, 0b1, 0b0, 0b100, 0b0, 0b01) => Some(Asisdlso::St2D22d),
        (0b0, 0b1, 0b0, 0b101, _, 0b00) => Some(Asisdlso::St4S44s),
        (0b0, 0b1, 0b0, 0b101, 0b0, 0b01) => Some(Asisdlso::St4D44d),
        (0b1, 0b0, 0b0, 0b000, _, _) => Some(Asisdlso::Ld1B11b),
        (0b1, 0b0, 0b0, 0b001, _, _) => Some(Asisdlso::Ld3B33b),
        (0b1, 0b0, 0b0, 0b010, _, 0b00 | 0b10) => Some(Asisdlso::Ld1H11h),
        (0b1, 0b0, 0b0, 0b011, _, 0b00 | 0b10) => Some(Asisdlso::Ld3H33h),
        (0b1, 0b0, 0b0, 0b100, _, 0b00) => Some(Asisdlso::Ld1S11s),
        (0b1, 0b0, 0b0, 0b100, 0b0, 0b01) => Some(Asisdlso::Ld1D11d),
        (0b1, 0b0, 0b0, 0b101, _, 0b00) => Some(Asisdlso::Ld3S33s),
        (0b1, 0b0, 0b0, 0b101, 0b0, 0b01) => Some(Asisdlso::Ld3D33d),
        (0b1, 0b0, 0b0, 0b110, 0b0, _) => Some(Asisdlso::Ld1rR1),
        (0b1, 0b0, 0b0, 0b111, 0b0, _) => Some(Asisdlso::Ld3rR3),
        (0b1, 0b1, 0b0, 0b000, _, _) => Some(Asisdlso::Ld2B22b),
        (0b1, 0b1, 0b0, 0b001, _, _) => Some(Asisdlso::Ld4B44b),
        (0b1, 0b1, 0b0, 0b010, _, 0b00 | 0b10) => Some(Asisdlso::Ld2H22h),
        (0b1, 0b1, 0b0, 0b011, _, 0b00 | 0b10) => Some(Asisdlso::Ld4H44h),
        (0b1, 0b1, 0b0, 0b100, _, 0b00) => Some(Asisdlso::Ld2S22s),
        (0b1, 0b1, 0b0, 0b100, 0b0, 0b01) => Some(Asisdlso::Ld2D22d),
        (0b1, 0b1, 0b0, 0b101, _, 0b00) => Some(Asisdlso::Ld4S44s),
        (0b1, 0b1, 0b0, 0b101, 0b0, 0b01) => Some(Asisdlso::Ld4D44d),
        (0b1, 0b1, 0b0, 0b110, 0b0, _) => Some(Asisdlso::Ld2rR2),
        (0b1, 0b1, 0b0, 0b111, 0b0, _) => Some(Asisdlso::Ld4rR4),
        _ => None,
    }
}

fn asisdlsop(word: u32) -> Option<Asisdlsop> {
    let l = bits(word, 22, 22);
    let r = bits(word, 21, 21);
    let rm = bits(word, 20, 16);
    let opcode = bits(word, 15, 13);
    let s = bits(word, 12, 12);
    let size = bits(word, 11, 10);
    match rm {
        // The immediate post-index form.
        0b11111 => match (l, r, opcode, s, size) {
            (0b0, 0b0, 0b000, _, _) => Some(Asisdlsop::St1B1I1b),
            (0b0, 0b0, 0b001, _, _) => Some(Asisdlsop::St3B3I3b),
            (0b0, 0b0, 0b010, _, 0b00 | 0b10) => Some(Asisdlsop::St1H1I1h),
            (0b0, 0b0, 0b011, _, 0b00 | 0b10) => Some(Asisdlsop::St3H3I3h),
            (0b0, 0b0, 0b100, _, 0b00) => Some(Asisdlsop::St1S1I1s),
            (0b0, 0b0, 0b100, 0b0, 0b01) => Some(Asisdlsop::St1D1I1d),
            (0b0, 0b0, 0b101, _, 0b00) => Some(Asisdlsop::St3S3I3s),
            (0b0, 0b0, 0b101, 0b0, 0b01) => Some(Asisdlsop::St3D3I3d),
            (0b0, 0b1, 0b000, _, _) => Some(Asisdlsop::St2B2I2b),
            (0b0, 0b1, 0b001, _, _) => Some(Asisdlsop::St4B4I4b),
            (0b0, 0b1, 0b010, _, 0b00 | 0b10) => Some(Asisdlsop::St2H2I2h),
            (0b0, 0b1, 0b011, _, 0b00 | 0b10) => Some(Asisdlsop::St4H4I4h),
            (0b0, 0b1, 0b100, _, 0b00) => Some(Asisdlsop::St2S2I2s),
            (0b0, 0b1, 0b100, 0b0, 0b01) => Some(Asisdlsop::St2D2I2d),
            (0b0, 0b1, 0b101, _, 0b00) => Some(Asisdlsop::St4S4I4s),
            (0b0, 0b1, 0b101, 0b0, 0b01) => Some(Asisdlsop::St4D4I4d),
            (0b1, 0b0, 0b000, _, _) => Some(Asisdlsop::Ld1B1I1b),
            (0b1, 0b0, 0b001, _, _) => Some(Asisdlsop::Ld3B3I3b),
            (0b1, 0b0, 0b010, _, 0b00 | 0b10) => Some(Asisdlsop::Ld1H1I1h),
            (0b1, 0b0, 0b011, _, 0b00 | 0b10) => Some(Asisdlsop::Ld3H3I3h),
            (0b1, 0b0, 0b100, _, 0b00) => Some(Asisdlsop::Ld1S1I1s),
            (0b1, 0b0, 0b100, 0b0, 0b01) => Some(Asisdlsop::Ld1D1I1d),
            (0b1, 0b0, 0b101, _, 0b00) => Some(Asisdlsop::Ld3S3I3s),
            (0b1, 0b0, 0b101, 0b0, 0b01) => Some(Asisdlsop::Ld3D3I3d),
            (0b1, 0b0, 0b110, 0b0, _) => Some(Asisdlsop::Ld1rR1I),
            (0b1, 0b0, 0b111, 0b0, _) => Some(Asisdlsop::Ld3rR3I),
            (0b1, 0b1, 0b000, _, _) => Some(Asisdlsop::Ld2B2I2b),
            (0b1, 0b1, 0b001, _, _) => Some(Asisdlsop::Ld4B4I4b),
            (0b1, 0b1, 0b010, _, 0b00 | 0b10) => Some(Asisdlsop::Ld2H2I2h),
            (0b1, 0b1, 0b011, _, 0b00 | 0b10) => Some(Asisdlsop::Ld4H4I4h),
            (0b1, 0b1, 0b100, _, 0b00) => Some(Asisdlsop::Ld2S2I2s),
            (0b1, 0b1, 0b100, 0b0, 0b01) => Some(Asisdlsop::Ld2D2I2d),
            (0b1, 0b1, 0b101, _, 0b00) => Some(Asisdlsop::Ld4S4I4s),
            (0b1, 0b1, 0b101, 0b0, 0b01) => Some(Asisdlsop::Ld4D4I4d),
            (0b1, 0b1, 0b110, 0b0, _) => Some(Asisdlsop::Ld2rR2I),
            (0b1, 0b1, 0b111, 0b0, _) => Some(Asisdlsop::Ld4rR4I),
            _ => None,
        },
        _ => match (l, r, opcode, s, size) {
            (0b0, 0b0, 0b000, _, _) => Some(Asisdlsop::St1Bx1R1b),
            (0b0, 0b0, 0b001, _, _) => Some(Asisdlsop::St3Bx3R3b),
            (0b0, 0b0, 0b010, _, 0b00 | 0b10) => Some(Asisdlsop::St1Hx1R1h),
            (0b0, 0b0, 0b011, _, 0b00 | 0b10) => Some(Asisdlsop::St3Hx3R3h),
            (0b0, 0b0, 0b100, _, 0b00) => Some(Asisdlsop::St1Sx1R1s),
            (0b0, 0b0, 0b100, 0b0, 0b01) => Some(Asisdlsop::St1Dx1R1d),
            (0b0, 0b0, 0b101, _, 0b00) => Some(Asisdlsop::St3Sx3R3s),
            (0b0, 0b0, 0b101, 0b0, 0b01) => Some(Asisdlsop::St3Dx3R3d),
            (0b0, 0b1, 0b000, _, _) => Some(Asisdlsop::St2Bx2R2b),
            (0b0, 0b1, 0b001, _, _) => Some(Asisdlsop::St4Bx4R4b),
            (0b0, 0b1, 0b010, _, 0b00 | 0b10) => Some(Asisdlsop::St2Hx2R2h),
            (0b0, 0b1, 0b011, _, 0b00 | 0b10) => Some(Asisdlsop::St4Hx4R4h),
            (0b0, 0b1, 0b100, _, 0b00) => Some(Asisdlsop::St2Sx2R2s),
            (0b0, 0b1, 0b100, 0b0, 0b01) => Some(Asisdlsop::St2Dx2R2d),
            (0b0, 0b1, 0b101, _, 0b00) => Some(Asisdlsop::St4Sx4R4s),
            (0b0, 0b1, 0b101, 0b0, 0b01) => Some(Asisdlsop::St4Dx4R4d),
            (0b1, 0b0, 0b000, _, _) => Some(Asisdlsop::Ld1Bx1R1b),
            (0b1, 0b0, 0b001, _, _) => Some(Asisdlsop::Ld3Bx3R3b),
            (0b1, 0b0, 0b010, _, 0b00 | 0b10) => Some(Asisdlsop::Ld1Hx1R1h),
            (0b1, 0b0, 0b011, _, 0b00 | 0b10) => Some(Asisdlsop::Ld3Hx3R3h),
            (0b1, 0b0, 0b100, _, 0b00) => Some(Asisdlsop::Ld1Sx1R1s),
            (0b1, 0b0, 0b100, 0b0, 0b01) => Some(Asisdlsop::Ld1Dx1R1d),
            (0b1, 0b0, 0b101, _, 0b00) => Some(Asisdlsop::Ld3Sx3R3s),
            (0b1, 0b0, 0b101, 0b0, 0b01) => Some(Asisdlsop::Ld3Dx3R3d),
            (0b1, 0b0, 0b110, 0b0, _) => Some(Asisdlsop::Ld1rRx1R),
            (0b1, 0b0, 0b111, 0b0, _) => Some(Asisdlsop::Ld3rRx3R),
            (0b1, 0b1, 0b000, _, _) => Some(Asisdlsop::Ld2Bx2R2b),
            (0b1, 0b1, 0b001, _, _) => Some(Asisdlsop::Ld4Bx4R4b),
            (0b1, 0b1, 0b010, _, 0b00 | 0b10) => Some(Asisdlsop::Ld2Hx2R2h),
            (0b1, 0b1, 0b011, _, 0b00 | 0b10) => Some(Asisdlsop::Ld4Hx4R4h),
            (0b1, 0b1, 0b100, _, 0b00) => Some(Asisdlsop::Ld2Sx2R2s),
            (0b1, 0b1, 0b100, 0b0, 0b01) => Some(Asisdlsop::Ld2Dx2R2d),
            (0b1, 0b1, 0b101, _, 0b00) => Some(Asisdlsop::Ld4Sx4R4s),
            (0b1, 0b1, 0b101, 0b0, 0b01) => Some(Asisdlsop::Ld4Dx4R4d),
            (0b1, 0b1, 0b110, 0b0, _) => Some(Asisdlsop::Ld2rRx2R),
            (0b1, 0b1, 0b111, 0b0, _) => Some(Asisdlsop::Ld4rRx4R),
            _ => None,
        },
    }
}

fn ldstexclp(word: u32) -> Option<Ldstexclp> {
    let sz = bits(word, 30, 30);
    let l = bits(word, 22, 22);
    let rs = bits(word, 20, 16);
    let o0 = bits(word, 15, 15);
    // Rs is should-be-one in the loads (Arm ARM (1) bits): fixed here.
    match (sz, l, rs, o0) {
        (0b0, 0b0, _, 0b0) => Some(Ldstexclp::StxpSp32),
        (0b0, 0b0, _, 0b1) => Some(Ldstexclp::StlxpSp32),
        (0b0, 0b1, 0b11111, 0b0) => Some(Ldstexclp::LdxpLp32),
        (0b0, 0b1, 0b11111, 0b1) => Some(Ldstexclp::LdaxpLp32),
        (0b1, 0b0, _, 0b0) => Some(Ldstexclp::StxpSp64),
        (0b1, 0b0, _, 0b1) => Some(Ldstexclp::StlxpSp64),
        (0b1, 0b1, 0b11111, 0b0) => Some(Ldstexclp::LdxpLp64),
        (0b1, 0b1, 0b11111, 0b1) => Some(Ldstexclp::LdaxpLp64),
        _ => None,
    }
}

fn ldstexclr(word: u32) -> Option<Ldstexclr> {
    let size = bits(word, 31, 30);
    let l = bits(word, 22, 22);
    let rs = bits(word, 20, 16);
    let o0 = bits(word, 15, 15);
    let rt2 = bits(word, 14, 10);
    // Rt2, and Rs in the loads, are should-be-one: fixed here.
    match (size, l, rs, o0, rt2) {
        (0b00, 0b0, _, 0b0, 0b11111) => Some(Ldstexclr::StxrbSr32),
        (0b00, 0b0, _, 0b1, 0b11111) => Some(Ldstexclr::StlxrbSr32),
        (0b00, 0b1, 0b11111, 0b0, 0b11111) => Some(Ldstexclr::LdxrbLr32),
        (0b00, 0b1, 0b11111, 0b1, 0b11111) => Some(Ldstexclr::LdaxrbLr32),
        (0b01, 0b0, _, 0b0, 0b11111) => Some(Ldstexclr::StxrhSr32),
        (0b01, 0b0, _, 0b1, 0b11111) => Some(Ldstexclr::StlxrhSr32),
        (0b01, 0b1, 0b11111, 0b0, 0b11111) => Some(Ldstexclr::LdxrhLr32),
        (0b01, 0b1, 0b11111, 0b1, 0b11111) => Some(Ldstexclr::LdaxrhLr32),
        (0b10, 0b0, _, 0b0, 0b11111) => Some(Ldstexclr::StxrSr32),
        (0b10, 0b0, _, 0b1, 0b11111) => Some(Ldstexclr::StlxrSr32),
        (0b10, 0b1, 0b11111, 0b0, 0b11111) => Some(Ldstexclr::LdxrLr32),
        (0b10, 0b1, 0b11111, 0b1, 0b11111) => Some(Ldstexclr::LdaxrLr32),
        (0b11, 0b0, _, 0b0, 0b11111) => Some(Ldstexclr::StxrSr64),
        (0b11, 0b0, _, 0b1, 0b11111) => Some(Ldstexclr::StlxrSr64),
        (0b11, 0b1, 0b11111, 0b0, 0b11111) => Some(Ldstexclr::LdxrLr64),
        (0b11, 0b1, 0b11111, 0b1, 0b11111) => Some(Ldstexclr::LdaxrLr64),
        _ => None,
    }
}

fn ldstord(word: u32) -> Option<Ldstord> {
    let size = bits(word, 31, 30);
    let l = bits(word, 22, 22);
    let rs = bits(word, 20, 16);
    let o0 = bits(word, 15, 15);
    let rt2 = bits(word, 14, 10);
    // o0 = 0 is LDLAR and STLLR (FEAT_LOR). Rs and Rt2 are should-be-one:
    // fixed here.
    match (size, l, rs, o0, rt2) {
        (0b00, 0b0, 0b11111, 0b1, 0b11111) => Some(Ldstord::StlrbSl32),
        (0b00, 0b1, 0b11111, 0b1, 0b11111) => Some(Ldstord::LdarbLr32),
        (0b01, 0b0, 0b11111, 0b1, 0b11111) => Some(Ldstord::StlrhSl32),
        (0b01, 0b1, 0b11111, 0b1, 0b11111) => Some(Ldstord::LdarhLr32),
        (0b10, 0b0, 0b11111, 0b1, 0b11111) => Some(Ldstord::StlrSl32),
        (0b10, 0b1, 0b11111, 0b1, 0b11111) => Some(Ldstord::LdarLr32),
        (0b11, 0b0, 0b11111, 0b1, 0b11111) => Some(Ldstord::StlrSl64),
        (0b11, 0b1, 0b11111, 0b1, 0b11111) => Some(Ldstord::LdarLr64),
        _ => None,
    }
}

fn loadlit(word: u32) -> Option<Loadlit> {
    let opc = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    match (opc, vr) {
        (0b00, 0b0) => Some(Loadlit::Ldr32),
        (0b00, 0b1) => Some(Loadlit::LdrS),
        (0b01, 0b0) => Some(Loadlit::Ldr64),
        (0b01, 0b1) => Some(Loadlit::LdrD),
        (0b10, 0b0) => Some(Loadlit::Ldrsw64),
        (0b10, 0b1) => Some(Loadlit::LdrQ),
        (0b11, 0b0) => Some(Loadlit::PrfmP),
        _ => None,
    }
}

fn ldstnapair_offs(word: u32) -> Option<LdstnapairOffs> {
    let opc = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let l = bits(word, 22, 22);
    // opc = 11 is the unprivileged pairs (FEAT_LSUI).
    match (opc, vr, l) {
        (0b00, 0b0, 0b0) => Some(LdstnapairOffs::Stnp32),
        (0b00, 0b0, 0b1) => Some(LdstnapairOffs::Ldnp32),
        (0b00, 0b1, 0b0) => Some(LdstnapairOffs::StnpS),
        (0b00, 0b1, 0b1) => Some(LdstnapairOffs::LdnpS),
        (0b01, 0b1, 0b0) => Some(LdstnapairOffs::StnpD),
        (0b01, 0b1, 0b1) => Some(LdstnapairOffs::LdnpD),
        (0b10, 0b0, 0b0) => Some(LdstnapairOffs::Stnp64),
        (0b10, 0b0, 0b1) => Some(LdstnapairOffs::Ldnp64),
        (0b10, 0b1, 0b0) => Some(LdstnapairOffs::StnpQ),
        (0b10, 0b1, 0b1) => Some(LdstnapairOffs::LdnpQ),
        _ => None,
    }
}

fn ldstpair_post(word: u32) -> Option<LdstpairPost> {
    let opc = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let l = bits(word, 22, 22);
    // opc = 01, V = 0, L = 0 is STGP (FEAT_MTE); opc = 11 the unprivileged
    // pairs (FEAT_LSUI).
    match (opc, vr, l) {
        (0b00, 0b0, 0b0) => Some(LdstpairPost::Stp32),
        (0b00, 0b0, 0b1) => Some(LdstpairPost::Ldp32),
        (0b00, 0b1, 0b0) => Some(LdstpairPost::StpS),
        (0b00, 0b1, 0b1) => Some(LdstpairPost::LdpS),
        (0b01, 0b0, 0b1) => Some(LdstpairPost::Ldpsw64),
        (0b01, 0b1, 0b0) => Some(LdstpairPost::StpD),
        (0b01, 0b1, 0b1) => Some(LdstpairPost::LdpD),
        (0b10, 0b0, 0b0) => Some(LdstpairPost::Stp64),
        (0b10, 0b0, 0b1) => Some(LdstpairPost::Ldp64),
        (0b10, 0b1, 0b0) => Some(LdstpairPost::StpQ),
        (0b10, 0b1, 0b1) => Some(LdstpairPost::LdpQ),
        _ => None,
    }
}

fn ldstpair_off(word: u32) -> Option<LdstpairOff> {
    let opc = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let l = bits(word, 22, 22);
    // opc = 01, V = 0, L = 0 is STGP (FEAT_MTE); opc = 11 the unprivileged
    // pairs (FEAT_LSUI).
    match (opc, vr, l) {
        (0b00, 0b0, 0b0) => Some(LdstpairOff::Stp32),
        (0b00, 0b0, 0b1) => Some(LdstpairOff::Ldp32),
        (0b00, 0b1, 0b0) => Some(LdstpairOff::StpS),
        (0b00, 0b1, 0b1) => Some(LdstpairOff::LdpS),
        (0b01, 0b0, 0b1) => Some(LdstpairOff::Ldpsw64),
        (0b01, 0b1, 0b0) => Some(LdstpairOff::StpD),
        (0b01, 0b1, 0b1) => Some(LdstpairOff::LdpD),
        (0b10, 0b0, 0b0) => Some(LdstpairOff::Stp64),
        (0b10, 0b0, 0b1) => Some(LdstpairOff::Ldp64),
        (0b10, 0b1, 0b0) => Some(LdstpairOff::StpQ),
        (0b10, 0b1, 0b1) => Some(LdstpairOff::LdpQ),
        _ => None,
    }
}

fn ldstpair_pre(word: u32) -> Option<LdstpairPre> {
    let opc = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let l = bits(word, 22, 22);
    // opc = 01, V = 0, L = 0 is STGP (FEAT_MTE); opc = 11 the unprivileged
    // pairs (FEAT_LSUI).
    match (opc, vr, l) {
        (0b00, 0b0, 0b0) => Some(LdstpairPre::Stp32),
        (0b00, 0b0, 0b1) => Some(LdstpairPre::Ldp32),
        (0b00, 0b1, 0b0) => Some(LdstpairPre::StpS),
        (0b00, 0b1, 0b1) => Some(LdstpairPre::LdpS),
        (0b01, 0b0, 0b1) => Some(LdstpairPre::Ldpsw64),
        (0b01, 0b1, 0b0) => Some(LdstpairPre::StpD),
        (0b01, 0b1, 0b1) => Some(LdstpairPre::LdpD),
        (0b10, 0b0, 0b0) => Some(LdstpairPre::Stp64),
        (0b10, 0b0, 0b1) => Some(LdstpairPre::Ldp64),
        (0b10, 0b1, 0b0) => Some(LdstpairPre::StpQ),
        (0b10, 0b1, 0b1) => Some(LdstpairPre::LdpQ),
        _ => None,
    }
}

fn ldst_unscaled(word: u32) -> Option<LdstUnscaled> {
    let size = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let opc = bits(word, 23, 22);
    match (size, vr, opc) {
        (0b00, 0b0, 0b00) => Some(LdstUnscaled::Sturb32),
        (0b00, 0b0, 0b01) => Some(LdstUnscaled::Ldurb32),
        (0b00, 0b0, 0b10) => Some(LdstUnscaled::Ldursb64),
        (0b00, 0b0, 0b11) => Some(LdstUnscaled::Ldursb32),
        (0b00, 0b1, 0b00) => Some(LdstUnscaled::SturB),
        (0b00, 0b1, 0b01) => Some(LdstUnscaled::LdurB),
        (0b00, 0b1, 0b10) => Some(LdstUnscaled::SturQ),
        (0b00, 0b1, 0b11) => Some(LdstUnscaled::LdurQ),
        (0b01, 0b0, 0b00) => Some(LdstUnscaled::Sturh32),
        (0b01, 0b0, 0b01) => Some(LdstUnscaled::Ldurh32),
        (0b01, 0b0, 0b10) => Some(LdstUnscaled::Ldursh64),
        (0b01, 0b0, 0b11) => Some(LdstUnscaled::Ldursh32),
        (0b01, 0b1, 0b00) => Some(LdstUnscaled::SturH),
        (0b01, 0b1, 0b01) => Some(LdstUnscaled::LdurH),
        (0b10, 0b0, 0b00) => Some(LdstUnscaled::Stur32),
        (0b10, 0b0, 0b01) => Some(LdstUnscaled::Ldur32),
        (0b10, 0b0, 0b10) => Some(LdstUnscaled::Ldursw64),
        (0b10, 0b1, 0b00) => Some(LdstUnscaled::SturS),
        (0b10, 0b1, 0b01) => Some(LdstUnscaled::LdurS),
        (0b11, 0b0, 0b00) => Some(LdstUnscaled::Stur64),
        (0b11, 0b0, 0b01) => Some(LdstUnscaled::Ldur64),
        (0b11, 0b0, 0b10) => Some(LdstUnscaled::PrfumP),
        (0b11, 0b1, 0b00) => Some(LdstUnscaled::SturD),
        (0b11, 0b1, 0b01) => Some(LdstUnscaled::LdurD),
        _ => None,
    }
}

fn ldst_immpost(word: u32) -> Option<LdstImmpost> {
    let size = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let opc = bits(word, 23, 22);
    match (size, vr, opc) {
        (0b00, 0b0, 0b00) => Some(LdstImmpost::Strb32),
        (0b00, 0b0, 0b01) => Some(LdstImmpost::Ldrb32),
        (0b00, 0b0, 0b10) => Some(LdstImmpost::Ldrsb64),
        (0b00, 0b0, 0b11) => Some(LdstImmpost::Ldrsb32),
        (0b00, 0b1, 0b00) => Some(LdstImmpost::StrB),
        (0b00, 0b1, 0b01) => Some(LdstImmpost::LdrB),
        (0b00, 0b1, 0b10) => Some(LdstImmpost::StrQ),
        (0b00, 0b1, 0b11) => Some(LdstImmpost::LdrQ),
        (0b01, 0b0, 0b00) => Some(LdstImmpost::Strh32),
        (0b01, 0b0, 0b01) => Some(LdstImmpost::Ldrh32),
        (0b01, 0b0, 0b10) => Some(LdstImmpost::Ldrsh64),
        (0b01, 0b0, 0b11) => Some(LdstImmpost::Ldrsh32),
        (0b01, 0b1, 0b00) => Some(LdstImmpost::StrH),
        (0b01, 0b1, 0b01) => Some(LdstImmpost::LdrH),
        (0b10, 0b0, 0b00) => Some(LdstImmpost::Str32),
        (0b10, 0b0, 0b01) => Some(LdstImmpost::Ldr32),
        (0b10, 0b0, 0b10) => Some(LdstImmpost::Ldrsw64),
        (0b10, 0b1, 0b00) => Some(LdstImmpost::StrS),
        (0b10, 0b1, 0b01) => Some(LdstImmpost::LdrS),
        (0b11, 0b0, 0b00) => Some(LdstImmpost::Str64),
        (0b11, 0b0, 0b01) => Some(LdstImmpost::Ldr64),
        (0b11, 0b1, 0b00) => Some(LdstImmpost::StrD),
        (0b11, 0b1, 0b01) => Some(LdstImmpost::LdrD),
        _ => None,
    }
}

fn ldst_unpriv(word: u32) -> Option<LdstUnpriv> {
    let size = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let opc = bits(word, 23, 22);
    match (size, vr, opc) {
        (0b00, 0b0, 0b00) => Some(LdstUnpriv::Sttrb32),
        (0b00, 0b0, 0b01) => Some(LdstUnpriv::Ldtrb32),
        (0b00, 0b0, 0b10) => Some(LdstUnpriv::Ldtrsb64),
        (0b00, 0b0, 0b11) => Some(LdstUnpriv::Ldtrsb32),
        (0b01, 0b0, 0b00) => Some(LdstUnpriv::Sttrh32),
        (0b01, 0b0, 0b01) => Some(LdstUnpriv::Ldtrh32),
        (0b01, 0b0, 0b10) => Some(LdstUnpriv::Ldtrsh64),
        (0b01, 0b0, 0b11) => Some(LdstUnpriv::Ldtrsh32),
        (0b10, 0b0, 0b00) => Some(LdstUnpriv::Sttr32),
        (0b10, 0b0, 0b01) => Some(LdstUnpriv::Ldtr32),
        (0b10, 0b0, 0b10) => Some(LdstUnpriv::Ldtrsw64),
        (0b11, 0b0, 0b00) => Some(LdstUnpriv::Sttr64),
        (0b11, 0b0, 0b01) => Some(LdstUnpriv::Ldtr64),
        _ => None,
    }
}

fn ldst_immpre(word: u32) -> Option<LdstImmpre> {
    let size = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let opc = bits(word, 23, 22);
    match (size, vr, opc) {
        (0b00, 0b0, 0b00) => Some(LdstImmpre::Strb32),
        (0b00, 0b0, 0b01) => Some(LdstImmpre::Ldrb32),
        (0b00, 0b0, 0b10) => Some(LdstImmpre::Ldrsb64),
        (0b00, 0b0, 0b11) => Some(LdstImmpre::Ldrsb32),
        (0b00, 0b1, 0b00) => Some(LdstImmpre::StrB),
        (0b00, 0b1, 0b01) => Some(LdstImmpre::LdrB),
        (0b00, 0b1, 0b10) => Some(LdstImmpre::StrQ),
        (0b00, 0b1, 0b11) => Some(LdstImmpre::LdrQ),
        (0b01, 0b0, 0b00) => Some(LdstImmpre::Strh32),
        (0b01, 0b0, 0b01) => Some(LdstImmpre::Ldrh32),
        (0b01, 0b0, 0b10) => Some(LdstImmpre::Ldrsh64),
        (0b01, 0b0, 0b11) => Some(LdstImmpre::Ldrsh32),
        (0b01, 0b1, 0b00) => Some(LdstImmpre::StrH),
        (0b01, 0b1, 0b01) => Some(LdstImmpre::LdrH),
        (0b10, 0b0, 0b00) => Some(LdstImmpre::Str32),
        (0b10, 0b0, 0b01) => Some(LdstImmpre::Ldr32),
        (0b10, 0b0, 0b10) => Some(LdstImmpre::Ldrsw64),
        (0b10, 0b1, 0b00) => Some(LdstImmpre::StrS),
        (0b10, 0b1, 0b01) => Some(LdstImmpre::LdrS),
        (0b11, 0b0, 0b00) => Some(LdstImmpre::Str64),
        (0b11, 0b0, 0b01) => Some(LdstImmpre::Ldr64),
        (0b11, 0b1, 0b00) => Some(LdstImmpre::StrD),
        (0b11, 0b1, 0b01) => Some(LdstImmpre::LdrD),
        _ => None,
    }
}

fn ldst_regoff(word: u32) -> Option<LdstRegoff> {
    let size = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let opc = bits(word, 23, 22);
    let option = bits(word, 15, 13);
    // A byte access with option = 011 is the shifted-register (LSL) form.
    match (size, vr, opc, option) {
        (0b00, 0b0, 0b00, 0b011) => Some(LdstRegoff::Strb32bl),
        (0b00, 0b0, 0b00, _) => Some(LdstRegoff::Strb32b),
        (0b00, 0b0, 0b01, 0b011) => Some(LdstRegoff::Ldrb32bl),
        (0b00, 0b0, 0b01, _) => Some(LdstRegoff::Ldrb32b),
        (0b00, 0b0, 0b10, 0b011) => Some(LdstRegoff::Ldrsb64bl),
        (0b00, 0b0, 0b10, _) => Some(LdstRegoff::Ldrsb64b),
        (0b00, 0b0, 0b11, 0b011) => Some(LdstRegoff::Ldrsb32bl),
        (0b00, 0b0, 0b11, _) => Some(LdstRegoff::Ldrsb32b),
        (0b00, 0b1, 0b00, 0b011) => Some(LdstRegoff::StrBl),
        (0b00, 0b1, 0b00, _) => Some(LdstRegoff::StrB),
        (0b00, 0b1, 0b01, 0b011) => Some(LdstRegoff::LdrBl),
        (0b00, 0b1, 0b01, _) => Some(LdstRegoff::LdrB),
        (0b00, 0b1, 0b10, _) => Some(LdstRegoff::StrQ),
        (0b00, 0b1, 0b11, _) => Some(LdstRegoff::LdrQ),
        (0b01, 0b0, 0b00, _) => Some(LdstRegoff::Strh32),
        (0b01, 0b0, 0b01, _) => Some(LdstRegoff::Ldrh32),
        (0b01, 0b0, 0b10, _) => Some(LdstRegoff::Ldrsh64),
        (0b01, 0b0, 0b11, _) => Some(LdstRegoff::Ldrsh32),
        (0b01, 0b1, 0b00, _) => Some(LdstRegoff::StrH),
        (0b01, 0b1, 0b01, _) => Some(LdstRegoff::LdrH),
        (0b10, 0b0, 0b00, _) => Some(LdstRegoff::Str32),
        (0b10, 0b0, 0b01, _) => Some(LdstRegoff::Ldr32),
        (0b10, 0b0, 0b10, _) => Some(LdstRegoff::Ldrsw64),
        (0b10, 0b1, 0b00, _) => Some(LdstRegoff::StrS),
        (0b10, 0b1, 0b01, _) => Some(LdstRegoff::LdrS),
        (0b11, 0b0, 0b00, _) => Some(LdstRegoff::Str64),
        (0b11, 0b0, 0b01, _) => Some(LdstRegoff::Ldr64),
        // Every Rt: a word RPRFM (FEAT_RPRFM) would take is a PRFM with a
        // reserved prfop on a core without the feature, a prefetch hint
        // (Arm ARM C6.2, PRFM (register)).
        (0b11, 0b0, 0b10, 0b010 | 0b011 | 0b110 | 0b111) => Some(LdstRegoff::PrfmP),
        (0b11, 0b1, 0b00, _) => Some(LdstRegoff::StrD),
        (0b11, 0b1, 0b01, _) => Some(LdstRegoff::LdrD),
        _ => None,
    }
}

fn ldst_pos(word: u32) -> Option<LdstPos> {
    let size = bits(word, 31, 30);
    let vr = bits(word, 26, 26);
    let opc = bits(word, 23, 22);
    match (size, vr, opc) {
        (0b00, 0b0, 0b00) => Some(LdstPos::Strb32),
        (0b00, 0b0, 0b01) => Some(LdstPos::Ldrb32),
        (0b00, 0b0, 0b10) => Some(LdstPos::Ldrsb64),
        (0b00, 0b0, 0b11) => Some(LdstPos::Ldrsb32),
        (0b00, 0b1, 0b00) => Some(LdstPos::StrB),
        (0b00, 0b1, 0b01) => Some(LdstPos::LdrB),
        (0b00, 0b1, 0b10) => Some(LdstPos::StrQ),
        (0b00, 0b1, 0b11) => Some(LdstPos::LdrQ),
        (0b01, 0b0, 0b00) => Some(LdstPos::Strh32),
        (0b01, 0b0, 0b01) => Some(LdstPos::Ldrh32),
        (0b01, 0b0, 0b10) => Some(LdstPos::Ldrsh64),
        (0b01, 0b0, 0b11) => Some(LdstPos::Ldrsh32),
        (0b01, 0b1, 0b00) => Some(LdstPos::StrH),
        (0b01, 0b1, 0b01) => Some(LdstPos::LdrH),
        (0b10, 0b0, 0b00) => Some(LdstPos::Str32),
        (0b10, 0b0, 0b01) => Some(LdstPos::Ldr32),
        (0b10, 0b0, 0b10) => Some(LdstPos::Ldrsw64),
        (0b10, 0b1, 0b00) => Some(LdstPos::StrS),
        (0b10, 0b1, 0b01) => Some(LdstPos::LdrS),
        (0b11, 0b0, 0b00) => Some(LdstPos::Str64),
        (0b11, 0b0, 0b01) => Some(LdstPos::Ldr64),
        (0b11, 0b0, 0b10) => Some(LdstPos::PrfmP),
        (0b11, 0b1, 0b00) => Some(LdstPos::StrD),
        (0b11, 0b1, 0b01) => Some(LdstPos::LdrD),
        _ => None,
    }
}

// The classes that hold both general-purpose and SIMD&FP register forms:
// the latter are FEAT_FP.

#[cfg(test)]
impl Loadlit {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Ldr32 | Self::Ldr64 | Self::Ldrsw64 | Self::PrfmP => Feature::Base,
            Self::LdrS | Self::LdrD | Self::LdrQ => Feature::Fp,
        }
    }
}

#[cfg(test)]
impl LdstnapairOffs {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Stnp32 | Self::Ldnp32 | Self::Stnp64 | Self::Ldnp64 => Feature::Base,
            Self::StnpS | Self::LdnpS | Self::StnpD | Self::LdnpD | Self::StnpQ | Self::LdnpQ => Feature::Fp,
        }
    }
}

#[cfg(test)]
impl LdstpairPost {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Stp32 | Self::Ldp32 | Self::Ldpsw64 | Self::Stp64 | Self::Ldp64 => Feature::Base,
            Self::StpS | Self::LdpS | Self::StpD | Self::LdpD | Self::StpQ | Self::LdpQ => Feature::Fp,
        }
    }
}

#[cfg(test)]
impl LdstpairOff {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Stp32 | Self::Ldp32 | Self::Ldpsw64 | Self::Stp64 | Self::Ldp64 => Feature::Base,
            Self::StpS | Self::LdpS | Self::StpD | Self::LdpD | Self::StpQ | Self::LdpQ => Feature::Fp,
        }
    }
}

#[cfg(test)]
impl LdstpairPre {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Stp32 | Self::Ldp32 | Self::Ldpsw64 | Self::Stp64 | Self::Ldp64 => Feature::Base,
            Self::StpS | Self::LdpS | Self::StpD | Self::LdpD | Self::StpQ | Self::LdpQ => Feature::Fp,
        }
    }
}

#[cfg(test)]
impl LdstUnscaled {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Sturb32
            | Self::Ldurb32
            | Self::Ldursb64
            | Self::Ldursb32
            | Self::Sturh32
            | Self::Ldurh32
            | Self::Ldursh64
            | Self::Ldursh32
            | Self::Stur32
            | Self::Ldur32
            | Self::Ldursw64
            | Self::Stur64
            | Self::Ldur64
            | Self::PrfumP => Feature::Base,
            Self::SturB
            | Self::LdurB
            | Self::SturQ
            | Self::LdurQ
            | Self::SturH
            | Self::LdurH
            | Self::SturS
            | Self::LdurS
            | Self::SturD
            | Self::LdurD => Feature::Fp,
        }
    }
}

#[cfg(test)]
impl LdstImmpost {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Strb32
            | Self::Ldrb32
            | Self::Ldrsb64
            | Self::Ldrsb32
            | Self::Strh32
            | Self::Ldrh32
            | Self::Ldrsh64
            | Self::Ldrsh32
            | Self::Str32
            | Self::Ldr32
            | Self::Ldrsw64
            | Self::Str64
            | Self::Ldr64 => Feature::Base,
            Self::StrB
            | Self::LdrB
            | Self::StrQ
            | Self::LdrQ
            | Self::StrH
            | Self::LdrH
            | Self::StrS
            | Self::LdrS
            | Self::StrD
            | Self::LdrD => Feature::Fp,
        }
    }
}

#[cfg(test)]
impl LdstImmpre {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Strb32
            | Self::Ldrb32
            | Self::Ldrsb64
            | Self::Ldrsb32
            | Self::Strh32
            | Self::Ldrh32
            | Self::Ldrsh64
            | Self::Ldrsh32
            | Self::Str32
            | Self::Ldr32
            | Self::Ldrsw64
            | Self::Str64
            | Self::Ldr64 => Feature::Base,
            Self::StrB
            | Self::LdrB
            | Self::StrQ
            | Self::LdrQ
            | Self::StrH
            | Self::LdrH
            | Self::StrS
            | Self::LdrS
            | Self::StrD
            | Self::LdrD => Feature::Fp,
        }
    }
}

#[cfg(test)]
impl LdstRegoff {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Strb32b
            | Self::Strb32bl
            | Self::Ldrb32b
            | Self::Ldrb32bl
            | Self::Ldrsb64b
            | Self::Ldrsb64bl
            | Self::Ldrsb32b
            | Self::Ldrsb32bl
            | Self::Strh32
            | Self::Ldrh32
            | Self::Ldrsh64
            | Self::Ldrsh32
            | Self::Str32
            | Self::Ldr32
            | Self::Ldrsw64
            | Self::Str64
            | Self::Ldr64
            | Self::PrfmP => Feature::Base,
            Self::StrB
            | Self::StrBl
            | Self::LdrB
            | Self::LdrBl
            | Self::StrQ
            | Self::LdrQ
            | Self::StrH
            | Self::LdrH
            | Self::StrS
            | Self::LdrS
            | Self::StrD
            | Self::LdrD => Feature::Fp,
        }
    }
}

#[cfg(test)]
impl LdstPos {
    pub(crate) const fn feature(self) -> Feature {
        match self {
            Self::Strb32
            | Self::Ldrb32
            | Self::Ldrsb64
            | Self::Ldrsb32
            | Self::Strh32
            | Self::Ldrh32
            | Self::Ldrsh64
            | Self::Ldrsh32
            | Self::Str32
            | Self::Ldr32
            | Self::Ldrsw64
            | Self::Str64
            | Self::Ldr64
            | Self::PrfmP => Feature::Base,
            Self::StrB
            | Self::LdrB
            | Self::StrQ
            | Self::LdrQ
            | Self::StrH
            | Self::LdrH
            | Self::StrS
            | Self::LdrS
            | Self::StrD
            | Self::LdrD => Feature::Fp,
        }
    }
}
