//! VEX/EVEX encoding rules shared by the 32-bit and 64-bit decoders.
//!
//! Bochs keeps `decoder_vex32`/`decoder_evex32` (fetchdecode32.cc) separate from
//! `decoder_vex64`/`decoder_evex64` (fetchdecode64.cc) and duplicates the prefix
//! parsing between them, but the rules that turn a decoded encoding into
//! operands are shared upstream: both call the same `assign_srcs`, defined once
//! in fetchdecode32.cc, and both resolve against the same attribute tables.
//!
//! This module is that shared half. Every rule here was a porting defect at some
//! point — the shift groups writing their own source, KMOV landing on the SETcc
//! operand order, the vector-length thermometer, compressed displacement — so
//! transcribing Bochs's per-mode duplication a second time for 32-bit mode would
//! have reintroduced all of them.

use crate::error::{DecodeError, DecodeResult};
use crate::opcode::Opcode;

use super::opmap_evex::{EVEX_MAPS, EVEX_TABLE};
use super::tables::BxDecodeError;
use super::find_opcode_in_table;

/// Resolve an EVEX-encoded opcode.
///
/// Bochs `BxOpcodeTableEVEX[(map - 1) * 256 + opcode]`, then the ordinary
/// decmask walk over that group. EVEX decode consults *only* this table —
/// upstream never falls back to the SSE/VEX maps — so a byte with no group, or
/// a group with no entry matching this encoding, is a guest #UD.
///
/// `opcode_map` is already the table's own numbering, in which EVEX map 4 is
/// absent and maps 5 and 6 have shifted down into slots 4 and 5.
pub(super) const fn lookup_evex_opcode(opcode_map: u8, opcode: u8, decmask: u32) -> Opcode {
    if opcode_map == 0 || opcode_map as usize > EVEX_MAPS {
        return Opcode::IaError;
    }
    let idx = (opcode_map as usize - 1) * 256 + opcode as usize;
    find_opcode_in_table(EVEX_TABLE[idx], decmask)
}

/// Which ModRM field holds the destination of a VEX/EVEX instruction whose
/// opcode byte means something else without the prefix.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VexModrmDst {
    /// VEX.vvvv — the shift/rotate groups.
    Vvvv,
    /// ModRM.reg — the opmask moves and tests.
    Nnn,
}

/// Destination override for a VEX/EVEX ModRM instruction, or `None` when the
/// ordinary opcode-byte rules already place it correctly.
///
/// Two opcode ranges change meaning under the prefix.
///
/// **Groups 12-14 (`0F 71/72/73`), shift/rotate by immediate.** The legacy SSE
/// form shifts the rm register in place, so rm is both source and destination —
/// Bochs ia_opcodes.def gives `PSRLD_UdqIb` operands `OP_Wdq, OP_Ib`. The VEX
/// and EVEX forms are non-destructive three-operand instructions whose
/// destination is VEX.vvvv: both `V128_VPSRLD_UdqIb` and `EVEX_VPSRLD_UdqIb`
/// lead with `OP_Hdq`. Under the legacy DST=rm assignment these write their own
/// source register and read whichever register the /digit happens to name,
/// which is how EVEX VPRORD came to clobber its source with the rotation of an
/// unrelated register.
///
/// **Opmask moves and tests (`0F 90/92/93/98/99`).** These share their opcode
/// bytes with SETcc, whose destination is rm. They go the other way — Bochs
/// ia_opcodes.def leads each of them with the reg-field operand:
///
/// ```text
///     0F 90  k <- k/m       KMOVQ_KGqKEq     OP_KGq, OP_KEq
///     0F 92  k <- r32/r64   KMOVW_KGwEw      OP_KGw, OP_Ed
///     0F 93  r32/r64 <- k   KMOVW_GdKEw      OP_Gd,  OP_KEw
///     0F 98  flags only     KORTESTQ_KGqKEq  OP_NONE, OP_KGq, OP_KEq
///     0F 99  flags only     KTESTQ_KGqKEq    OP_NONE, OP_KGq, OP_KEq
/// ```
///
/// KORTEST/KTEST write no register at all; the reg field is their FIRST source.
/// Under the SETcc rule the two sources arrived transposed, which KORTEST
/// survives (OR is commutative and both its flag tests are symmetric) but KTEST
/// does not: its `CF = ((~op1 & op2) == 0)` term is asymmetric, so
/// `ktestb k1,k2` reported the flags of `ktestb k2,k1`.
///
/// `0F 91` stores an opmask to memory (`KMOVQ_KEqKGq`: `OP_KEq, OP_KGq`) and
/// really does write rm, so it is deliberately absent.
///
/// Under the SETcc rule `kmovd %k0, %eax` wrote k0 and left eax untouched — and
/// that is exactly the sequence glibc's AVX-512 strlen and memchr use to turn a
/// compare mask into an index, so every such string call silently returned a
/// stale value.
pub(super) const fn vex_modrm_dst(b1: u32, is_vex_or_evex: bool) -> Option<VexModrmDst> {
    if !is_vex_or_evex {
        return None;
    }
    match b1 {
        0x171 | 0x172 | 0x173 => Some(VexModrmDst::Vvvv),
        0x190 | 0x192 | 0x193 | 0x198 | 0x199 => Some(VexModrmDst::Nnn),
        _ => None,
    }
}

/// Which slots of Bochs's `BxOpcodeTableVEX` hold a real opcode group, as one
/// 256-bit map per VEX opcode map.
///
/// Transcribed from `BxOpcodeTableVEX` in fetchdecode_opmap_avx.cc with
/// `BX_SUPPORT_AMX` off, which is what this port implements. The AMX-only VEX
/// maps 5 and 7 are rejected earlier, when the prefix is parsed.
const VEX_POPULATED_SLOTS: [[u64; 4]; 3] = [
    // map 1 (0F) — 126 slots
    [
        0x0000_FF00_00FF_0000,
        0xF0FF_FFFF_FFFF_0CF6,
        0x0000_4000_030F_0000,
        0x7FFF_FFFF_FFFF_0074,
    ],
    // map 2 (0F38) — 143 slots
    [
        0xFFFF_FF3F_77C8_FFFF,
        0x0304_0000_070F_00E3,
        0xFFF3_FFC0_FFCF_5000,
        0x00EC_FFFF_FC0C_B800,
    ],
    // map 3 (0F3A) — 69 slots
    [
        0x030F_0007_23F0_FF77,
        0xFF00_FF0F_F000_1F57,
        0x0000_0000_0000_0000,
        0x0001_0000_C000_C000,
    ],
];

/// Whether Bochs defines any VEX instruction at this opcode slot.
///
/// Upstream resolves a VEX encoding against `BxOpcodeTableVEX` and nothing
/// else, so a byte with no group there is `BxOpcodeGroup_ERR` — a guest #UD.
/// This port shares the legacy SSE tables with the VEX path instead, and those
/// hold entries for bytes that have no VEX form at all. Without this test
/// `VEX.0F 80` matched the `JO rel32` entry and the guest took a *branch*
/// instead of the #UD it had earned; `0F A4`/`0F AC` matched SHLD/SHRD and
/// `0F BA` the BT group, each of them also consuming an immediate that was not
/// there.
///
/// Testing the slot before consulting the shared table restores upstream's
/// shape for the whole class at once, rather than one opcode at a time.
pub(super) const fn vex_slot_populated(opcode_map: u8, opcode_byte: u8) -> bool {
    if opcode_map == 0 || opcode_map > 3 {
        return false;
    }
    let words = VEX_POPULATED_SLOTS[opcode_map as usize - 1];
    (words[(opcode_byte >> 6) as usize] >> (opcode_byte & 0x3F)) & 1 != 0
}

/// Size of the trailing immediate of a VEX/EVEX-encoded instruction, in bytes.
///
/// Bochs derives this from the table-relative opcode number, not from the legacy
/// immediate rules:
///
/// ```text
/// (opcode_byte >= 0x70 && opcode_byte <= 0x73) ||
/// (opcode_byte >= 0xC2 && opcode_byte <= 0xC6) || (opcode_byte >= 0x200)
/// ```
///
/// over an index of `(map - 1) * 256 + opcode` (`decoder_vex32`,
/// `decoder_evex32`; the EVEX form bounds the last term at `< 0x300` because
/// maps 5 and 6 sit above it).
///
/// The legacy rules for the same bytes are much wider — `0F 80..8F` is a
/// `rel32` Jcc and `0F A4/AC/BA` take an imm8 — and none of those has a VEX
/// form. Applying them to a VEX encoding consumed bytes the instruction does
/// not have before the table lookup could reject it, and at a page boundary
/// those are fetches the guest never asked for.
///
/// `opcode_map` is the internal numbering: 1 = `0F`, 2 = `0F38`, 3 = `0F3A`,
/// and 4/5 are the EVEX map 5/6 blocks, which carry no immediate.
pub(super) const fn vex_immediate_size(opcode_map: u8, opcode_byte: u8) -> u8 {
    match opcode_map {
        1 => match opcode_byte {
            0x70..=0x73 | 0xC2..=0xC6 => 1,
            _ => 0,
        },
        3 => 1,
        _ => 0,
    }
}

/// Vector-length field of the decoding mask.
///
/// Bochs builds it as `i->getVL()-1` over a `getVL()` of 1/2/4, giving 0 for
/// VL128, 1 for VL256 and **3** for VL512 — not the raw `L'L` bits
/// (fetchdecode64.cc `decoder_evex64`). The attributes rely on that shape:
/// `ATTR_VL512` tests both bits against 3 and `ATTR_VL256_512` tests only the
/// low bit, so feeding the raw 2 for 512-bit makes every VL512 and VL256_512
/// entry fail to match and the instruction decodes as #UD. VEX only ever
/// reaches 1, so the mapping is a no-op there.
pub(super) const fn vl_thermometer(vl: u8) -> u32 {
    match vl {
        0 => 0,
        1 => 1,
        _ => 3,
    }
}

/// Vector length actually in force, after EVEX.b's register-form override.
///
/// `L'L` is overloaded: on a register operand with EVEX.b it carries the
/// embedded rounding mode instead, and the operation is always full width.
/// Bochs runs `setVL(1 << evex_vl_rc)` and then, inside the `modC0` branch,
/// `if (i->getEvexb()) i->setVL(BX_VL512)` — and every later use, the decoding
/// mask included, reads back the overridden value.
pub(super) const fn evex_effective_vl(raw_vl: u8, evex_b: bool, mod_c0: bool) -> u8 {
    if evex_b && mod_c0 {
        2 // VL512
    } else {
        raw_vl
    }
}

/// `L'L = 11b` is reserved; only EVEX.b's register-form override can produce a
/// legal encoding with those bits set, by replacing the length outright.
///
/// Bochs closes `decoder_evex32` and `decoder_evex64` with
/// `if (i->getVL() > BX_VL512) ia_opcode = BX_IA_ERROR;` — placed after the
/// override, so the embedded-rounding forms survive it.
pub(super) const fn evex_vector_length_ok(effective_vl: u8) -> bool {
    effective_vl <= 2
}

/// The `is4` operand of VBLENDVPS/VBLENDVPD/VPBLENDVB: a fourth source register
/// encoded in `imm8[7:4]`.
///
/// Bochs `assign_srcs` `BX_SRC_VIB` takes four bits in 64-bit mode (xmm0-15) and
/// three outside it, so a 32-bit guest that leaves imm8 bit 7 set still selects
/// xmm0-7 rather than a register it cannot address.
pub(super) const fn vex_is4_src3(opcode: Opcode, imm: u32, is_64: bool) -> Option<u8> {
    if matches!(
        opcode,
        Opcode::VblendvpsVpsHpsWpsIb
            | Opcode::VblendvpdVpdHpdWpdIb
            | Opcode::V128VpblendvbVdqHdqWdqIb
            | Opcode::V256VpblendvbVdqHdqWdqIb
    ) {
        let mask = if is_64 { 0xF } else { 0x7 };
        Some(((imm >> 4) & mask) as u8)
    } else {
        None
    }
}

/// EVEX compressed displacement.
///
/// A `mod=01` memory operand stores its displacement already divided by N, the
/// size of the memory element the instruction actually touches, so the byte has
/// to be scaled back up before it can be used as an address. Bochs recovers N in
/// `evex_displ8_compression` and applies it in `assign_srcs` — after the opcode
/// is known, which is why this runs at the end of decode and not where the
/// displacement byte was read.
///
/// Left unscaled, `vmovdqa64 ymm1, [rsi+0x20]` (encoded disp8=1) addresses
/// rsi+1, so an access glibc had just aligned with `and rsi,-32` fails the
/// alignment check and the guest takes a #GP it never earned.
pub(super) const fn evex_scale_displ8(
    opcode: Opcode,
    displacement: u32,
    vl: u8,
    evex_b: bool,
    vex_w: bool,
) -> u32 {
    let scale = super::evex_operands::evex_tuple(opcode).scale(vl, evex_b, vex_w);
    if scale > 1 {
        (displacement as i32).wrapping_mul(scale as i32) as u32
    } else {
        displacement
    }
}

/// The VSIB gather/scatter groups are the only EVEX entries carrying
/// `ATTR_MOD_MEM | ATTR_MASK_REQUIRED` (Bochs fetchdecode_opmap_evex.cc
/// `BxOpcodeGroup_EVEX_0F3890..93` and `0F38A0..A3`): they have no register
/// form, and the merging opmask is what selects the elements, so k0 has no
/// meaning. Those attributes live in the opmap tables rather than in
/// ia_opcodes_evex.def, so they are not covered by the generated flag table and
/// are listed here instead.
pub(super) const fn evex_vsib_form_illegal(opcode: Opcode, mod_c0: bool, opmask: u8) -> bool {
    matches!(
        opcode,
        Opcode::EvexVgatherddVdqVsib
            | Opcode::EvexVgatherdqVdqVsib
            | Opcode::EvexVgatherqdVdqVsib
            | Opcode::EvexVgatherqqVdqVsib
            | Opcode::EvexVscatterddVsibVdq
            | Opcode::EvexVscatterdqVsibVdq
            | Opcode::EvexVscatterqdVsibVdq
            | Opcode::EvexVscatterqqVsibVdq
            | Opcode::EvexVscatterdpsVsibVps
            | Opcode::EvexVscatterdpdVsibVpd
            | Opcode::EvexVscatterqpsVsibVps
            | Opcode::EvexVscatterqpdVsibVpd
    ) && (mod_c0 || opmask == 0)
}

/// EVEX.b overloads one bit: embedded broadcast on a memory operand, SAE or
/// embedded rounding on a register one. An opcode that supports neither must
/// raise #UD instead of ignoring the bit — Bochs fetchdecode32.cc applies
/// exactly this pair of tests after resolving the opcode, pointing `execute1` at
/// `BxError`.
pub(super) const fn validate_evex_b(opcode: Opcode, mod_c0: bool) -> DecodeResult<()> {
    let flags = crate::opcode_isa::opcode_evex_flags(opcode);
    if (flags & crate::opcode_isa::PREPARE_EVEX) != 0 {
        let (forbidden, err) = if mod_c0 {
            (
                crate::opcode_isa::PREPARE_EVEX_NO_SAE,
                BxDecodeError::BxEvexIllegalEvexBSaeNotAllowed,
            )
        } else {
            (
                crate::opcode_isa::PREPARE_EVEX_NO_BROADCAST,
                BxDecodeError::BxEvexIllegalEvexBBroadcastNotAllowed,
            )
        };
        if (flags & forbidden) == forbidden {
            return Err(DecodeError::Decoder(err));
        }
    }
    Ok(())
}

/// Encoding limits Bochs places on VEX forms that rusty_box reaches through a
/// shared legacy SSE table entry.
///
/// Those entries carry no VEX-specific attributes, so nothing in the table
/// constrains vector length or ModRM form; Bochs states the limits in its
/// separate VEX groups (`fetchdecode_opmap_avx.cc`). The decoded *results* are
/// already correct — this supplies the reserved-encoding `#UD` that was
/// missing. Only called on the VEX path, so legacy encodings are unaffected.
pub(super) const fn validate_vex_legacy_form(
    opcode: Opcode,
    vex_l: u8,
    mod_c0: bool,
) -> DecodeResult<()> {
    use Opcode::*;

    // 0F 90..9F under VEX is the opmask block (KMOV/KADD/KAND/.../KTEST) —
    // Bochs BxOpcodeGroup_VEX_0F90..0F9F. SETcc has no VEX encoding at all, so
    // reaching one here means the shared legacy entry caught a reserved VEX
    // form its VEX siblings rejected (a bad SSE prefix, a memory operand where
    // the group demands MODC0, VEX.W1 outside 64-bit mode) and the SETcc entry
    // matched because its attribute mask constrains nothing.
    if matches!(
        opcode,
        SetoEb
            | SetnoEb
            | SetbEb
            | SetnbEb
            | SetzEb
            | SetnzEb
            | SetbeEb
            | SetnbeEb
            | SetsEb
            | SetnsEb
            | SetpEb
            | SetnpEb
            | SetlEb
            | SetnlEb
            | SetleEb
            | SetnleEb
    ) {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
    }

    // 0F AE under VEX is only VLDMXCSR (/2) and VSTMXCSR (/3), both memory
    // forms — Bochs BxOpcodeGroup_VEX_0FAE. Every other nnn at this opcode
    // (FXSAVE/FXRSTOR/XSAVE/XRSTOR/CLFLUSH/fences/FSGSBASE/CET/WAITPKG) has no
    // VEX encoding at all.
    if matches!(
        opcode,
        Fxsave
            | Fxrstor
            | Xsave
            | Xrstor
            | Xsaveopt
            | Xsaves
            | Xrstors
            | Clflush
            | Clflushopt
            | Clwb
            | Lfence
            | Mfence
            | Sfence
            | Incsspd
            | Incsspq
            | Clrssbsy
            | TpauseEd
            | UmwaitEd
            | UmonitorEd
            | UmonitorEq
            | RdfsbaseEd
            | RdfsbaseEq
            | WrfsbaseEd
            | WrfsbaseEq
            | RdgsbaseEd
            | RdgsbaseEq
            | WrgsbaseEd
            | WrgsbaseEq
    ) {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
    }

    // ATTR_VL128: a VEX.256 encoding of these is reserved.
    if vex_l != 0
        && matches!(
            opcode,
            MovlpsMqVps
                | MovlpdMqVsd
                | MovhpsMqVps
                | MovhpdMqVsd
                | MovdVdqEd
                | MovqVdqEq
                | MovdEdVd
                | MovqEqVq
                | PextrwGdUdqIb
                | MaskmovdquVdqUdq
                | Ldmxcsr
                | Stmxcsr
        )
    {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
    }

    // ATTR_MODC0: register operand only.
    if !mod_c0 && matches!(opcode, PextrwGdUdqIb | MaskmovdquVdqUdq) {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
    }

    // ATTR_MOD_MEM: memory operand only.
    if mod_c0 && matches!(opcode, Ldmxcsr | Stmxcsr) {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalOpcode));
    }

    Ok(())
}

/// VEX forms that take no `vvvv` source operand; Intel reserves every encoding
/// except VEX.vvvv = 1111b (decoded here as zero). Bochs marks these
/// "VEX.VVV #UD" in the opcode comments of `cpu/avx/*.cc`.
pub(super) const fn validate_reserved_vex_vvvv(opcode: Opcode, vex_vvv: u8) -> DecodeResult<()> {
    if vex_vvv != 0
        && matches!(
            opcode,
            Opcode::V256Vextractf128WdqVdqIb
                | Opcode::V256Vextracti128WdqVdqIb
                | Opcode::VtestpsVpsWps
                | Opcode::VtestpdVpdWpd
                | Opcode::VpermilpsVpsWpsIb
                | Opcode::VpermilpdVpdWpdIb
                | Opcode::V256VpermpdVpdWpdIb
                | Opcode::Vcvtph2psVpsWps
                | Opcode::Vcvtps2phWpsVpsIb
                | Opcode::V128VmovntdqaVdqMdq
                | Opcode::V256VmovntdqaVdqMdq
                | Opcode::V128VpextrbEdVdqIbR
                | Opcode::V128VpextrbMbVdqIbM
                | Opcode::V128VpextrwEdVdqIbR
                | Opcode::V128VpextrwMwVdqIbM
                | Opcode::V128VpextrdEdVdqIb
                | Opcode::V128VpextrqEqVdqIb
                | Opcode::V128VpcmpestrmVdqWdqIb
                | Opcode::V128VpcmpestriVdqWdqIb
                | Opcode::V128VpcmpistrmVdqWdqIb
                | Opcode::V128VpcmpistriVdqWdqIb
                // VEX forms reached through a shared legacy SSE table entry.
                // None of them has an `H` operand in Bochs ia_opcodes.def, so
                // VEX.vvvv is reserved for all of them.
                | Opcode::MovlpsMqVps
                | Opcode::MovlpdMqVsd
                | Opcode::MovhpsMqVps
                | Opcode::MovhpdMqVsd
                | Opcode::MovdVdqEd
                | Opcode::MovqVdqEq
                | Opcode::MovdEdVd
                | Opcode::MovqEqVq
                | Opcode::PextrwGdUdqIb
                | Opcode::MaskmovdquVdqUdq
                | Opcode::Ldmxcsr
                | Opcode::Stmxcsr
                | Opcode::UcomissVssWss
                | Opcode::UcomisdVsdWsd
                | Opcode::ComissVssWss
                | Opcode::ComisdVsdWsd
                | Opcode::Cvttss2siGdWss
                | Opcode::Cvttss2siGqWss
                | Opcode::Cvttsd2siGdWsd
                | Opcode::Cvttsd2siGqWsd
                | Opcode::Cvtss2siGdWss
                | Opcode::Cvtss2siGqWss
                | Opcode::Cvtsd2siGdWsd
                | Opcode::Cvtsd2siGqWsd
        )
    {
        return Err(DecodeError::Decoder(BxDecodeError::BxIllegalVexXopVvv));
    }
    Ok(())
}

/// Remap SSE opcodes to VEX opcodes when VEX prefix is active.
///
/// The opcode tables are shared between SSE and VEX instructions. When a VEX
/// prefix is present, the table lookup may return an SSE opcode (2-operand form
/// that ignores VEX.vvvv). This function remaps to the proper VEX opcode so the
/// 3-operand VEX handler is dispatched.
///
/// `vl`: VEX.L field — 0 = 128-bit (XMM), 1 = 256-bit (YMM)
pub(super) const fn remap_sse_to_vex(op: Opcode, vl: u8) -> Opcode {
    use Opcode::*;
    match op {
        // ===== Integer arithmetic =====
        PadddVdqWdq => {
            if vl == 0 {
                V128VpadddVdqHdqWdq
            } else {
                V256VpadddVdqHdqWdq
            }
        }
        PaddqVdqWdq => {
            if vl == 0 {
                V128VpaddqVdqHdqWdq
            } else {
                V256VpaddqVdqHdqWdq
            }
        }
        PaddwVdqWdq => {
            if vl == 0 {
                V128VpaddwVdqHdqWdq
            } else {
                V256VpaddwVdqHdqWdq
            }
        }
        PaddbVdqWdq => {
            if vl == 0 {
                V128VpaddbVdqHdqWdq
            } else {
                V256VpaddbVdqHdqWdq
            }
        }
        PsubdVdqWdq => {
            if vl == 0 {
                V128VpsubdVdqHdqWdq
            } else {
                V256VpsubdVdqHdqWdq
            }
        }
        PsubqVdqWdq => {
            if vl == 0 {
                V128VpsubqVdqHdqWdq
            } else {
                V256VpsubqVdqHdqWdq
            }
        }
        PsubwVdqWdq => {
            if vl == 0 {
                V128VpsubwVdqHdqWdq
            } else {
                V256VpsubwVdqHdqWdq
            }
        }
        PsubbVdqWdq => {
            if vl == 0 {
                V128VpsubbVdqHdqWdq
            } else {
                V256VpsubbVdqHdqWdq
            }
        }
        // Saturating
        PaddsbVdqWdq => {
            if vl == 0 {
                V128VpaddsbVdqHdqWdq
            } else {
                V256VpaddsbVdqHdqWdq
            }
        }
        PaddswVdqWdq => {
            if vl == 0 {
                V128VpaddswVdqHdqWdq
            } else {
                V256VpaddswVdqHdqWdq
            }
        }
        PsubsbVdqWdq => {
            if vl == 0 {
                V128VpsubsbVdqHdqWdq
            } else {
                V256VpsubsbVdqHdqWdq
            }
        }
        PsubswVdqWdq => {
            if vl == 0 {
                V128VpsubswVdqHdqWdq
            } else {
                V256VpsubswVdqHdqWdq
            }
        }
        PsubusbVdqWdq => {
            if vl == 0 {
                V128VpsubusbVdqHdqWdq
            } else {
                V256VpsubusbVdqHdqWdq
            }
        }
        PsubuswVdqWdq => {
            if vl == 0 {
                V128VpsubuswVdqHdqWdq
            } else {
                V256VpsubuswVdqHdqWdq
            }
        }
        PaddusbVdqWdq => {
            if vl == 0 {
                V128VpaddusbVdqHdqWdq
            } else {
                V256VpaddusbVdqHdqWdq
            }
        }
        PadduswVdqWdq => {
            if vl == 0 {
                V128VpadduswVdqHdqWdq
            } else {
                V256VpadduswVdqHdqWdq
            }
        }

        // ===== Logical =====
        PxorVdqWdq => {
            if vl == 0 {
                V128VpxorVdqHdqWdq
            } else {
                V256VpxorVdqHdqWdq
            }
        }
        PandVdqWdq => {
            if vl == 0 {
                V128VpandVdqHdqWdq
            } else {
                V256VpandVdqHdqWdq
            }
        }
        PorVdqWdq => {
            if vl == 0 {
                V128VporVdqHdqWdq
            } else {
                V256VporVdqHdqWdq
            }
        }
        PandnVdqWdq => {
            if vl == 0 {
                V128VpandnVdqHdqWdq
            } else {
                V256VpandnVdqHdqWdq
            }
        }

        // ===== Multiply =====
        PmuludqVdqWdq => {
            if vl == 0 {
                V128VpmuludqVdqHdqWdq
            } else {
                V256VpmuludqVdqHdqWdq
            }
        }
        PmuldqVdqWdq => {
            if vl == 0 {
                V128VpmuldqVdqHdqWdq
            } else {
                V256VpmuldqVdqHdqWdq
            }
        }
        PmulldVdqWdq => {
            if vl == 0 {
                V128VpmulldVdqHdqWdq
            } else {
                V256VpmulldVdqHdqWdq
            }
        }
        PmullwVdqWdq => {
            if vl == 0 {
                V128VpmullwVdqHdqWdq
            } else {
                V256VpmullwVdqHdqWdq
            }
        }
        PmulhwVdqWdq => {
            if vl == 0 {
                V128VpmulhwVdqHdqWdq
            } else {
                V256VpmulhwVdqHdqWdq
            }
        }
        PmulhuwVdqWdq => {
            if vl == 0 {
                V128VpmulhuwVdqHdqWdq
            } else {
                V256VpmulhuwVdqHdqWdq
            }
        }
        PmulhrswVdqWdq => {
            if vl == 0 {
                V128VpmulhrswVdqHdqWdq
            } else {
                V256VpmulhrswVdqHdqWdq
            }
        }

        // ===== Compare =====
        PcmpeqbVdqWdq => {
            if vl == 0 {
                V128VpcmpeqbVdqHdqWdq
            } else {
                V256VpcmpeqbVdqHdqWdq
            }
        }
        PcmpeqwVdqWdq => {
            if vl == 0 {
                V128VpcmpeqwVdqHdqWdq
            } else {
                V256VpcmpeqwVdqHdqWdq
            }
        }
        PcmpeqdVdqWdq => {
            if vl == 0 {
                V128VpcmpeqdVdqHdqWdq
            } else {
                V256VpcmpeqdVdqHdqWdq
            }
        }
        PcmpeqqVdqWdq => {
            if vl == 0 {
                V128VpcmpeqqVdqHdqWdq
            } else {
                V256VpcmpeqqVdqHdqWdq
            }
        }
        PcmpgtbVdqWdq => {
            if vl == 0 {
                V128VpcmpgtbVdqHdqWdq
            } else {
                V256VpcmpgtbVdqHdqWdq
            }
        }
        PcmpgtwVdqWdq => {
            if vl == 0 {
                V128VpcmpgtwVdqHdqWdq
            } else {
                V256VpcmpgtwVdqHdqWdq
            }
        }
        PcmpgtdVdqWdq => {
            if vl == 0 {
                V128VpcmpgtdVdqHdqWdq
            } else {
                V256VpcmpgtdVdqHdqWdq
            }
        }
        PcmpgtqVdqWdq => {
            if vl == 0 {
                V128VpcmpgtqVdqHdqWdq
            } else {
                V256VpcmpgtqVdqHdqWdq
            }
        }

        // ===== Shift by register =====
        PsrlwVdqWdq => {
            if vl == 0 {
                V128VpsrlwVdqHdqWdq
            } else {
                V256VpsrlwVdqHdqWdq
            }
        }
        PsrldVdqWdq => {
            if vl == 0 {
                V128VpsrldVdqHdqWdq
            } else {
                V256VpsrldVdqHdqWdq
            }
        }
        PsrlqVdqWdq => {
            if vl == 0 {
                V128VpsrlqVdqHdqWdq
            } else {
                V256VpsrlqVdqHdqWdq
            }
        }
        PsrawVdqWdq => {
            if vl == 0 {
                V128VpsrawVdqHdqWdq
            } else {
                V256VpsrawVdqHdqWdq
            }
        }
        PsradVdqWdq => {
            if vl == 0 {
                V128VpsradVdqHdqWdq
            } else {
                V256VpsradVdqHdqWdq
            }
        }
        PsllwVdqWdq => {
            if vl == 0 {
                V128VpsllwVdqHdqWdq
            } else {
                V256VpsllwVdqHdqWdq
            }
        }
        PslldVdqWdq => {
            if vl == 0 {
                V128VpslldVdqHdqWdq
            } else {
                V256VpslldVdqHdqWdq
            }
        }
        PsllqVdqWdq => {
            if vl == 0 {
                V128VpsllqVdqHdqWdq
            } else {
                V256VpsllqVdqHdqWdq
            }
        }

        // ===== Shift by immediate (Group 12/13/14) =====
        PsrlwUdqIb => {
            if vl == 0 {
                V128VpsrlwUdqIb
            } else {
                V256VpsrlwUdqIb
            }
        }
        PsrldUdqIb => {
            if vl == 0 {
                V128VpsrldUdqIb
            } else {
                V256VpsrldUdqIb
            }
        }
        PsrlqUdqIb => {
            if vl == 0 {
                V128VpsrlqUdqIb
            } else {
                V256VpsrlqUdqIb
            }
        }
        PsrawUdqIb => {
            if vl == 0 {
                V128VpsrawUdqIb
            } else {
                V256VpsrawUdqIb
            }
        }
        PsradUdqIb => {
            if vl == 0 {
                V128VpsradUdqIb
            } else {
                V256VpsradUdqIb
            }
        }
        PsllwUdqIb => {
            if vl == 0 {
                V128VpsllwUdqIb
            } else {
                V256VpsllwUdqIb
            }
        }
        PslldUdqIb => {
            if vl == 0 {
                V128VpslldUdqIb
            } else {
                V256VpslldUdqIb
            }
        }
        PsllqUdqIb => {
            if vl == 0 {
                V128VpsllqUdqIb
            } else {
                V256VpsllqUdqIb
            }
        }
        PsrldqUdqIb => {
            if vl == 0 {
                V128VpsrldqUdqIb
            } else {
                V256VpsrldqUdqIb
            }
        }
        PslldqUdqIb => {
            if vl == 0 {
                V128VpslldqUdqIb
            } else {
                V256VpslldqUdqIb
            }
        }

        // ===== Shuffle / Unpack =====
        PshufbVdqWdq => {
            if vl == 0 {
                V128VpshufbVdqHdqWdq
            } else {
                V256VpshufbVdqHdqWdq
            }
        }
        PshufdVdqWdqIb => {
            if vl == 0 {
                V128VpshufdVdqWdqIb
            } else {
                V256VpshufdVdqWdqIb
            }
        }
        PshufhwVdqWdqIb => {
            if vl == 0 {
                V128VpshufhwVdqWdqIb
            } else {
                V256VpshufhwVdqWdqIb
            }
        }
        PshuflwVdqWdqIb => {
            if vl == 0 {
                V128VpshuflwVdqWdqIb
            } else {
                V256VpshuflwVdqWdqIb
            }
        }
        PunpckldqVdqWdq => {
            if vl == 0 {
                V128VpunpckldqVdqHdqWdq
            } else {
                V256VpunpckldqVdqHdqWdq
            }
        }
        PunpckhdqVdqWdq => {
            if vl == 0 {
                V128VpunpckhdqVdqHdqWdq
            } else {
                V256VpunpckhdqVdqHdqWdq
            }
        }
        PunpcklbwVdqWdq => {
            if vl == 0 {
                V128VpunpcklbwVdqHdqWdq
            } else {
                V256VpunpcklbwVdqHdqWdq
            }
        }
        PunpckhbwVdqWdq => {
            if vl == 0 {
                V128VpunpckhbwVdqHdqWdq
            } else {
                V256VpunpckhbwVdqHdqWdq
            }
        }
        PunpcklwdVdqWdq => {
            if vl == 0 {
                V128VpunpcklwdVdqHdqWdq
            } else {
                V256VpunpcklwdVdqHdqWdq
            }
        }
        PunpckhwdVdqWdq => {
            if vl == 0 {
                V128VpunpckhwdVdqHdqWdq
            } else {
                V256VpunpckhwdVdqHdqWdq
            }
        }
        PunpcklqdqVdqWdq => {
            if vl == 0 {
                V128VpunpcklqdqVdqHdqWdq
            } else {
                V256VpunpcklqdqVdqHdqWdq
            }
        }
        PunpckhqdqVdqWdq => {
            if vl == 0 {
                V128VpunpckhqdqVdqHdqWdq
            } else {
                V256VpunpckhqdqVdqHdqWdq
            }
        }

        // ===== PALIGNR =====
        PalignrVdqWdqIb => {
            if vl == 0 {
                V128VpalignrVdqHdqWdqIb
            } else {
                V256VpalignrVdqHdqWdqIb
            }
        }

        // ===== Pack =====
        PacksswbVdqWdq => {
            if vl == 0 {
                V128VpacksswbVdqHdqWdq
            } else {
                V256VpacksswbVdqHdqWdq
            }
        }
        PackuswbVdqWdq => {
            if vl == 0 {
                V128VpackuswbVdqHdqWdq
            } else {
                V256VpackuswbVdqHdqWdq
            }
        }
        PackssdwVdqWdq => {
            if vl == 0 {
                V128VpackssdwVdqHdqWdq
            } else {
                V256VpackssdwVdqHdqWdq
            }
        }
        PackusdwVdqWdq => {
            if vl == 0 {
                V128VpackusdwVdqHdqWdq
            } else {
                V256VpackusdwVdqHdqWdq
            }
        }

        // ===== Min/Max (SSE2 + SSE4.1) =====
        PminubVdqWdq => {
            if vl == 0 {
                V128VpminubVdqHdqWdq
            } else {
                V256VpminubVdqHdqWdq
            }
        }
        PminswVdqWdq => {
            if vl == 0 {
                V128VpminswVdqHdqWdq
            } else {
                V256VpminswVdqHdqWdq
            }
        }
        PmaxubVdqWdq => {
            if vl == 0 {
                V128VpmaxubVdqHdqWdq
            } else {
                V256VpmaxubVdqHdqWdq
            }
        }
        PmaxswVdqWdq => {
            if vl == 0 {
                V128VpmaxswVdqHdqWdq
            } else {
                V256VpmaxswVdqHdqWdq
            }
        }
        PminsbVdqWdq => {
            if vl == 0 {
                V128VpminsbVdqHdqWdq
            } else {
                V256VpminsbVdqHdqWdq
            }
        }
        PminsdVdqWdq => {
            if vl == 0 {
                V128VpminsdVdqHdqWdq
            } else {
                V256VpminsdVdqHdqWdq
            }
        }
        PminuwVdqWdq => {
            if vl == 0 {
                V128VpminuwVdqHdqWdq
            } else {
                V256VpminuwVdqHdqWdq
            }
        }
        PminudVdqWdq => {
            if vl == 0 {
                V128VpminudVdqHdqWdq
            } else {
                V256VpminudVdqHdqWdq
            }
        }
        PmaxsbVdqWdq => {
            if vl == 0 {
                V128VpmaxsbVdqHdqWdq
            } else {
                V256VpmaxsbVdqHdqWdq
            }
        }
        PmaxsdVdqWdq => {
            if vl == 0 {
                V128VpmaxsdVdqHdqWdq
            } else {
                V256VpmaxsdVdqHdqWdq
            }
        }
        PmaxuwVdqWdq => {
            if vl == 0 {
                V128VpmaxuwVdqHdqWdq
            } else {
                V256VpmaxuwVdqHdqWdq
            }
        }
        PmaxudVdqWdq => {
            if vl == 0 {
                V128VpmaxudVdqHdqWdq
            } else {
                V256VpmaxudVdqHdqWdq
            }
        }

        // ===== Average / SAD =====
        PavgbVdqWdq => {
            if vl == 0 {
                V128VpavgbVdqWdq
            } else {
                V256VpavgbVdqWdq
            }
        }
        PavgwVdqWdq => {
            if vl == 0 {
                V128VpavgwVdqWdq
            } else {
                V256VpavgwVdqWdq
            }
        }
        PsadbwVdqWdq => {
            if vl == 0 {
                V128VpsadbwVdqHdqWdq
            } else {
                V256VpsadbwVdqHdqWdq
            }
        }

        // ===== PMADDWD / PMADDUBSW =====
        PmaddwdVdqWdq => {
            if vl == 0 {
                V128VpmaddwdVdqHdqWdq
            } else {
                V256VpmaddwdVdqHdqWdq
            }
        }
        PmaddubswVdqWdq => {
            if vl == 0 {
                V128VpmaddubswVdqHdqWdq
            } else {
                V256VpmaddubswVdqHdqWdq
            }
        }

        // ===== SSSE3: PHADD/PHSUB/PSIGN =====
        PhaddwVdqWdq => {
            if vl == 0 {
                V128VphaddwVdqHdqWdq
            } else {
                V256VphaddwVdqHdqWdq
            }
        }
        PhadddVdqWdq => {
            if vl == 0 {
                V128VphadddVdqHdqWdq
            } else {
                V256VphadddVdqHdqWdq
            }
        }
        PhaddswVdqWdq => {
            if vl == 0 {
                V128VphaddswVdqHdqWdq
            } else {
                V256VphaddswVdqHdqWdq
            }
        }
        PhsubwVdqWdq => {
            if vl == 0 {
                V128VphsubwVdqHdqWdq
            } else {
                V256VphsubwVdqHdqWdq
            }
        }
        PhsubdVdqWdq => {
            if vl == 0 {
                V128VphsubdVdqHdqWdq
            } else {
                V256VphsubdVdqHdqWdq
            }
        }
        PhsubswVdqWdq => {
            if vl == 0 {
                V128VphsubswVdqHdqWdq
            } else {
                V256VphsubswVdqHdqWdq
            }
        }
        PsignbVdqWdq => {
            if vl == 0 {
                V128VpsignbVdqHdqWdq
            } else {
                V256VpsignbVdqHdqWdq
            }
        }
        PsignwVdqWdq => {
            if vl == 0 {
                V128VpsignwVdqHdqWdq
            } else {
                V256VpsignwVdqHdqWdq
            }
        }
        PsigndVdqWdq => {
            if vl == 0 {
                V128VpsigndVdqHdqWdq
            } else {
                V256VpsigndVdqHdqWdq
            }
        }

        // ===== Floating-point bitwise (VEX handler checks get_vl()) =====
        AndpsVpsWps => VandpsVpsHpsWps,
        AndnpsVpsWps => VandnpsVpsHpsWps,
        OrpsVpsWps => VorpsVpsHpsWps,
        XorpsVpsWps => VxorpsVpsHpsWps,
        AddpsVpsWps => VaddpsVpsHpsWps,
        MulpsVpsWps => VmulpsVpsHpsWps,
        SubpsVpsWps => VsubpsVpsHpsWps,
        DivpsVpsWps => VdivpsVpsHpsWps,
        AndpdVpdWpd => VandpdVpdHpdWpd,
        AndnpdVpdWpd => VandnpdVpdHpdWpd,
        OrpdVpdWpd => VorpdVpdHpdWpd,
        XorpdVpdWpd => VxorpdVpdHpdWpd,
        AddpdVpdWpd => VaddpdVpdHpdWpd,
        MulpdVpdWpd => VmulpdVpdHpdWpd,
        SubpdVpdWpd => VsubpdVpdHpdWpd,
        DivpdVpdWpd => VdivpdVpdHpdWpd,
        MinpsVpsWps => VminpsVpsHpsWps,
        MinpdVpdWpd => VminpdVpdHpdWpd,
        MaxpsVpsWps => VmaxpsVpsHpsWps,
        MaxpdVpdWpd => VmaxpdVpdHpdWpd,
        AddsubpsVpsWps => VaddsubpsVpsHpsWps,
        AddsubpdVpdWpd => VaddsubpdVpdHpdWpd,
        HaddpsVpsWps => VhaddpsVpsHpsWps,
        HaddpdVpdWpd => VhaddpdVpdHpdWpd,
        HsubpsVpsWps => VhsubpsVpsHpsWps,
        HsubpdVpdWpd => VhsubpdVpdHpdWpd,

        // ===== SSE4.1 blend / dot-product (vvvv is first source) =====
        BlendpsVpsWpsIb => VblendpsVpsHpsWpsIb,
        BlendpdVpdWpdIb => VblendpdVpdHpdWpdIb,
        DppsVpsWpsIb => VdppsVpsHpsWpsIb,
        // VDPPD is VL128-only; VEX.256 encodings #UD
        // (Bochs fetchdecode_opmap_avx.cc BxOpcodeGroup_VEX_0F3A41 ATTR_VL128)
        DppdVpdWpdIb => {
            if vl == 0 {
                VdppdVpdHpdWpdIb
            } else {
                IaError
            }
        }
        // VEX-encoded 66 0F 38 10/14/15 do not exist in AVX: the variable
        // blends moved to VEX.0F3A 4A-4C with an is4 mask register (Bochs
        // fetchdecode_opmap_avx.cc marks these VEX slots BxOpcodeGroup_ERR).
        PblendvbVdqWdq => IaError,
        BlendvpsVpsWps => IaError,
        BlendvpdVpdWpd => IaError,

        // ===== Floating-point scalar arithmetic (VEX.vvvv is first source;
        // the legacy 2-operand SSE handler would destructively use dst) =====
        AddssVssWss => VaddssVssHpsWss,
        AddsdVsdWsd => VaddsdVsdHpdWsd,
        SubssVssWss => VsubssVssHpsWss,
        SubsdVsdWsd => VsubsdVsdHpdWsd,
        MulssVssWss => VmulssVssHpsWss,
        MulsdVsdWsd => VmulsdVsdHpdWsd,
        DivssVssWss => VdivssVssHpsWss,
        DivsdVsdWsd => VdivsdVsdHpdWsd,
        MinssVssWss => VminssVssHpsWss,
        MinsdVsdWsd => VminsdVsdHpdWsd,
        MaxssVssWss => VmaxssVssHpsWss,
        MaxsdVsdWsd => VmaxsdVsdHpdWsd,

        // ===== Square root (scalar forms merge upper elements from vvvv) =====
        SqrtpsVpsWps => VsqrtpsVpsWps,
        SqrtpdVpdWpd => VsqrtpdVpdWpd,
        SqrtssVssWss => VsqrtssVssHpsWss,
        SqrtsdVsdWsd => VsqrtsdVsdHpdWsd,

        // ===== FP compare (32 AVX predicates; vvvv is first source) =====
        CmppsVpsWpsIb => VcmppsVpsHpsWpsIb,
        CmppdVpdWpdIb => VcmppdVpdHpdWpdIb,
        CmpssVssWssIb => VcmpssVssHpsWssIb,
        CmpsdVsdWsdIb => VcmpsdVsdHpdWsdIb,

        // ===== FP shuffle / unpack (vvvv is first source) =====
        ShufpsVpsWpsIb => VshufpsVpsHpsWpsIb,
        ShufpdVpdWpdIb => VshufpdVpdHpdWpdIb,
        UnpcklpsVpsWdq => VunpcklpsVpsHpsWps,
        UnpckhpsVpsWdq => VunpckhpsVpsHpsWps,
        UnpcklpdVpdWdq => VunpcklpdVpdHpdWpd,
        UnpckhpdVpdWdq => VunpckhpdVpdHpdWpd,

        // ===== Scalar moves (register forms merge low element into vvvv's
        // upper elements; the VEX handler splits mod internally) =====
        MovssVssWss => V128VmovssVssHpsWss,
        MovsdVsdWsd => V128VmovsdVsdHpdWsd,
        MovssWssVss => V128VmovssWssHpsVss,
        MovsdWsdVsd => V128VmovsdWsdHpdVsd,
        MovlpsVpsMq => V128VmovlpsVpsHpsMq,
        MovlpdVsdMq => V128VmovlpdVpdHpdMq,
        MovhpsVpsMq => V128VmovhpsVpsHpsMq,
        MovhpdVsdMq => V128VmovhpdVpdHpdMq,
        MovhlpsVpsWps => V128VmovhlpsVpsHpsWps,
        MovlhpsVpsWps => V128VmovlhpsVpsHpsWps,
        MovsldupVpsWps => VmovsldupVpsWps,
        MovshdupVpsWps => VmovshdupVpsWps,
        MovddupVpdWq => {
            if vl == 0 {
                V128VmovddupVpdWpd
            } else {
                V256VmovddupVpdWpd
            }
        }

        // ===== Scalar conversions (vvvv provides the upper elements) =====
        Cvtss2sdVsdWss => Vcvtss2sdVsdWss,
        Cvtsd2ssVssWsd => Vcvtsd2ssVssWsd,
        Cvtsi2sdVsdEd => Vcvtsi2sdVsdEd,
        Cvtsi2sdVsdEq => Vcvtsi2sdVsdEq,
        Cvtsi2ssVssEd => Vcvtsi2ssVssEd,
        Cvtsi2ssVssEq => Vcvtsi2ssVssEq,

        // ===== Packed conversions (single-source; VL selects lane count) =====
        Cvtdq2psVpsWdq => Vcvtdq2psVpsWdq,
        Cvtps2dqVdqWps => Vcvtps2dqVdqWps,
        Cvttps2dqVdqWps => Vcvttps2dqVdqWps,
        Cvtdq2pdVpdWq => Vcvtdq2pdVpdWdq,
        Cvtps2pdVpdWps => Vcvtps2pdVpdWps,
        Cvtpd2psVpsWpd => Vcvtpd2psVpsWpd,
        Cvtpd2dqVqWpd => Vcvtpd2dqVdqWpd,
        Cvttpd2dqVqWpd => Vcvttpd2dqVdqWpd,

        // ===== Round / reciprocal approximations =====
        RoundpsVpsWpsIb => VroundpsVpsWpsIb,
        RoundpdVpdWpdIb => VroundpdVpdWpdIb,
        RoundssVssWssIb => VroundssVssHpsWssIb,
        RoundsdVsdWsdIb => VroundsdVsdHpdWsdIb,
        RcppsVpsWps => VrcppsVpsWps,
        RcpssVssWss => VrcpssVssHpsWss,
        RsqrtpsVpsWps => VrsqrtpsVpsWps,
        RsqrtssVssWss => VrsqrtssVssHpsWss,

        // ===== Store-form moves (VEX handler does VL-aware stores + register form) =====
        MovdquWdqVdq => {
            if vl == 0 {
                V128VmovdquWdqVdq
            } else {
                V256VmovdquWdqVdq
            }
        }
        MovdqaWdqVdq => {
            if vl == 0 {
                V128VmovdqaWdqVdq
            } else {
                V256VmovdqaWdqVdq
            }
        }
        MovupsWpsVps => {
            if vl == 0 {
                V128VmovupsWpsVps
            } else {
                V256VmovupsWpsVps
            }
        }
        MovapsWpsVps => {
            if vl == 0 {
                V128VmovapsWpsVps
            } else {
                V256VmovapsWpsVps
            }
        }
        MovupdWpdVpd => {
            if vl == 0 {
                V128VmovupdWpdVpd
            } else {
                V256VmovupdWpdVpd
            }
        }
        MovapdWpdVpd => {
            if vl == 0 {
                V128VmovapdWpdVpd
            } else {
                V256VmovapdWpdVpd
            }
        }
        MovntdqMdqVdq => {
            if vl == 0 {
                V128VmovntdqMdqVdq
            } else {
                V256VmovntdqMdqVdq
            }
        }
        MovntpsMpsVps => {
            if vl == 0 {
                V128VmovntpsMpsVps
            } else {
                V256VmovntpsMpsVps
            }
        }
        MovntpdMpdVpd => {
            if vl == 0 {
                V128VmovntpdMpdVpd
            } else {
                V256VmovntpdMpdVpd
            }
        }

        // ===== Load-form moves (SSE handler only reads 128-bit; VEX handler is VL-aware) =====
        // These use a single VEX opcode (no V128/V256 prefix) — handler checks get_vl()
        MovdquVdqWdq => VmovdquVdqWdq,
        MovdqaVdqWdq => VmovdqaVdqWdq,
        MovupsVpsWps => VmovupsVpsWps,
        MovapsVpsWps => VmovapsVpsWps,
        MovupdVpdWpd => VmovupdVpdWpd,
        MovapdVpdWpd => VmovapdVpdWpd,

        // ===== Misc =====
        PmovmskbGdUdq => {
            if vl == 0 {
                V128VpmovmskbGdUdq
            } else {
                V256VpmovmskbGdUdq
            }
        }

        // ===== VPTEST (flags-only; must AND/ANDN the full VL, Bochs avx.cc
        // VPTEST_VdqWdqR) — single VEX opcode, handler checks get_vl() =====
        PtestVdqWdq => VptestVdqWdq,

        // ===== VMOVMSKPS/VMOVMSKPD (VL256 → 8/4-bit masks, Bochs avx.cc
        // VMOVMSKPS_GdUps / VMOVMSKPD_GdUpd) =====
        MovmskpsGdUps => VmovmskpsGdUps,
        MovmskpdGdUpd => VmovmskpdGdUpd,

        // ===== VPMOVSX/VPMOVZX (Bochs avx2.cc VPMOVSXBW_VdqWdqR et al.;
        // VL256 forms read a doubled source width — Bochs
        // fetchdecode_opmap_avx.cc BxOpcodeGroup_VEX_0F3820..25 / 30..35) =====
        PmovsxbwVdqWq => {
            if vl == 0 {
                V128VpmovsxbwVdqWq
            } else {
                V256VpmovsxbwVdqWdq
            }
        }
        PmovsxbdVdqWd => {
            if vl == 0 {
                V128VpmovsxbdVdqWd
            } else {
                V256VpmovsxbdVdqWq
            }
        }
        PmovsxbqVdqWw => {
            if vl == 0 {
                V128VpmovsxbqVdqWw
            } else {
                V256VpmovsxbqVdqWd
            }
        }
        PmovsxwdVdqWq => {
            if vl == 0 {
                V128VpmovsxwdVdqWq
            } else {
                V256VpmovsxwdVdqWdq
            }
        }
        PmovsxwqVdqWd => {
            if vl == 0 {
                V128VpmovsxwqVdqWd
            } else {
                V256VpmovsxwqVdqWq
            }
        }
        PmovsxdqVdqWq => {
            if vl == 0 {
                V128VpmovsxdqVdqWq
            } else {
                V256VpmovsxdqVdqWdq
            }
        }
        PmovzxbwVdqWq => {
            if vl == 0 {
                V128VpmovzxbwVdqWq
            } else {
                V256VpmovzxbwVdqWdq
            }
        }
        PmovzxbdVdqWd => {
            if vl == 0 {
                V128VpmovzxbdVdqWd
            } else {
                V256VpmovzxbdVdqWq
            }
        }
        PmovzxbqVdqWw => {
            if vl == 0 {
                V128VpmovzxbqVdqWw
            } else {
                V256VpmovzxbqVdqWd
            }
        }
        PmovzxwdVdqWq => {
            if vl == 0 {
                V128VpmovzxwdVdqWq
            } else {
                V256VpmovzxwdVdqWdq
            }
        }
        PmovzxwqVdqWd => {
            if vl == 0 {
                V128VpmovzxwqVdqWd
            } else {
                V256VpmovzxwqVdqWq
            }
        }
        PmovzxdqVdqWq => {
            if vl == 0 {
                V128VpmovzxdqVdqWq
            } else {
                V256VpmovzxdqVdqWdq
            }
        }

        // ===== VPABSB/W/D (Bochs HANDLE_AVX_1OP<xmm_pabsb> et al.) =====
        PabsbVdqWdq => {
            if vl == 0 {
                V128VpabsbVdqWdq
            } else {
                V256VpabsbVdqWdq
            }
        }
        PabswVdqWdq => {
            if vl == 0 {
                V128VpabswVdqWdq
            } else {
                V256VpabswVdqWdq
            }
        }
        PabsdVdqWdq => {
            if vl == 0 {
                V128VpabsdVdqWdq
            } else {
                V256VpabsdVdqWdq
            }
        }

        // ===== VLDDQU — identical to the VL-aware vmovdqu/vmovups load
        // (Bochs ia_opcodes.def BX_IA_VLDDQU_VdqMdq → VMOVUPS_VpsWpsM) =====
        LddquVdqMdq => VlddquVdqMdq,

        // ===== VINSERTPS (vvvv is first source; VL128-only, VEX.256 #UD —
        // Bochs fetchdecode_opmap_avx.cc BxOpcodeGroup_VEX_0F3A21) =====
        InsertpsVpsWssIb => {
            if vl == 0 {
                V128VinsertpsVpsWssIb
            } else {
                IaError
            }
        }

        // ===== VPINSRB/W/D/Q (vvvv is first source; VL128-only, VEX.256 #UD —
        // Bochs fetchdecode_opmap_avx.cc BxOpcodeGroup_VEX_0F3A20/22 and the
        // 0F C4 VPINSRW group). The legacy SSE forms are 2-operand (dst==base);
        // remap so the 3-operand VEX handler that sources vvvv is dispatched.
        // W-bit split for D/Q is already resolved by the table's OS64 match
        // (VEX.W1 implies OS64 → PinsrqVdqEqIb; W0 → PinsrdVdqEdIb). =====
        PinsrwVdqEwIb => {
            if vl == 0 {
                V128VpinsrwVdqEwIb
            } else {
                IaError
            }
        }
        PinsrbVdqEbIb => {
            if vl == 0 {
                V128VpinsrbVdqEbIb
            } else {
                IaError
            }
        }
        PinsrdVdqEdIb => {
            if vl == 0 {
                V128VpinsrdVdqEdIb
            } else {
                IaError
            }
        }
        PinsrqVdqEqIb => {
            if vl == 0 {
                V128VpinsrqVdqEqIb
            } else {
                IaError
            }
        }

        // ===== VMOVNTDQA — VL-sized non-temporal load. Bochs routes both
        // widths to VMOVAPS_VpsWpsM (fetchdecode_opmap_avx.cc
        // BxOpcodeGroup_VEX_0F382A), so the VEX form must load VL bytes and
        // zero above them rather than reuse the 16-byte legacy path. =====
        MovntdqaVdqMdq => {
            if vl == 0 {
                V128VmovntdqaVdqMdq
            } else {
                V256VmovntdqaVdqMdq
            }
        }

        // ===== VPCLMULQDQ (vvvv is the first source; the VL256 form is the
        // separate VPCLMULQDQ extension — Bochs fetchdecode_opmap_avx.cc
        // BxOpcodeGroup_VEX_0F3A44) =====
        PclmulqdqVdqWdqIb => {
            if vl == 0 {
                V128VpclmulqdqVdqHdqWdqIb
            } else {
                V256VpclmulqdqVdqHdqWdqIb
            }
        }

        // ===== VPEXTRB/W/D/Q and VPCMPESTRM/ESTRI/ISTRM/ISTRI: VL128-only
        // under VEX (Bochs marks every one of these groups ATTR_VL128), so a
        // VEX.256 encoding is reserved. They take no vvvv source; that is
        // enforced in validate_reserved_vex_vvvv. =====
        PextrbEdVdqIbR => {
            if vl == 0 {
                V128VpextrbEdVdqIbR
            } else {
                IaError
            }
        }
        PextrbMbVdqIbM => {
            if vl == 0 {
                V128VpextrbMbVdqIbM
            } else {
                IaError
            }
        }
        PextrwEdVdqIbR => {
            if vl == 0 {
                V128VpextrwEdVdqIbR
            } else {
                IaError
            }
        }
        PextrwMwVdqIbM => {
            if vl == 0 {
                V128VpextrwMwVdqIbM
            } else {
                IaError
            }
        }
        PextrdEdVdqIb => {
            if vl == 0 {
                V128VpextrdEdVdqIb
            } else {
                IaError
            }
        }
        PextrqEqVdqIb => {
            if vl == 0 {
                V128VpextrqEqVdqIb
            } else {
                IaError
            }
        }
        PcmpestrmVdqWdqIb => {
            if vl == 0 {
                V128VpcmpestrmVdqWdqIb
            } else {
                IaError
            }
        }
        PcmpestriVdqWdqIb => {
            if vl == 0 {
                V128VpcmpestriVdqWdqIb
            } else {
                IaError
            }
        }
        PcmpistrmVdqWdqIb => {
            if vl == 0 {
                V128VpcmpistrmVdqWdqIb
            } else {
                IaError
            }
        }
        PcmpistriVdqWdqIb => {
            if vl == 0 {
                V128VpcmpistriVdqWdqIb
            } else {
                IaError
            }
        }

        // ===== VMPSADBW (vvvv is first source; per-128-bit-lane control —
        // Bochs avx2.cc VMPSADBW_VdqHdqWdqIbR) =====
        MpsadbwVdqWdqIb => {
            if vl == 0 {
                V128VmpsadbwVdqHdqWdqIb
            } else {
                V256VmpsadbwVdqHdqWdqIb
            }
        }

        // ===== VPHMINPOSUW (VL128-only, VEX.256 #UD — Bochs
        // fetchdecode_opmap_avx.cc BxOpcodeGroup_VEX_0F3841) =====
        PhminposuwVdqWdq => {
            if vl == 0 {
                V128VphminposuwVdqWdq
            } else {
                IaError
            }
        }

        // ===== VMOVQ (F3 0F 7E load / 66 0F D6 store; register forms must
        // zero above VL — Bochs fetchdecode_opmap_avx.cc
        // BxOpcodeGroup_VEX_0F7E / BxOpcodeGroup_VEX_0FD6, both ATTR_VL128) =====
        MovqVqWq => {
            if vl == 0 {
                VmovqVqWq
            } else {
                IaError
            }
        }
        MovqWqVq => {
            if vl == 0 {
                VmovqWqVq
            } else {
                IaError
            }
        }

        // ===== EMMS → VZEROUPPER/VZEROALL (VEX.0F 77) =====
        Emms => {
            if vl == 0 {
                Vzeroupper
            } else {
                Vzeroall
            }
        }

        // No remap — instruction either has no VEX form, is already VEX, or
        // works correctly as-is (e.g. 2-operand loads where VEX.vvvv must be 1111b)
        _ => op,
    }
}
