//! EVEX opcode maps — generated, do not edit by hand.
//!
//! Regenerate with `python scripts/gen_opmap_evex.py`.
//!
//! Transcribed from Bochs `cpu/decoder/fetchdecode_opmap_evex.cc`,
//! which is itself the table: one group per (map, opcode byte), each
//! entry a `form_opcode(attrs, opcode)`, selected by the same decmask
//! machinery `tables.rs` already implements. The master table is
//! indexed `(map - 1) * 256 + opcode`, matching `BxOpcodeTableEVEX`.
//!
//! Opcodes rusty does not implement (FP16/BF16/FP8 forms, which
//! Skylake-X does not advertise) resolve to `Opcode::IaError`, i.e. a
//! guest #UD — what Bochs produces with those ISA bits off.

use super::form_opcode;
use super::tables::OpcodeAttrs as A;
use crate::opcode::Opcode;

/// Empty slot — every encoding for this byte is undefined.
pub(crate) static EVEX_GROUP_ERR: &[u64] = &[];

static EVEX_0F10: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVmovupsVpsWps),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmovupdVpdWpd),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::MOD_REG).union(A::SSE_PREFIX_F3), Opcode::EvexVmovssVssHpsWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::MOD_REG).union(A::SSE_PREFIX_F2), Opcode::EvexVmovsdVsdHpdWsd),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::MOD_MEM).union(A::SSE_PREFIX_F3), Opcode::EvexVmovssVssWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::MOD_MEM).union(A::SSE_PREFIX_F2), Opcode::EvexVmovsdVsdWsd),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVmovupsVpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVmovupdVpdWpdKmask),
    form_opcode(A::VEX_W0.union(A::MOD_REG).union(A::SSE_PREFIX_F3), Opcode::EvexVmovssVssHpsWssKmask),
    form_opcode(A::VEX_W1.union(A::MOD_REG).union(A::SSE_PREFIX_F2), Opcode::EvexVmovsdVsdHpdWsdKmask),
    form_opcode(A::VEX_W0.union(A::MOD_MEM).union(A::SSE_PREFIX_F3), Opcode::EvexVmovssVssWssKmask),
    form_opcode(A::VEX_W1.union(A::MOD_MEM).union(A::SSE_PREFIX_F2), Opcode::EvexVmovsdVsdWsdKmask),
];

static EVEX_0F11: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVmovupsWpsVps),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmovupdWpdVpd),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::MOD_REG).union(A::SSE_PREFIX_F3), Opcode::EvexVmovssWssHpsVss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::MOD_REG).union(A::SSE_PREFIX_F2), Opcode::EvexVmovsdWsdHpdVsd),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::MOD_MEM).union(A::SSE_PREFIX_F3), Opcode::EvexVmovssWssVss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::MOD_MEM).union(A::SSE_PREFIX_F2), Opcode::EvexVmovsdWsdVsd),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVmovupsWpsVpsKmask),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVmovupdWpdVpdKmask),
    form_opcode(A::VEX_W0.union(A::MOD_REG).union(A::SSE_PREFIX_F3), Opcode::EvexVmovssWssHpsVssKmask),
    form_opcode(A::VEX_W1.union(A::MOD_REG).union(A::SSE_PREFIX_F2), Opcode::EvexVmovsdWsdHpdVsdKmask),
    form_opcode(A::VEX_W0.union(A::MOD_MEM).union(A::SSE_PREFIX_F3), Opcode::EvexVmovssWssVssKmask),
    form_opcode(A::VEX_W1.union(A::MOD_MEM).union(A::SSE_PREFIX_F2), Opcode::EvexVmovsdWsdVsdKmask),
];

static EVEX_0F12: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::VL128).union(A::SSE_NO_PREFIX).union(A::MOD_MEM), Opcode::EvexVmovlpsVpsHpsMq),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::VL128).union(A::SSE_NO_PREFIX).union(A::MOD_REG), Opcode::EvexVmovhlpsVpsHpsWps),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::VL128).union(A::SSE_PREFIX_66).union(A::MOD_MEM), Opcode::EvexVmovlpdVpdHpdMq),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmovsldupVpsWps),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVmovddupVpdWpd),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVmovsldupVpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVmovddupVpdWpdKmask),
];

static EVEX_0F13: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::VL128).union(A::MOD_MEM).union(A::SSE_NO_PREFIX), Opcode::EvexVmovlpsMqVps),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::VL128).union(A::MOD_MEM).union(A::SSE_PREFIX_66), Opcode::EvexVmovlpdMqVsd),
];

static EVEX_0F14: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVunpcklpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVunpcklpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVunpcklpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVunpcklpdVpdHpdWpdKmask),
];

static EVEX_0F15: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVunpckhpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVunpckhpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVunpckhpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVunpckhpdVpdHpdWpdKmask),
];

static EVEX_0F16: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::VL128).union(A::SSE_NO_PREFIX).union(A::MOD_MEM), Opcode::EvexVmovhpsVpsHpsMq),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::VL128).union(A::SSE_NO_PREFIX).union(A::MOD_REG), Opcode::EvexVmovlhpsVpsHpsWps),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::VL128).union(A::SSE_PREFIX_66).union(A::MOD_MEM), Opcode::EvexVmovhpdVpdHpdMq),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmovshdupVpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVmovshdupVpsWpsKmask),
];

static EVEX_0F17: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::MOD_MEM).union(A::VL128).union(A::SSE_NO_PREFIX), Opcode::EvexVmovhpsMqVps),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::MOD_MEM).union(A::VL128).union(A::SSE_PREFIX_66), Opcode::EvexVmovhpdMqVsd),
];

static EVEX_0F28: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVmovapsVpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVmovapsVpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmovapdVpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVmovapdVpdWpdKmask),
];

static EVEX_0F29: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVmovapsWpsVps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVmovapsWpsVpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmovapdWpdVpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVmovapdWpdVpdKmask),
];

static EVEX_0F2A: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtsi2ssVssEd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvtsi2ssVssEq),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtsi2sdVsdEd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2).union(A::IS64), Opcode::EvexVcvtsi2sdVsdEq),
];

static EVEX_0F2B: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::MOD_MEM).union(A::SSE_NO_PREFIX), Opcode::EvexVmovntpsMpsVps),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::MOD_MEM).union(A::SSE_PREFIX_66), Opcode::EvexVmovntpdMpdVpd),
];

static EVEX_0F2C: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvttss2siGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvttss2siGqWss),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvttsd2siGdWsd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2).union(A::IS64), Opcode::EvexVcvttsd2siGqWsd),
];

static EVEX_0F2D: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtss2siGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvtss2siGqWss),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtsd2siGdWsd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2).union(A::IS64), Opcode::EvexVcvtsd2siGqWsd),
];

static EVEX_0F2E: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVucomissVssWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVucomisdVsdWsd),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVucomxsdVsdWsd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVucomxssVssWss),
];

static EVEX_0F2F: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcomissVssWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcomisdVsdWsd),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcomxsdVsdWsd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcomxssVssWss),
];

static EVEX_0F3800: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpshufbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpshufbVdqHdqWdqKmask),
];

static EVEX_0F3804: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmaddubswVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmaddubswVdqHdqWdqKmask),
];

static EVEX_0F380B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmulhrswVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmulhrswVdqHdqWdqKmask),
];

static EVEX_0F380C: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpermilpsVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpermilpsVpsHpsWpsKmask),
];

static EVEX_0F380D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpermilpdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpermilpdVpdHpdWpdKmask),
];

static EVEX_0F3810: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsrlvwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsrlvwVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovuswbWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovuswbWdqVdqKmask),
];

static EVEX_0F3811: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsravwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsravwVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovusdbWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovusdbWdqVdqKmask),
];

static EVEX_0F3812: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsllvwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsllvwVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovusqbWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovusqbWdqVdqKmask),
];

static EVEX_0F3813: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVcvtph2psVpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVcvtph2psVpsWpsKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovusdwWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovusdwWdqVdqKmask),
];

static EVEX_0F3814: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVprorvdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVprorvdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVprorvqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVprorvqVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovusqwWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovusqwWdqVdqKmask),
];

static EVEX_0F3815: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVprolvdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVprolvdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVprolvqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVprolvqVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovusqdWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovusqdWdqVdqKmask),
];

static EVEX_0F3816: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::VL256_512), Opcode::EvexVpermpsVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::VL256_512), Opcode::EvexVpermpdVpdHpdWpdKmask),
];

static EVEX_0F3818: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVbroadcastssVpsWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVbroadcastssVpsWssKmask),
];

static EVEX_0F3819: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVbroadcastf32x2VpsWq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVbroadcastf32x2VpsWqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVbroadcastsdVpdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVbroadcastsdVpdWsdKmask),
];

static EVEX_0F381A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::VL256_512).union(A::MASK_K0), Opcode::EvexVbroadcastf32x4VpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::VL256_512), Opcode::EvexVbroadcastf32x4VpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::VL256_512).union(A::MASK_K0), Opcode::EvexVbroadcastf64x2VpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::VL256_512), Opcode::EvexVbroadcastf64x2VpdWpdKmask),
];

static EVEX_0F381B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::VL512).union(A::MASK_K0), Opcode::EvexVbroadcastf32x8VpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::VL512), Opcode::EvexVbroadcastf32x8VpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::VL512).union(A::MASK_K0), Opcode::EvexVbroadcastf64x4VpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::VL512), Opcode::EvexVbroadcastf64x4VpdWpdKmask),
];

static EVEX_0F381C: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpabsbVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpabsbVdqWdqKmask),
];

static EVEX_0F381D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpabswVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpabswVdqWdqKmask),
];

static EVEX_0F381E: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpabsdVdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpabsdVdqWdqKmask),
];

static EVEX_0F381F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpabsqVdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpabsqVdqWdqKmask),
];

static EVEX_0F3820: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovsxbwVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovsxbwVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovswbWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovswbWdqVdqKmask),
];

static EVEX_0F3821: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovsxbdVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovsxbdVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovsdbWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovsdbWdqVdqKmask),
];

static EVEX_0F3822: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovsxbqVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovsxbqVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovsqbWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovsqbWdqVdqKmask),
];

static EVEX_0F3823: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovsxwdVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovsxwdVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovsdwWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovsdwWdqVdqKmask),
];

static EVEX_0F3824: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovsxwqVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovsxwqVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovsqwWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovsqwWdqVdqKmask),
];

static EVEX_0F3825: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovsxdqVdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpmovsxdqVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovsqdWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovsqdWdqVdqKmask),
];

static EVEX_0F3826: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVptestmbKgqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVptestmwKgdHdqWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVptestnmbKgqHdqWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W1), Opcode::EvexVptestnmwKgdHdqWdq),
];

static EVEX_0F3827: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVptestmdKgwHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVptestmqKgbHdqWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVptestnmdKgwHdqWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W1), Opcode::EvexVptestnmqKgbHdqWdq),
];

static EVEX_0F3828: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpmuldqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpmuldqVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::MOD_REG).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovm2bVdqKeq),
    form_opcode(A::SSE_PREFIX_F3.union(A::MOD_REG).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpmovm2wVdqKed),
];

static EVEX_0F3829: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpcmpeqqKgbHdqWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::MOD_REG).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovb2mKgqWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::MOD_REG).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpmovw2mKgdWdq),
];

static EVEX_0F382A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::MASK_K0), Opcode::EvexVmovntdqaVdqMdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W1).union(A::MOD_REG).union(A::MASK_K0), Opcode::EvexVpbroadcastmb2qVdqKeb),
];

static EVEX_0F382B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpackusdwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpackusdwVdqHdqWdqKmask),
];

static EVEX_0F382C: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVscalefpsVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVscalefpsVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVscalefpdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVscalefpdVpdHpdWpdKmask),
];

static EVEX_0F382D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVscalefssVssHpsWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVscalefssVssHpsWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVscalefsdVsdHpdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVscalefsdVsdHpdWsdKmask),
];

static EVEX_0F3830: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovzxbwVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovzxbwVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovwbWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovwbWdqVdqKmask),
];

static EVEX_0F3831: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovzxbdVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovzxbdVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovdbWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovdbWdqVdqKmask),
];

static EVEX_0F3832: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovzxbqVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovzxbqVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovqbWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovqbWdqVdqKmask),
];

static EVEX_0F3833: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovzxwdVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovzxwdVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovdwWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovdwWdqVdqKmask),
];

static EVEX_0F3834: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmovzxwqVdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmovzxwqVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovqwWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovqwWdqVdqKmask),
];

static EVEX_0F3835: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovzxdqVdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpmovzxdqVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmovqdWdqVdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpmovqdWdqVdqKmask),
];

static EVEX_0F3836: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::VL256_512), Opcode::EvexVpermdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::VL256_512), Opcode::EvexVpermqVdqHdqWdqKmask),
];

static EVEX_0F3837: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpcmpgtqKgbHdqWdq),
];

static EVEX_0F3838: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpminsbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpminsbVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::MOD_REG).union(A::MASK_K0).union(A::VEX_W0), Opcode::EvexVpmovm2dVdqKew),
    form_opcode(A::SSE_PREFIX_F3.union(A::MOD_REG).union(A::MASK_K0).union(A::VEX_W1), Opcode::EvexVpmovm2qVdqKeb),
];

static EVEX_0F3839: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpminsdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpminsdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpminsqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpminsqVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::MOD_REG).union(A::MASK_K0).union(A::VEX_W0), Opcode::EvexVpmovd2mKgwWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::MOD_REG).union(A::MASK_K0).union(A::VEX_W1), Opcode::EvexVpmovq2mKgbWdq),
];

static EVEX_0F383A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpminuwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpminuwVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::MOD_REG).union(A::MASK_K0).union(A::VEX_W0), Opcode::EvexVpbroadcastmw2dVdqKew),
];

static EVEX_0F383B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpminudVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpminudVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpminuqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpminuqVdqHdqWdqKmask),
];

static EVEX_0F383C: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmaxsbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmaxsbVdqHdqWdqKmask),
];

static EVEX_0F383D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmaxsdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpmaxsdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpmaxsqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpmaxsqVdqHdqWdqKmask),
];

static EVEX_0F383E: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmaxuwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmaxuwVdqHdqWdqKmask),
];

static EVEX_0F383F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmaxudVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpmaxudVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpmaxuqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpmaxuqVdqHdqWdqKmask),
];

static EVEX_0F3840: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpmulldVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpmulldVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpmullqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpmullqVdqHdqWdqKmask),
];

static EVEX_0F3842: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVgetexppsVpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVgetexppsVpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVgetexppdVpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVgetexppdVpdWpdKmask),
];

static EVEX_0F3843: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVgetexpssVssHpsWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVgetexpssVssHpsWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVgetexpsdVsdHpdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVgetexpsdVsdHpdWsdKmask),
];

static EVEX_0F3844: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVplzcntdVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVplzcntqVdqWdqKmask),
];

static EVEX_0F3845: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpsrlvdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpsrlvdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsrlvqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsrlvqVdqHdqWdqKmask),
];

static EVEX_0F3846: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpsravdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpsravdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsravqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsravqVdqHdqWdqKmask),
];

static EVEX_0F3847: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpsllvdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpsllvdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsllvqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsllvqVdqHdqWdqKmask),
];

static EVEX_0F384A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTilemovrowVdqTrmBd),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowd2psVpsTrmBd),
];

static EVEX_0F384C: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVrcp14psVpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVrcp14pdVpdWpdKmask),
];

static EVEX_0F384D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVrcp14ssVssHpsWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVrcp14sdVsdHpdWsdKmask),
];

static EVEX_0F384E: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVrsqrt14psVpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVrsqrt14pdVpdWpdKmask),
];

static EVEX_0F384F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVrsqrt14ssVssHpsWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVrsqrt14sdVsdHpdWsdKmask),
];

static EVEX_0F3850: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpbuudVdqHdqWdq),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVpdpbuudVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpbusdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpdpbusdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpbsudVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpdpbsudVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpbssdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVpdpbssdVdqHdqWdqKmask),
];

static EVEX_0F3851: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpbuudsVdqHdqWdq),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVpdpbuudsVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpbusdsVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpdpbusdsVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpbsudsVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVpdpbsudsVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpbssdsVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVpdpbssdsVdqHdqWdqKmask),
];

static EVEX_0F3852: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVdpphpsVpsHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpwssdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpdpwssdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVdpbf16psVpsHdqWdqKmask),
];

static EVEX_0F3853: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpdpwssdsVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpdpwssdsVdqHdqWdqKmask),
];

static EVEX_0F3854: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpopcntbVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpopcntwVdqWdqKmask),
];

static EVEX_0F3855: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpopcntdVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpopcntqVdqWdqKmask),
];

static EVEX_0F3858: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpbroadcastdVdqWd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpbroadcastdVdqWdKmask),
];

static EVEX_0F3859: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVbroadcasti32x2VdqWq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVbroadcasti32x2VdqWqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpbroadcastqVdqWq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpbroadcastqVdqWqKmask),
];

static EVEX_0F385A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::MOD_MEM).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVbroadcasti32x4VdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::MOD_MEM).union(A::VEX_W0), Opcode::EvexVbroadcasti32x4VdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::MOD_MEM).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVbroadcasti64x2VdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::MOD_MEM).union(A::VEX_W1), Opcode::EvexVbroadcasti64x2VdqWdqKmask),
];

static EVEX_0F385B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::MOD_MEM).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVbroadcasti32x8VdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::MOD_MEM).union(A::VEX_W0), Opcode::EvexVbroadcasti32x8VdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::MOD_MEM).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVbroadcasti64x4VdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::MOD_MEM).union(A::VEX_W1), Opcode::EvexVbroadcasti64x4VdqWdqKmask),
];

static EVEX_0F3862: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpexpandbVdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpexpandbVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpexpandwVdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpexpandwVdqWdqKmask),
];

static EVEX_0F3863: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpcompressbWdqVdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpcompressbWdqVdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpcompresswWdqVdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpcompresswWdqVdqKmask),
];

static EVEX_0F3864: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpblendmdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpblendmqVdqHdqWdq),
];

static EVEX_0F3865: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVblendmpsVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVblendmpdVpdHpdWpd),
];

static EVEX_0F3866: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpblendmbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpblendmwVdqHdqWdq),
];

static EVEX_0F3867: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::IaError),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVcvt2ps2phxVphHpsWpsKmask),
];

static EVEX_0F3868: &[u64] = &[
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVp2intersectdKgqHdqWdq),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVp2intersectqKgqHdqWdq),
];

static EVEX_0F386D: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowps2phhVphTrmBd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowps2phlVphTrmBd),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowps2bf16lVphTrmBd),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowps2bf16hVphTrmBd),
];

static EVEX_0F3870: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpshldvwVdqHdqWdqKmask),
];

static EVEX_0F3871: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpshldvdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpshldvqVdqHdqWdqKmask),
];

static EVEX_0F3872: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpshrdvwVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVcvtneps2bf16VphWpsKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVcvtne2ps2bf16VphHpsWpsKmask),
];

static EVEX_0F3873: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpshrdvdVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpshrdvqVdqHdqWdqKmask),
];

static EVEX_0F3874: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVcvtbiasph2bf8Vf8hdqWphKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVcvtph2bf8Vf8hdqWphKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVcvt2ph2bf8Vf8hdqWphKmask),
];

static EVEX_0F3875: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpermi2bVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpermi2wVdqHdqWdqKmask),
];

static EVEX_0F3876: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpermi2dVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpermi2qVdqHdqWdqKmask),
];

static EVEX_0F3877: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpermi2psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpermi2pdVpdHpdWpdKmask),
];

static EVEX_0F3878: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpbroadcastbVdqWb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpbroadcastbVdqWbKmask),
];

static EVEX_0F3879: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpbroadcastwVdqWw),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpbroadcastwVdqWwKmask),
];

static EVEX_0F387A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_REG).union(A::MASK_K0), Opcode::EvexVpbroadcastbVdqEb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_REG), Opcode::EvexVpbroadcastbVdqEbKmask),
];

static EVEX_0F387B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_REG).union(A::MASK_K0), Opcode::EvexVpbroadcastwVdqEw),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_REG), Opcode::EvexVpbroadcastwVdqEwKmask),
];

static EVEX_0F387C: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_REG).union(A::MASK_K0), Opcode::EvexVpbroadcastdVdqEd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_REG), Opcode::EvexVpbroadcastdVdqEdKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_REG).union(A::MASK_K0).union(A::IS64), Opcode::EvexVpbroadcastqVdqEq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_REG).union(A::IS64), Opcode::EvexVpbroadcastqVdqEqKmask),
];

static EVEX_0F387D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpermt2bVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpermt2wVdqHdqWdqKmask),
];

static EVEX_0F387E: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpermt2dVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpermt2qVdqHdqWdqKmask),
];

static EVEX_0F387F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpermt2psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpermt2pdVpdHpdWpdKmask),
];

static EVEX_0F3883: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpmultishiftqbVdqHdqWdqKmask),
];

static EVEX_0F3888: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVexpandpsVpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVexpandpsVpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVexpandpdVpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVexpandpdVpdWpdKmask),
];

static EVEX_0F3889: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpexpanddVdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpexpanddVdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpexpandqVdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpexpandqVdqWdqKmask),
];

static EVEX_0F388A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVcompresspsWpsVps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVcompresspsWpsVpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVcompresspdWpdVpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVcompresspdWpdVpdKmask),
];

static EVEX_0F388B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpcompressdWdqVdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpcompressdWdqVdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpcompressqWdqVdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpcompressqWdqVdqKmask),
];

static EVEX_0F388D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpermbVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpermwVdqHdqWdqKmask),
];

static EVEX_0F388F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpshufbitqmbKgqHdqWdqKmask),
];

static EVEX_0F3890: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVgatherddVdqVsib),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVgatherdqVdqVsib),
];

static EVEX_0F3891: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVgatherqdVdqVsib),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVgatherqqVdqVsib),
];

static EVEX_0F3892: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVgatherdpsVpsVsib),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVgatherdpdVpdVsib),
];

static EVEX_0F3893: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVgatherqpsVpsVsib),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVgatherqpdVpdVsib),
];

static EVEX_0F3896: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmaddsub132psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmaddsub132psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmaddsub132pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmaddsub132pdVpdHpdWpdKmask),
];

static EVEX_0F3897: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsubadd132psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsubadd132psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmsubadd132pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmsubadd132pdVpdHpdWpdKmask),
];

static EVEX_0F3898: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd132psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd132psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmadd132pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmadd132pdVpdHpdWpdKmask),
];

static EVEX_0F3899: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd132ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd132ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmadd132sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmadd132sdVpdHsdWsdKmask),
];

static EVEX_0F389A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub132psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub132psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmsub132pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmsub132pdVpdHpdWpdKmask),
];

static EVEX_0F389B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub132ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub132ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmsub132sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmsub132sdVpdHsdWsdKmask),
];

static EVEX_0F389C: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd132psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd132psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmadd132pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmadd132pdVpdHpdWpdKmask),
];

static EVEX_0F389D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd132ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd132ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmadd132sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmadd132sdVpdHsdWsdKmask),
];

static EVEX_0F389E: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub132psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub132psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmsub132pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmsub132pdVpdHpdWpdKmask),
];

static EVEX_0F389F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub132ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub132ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmsub132sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmsub132sdVpdHsdWsdKmask),
];

static EVEX_0F38A0: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVscatterddVsibVdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVscatterdqVsibVdq),
];

static EVEX_0F38A1: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVscatterqdVsibVdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVscatterqqVsibVdq),
];

static EVEX_0F38A2: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVscatterdpsVsibVps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVscatterdpdVsibVpd),
];

static EVEX_0F38A3: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVscatterqpsVsibVps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MOD_MEM).union(A::MASK_REQUIRED), Opcode::EvexVscatterqpdVsibVpd),
];

static EVEX_0F38A6: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmaddsub213psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmaddsub213psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmaddsub213pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmaddsub213pdVpdHpdWpdKmask),
];

static EVEX_0F38A7: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsubadd213psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsubadd213psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmsubadd213pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmsubadd213pdVpdHpdWpdKmask),
];

static EVEX_0F38A8: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd213psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd213psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmadd213pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmadd213pdVpdHpdWpdKmask),
];

static EVEX_0F38A9: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd213ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd213ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmadd213sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmadd213sdVpdHsdWsdKmask),
];

static EVEX_0F38AA: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub213psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub213psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmsub213pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmsub213pdVpdHpdWpdKmask),
];

static EVEX_0F38AB: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub213ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub213ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmsub213sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmsub213sdVpdHsdWsdKmask),
];

static EVEX_0F38AC: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd213psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd213psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmadd213pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmadd213pdVpdHpdWpdKmask),
];

static EVEX_0F38AD: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd213ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd213ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmadd213sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmadd213sdVpdHsdWsdKmask),
];

static EVEX_0F38AE: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub213psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub213psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmsub213pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmsub213pdVpdHpdWpdKmask),
];

static EVEX_0F38AF: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub213ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub213ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmsub213sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmsub213sdVpdHsdWsdKmask),
];

static EVEX_0F38B4: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpmadd52luqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpmadd52luqVdqHdqWdqKmask),
];

static EVEX_0F38B5: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpmadd52huqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpmadd52huqVdqHdqWdqKmask),
];

static EVEX_0F38B6: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmaddsub231psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmaddsub231psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmaddsub231pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmaddsub231pdVpdHpdWpdKmask),
];

static EVEX_0F38B7: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsubadd231psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsubadd231psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmsubadd231pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmsubadd231pdVpdHpdWpdKmask),
];

static EVEX_0F38B8: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd231psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd231psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmadd231pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmadd231pdVpdHpdWpdKmask),
];

static EVEX_0F38B9: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd231ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd231ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmadd231sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmadd231sdVpdHsdWsdKmask),
];

static EVEX_0F38BA: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub231psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub231psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmsub231pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmsub231pdVpdHpdWpdKmask),
];

static EVEX_0F38BB: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub231ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub231ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfmsub231sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfmsub231sdVpdHsdWsdKmask),
];

static EVEX_0F38BC: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd231psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd231psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmadd231pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmadd231pdVpdHpdWpdKmask),
];

static EVEX_0F38BD: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd231ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd231ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmadd231sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmadd231sdVpdHsdWsdKmask),
];

static EVEX_0F38BE: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub231psVpsHpsWps),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub231psVpsHpsWpsKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmsub231pdVpdHpdWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmsub231pdVpdHpdWpdKmask),
];

static EVEX_0F38BF: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub231ssVpsHssWss),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub231ssVpsHssWssKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfnmsub231sdVpdHsdWsd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfnmsub231sdVpdHsdWsdKmask),
];

static EVEX_0F38C4: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVpconflictdVdqWdqKmask),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVpconflictqVdqWdqKmask),
];

static EVEX_0F38CF: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVgf2p8mulbVdqHdqWdqKmask),
];

static EVEX_0F38DC: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVaesencVdqHdqWdq),
];

static EVEX_0F38DD: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVaesenclastVdqHdqWdq),
];

static EVEX_0F38DE: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVaesdecVdqHdqWdq),
];

static EVEX_0F38DF: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVaesdeclastVdqHdqWdq),
];

static EVEX_0F3A00: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::VL256_512), Opcode::EvexVpermqVdqWdqIbKmask),
];

static EVEX_0F3A01: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::VL256_512), Opcode::EvexVpermpdVpdWpdIbKmask),
];

static EVEX_0F3A03: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexValigndVdqHdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexValignqVdqHdqWdqIbKmask),
];

static EVEX_0F3A04: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpermilpsVpsWpsIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpermilpsVpsWpsIbKmask),
];

static EVEX_0F3A05: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpermilpdVpdWpdIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpermilpdVpdWpdIbKmask),
];

static EVEX_0F3A07: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTilemovrowVdqTrmIb),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowd2psVpsTrmIb),
];

static EVEX_0F3A08: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVrndscalephVphWphIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVrndscalepsVpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVrndscalebf16VphWphIbKmask),
];

static EVEX_0F3A09: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVrndscalepdVpdWpdIbKmask),
];

static EVEX_0F3A0A: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVrndscaleshVshHphWshIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVrndscalessVssHpsWssIbKmask),
];

static EVEX_0F3A0B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVrndscalesdVsdHpdWsdIbKmask),
];

static EVEX_0F3A0F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpalignrVdqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpalignrVdqHdqWdqIbKmask),
];

static EVEX_0F3A14: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::MASK_K0).union(A::MOD_REG), Opcode::EvexVpextrbEdVdqIbR),
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::MASK_K0).union(A::MOD_MEM), Opcode::EvexVpextrbMbVdqIbM),
];

static EVEX_0F3A15: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::MASK_K0).union(A::MOD_REG), Opcode::EvexVpextrwEdVdqIbR),
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::MASK_K0).union(A::MOD_MEM), Opcode::EvexVpextrwMwVdqIbM),
];

static EVEX_0F3A16: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpextrdEdVdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::VEX_W1).union(A::MASK_K0).union(A::IS64), Opcode::EvexVpextrqEqVdqIb),
];

static EVEX_0F3A17: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::MASK_K0), Opcode::EvexVextractpsEdVpsIb),
];

static EVEX_0F3A18: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVinsertf32x4VpsHpsWpsIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0), Opcode::EvexVinsertf32x4VpsHpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVinsertf64x2VpdHpdWpdIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1), Opcode::EvexVinsertf64x2VpdHpdWpdIbKmask),
];

static EVEX_0F3A19: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVextractf32x4WpsVpsIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0), Opcode::EvexVextractf32x4WpsVpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVextractf64x2WpdVpdIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1), Opcode::EvexVextractf64x2WpdVpdIbKmask),
];

static EVEX_0F3A1A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVinsertf32x8VpsHpsWpsIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W0), Opcode::EvexVinsertf32x8VpsHpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVinsertf64x4VpdHpdWpdIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W1), Opcode::EvexVinsertf64x4VpdHpdWpdIbKmask),
];

static EVEX_0F3A1B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVextractf32x8WpsVpsIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W0), Opcode::EvexVextractf32x8WpsVpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVextractf64x4WpdVpdIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W1), Opcode::EvexVextractf64x4WpdVpdIbKmask),
];

static EVEX_0F3A1D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVcvtps2phWpsVpsIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVcvtps2phWpsVpsIbKmask),
];

static EVEX_0F3A1E: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpcmpudKgwHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpcmpuqKgbHdqWdqIb),
];

static EVEX_0F3A1F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpcmpdKgwHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpcmpqKgbHdqWdqIb),
];

static EVEX_0F3A20: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::MASK_K0), Opcode::EvexVpinsrbVdqEbIb),
];

static EVEX_0F3A21: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVinsertpsVpsWssIb),
];

static EVEX_0F3A22: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpinsrdVdqEdIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::VEX_W1).union(A::MASK_K0).union(A::IS64), Opcode::EvexVpinsrqVdqEqIb),
];

static EVEX_0F3A23: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0), Opcode::EvexVshuff32x4VpsHpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1), Opcode::EvexVshuff64x2VpdHpdWpdIbKmask),
];

static EVEX_0F3A25: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpternlogdVdqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpternlogdVdqHdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpternlogqVdqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpternlogqVdqHdqWdqIbKmask),
];

static EVEX_0F3A26: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVgetmantphVphWphIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVgetmantpsVpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVgetmantpdVpdWpdIbKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVgetmantpbf16VphWphIbKmask),
];

static EVEX_0F3A27: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVgetmantshVshHphWshIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVgetmantssVssHpsWssIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVgetmantsdVsdHpdWsdIbKmask),
];

static EVEX_0F3A38: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVinserti32x4VdqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0), Opcode::EvexVinserti32x4VdqHdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVinserti64x2VdqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1), Opcode::EvexVinserti64x2VdqHdqWdqIbKmask),
];

static EVEX_0F3A39: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVextracti32x4WdqVdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0), Opcode::EvexVextracti32x4WdqVdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVextracti64x2WdqVdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1), Opcode::EvexVextracti64x2WdqVdqIbKmask),
];

static EVEX_0F3A3A: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVinserti32x8VdqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W0), Opcode::EvexVinserti32x8VdqHdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVinserti64x4VdqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W1), Opcode::EvexVinserti64x4VdqHdqWdqIbKmask),
];

static EVEX_0F3A3B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVextracti32x8WdqVdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W0), Opcode::EvexVextracti32x8WdqVdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVextracti64x4WdqVdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VL512).union(A::VEX_W1), Opcode::EvexVextracti64x4WdqVdqIbKmask),
];

static EVEX_0F3A3E: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpcmpubKgqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpcmpuwKgdHdqWdqIb),
];

static EVEX_0F3A3F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpcmpbKgqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpcmpwKgdHdqWdqIb),
];

static EVEX_0F3A42: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVdbpsadbwVdqHdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVmpsadbwVdqHdqWdqIb),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVmpsadbwVdqHdqWdqIbKmask),
];

static EVEX_0F3A43: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W0), Opcode::EvexVshufi32x4VdqHdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VL256_512).union(A::VEX_W1), Opcode::EvexVshufi64x2VdqHdqWdqIbKmask),
];

static EVEX_0F3A44: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpclmulqdqVdqHdqWdqIb),
];

static EVEX_0F3A50: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVrangepsVpsHpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVrangepdVpdHpdWpdIbKmask),
];

static EVEX_0F3A51: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVrangessVssHpsWssIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVrangesdVsdHpdWsdIbKmask),
];

static EVEX_0F3A52: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVminmaxphVphHphWphIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVminmaxpsVpsHpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVminmaxpdVpdHpdWpdIbKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVminmaxbf16VphHphWphIbKmask),
];

static EVEX_0F3A53: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVminmaxshVshHphWshIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVminmaxssVssHpsWssIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVminmaxsdVsdHpdWsdIbKmask),
];

static EVEX_0F3A54: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfixupimmpsVpsHpsWpsIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfixupimmpsVpsHpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVfixupimmpdVpdHpdWpdIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfixupimmpdVpdHpdWpdIbKmask),
];

static EVEX_0F3A55: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfixupimmssVssHssWssIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfixupimmsdVsdHsdWsdIbKmask),
];

static EVEX_0F3A56: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVreducephVphWphIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVreducepsVpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVreducepdVpdWpdIbKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVreducebf16VphWphIbKmask),
];

static EVEX_0F3A57: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVreduceshVshHphWshIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVreducessVssHpsWssIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVreducesdVsdHpdWsdIbKmask),
];

static EVEX_0F3A66: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfpclassphKgdWphIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfpclasspsKgwWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfpclasspdKgbWpdIbKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVfpclasspbf16KgdWphIbKmask),
];

static EVEX_0F3A67: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfpclassshKgbWshIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfpclassssKgbWssIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVfpclasssdKgbWsdIbKmask),
];

static EVEX_0F3A70: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpshldwVdqHdqWdqIbKmask),
];

static EVEX_0F3A71: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpshlddVdqHdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpshldqVdqHdqWdqIbKmask),
];

static EVEX_0F3A72: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpshrdwVdqHdqWdqIbKmask),
];

static EVEX_0F3A73: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpshrddVdqHdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpshrdqVdqHdqWdqIbKmask),
];

static EVEX_0F3A77: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowps2phhVphTrmIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowps2phlVphTrmIb),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowps2bf16lVphTrmIb),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0).union(A::VL512).union(A::MASK_K0).union(A::MOD_REG).union(A::IS64), Opcode::EvexTcvtrowps2bf16hVphTrmIb),
];

static EVEX_0F3AC2: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcmpphKgdHphWphIb),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVcmpshKgbHshWshIb),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVcmppbf16KgdHphWphIb),
];

static EVEX_0F3ACE: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVgf2p8affineqbVdqHdqWdqIbKmask),
];

static EVEX_0F3ACF: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVgf2p8affineinvqbVdqHdqWdqIbKmask),
];

static EVEX_0F51: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVsqrtpsVpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVsqrtpsVpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVsqrtpdVpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVsqrtpdVpdWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVsqrtssVssHpsWss),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVsqrtssVssHpsWssKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVsqrtsdVsdHpdWsd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVsqrtsdVsdHpdWsdKmask),
];

static EVEX_0F54: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVandpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVandpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVandpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVandpdVpdHpdWpdKmask),
];

static EVEX_0F55: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVandnpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVandnpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVandnpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVandnpdVpdHpdWpdKmask),
];

static EVEX_0F56: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVorpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVorpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVorpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVorpdVpdHpdWpdKmask),
];

static EVEX_0F57: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVxorpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVxorpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVxorpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVxorpdVpdHpdWpdKmask),
];

static EVEX_0F58: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVaddpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVaddpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVaddpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVaddpdVpdHpdWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVaddssVssHpsWss),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVaddssVssHpsWssKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVaddsdVsdHpdWsd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVaddsdVsdHpdWsdKmask),
];

static EVEX_0F59: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVmulpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVmulpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmulpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVmulpdVpdHpdWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmulssVssHpsWss),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVmulssVssHpsWssKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVmulsdVsdHpdWsd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVmulsdVsdHpdWsdKmask),
];

static EVEX_0F5A: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtps2pdVpdWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtps2pdVpdWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtpd2psVpsWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVcvtpd2psVpsWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtss2sdVsdWss),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVcvtss2sdVsdWssKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtsd2ssVssWsd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVcvtsd2ssVssWsdKmask),
];

static EVEX_0F5B: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtdq2psVpsWdq),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtdq2psVpsWdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtqq2psVpsWdq),
    form_opcode(A::VEX_W1.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtqq2psVpsWdqKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2dqVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2dqVdqWpsKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvttps2dqVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVcvttps2dqVdqWpsKmask),
];

static EVEX_0F5C: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVsubpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVsubpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVsubpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVsubpdVpdHpdWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVsubssVssHpsWss),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVsubssVssHpsWssKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVsubsdVsdHpdWsd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVsubsdVsdHpdWsdKmask),
];

static EVEX_0F5D: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVminpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVminpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVminpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVminpdVpdHpdWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVminssVssHpsWss),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVminssVssHpsWssKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVminsdVsdHpdWsd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVminsdVsdHpdWsdKmask),
];

static EVEX_0F5E: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVdivpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVdivpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVdivpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVdivpdVpdHpdWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVdivssVssHpsWss),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVdivssVssHpsWssKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVdivsdVsdHpdWsd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVdivsdVsdHpdWsdKmask),
];

static EVEX_0F5F: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVmaxpsVpsHpsWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVmaxpsVpsHpsWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmaxpdVpdHpdWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVmaxpdVpdHpdWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmaxssVssHpsWss),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVmaxssVssHpsWssKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVmaxsdVsdHpdWsd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVmaxsdVsdHpdWsdKmask),
];

static EVEX_0F60: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpunpcklbwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpunpcklbwVdqHdqWdqKmask),
];

static EVEX_0F61: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpunpcklwdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpunpcklwdVdqHdqWdqKmask),
];

static EVEX_0F62: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66).union(A::MASK_K0), Opcode::EvexVpunpckldqVdqHdqWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVpunpckldqVdqHdqWdqKmask),
];

static EVEX_0F63: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66).union(A::MASK_K0), Opcode::EvexVpacksswbVdqHdqWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVpacksswbVdqHdqWdqKmask),
];

static EVEX_0F64: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpcmpgtbKgqHdqWdq),
];

static EVEX_0F65: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpcmpgtwKgdHdqWdq),
];

static EVEX_0F66: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpcmpgtdKgwHdqWdq),
];

static EVEX_0F67: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpackuswbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpackuswbVdqHdqWdqKmask),
];

static EVEX_0F68: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpunpckhbwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpunpckhbwVdqHdqWdqKmask),
];

static EVEX_0F69: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpunpckhwdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpunpckhwdVdqHdqWdqKmask),
];

static EVEX_0F6A: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66).union(A::MASK_K0), Opcode::EvexVpunpckhdqVdqHdqWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVpunpckhdqVdqHdqWdqKmask),
];

static EVEX_0F6B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpackssdwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpackssdwVdqHdqWdqKmask),
];

static EVEX_0F6C: &[u64] = &[
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66).union(A::MASK_K0), Opcode::EvexVpunpcklqdqVdqHdqWdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVpunpcklqdqVdqHdqWdqKmask),
];

static EVEX_0F6D: &[u64] = &[
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66).union(A::MASK_K0), Opcode::EvexVpunpckhqdqVdqHdqWdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVpunpckhqdqVdqHdqWdqKmask),
];

static EVEX_0F6E: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66).union(A::VL128), Opcode::EvexVmovdVdqEd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66).union(A::VL128).union(A::IS64), Opcode::EvexVmovqVdqEq),
];

static EVEX_0F6F: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmovdqa32VdqWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVmovdqa32VdqWdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmovdqa64VdqWdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVmovdqa64VdqWdqKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmovdqu32VdqWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVmovdqu32VdqWdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmovdqu64VdqWdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F3), Opcode::EvexVmovdqu64VdqWdqKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVmovdqu8VdqWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVmovdqu8VdqWdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVmovdqu16VdqWdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVmovdqu16VdqWdqKmask),
];

static EVEX_0F70: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpshufdVdqWdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpshufdVdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::MASK_K0), Opcode::EvexVpshufhwVdqWdqIb),
    form_opcode(A::SSE_PREFIX_F3, Opcode::EvexVpshufhwVdqWdqIbKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::MASK_K0), Opcode::EvexVpshuflwVdqWdqIb),
    form_opcode(A::SSE_PREFIX_F2, Opcode::EvexVpshuflwVdqWdqIbKmask),
];

static EVEX_0F71: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::NNN2).union(A::MASK_K0), Opcode::EvexVpsrlwUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::NNN2), Opcode::EvexVpsrlwUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::NNN4).union(A::MASK_K0), Opcode::EvexVpsrawUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::NNN4), Opcode::EvexVpsrawUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::NNN6).union(A::MASK_K0), Opcode::EvexVpsllwUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::NNN6), Opcode::EvexVpsllwUdqIbKmask),
];

static EVEX_0F72: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN0).union(A::MASK_K0), Opcode::EvexVprordUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN0), Opcode::EvexVprordUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN0).union(A::MASK_K0), Opcode::EvexVprorqUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN0), Opcode::EvexVprorqUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN1).union(A::MASK_K0), Opcode::EvexVproldUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN1), Opcode::EvexVproldUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN1).union(A::MASK_K0), Opcode::EvexVprolqUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN1), Opcode::EvexVprolqUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN2).union(A::MASK_K0), Opcode::EvexVpsrldUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN2), Opcode::EvexVpsrldUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN4).union(A::MASK_K0), Opcode::EvexVpsradUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN4), Opcode::EvexVpsradUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN4).union(A::MASK_K0), Opcode::EvexVpsraqUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN4), Opcode::EvexVpsraqUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN6).union(A::MASK_K0), Opcode::EvexVpslldUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::NNN6), Opcode::EvexVpslldUdqIbKmask),
];

static EVEX_0F73: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN2).union(A::MASK_K0), Opcode::EvexVpsrlqUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN2), Opcode::EvexVpsrlqUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::NNN3).union(A::MASK_K0), Opcode::EvexVpsrldqUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN6).union(A::MASK_K0), Opcode::EvexVpsllqUdqIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::NNN6), Opcode::EvexVpsllqUdqIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::NNN7).union(A::MASK_K0), Opcode::EvexVpslldqUdqIb),
];

static EVEX_0F74: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpcmpeqbKgqHdqWdq),
];

static EVEX_0F75: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpcmpeqwKgdHdqWdq),
];

static EVEX_0F76: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpcmpeqdKgwHdqWdq),
];

static EVEX_0F78: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttps2udqVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttps2udqVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttpd2udqVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttpd2udqVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2uqqVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2uqqVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttpd2uqqVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVcvttpd2uqqVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvttss2usiGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvttss2usiGqWss),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvttsd2usiGdWsd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2).union(A::IS64), Opcode::EvexVcvttsd2usiGqWsd),
];

static EVEX_0F79: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtps2udqVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtps2udqVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtpd2udqVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtpd2udqVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2uqqVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2uqqVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtpd2uqqVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVcvtpd2uqqVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtss2usiGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvtss2usiGqWss),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtsd2usiGdWsd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2).union(A::IS64), Opcode::EvexVcvtsd2usiGqWsd),
];

static EVEX_0F7A: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2qqVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2qqVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttpd2qqVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVcvttpd2qqVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtudq2pdVpdWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVcvtudq2pdVpdWdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtuqq2pdVpdWdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F3), Opcode::EvexVcvtuqq2pdVpdWdqKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtudq2psVpsWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVcvtudq2psVpsWdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtuqq2psVpsWdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVcvtuqq2psVpsWdqKmask),
];

static EVEX_0F7B: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2qqVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2qqVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtpd2qqVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVcvtpd2qqVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtusi2ssVssEd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvtusi2ssVssEq),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtusi2sdVsdEd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2).union(A::IS64), Opcode::EvexVcvtusi2sdVsdEq),
];

static EVEX_0F7E: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66).union(A::VL128), Opcode::EvexVmovdEdVd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66).union(A::VL128).union(A::IS64), Opcode::EvexVmovqEqVq),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::VL128), Opcode::EvexVmovdVdWd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::VL128), Opcode::EvexVmovqVqWq),
];

static EVEX_0F7F: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmovdqa32WdqVdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVmovdqa32WdqVdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmovdqa64WdqVdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVmovdqa64WdqVdqKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmovdqu32WdqVdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVmovdqu32WdqVdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmovdqu64WdqVdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F3), Opcode::EvexVmovdqu64WdqVdqKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVmovdqu8WdqVdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVmovdqu8WdqVdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVmovdqu16WdqVdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVmovdqu16WdqVdqKmask),
];

static EVEX_0FC2: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcmppsKgwHpsWpsIb),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVcmppdKgbHpdWpdIb),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVcmpssKgbHssWssIb),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVcmpsdKgbHsdWsdIb),
];

static EVEX_0FC4: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::MASK_K0), Opcode::EvexVpinsrwVdqEwIb),
];

static EVEX_0FC5: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::MASK_K0).union(A::MOD_REG), Opcode::EvexVpextrwGdUdqIb),
];

static EVEX_0FC6: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVshufpsVpsHpsWpsIb),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVshufpsVpsHpsWpsIbKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVshufpdVpdHpdWpdIb),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVshufpdVpdHpdWpdIbKmask),
];

static EVEX_0FD1: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsrlwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpsrlwVdqHdqWdqKmask),
];

static EVEX_0FD2: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpsrldVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpsrldVdqHdqWdqKmask),
];

static EVEX_0FD3: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsrlqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsrlqVdqHdqWdqKmask),
];

static EVEX_0FD4: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpaddqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpaddqVdqHdqWdqKmask),
];

static EVEX_0FD5: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmullwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmullwVdqHdqWdqKmask),
];

static EVEX_0FD6: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVmovdWdVd),
    form_opcode(A::SSE_PREFIX_66.union(A::VL128).union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVmovqWqVq),
];

static EVEX_0FD8: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsubusbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpsubusbVdqHdqWdqKmask),
];

static EVEX_0FD9: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsubuswVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpsubuswVdqHdqWdqKmask),
];

static EVEX_0FDA: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpminubVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpminubVdqHdqWdqKmask),
];

static EVEX_0FDB: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpanddVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpanddVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpandqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpandqVdqHdqWdqKmask),
];

static EVEX_0FDC: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpaddusbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpaddusbVdqHdqWdqKmask),
];

static EVEX_0FDD: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpadduswVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpadduswVdqHdqWdqKmask),
];

static EVEX_0FDE: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmaxubVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmaxubVdqHdqWdqKmask),
];

static EVEX_0FDF: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpandndVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpandndVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpandnqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpandnqVdqHdqWdqKmask),
];

static EVEX_0FE0: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpavgbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpavgbVdqHdqWdqKmask),
];

static EVEX_0FE1: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsrawVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpsrawVdqHdqWdqKmask),
];

static EVEX_0FE2: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpsradVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpsradVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsraqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsraqVdqHdqWdqKmask),
];

static EVEX_0FE3: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpavgwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpavgwVdqHdqWdqKmask),
];

static EVEX_0FE4: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmulhuwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmulhuwVdqHdqWdqKmask),
];

static EVEX_0FE5: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmulhwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmulhwVdqHdqWdqKmask),
];

static EVEX_0FE6: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVcvttpd2dqVdqWpd),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVcvttpd2dqVdqWpdKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVcvtdq2pdVpdWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVcvtdq2pdVpdWdqKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVcvtqq2pdVpdWdq),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W1), Opcode::EvexVcvtqq2pdVpdWdqKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVcvtpd2dqVdqWpd),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W1), Opcode::EvexVcvtpd2dqVdqWpdKmask),
];

static EVEX_0FE7: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::MOD_MEM).union(A::SSE_PREFIX_66), Opcode::EvexVmovntdqMdqVdq),
];

static EVEX_0FE8: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsubsbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpsubsbVdqHdqWdqKmask),
];

static EVEX_0FE9: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsubswVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpsubswVdqHdqWdqKmask),
];

static EVEX_0FEA: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpminswVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpminswVdqHdqWdqKmask),
];

static EVEX_0FEB: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpordVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpordVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVporqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVporqVdqHdqWdqKmask),
];

static EVEX_0FEC: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpaddsbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpaddsbVdqHdqWdqKmask),
];

static EVEX_0FED: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpaddswVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpaddswVdqHdqWdqKmask),
];

static EVEX_0FEE: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmaxswVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmaxswVdqHdqWdqKmask),
];

static EVEX_0FEF: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpxordVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpxordVdqHdqWdqKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpxorqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpxorqVdqHdqWdqKmask),
];

static EVEX_0FF1: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsllwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpsllwVdqHdqWdqKmask),
];

static EVEX_0FF2: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpslldVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpslldVdqHdqWdqKmask),
];

static EVEX_0FF3: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsllqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsllqVdqHdqWdqKmask),
];

static EVEX_0FF4: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpmuludqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpmuludqVdqHdqWdqKmask),
];

static EVEX_0FF5: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpmaddwdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpmaddwdVdqHdqWdqKmask),
];

static EVEX_0FF6: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsadbwVdqHdqWdq),
];

static EVEX_0FF8: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsubbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpsubbVdqHdqWdqKmask),
];

static EVEX_0FF9: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpsubwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpsubwVdqHdqWdqKmask),
];

static EVEX_0FFA: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpsubdVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpsubdVdqHdqWdqKmask),
];

static EVEX_0FFB: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1).union(A::MASK_K0), Opcode::EvexVpsubqVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W1), Opcode::EvexVpsubqVdqHdqWdqKmask),
];

static EVEX_0FFC: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpaddbVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpaddbVdqHdqWdqKmask),
];

static EVEX_0FFD: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::MASK_K0), Opcode::EvexVpaddwVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66, Opcode::EvexVpaddwVdqHdqWdqKmask),
];

static EVEX_0FFE: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVpadddVdqHdqWdq),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVpadddVdqHdqWdqKmask),
];

static EVEX_MAP5_10: &[u64] = &[
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0).union(A::MOD_REG), Opcode::EvexVmovshVshHphWsh),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0).union(A::MOD_MEM), Opcode::EvexVmovshVshWsh),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MOD_REG), Opcode::EvexVmovshVshHphWshKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MOD_MEM), Opcode::EvexVmovshVshWshKmask),
];

static EVEX_MAP5_11: &[u64] = &[
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0).union(A::MOD_REG), Opcode::EvexVmovshWshHphVsh),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MASK_K0).union(A::MOD_MEM), Opcode::EvexVmovshWshVsh),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MOD_REG), Opcode::EvexVmovshWshHphVshKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0).union(A::MOD_MEM), Opcode::EvexVmovshWshVshKmask),
];

static EVEX_MAP5_18: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVcvtbiasph2hf8Vf8hdqWphKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVcvtph2hf8Vf8hdqWphKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVcvt2ph2hf8Vf8hdqWphKmask),
];

static EVEX_MAP5_1B: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVcvtbiasph2hf8sVf8hdqWphKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVcvtph2hf8sVf8hdqWphKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVcvt2ph2hf8sVf8hdqWphKmask),
];

static EVEX_MAP5_1D: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtss2shVssWsh),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtss2shVssWshKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2phxVphWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2phxVphWdqKmask),
];

static EVEX_MAP5_1E: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVcvthf82phVphWf8Kmask),
];

static EVEX_MAP5_2A: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtsi2shVshEd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvtsi2shVshEq),
];

static EVEX_MAP5_2C: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvttsh2siGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvttsh2siGqWss),
];

static EVEX_MAP5_2D: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtsh2siGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvtsh2siGqWss),
];

static EVEX_MAP5_2E: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVucomishVshWsh),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVucomxshVshWsh),
];

static EVEX_MAP5_2F: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcomishVshWsh),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcomisbf16VshWsh),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcomxshVshWsh),
];

static EVEX_MAP5_51: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVsqrtphVphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVsqrtphVphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVsqrtbf16VphWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVsqrtbf16VphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVsqrtshVshHphWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVsqrtshVshHphWshKmask),
];

static EVEX_MAP5_58: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVaddphVphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVaddphVphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVaddbf16VphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVaddbf16VphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVaddshVshHphWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVaddshVshHphWshKmask),
];

static EVEX_MAP5_59: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVmulphVphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVmulphVphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVmulbf16VphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVmulbf16VphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmulshVshHphWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVmulshVshHphWshKmask),
];

static EVEX_MAP5_5A: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2pdVpdWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2pdVpdWphKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtpd2phVphWdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVcvtpd2phVphWdqKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtsh2sdVsdWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVcvtsh2sdVsdWshKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtsd2shVssWsh),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVcvtsd2shVssWshKmask),
];

static EVEX_MAP5_5B: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtdq2phVphWdq),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtdq2phVphWdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtqq2phVphWdq),
    form_opcode(A::VEX_W1.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtqq2phVphWdqKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2dqVdqWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2dqVdqWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvttph2dqVdqWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVcvttph2dqVdqWphKmask),
];

static EVEX_MAP5_5C: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVsubphVphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVsubphVphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVsubbf16VphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVsubbf16VphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVsubshVshHphWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVsubshVshHphWshKmask),
];

static EVEX_MAP5_5D: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVminphVphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVminphVphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::IaError),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::IaError),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVminshVshHphWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVminshVshHphWshKmask),
];

static EVEX_MAP5_5E: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVdivphVphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVdivphVphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVdivbf16VphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVdivbf16VphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVdivshVshHphWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVdivshVshHphWshKmask),
];

static EVEX_MAP5_5F: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVmaxphVphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVmaxphVphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::IaError),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::IaError),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVmaxshVshHphWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVmaxshVshHphWshKmask),
];

static EVEX_MAP5_68: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttph2ibsV8bWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttph2ibsV8bWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2ibsV8bWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2ibsV8bWpsKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvttbf162ibsV8bWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVcvttbf162ibsV8bWphKmask),
];

static EVEX_MAP5_69: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2ibsV8bWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2ibsV8bWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2ibsV8bWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2ibsV8bWpsKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtbf162ibsV8bWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVcvtbf162ibsV8bWphKmask),
];

static EVEX_MAP5_6A: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttph2iubsV8bWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttph2iubsV8bWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2iubsV8bWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2iubsV8bWpsKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvttbf162iubsV8bWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVcvttbf162iubsV8bWphKmask),
];

static EVEX_MAP5_6B: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2iubsV8bWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2iubsV8bWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2iubsV8bWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtps2iubsV8bWpsKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtbf162iubsV8bWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVcvtbf162iubsV8bWphKmask),
];

static EVEX_MAP5_6C: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttps2udqsVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttps2udqsVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttpd2udqsVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttpd2udqsVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2uqqsVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2uqqsVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttpd2uqqsVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVcvttpd2uqqsVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvttss2usisGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvttss2usisGqWss),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvttsd2usisGdWsd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2).union(A::IS64), Opcode::EvexVcvttsd2usisGqWsd),
];

static EVEX_MAP5_6D: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttps2dqsVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttps2dqsVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttpd2dqsVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttpd2dqsVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2qqsVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvttps2qqsVdqWpsKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttpd2qqsVdqWpd),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_66), Opcode::EvexVcvttpd2qqsVdqWpdKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvttss2sisGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvttss2sisGqWss),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvttsd2sisGdWsd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2).union(A::IS64), Opcode::EvexVcvttsd2sisGqWsd),
];

static EVEX_MAP5_6E: &[u64] = &[
    form_opcode(A::MASK_K0.union(A::SSE_PREFIX_66).union(A::VL128), Opcode::EvexVmovwVshEw),
    form_opcode(A::MASK_K0.union(A::SSE_PREFIX_F3).union(A::VL128).union(A::VEX_W0), Opcode::EvexVmovwVshWsh),
];

static EVEX_MAP5_6F: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MOD_MEM).union(A::SSE_PREFIX_F3), Opcode::EvexVmovrsdVdqWdq),
    form_opcode(A::VEX_W0.union(A::MOD_MEM).union(A::SSE_PREFIX_F3).union(A::MASK_K0), Opcode::EvexVmovrsdVdqWdqKmask),
    form_opcode(A::VEX_W1.union(A::MOD_MEM).union(A::SSE_PREFIX_F3), Opcode::EvexVmovrsqVdqWdq),
    form_opcode(A::VEX_W1.union(A::MOD_MEM).union(A::SSE_PREFIX_F3).union(A::MASK_K0), Opcode::EvexVmovrsqVdqWdqKmask),
    form_opcode(A::VEX_W0.union(A::MOD_MEM).union(A::SSE_PREFIX_F2), Opcode::EvexVmovrsbVdqWdq),
    form_opcode(A::VEX_W0.union(A::MOD_MEM).union(A::SSE_PREFIX_F2).union(A::MASK_K0), Opcode::EvexVmovrsbVdqWdqKmask),
    form_opcode(A::VEX_W1.union(A::MOD_MEM).union(A::SSE_PREFIX_F2), Opcode::EvexVmovrswVdqWdq),
    form_opcode(A::VEX_W1.union(A::MOD_MEM).union(A::SSE_PREFIX_F2).union(A::MASK_K0), Opcode::EvexVmovrswVdqWdqKmask),
];

static EVEX_MAP5_74: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVcvtbiasph2bf8sVf8hdqWphKmask),
    form_opcode(A::SSE_PREFIX_F3.union(A::VEX_W0), Opcode::EvexVcvtph2bf8sVf8hdqWphKmask),
    form_opcode(A::SSE_PREFIX_F2.union(A::VEX_W0), Opcode::EvexVcvt2ph2bf8sVf8hdqWphKmask),
];

static EVEX_MAP5_78: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttph2udqVdqWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttph2udqVdqWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttph2uqqVdqWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvttph2uqqVdqWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvttsh2usiGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvttsh2usiGqWss),
];

static EVEX_MAP5_79: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2udqVdqWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2udqVdqWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2uqqVdqWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2uqqVdqWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtsh2usiGdWss),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvtsh2usiGqWss),
];

static EVEX_MAP5_7A: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttph2qqVdqWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvttph2qqVdqWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtudq2phVphWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVcvtudq2phVphWdqKmask),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtuqq2phVphWdq),
    form_opcode(A::VEX_W1.union(A::SSE_PREFIX_F2), Opcode::EvexVcvtuqq2phVphWdqKmask),
];

static EVEX_MAP5_7B: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2qqVdqWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2qqVdqWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtusi2shVshEd),
    form_opcode(A::VEX_W1.union(A::MASK_K0).union(A::SSE_PREFIX_F3).union(A::IS64), Opcode::EvexVcvtusi2shVshEq),
];

static EVEX_MAP5_7C: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvttph2uwVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvttph2uwVdqWpsKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvttph2wVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvttph2wVdqWpsKmask),
];

static EVEX_MAP5_7D: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2uwVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtph2uwVdqWpsKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2wVdqWps),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2wVdqWpsKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F3), Opcode::EvexVcvtw2phVphWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVcvtw2phVphWdqKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_F2), Opcode::EvexVcvtuw2phVphWdq),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVcvtuw2phVphWdqKmask),
];

static EVEX_MAP5_7E: &[u64] = &[
    form_opcode(A::MASK_K0.union(A::SSE_PREFIX_66).union(A::VL128), Opcode::EvexVmovwEdVsh),
    form_opcode(A::MASK_K0.union(A::SSE_PREFIX_F3).union(A::VL128).union(A::VEX_W0), Opcode::EvexVmovwWshVsh),
];

static EVEX_MAP6_13: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVcvtsh2ssVssWsh),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVcvtsh2ssVssWshKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2psxVpsWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVcvtph2psxVpsWphKmask),
];

static EVEX_MAP6_2C: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVscalefpbf16VphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVscalefpbf16VphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVscalefphVphHphWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVscalefphVphHphWphKmask),
];

static EVEX_MAP6_2D: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVscalefshVshHphWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVscalefshVshHphWshKmask),
];

static EVEX_MAP6_42: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_NO_PREFIX), Opcode::EvexVgetexppbf16VphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVgetexppbf16VphWphKmask),
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVgetexpphVphWph),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVgetexpphVphWphKmask),
];

static EVEX_MAP6_43: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::MASK_K0).union(A::SSE_PREFIX_66), Opcode::EvexVgetexpshVshHphWsh),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVgetexpshVshHphWshKmask),
];

static EVEX_MAP6_4C: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX).union(A::MASK_K0), Opcode::EvexVrcppbf16VphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVrcppbf16VphWphKmask),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVrcpphVphWphKmask),
];

static EVEX_MAP6_4D: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVrcpshVshHphWshKmask),
];

static EVEX_MAP6_4E: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX).union(A::MASK_K0), Opcode::EvexVrsqrtpbf16VphWph),
    form_opcode(A::VEX_W0.union(A::SSE_NO_PREFIX), Opcode::EvexVrsqrtpbf16VphWphKmask),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVrsqrtphVphWphKmask),
];

static EVEX_MAP6_4F: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_66), Opcode::EvexVrsqrtshVshHphWshKmask),
];

static EVEX_MAP6_56: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVfmaddcphVphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVfcmaddcphVphHphWphKmask),
];

static EVEX_MAP6_57: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVfmaddcshVshHphWshKmask),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVfcmaddcshVshHphWshKmask),
];

static EVEX_MAP6_96: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmaddsub132phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmaddsub132phVphHphWphKmask),
];

static EVEX_MAP6_97: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsubadd132phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsubadd132phVphHphWphKmask),
];

static EVEX_MAP6_98: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd132bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfmadd132bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd132phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd132phVphHphWphKmask),
];

static EVEX_MAP6_99: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd132shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd132shVphHshWshKmask),
];

static EVEX_MAP6_9A: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub132bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfmsub132bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub132phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub132phVphHphWphKmask),
];

static EVEX_MAP6_9B: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub132shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub132shVphHshWshKmask),
];

static EVEX_MAP6_9C: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd132bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfnmadd132bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd132phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd132phVphHphWphKmask),
];

static EVEX_MAP6_9D: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd132shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd132shVphHshWshKmask),
];

static EVEX_MAP6_9E: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub132bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfnmsub132bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub132phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub132phVphHphWphKmask),
];

static EVEX_MAP6_9F: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub132shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub132shVphHshWshKmask),
];

static EVEX_MAP6_A6: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmaddsub213phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmaddsub213phVphHphWphKmask),
];

static EVEX_MAP6_A7: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsubadd213phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsubadd213phVphHphWphKmask),
];

static EVEX_MAP6_A8: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd213bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfmadd213bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd213phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd213phVphHphWphKmask),
];

static EVEX_MAP6_A9: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd213shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd213shVphHshWshKmask),
];

static EVEX_MAP6_AA: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub213bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfmsub213bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub213phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub213phVphHphWphKmask),
];

static EVEX_MAP6_AB: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub213shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub213shVphHshWshKmask),
];

static EVEX_MAP6_AC: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd213bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfnmadd213bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd213phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd213phVphHphWphKmask),
];

static EVEX_MAP6_AD: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd213shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd213shVphHshWshKmask),
];

static EVEX_MAP6_AE: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub213bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfnmsub213bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub213phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub213phVphHphWphKmask),
];

static EVEX_MAP6_AF: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub213shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub213shVphHshWshKmask),
];

static EVEX_MAP6_B6: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmaddsub231phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmaddsub231phVphHphWphKmask),
];

static EVEX_MAP6_B7: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsubadd231phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsubadd231phVphHphWphKmask),
];

static EVEX_MAP6_B8: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd231bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfmadd231bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd231phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd231phVphHphWphKmask),
];

static EVEX_MAP6_B9: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmadd231shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmadd231shVphHshWshKmask),
];

static EVEX_MAP6_BA: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub231bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfmsub231bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub231phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub231phVphHphWphKmask),
];

static EVEX_MAP6_BB: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfmsub231shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfmsub231shVphHshWshKmask),
];

static EVEX_MAP6_BC: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd231bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfnmadd231bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd231phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd231phVphHphWphKmask),
];

static EVEX_MAP6_BD: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmadd231shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmadd231shVphHshWshKmask),
];

static EVEX_MAP6_BE: &[u64] = &[
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub231bf16VphHphWph),
    form_opcode(A::SSE_NO_PREFIX.union(A::VEX_W0), Opcode::EvexVfnmsub231bf16VphHphWphKmask),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub231phVphHphWph),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub231phVphHphWphKmask),
];

static EVEX_MAP6_BF: &[u64] = &[
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0).union(A::MASK_K0), Opcode::EvexVfnmsub231shVphHshWsh),
    form_opcode(A::SSE_PREFIX_66.union(A::VEX_W0), Opcode::EvexVfnmsub231shVphHshWshKmask),
];

static EVEX_MAP6_D6: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVfmulcphVphHphWphKmask),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVfcmulcphVphHphWphKmask),
];

static EVEX_MAP6_D7: &[u64] = &[
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F3), Opcode::EvexVfmulcshVshHphWshKmask),
    form_opcode(A::VEX_W0.union(A::SSE_PREFIX_F2), Opcode::EvexVfcmulcshVshHphWshKmask),
];

/// Master EVEX table, indexed `(map - 1) * 256 + opcode`.
///
/// Bochs `BxOpcodeTableEVEX[256*5]`. Map 1 = 0F, 2 = 0F38, 3 = 0F3A,
/// 5 = MAP5, 6 = MAP6; the map-4 block is unused but kept so the
/// indexing matches upstream exactly.
pub(crate) static EVEX_TABLE: [&[u64]; 1280] = [
    // ---- map 1 (0F) ----
    /* 00 */ EVEX_GROUP_ERR,
    /* 01 */ EVEX_GROUP_ERR,
    /* 02 */ EVEX_GROUP_ERR,
    /* 03 */ EVEX_GROUP_ERR,
    /* 04 */ EVEX_GROUP_ERR,
    /* 05 */ EVEX_GROUP_ERR,
    /* 06 */ EVEX_GROUP_ERR,
    /* 07 */ EVEX_GROUP_ERR,
    /* 08 */ EVEX_GROUP_ERR,
    /* 09 */ EVEX_GROUP_ERR,
    /* 0A */ EVEX_GROUP_ERR,
    /* 0B */ EVEX_GROUP_ERR,
    /* 0C */ EVEX_GROUP_ERR,
    /* 0D */ EVEX_GROUP_ERR,
    /* 0E */ EVEX_GROUP_ERR,
    /* 0F */ EVEX_GROUP_ERR,
    /* 10 */ EVEX_0F10,
    /* 11 */ EVEX_0F11,
    /* 12 */ EVEX_0F12,
    /* 13 */ EVEX_0F13,
    /* 14 */ EVEX_0F14,
    /* 15 */ EVEX_0F15,
    /* 16 */ EVEX_0F16,
    /* 17 */ EVEX_0F17,
    /* 18 */ EVEX_GROUP_ERR,
    /* 19 */ EVEX_GROUP_ERR,
    /* 1A */ EVEX_GROUP_ERR,
    /* 1B */ EVEX_GROUP_ERR,
    /* 1C */ EVEX_GROUP_ERR,
    /* 1D */ EVEX_GROUP_ERR,
    /* 1E */ EVEX_GROUP_ERR,
    /* 1F */ EVEX_GROUP_ERR,
    /* 20 */ EVEX_GROUP_ERR,
    /* 21 */ EVEX_GROUP_ERR,
    /* 22 */ EVEX_GROUP_ERR,
    /* 23 */ EVEX_GROUP_ERR,
    /* 24 */ EVEX_GROUP_ERR,
    /* 25 */ EVEX_GROUP_ERR,
    /* 26 */ EVEX_GROUP_ERR,
    /* 27 */ EVEX_GROUP_ERR,
    /* 28 */ EVEX_0F28,
    /* 29 */ EVEX_0F29,
    /* 2A */ EVEX_0F2A,
    /* 2B */ EVEX_0F2B,
    /* 2C */ EVEX_0F2C,
    /* 2D */ EVEX_0F2D,
    /* 2E */ EVEX_0F2E,
    /* 2F */ EVEX_0F2F,
    /* 30 */ EVEX_GROUP_ERR,
    /* 31 */ EVEX_GROUP_ERR,
    /* 32 */ EVEX_GROUP_ERR,
    /* 33 */ EVEX_GROUP_ERR,
    /* 34 */ EVEX_GROUP_ERR,
    /* 35 */ EVEX_GROUP_ERR,
    /* 36 */ EVEX_GROUP_ERR,
    /* 37 */ EVEX_GROUP_ERR,
    /* 38 */ EVEX_GROUP_ERR,
    /* 39 */ EVEX_GROUP_ERR,
    /* 3A */ EVEX_GROUP_ERR,
    /* 3B */ EVEX_GROUP_ERR,
    /* 3C */ EVEX_GROUP_ERR,
    /* 3D */ EVEX_GROUP_ERR,
    /* 3E */ EVEX_GROUP_ERR,
    /* 3F */ EVEX_GROUP_ERR,
    /* 40 */ EVEX_GROUP_ERR,
    /* 41 */ EVEX_GROUP_ERR,
    /* 42 */ EVEX_GROUP_ERR,
    /* 43 */ EVEX_GROUP_ERR,
    /* 44 */ EVEX_GROUP_ERR,
    /* 45 */ EVEX_GROUP_ERR,
    /* 46 */ EVEX_GROUP_ERR,
    /* 47 */ EVEX_GROUP_ERR,
    /* 48 */ EVEX_GROUP_ERR,
    /* 49 */ EVEX_GROUP_ERR,
    /* 4A */ EVEX_GROUP_ERR,
    /* 4B */ EVEX_GROUP_ERR,
    /* 4C */ EVEX_GROUP_ERR,
    /* 4D */ EVEX_GROUP_ERR,
    /* 4E */ EVEX_GROUP_ERR,
    /* 4F */ EVEX_GROUP_ERR,
    /* 50 */ EVEX_GROUP_ERR,
    /* 51 */ EVEX_0F51,
    /* 52 */ EVEX_GROUP_ERR,
    /* 53 */ EVEX_GROUP_ERR,
    /* 54 */ EVEX_0F54,
    /* 55 */ EVEX_0F55,
    /* 56 */ EVEX_0F56,
    /* 57 */ EVEX_0F57,
    /* 58 */ EVEX_0F58,
    /* 59 */ EVEX_0F59,
    /* 5A */ EVEX_0F5A,
    /* 5B */ EVEX_0F5B,
    /* 5C */ EVEX_0F5C,
    /* 5D */ EVEX_0F5D,
    /* 5E */ EVEX_0F5E,
    /* 5F */ EVEX_0F5F,
    /* 60 */ EVEX_0F60,
    /* 61 */ EVEX_0F61,
    /* 62 */ EVEX_0F62,
    /* 63 */ EVEX_0F63,
    /* 64 */ EVEX_0F64,
    /* 65 */ EVEX_0F65,
    /* 66 */ EVEX_0F66,
    /* 67 */ EVEX_0F67,
    /* 68 */ EVEX_0F68,
    /* 69 */ EVEX_0F69,
    /* 6A */ EVEX_0F6A,
    /* 6B */ EVEX_0F6B,
    /* 6C */ EVEX_0F6C,
    /* 6D */ EVEX_0F6D,
    /* 6E */ EVEX_0F6E,
    /* 6F */ EVEX_0F6F,
    /* 70 */ EVEX_0F70,
    /* 71 */ EVEX_0F71,
    /* 72 */ EVEX_0F72,
    /* 73 */ EVEX_0F73,
    /* 74 */ EVEX_0F74,
    /* 75 */ EVEX_0F75,
    /* 76 */ EVEX_0F76,
    /* 77 */ EVEX_GROUP_ERR,
    /* 78 */ EVEX_0F78,
    /* 79 */ EVEX_0F79,
    /* 7A */ EVEX_0F7A,
    /* 7B */ EVEX_0F7B,
    /* 7C */ EVEX_GROUP_ERR,
    /* 7D */ EVEX_GROUP_ERR,
    /* 7E */ EVEX_0F7E,
    /* 7F */ EVEX_0F7F,
    /* 80 */ EVEX_GROUP_ERR,
    /* 81 */ EVEX_GROUP_ERR,
    /* 82 */ EVEX_GROUP_ERR,
    /* 83 */ EVEX_GROUP_ERR,
    /* 84 */ EVEX_GROUP_ERR,
    /* 85 */ EVEX_GROUP_ERR,
    /* 86 */ EVEX_GROUP_ERR,
    /* 87 */ EVEX_GROUP_ERR,
    /* 88 */ EVEX_GROUP_ERR,
    /* 89 */ EVEX_GROUP_ERR,
    /* 8A */ EVEX_GROUP_ERR,
    /* 8B */ EVEX_GROUP_ERR,
    /* 8C */ EVEX_GROUP_ERR,
    /* 8D */ EVEX_GROUP_ERR,
    /* 8E */ EVEX_GROUP_ERR,
    /* 8F */ EVEX_GROUP_ERR,
    /* 90 */ EVEX_GROUP_ERR,
    /* 91 */ EVEX_GROUP_ERR,
    /* 92 */ EVEX_GROUP_ERR,
    /* 93 */ EVEX_GROUP_ERR,
    /* 94 */ EVEX_GROUP_ERR,
    /* 95 */ EVEX_GROUP_ERR,
    /* 96 */ EVEX_GROUP_ERR,
    /* 97 */ EVEX_GROUP_ERR,
    /* 98 */ EVEX_GROUP_ERR,
    /* 99 */ EVEX_GROUP_ERR,
    /* 9A */ EVEX_GROUP_ERR,
    /* 9B */ EVEX_GROUP_ERR,
    /* 9C */ EVEX_GROUP_ERR,
    /* 9D */ EVEX_GROUP_ERR,
    /* 9E */ EVEX_GROUP_ERR,
    /* 9F */ EVEX_GROUP_ERR,
    /* A0 */ EVEX_GROUP_ERR,
    /* A1 */ EVEX_GROUP_ERR,
    /* A2 */ EVEX_GROUP_ERR,
    /* A3 */ EVEX_GROUP_ERR,
    /* A4 */ EVEX_GROUP_ERR,
    /* A5 */ EVEX_GROUP_ERR,
    /* A6 */ EVEX_GROUP_ERR,
    /* A7 */ EVEX_GROUP_ERR,
    /* A8 */ EVEX_GROUP_ERR,
    /* A9 */ EVEX_GROUP_ERR,
    /* AA */ EVEX_GROUP_ERR,
    /* AB */ EVEX_GROUP_ERR,
    /* AC */ EVEX_GROUP_ERR,
    /* AD */ EVEX_GROUP_ERR,
    /* AE */ EVEX_GROUP_ERR,
    /* AF */ EVEX_GROUP_ERR,
    /* B0 */ EVEX_GROUP_ERR,
    /* B1 */ EVEX_GROUP_ERR,
    /* B2 */ EVEX_GROUP_ERR,
    /* B3 */ EVEX_GROUP_ERR,
    /* B4 */ EVEX_GROUP_ERR,
    /* B5 */ EVEX_GROUP_ERR,
    /* B6 */ EVEX_GROUP_ERR,
    /* B7 */ EVEX_GROUP_ERR,
    /* B8 */ EVEX_GROUP_ERR,
    /* B9 */ EVEX_GROUP_ERR,
    /* BA */ EVEX_GROUP_ERR,
    /* BB */ EVEX_GROUP_ERR,
    /* BC */ EVEX_GROUP_ERR,
    /* BD */ EVEX_GROUP_ERR,
    /* BE */ EVEX_GROUP_ERR,
    /* BF */ EVEX_GROUP_ERR,
    /* C0 */ EVEX_GROUP_ERR,
    /* C1 */ EVEX_GROUP_ERR,
    /* C2 */ EVEX_0FC2,
    /* C3 */ EVEX_GROUP_ERR,
    /* C4 */ EVEX_0FC4,
    /* C5 */ EVEX_0FC5,
    /* C6 */ EVEX_0FC6,
    /* C7 */ EVEX_GROUP_ERR,
    /* C8 */ EVEX_GROUP_ERR,
    /* C9 */ EVEX_GROUP_ERR,
    /* CA */ EVEX_GROUP_ERR,
    /* CB */ EVEX_GROUP_ERR,
    /* CC */ EVEX_GROUP_ERR,
    /* CD */ EVEX_GROUP_ERR,
    /* CE */ EVEX_GROUP_ERR,
    /* CF */ EVEX_GROUP_ERR,
    /* D0 */ EVEX_GROUP_ERR,
    /* D1 */ EVEX_0FD1,
    /* D2 */ EVEX_0FD2,
    /* D3 */ EVEX_0FD3,
    /* D4 */ EVEX_0FD4,
    /* D5 */ EVEX_0FD5,
    /* D6 */ EVEX_0FD6,
    /* D7 */ EVEX_GROUP_ERR,
    /* D8 */ EVEX_0FD8,
    /* D9 */ EVEX_0FD9,
    /* DA */ EVEX_0FDA,
    /* DB */ EVEX_0FDB,
    /* DC */ EVEX_0FDC,
    /* DD */ EVEX_0FDD,
    /* DE */ EVEX_0FDE,
    /* DF */ EVEX_0FDF,
    /* E0 */ EVEX_0FE0,
    /* E1 */ EVEX_0FE1,
    /* E2 */ EVEX_0FE2,
    /* E3 */ EVEX_0FE3,
    /* E4 */ EVEX_0FE4,
    /* E5 */ EVEX_0FE5,
    /* E6 */ EVEX_0FE6,
    /* E7 */ EVEX_0FE7,
    /* E8 */ EVEX_0FE8,
    /* E9 */ EVEX_0FE9,
    /* EA */ EVEX_0FEA,
    /* EB */ EVEX_0FEB,
    /* EC */ EVEX_0FEC,
    /* ED */ EVEX_0FED,
    /* EE */ EVEX_0FEE,
    /* EF */ EVEX_0FEF,
    /* F0 */ EVEX_GROUP_ERR,
    /* F1 */ EVEX_0FF1,
    /* F2 */ EVEX_0FF2,
    /* F3 */ EVEX_0FF3,
    /* F4 */ EVEX_0FF4,
    /* F5 */ EVEX_0FF5,
    /* F6 */ EVEX_0FF6,
    /* F7 */ EVEX_GROUP_ERR,
    /* F8 */ EVEX_0FF8,
    /* F9 */ EVEX_0FF9,
    /* FA */ EVEX_0FFA,
    /* FB */ EVEX_0FFB,
    /* FC */ EVEX_0FFC,
    /* FD */ EVEX_0FFD,
    /* FE */ EVEX_0FFE,
    /* FF */ EVEX_GROUP_ERR,
    // ---- map 2 (0F38) ----
    /* 00 */ EVEX_0F3800,
    /* 01 */ EVEX_GROUP_ERR,
    /* 02 */ EVEX_GROUP_ERR,
    /* 03 */ EVEX_GROUP_ERR,
    /* 04 */ EVEX_0F3804,
    /* 05 */ EVEX_GROUP_ERR,
    /* 06 */ EVEX_GROUP_ERR,
    /* 07 */ EVEX_GROUP_ERR,
    /* 08 */ EVEX_GROUP_ERR,
    /* 09 */ EVEX_GROUP_ERR,
    /* 0A */ EVEX_GROUP_ERR,
    /* 0B */ EVEX_0F380B,
    /* 0C */ EVEX_0F380C,
    /* 0D */ EVEX_0F380D,
    /* 0E */ EVEX_GROUP_ERR,
    /* 0F */ EVEX_GROUP_ERR,
    /* 10 */ EVEX_0F3810,
    /* 11 */ EVEX_0F3811,
    /* 12 */ EVEX_0F3812,
    /* 13 */ EVEX_0F3813,
    /* 14 */ EVEX_0F3814,
    /* 15 */ EVEX_0F3815,
    /* 16 */ EVEX_0F3816,
    /* 17 */ EVEX_GROUP_ERR,
    /* 18 */ EVEX_0F3818,
    /* 19 */ EVEX_0F3819,
    /* 1A */ EVEX_0F381A,
    /* 1B */ EVEX_0F381B,
    /* 1C */ EVEX_0F381C,
    /* 1D */ EVEX_0F381D,
    /* 1E */ EVEX_0F381E,
    /* 1F */ EVEX_0F381F,
    /* 20 */ EVEX_0F3820,
    /* 21 */ EVEX_0F3821,
    /* 22 */ EVEX_0F3822,
    /* 23 */ EVEX_0F3823,
    /* 24 */ EVEX_0F3824,
    /* 25 */ EVEX_0F3825,
    /* 26 */ EVEX_0F3826,
    /* 27 */ EVEX_0F3827,
    /* 28 */ EVEX_0F3828,
    /* 29 */ EVEX_0F3829,
    /* 2A */ EVEX_0F382A,
    /* 2B */ EVEX_0F382B,
    /* 2C */ EVEX_0F382C,
    /* 2D */ EVEX_0F382D,
    /* 2E */ EVEX_GROUP_ERR,
    /* 2F */ EVEX_GROUP_ERR,
    /* 30 */ EVEX_0F3830,
    /* 31 */ EVEX_0F3831,
    /* 32 */ EVEX_0F3832,
    /* 33 */ EVEX_0F3833,
    /* 34 */ EVEX_0F3834,
    /* 35 */ EVEX_0F3835,
    /* 36 */ EVEX_0F3836,
    /* 37 */ EVEX_0F3837,
    /* 38 */ EVEX_0F3838,
    /* 39 */ EVEX_0F3839,
    /* 3A */ EVEX_0F383A,
    /* 3B */ EVEX_0F383B,
    /* 3C */ EVEX_0F383C,
    /* 3D */ EVEX_0F383D,
    /* 3E */ EVEX_0F383E,
    /* 3F */ EVEX_0F383F,
    /* 40 */ EVEX_0F3840,
    /* 41 */ EVEX_GROUP_ERR,
    /* 42 */ EVEX_0F3842,
    /* 43 */ EVEX_0F3843,
    /* 44 */ EVEX_0F3844,
    /* 45 */ EVEX_0F3845,
    /* 46 */ EVEX_0F3846,
    /* 47 */ EVEX_0F3847,
    /* 48 */ EVEX_GROUP_ERR,
    /* 49 */ EVEX_GROUP_ERR,
    /* 4A */ EVEX_0F384A,
    /* 4B */ EVEX_GROUP_ERR,
    /* 4C */ EVEX_0F384C,
    /* 4D */ EVEX_0F384D,
    /* 4E */ EVEX_0F384E,
    /* 4F */ EVEX_0F384F,
    /* 50 */ EVEX_0F3850,
    /* 51 */ EVEX_0F3851,
    /* 52 */ EVEX_0F3852,
    /* 53 */ EVEX_0F3853,
    /* 54 */ EVEX_0F3854,
    /* 55 */ EVEX_0F3855,
    /* 56 */ EVEX_GROUP_ERR,
    /* 57 */ EVEX_GROUP_ERR,
    /* 58 */ EVEX_0F3858,
    /* 59 */ EVEX_0F3859,
    /* 5A */ EVEX_0F385A,
    /* 5B */ EVEX_0F385B,
    /* 5C */ EVEX_GROUP_ERR,
    /* 5D */ EVEX_GROUP_ERR,
    /* 5E */ EVEX_GROUP_ERR,
    /* 5F */ EVEX_GROUP_ERR,
    /* 60 */ EVEX_GROUP_ERR,
    /* 61 */ EVEX_GROUP_ERR,
    /* 62 */ EVEX_0F3862,
    /* 63 */ EVEX_0F3863,
    /* 64 */ EVEX_0F3864,
    /* 65 */ EVEX_0F3865,
    /* 66 */ EVEX_0F3866,
    /* 67 */ EVEX_0F3867,
    /* 68 */ EVEX_0F3868,
    /* 69 */ EVEX_GROUP_ERR,
    /* 6A */ EVEX_GROUP_ERR,
    /* 6B */ EVEX_GROUP_ERR,
    /* 6C */ EVEX_GROUP_ERR,
    /* 6D */ EVEX_0F386D,
    /* 6E */ EVEX_GROUP_ERR,
    /* 6F */ EVEX_GROUP_ERR,
    /* 70 */ EVEX_0F3870,
    /* 71 */ EVEX_0F3871,
    /* 72 */ EVEX_0F3872,
    /* 73 */ EVEX_0F3873,
    /* 74 */ EVEX_0F3874,
    /* 75 */ EVEX_0F3875,
    /* 76 */ EVEX_0F3876,
    /* 77 */ EVEX_0F3877,
    /* 78 */ EVEX_0F3878,
    /* 79 */ EVEX_0F3879,
    /* 7A */ EVEX_0F387A,
    /* 7B */ EVEX_0F387B,
    /* 7C */ EVEX_0F387C,
    /* 7D */ EVEX_0F387D,
    /* 7E */ EVEX_0F387E,
    /* 7F */ EVEX_0F387F,
    /* 80 */ EVEX_GROUP_ERR,
    /* 81 */ EVEX_GROUP_ERR,
    /* 82 */ EVEX_GROUP_ERR,
    /* 83 */ EVEX_0F3883,
    /* 84 */ EVEX_GROUP_ERR,
    /* 85 */ EVEX_GROUP_ERR,
    /* 86 */ EVEX_GROUP_ERR,
    /* 87 */ EVEX_GROUP_ERR,
    /* 88 */ EVEX_0F3888,
    /* 89 */ EVEX_0F3889,
    /* 8A */ EVEX_0F388A,
    /* 8B */ EVEX_0F388B,
    /* 8C */ EVEX_GROUP_ERR,
    /* 8D */ EVEX_0F388D,
    /* 8E */ EVEX_GROUP_ERR,
    /* 8F */ EVEX_0F388F,
    /* 90 */ EVEX_0F3890,
    /* 91 */ EVEX_0F3891,
    /* 92 */ EVEX_0F3892,
    /* 93 */ EVEX_0F3893,
    /* 94 */ EVEX_GROUP_ERR,
    /* 95 */ EVEX_GROUP_ERR,
    /* 96 */ EVEX_0F3896,
    /* 97 */ EVEX_0F3897,
    /* 98 */ EVEX_0F3898,
    /* 99 */ EVEX_0F3899,
    /* 9A */ EVEX_0F389A,
    /* 9B */ EVEX_0F389B,
    /* 9C */ EVEX_0F389C,
    /* 9D */ EVEX_0F389D,
    /* 9E */ EVEX_0F389E,
    /* 9F */ EVEX_0F389F,
    /* A0 */ EVEX_0F38A0,
    /* A1 */ EVEX_0F38A1,
    /* A2 */ EVEX_0F38A2,
    /* A3 */ EVEX_0F38A3,
    /* A4 */ EVEX_GROUP_ERR,
    /* A5 */ EVEX_GROUP_ERR,
    /* A6 */ EVEX_0F38A6,
    /* A7 */ EVEX_0F38A7,
    /* A8 */ EVEX_0F38A8,
    /* A9 */ EVEX_0F38A9,
    /* AA */ EVEX_0F38AA,
    /* AB */ EVEX_0F38AB,
    /* AC */ EVEX_0F38AC,
    /* AD */ EVEX_0F38AD,
    /* AE */ EVEX_0F38AE,
    /* AF */ EVEX_0F38AF,
    /* B0 */ EVEX_GROUP_ERR,
    /* B1 */ EVEX_GROUP_ERR,
    /* B2 */ EVEX_GROUP_ERR,
    /* B3 */ EVEX_GROUP_ERR,
    /* B4 */ EVEX_0F38B4,
    /* B5 */ EVEX_0F38B5,
    /* B6 */ EVEX_0F38B6,
    /* B7 */ EVEX_0F38B7,
    /* B8 */ EVEX_0F38B8,
    /* B9 */ EVEX_0F38B9,
    /* BA */ EVEX_0F38BA,
    /* BB */ EVEX_0F38BB,
    /* BC */ EVEX_0F38BC,
    /* BD */ EVEX_0F38BD,
    /* BE */ EVEX_0F38BE,
    /* BF */ EVEX_0F38BF,
    /* C0 */ EVEX_GROUP_ERR,
    /* C1 */ EVEX_GROUP_ERR,
    /* C2 */ EVEX_GROUP_ERR,
    /* C3 */ EVEX_GROUP_ERR,
    /* C4 */ EVEX_0F38C4,
    /* C5 */ EVEX_GROUP_ERR,
    /* C6 */ EVEX_GROUP_ERR,
    /* C7 */ EVEX_GROUP_ERR,
    /* C8 */ EVEX_GROUP_ERR,
    /* C9 */ EVEX_GROUP_ERR,
    /* CA */ EVEX_GROUP_ERR,
    /* CB */ EVEX_GROUP_ERR,
    /* CC */ EVEX_GROUP_ERR,
    /* CD */ EVEX_GROUP_ERR,
    /* CE */ EVEX_GROUP_ERR,
    /* CF */ EVEX_0F38CF,
    /* D0 */ EVEX_GROUP_ERR,
    /* D1 */ EVEX_GROUP_ERR,
    /* D2 */ EVEX_GROUP_ERR,
    /* D3 */ EVEX_GROUP_ERR,
    /* D4 */ EVEX_GROUP_ERR,
    /* D5 */ EVEX_GROUP_ERR,
    /* D6 */ EVEX_GROUP_ERR,
    /* D7 */ EVEX_GROUP_ERR,
    /* D8 */ EVEX_GROUP_ERR,
    /* D9 */ EVEX_GROUP_ERR,
    /* DA */ EVEX_GROUP_ERR,
    /* DB */ EVEX_GROUP_ERR,
    /* DC */ EVEX_0F38DC,
    /* DD */ EVEX_0F38DD,
    /* DE */ EVEX_0F38DE,
    /* DF */ EVEX_0F38DF,
    /* E0 */ EVEX_GROUP_ERR,
    /* E1 */ EVEX_GROUP_ERR,
    /* E2 */ EVEX_GROUP_ERR,
    /* E3 */ EVEX_GROUP_ERR,
    /* E4 */ EVEX_GROUP_ERR,
    /* E5 */ EVEX_GROUP_ERR,
    /* E6 */ EVEX_GROUP_ERR,
    /* E7 */ EVEX_GROUP_ERR,
    /* E8 */ EVEX_GROUP_ERR,
    /* E9 */ EVEX_GROUP_ERR,
    /* EA */ EVEX_GROUP_ERR,
    /* EB */ EVEX_GROUP_ERR,
    /* EC */ EVEX_GROUP_ERR,
    /* ED */ EVEX_GROUP_ERR,
    /* EE */ EVEX_GROUP_ERR,
    /* EF */ EVEX_GROUP_ERR,
    /* F0 */ EVEX_GROUP_ERR,
    /* F1 */ EVEX_GROUP_ERR,
    /* F2 */ EVEX_GROUP_ERR,
    /* F3 */ EVEX_GROUP_ERR,
    /* F4 */ EVEX_GROUP_ERR,
    /* F5 */ EVEX_GROUP_ERR,
    /* F6 */ EVEX_GROUP_ERR,
    /* F7 */ EVEX_GROUP_ERR,
    /* F8 */ EVEX_GROUP_ERR,
    /* F9 */ EVEX_GROUP_ERR,
    /* FA */ EVEX_GROUP_ERR,
    /* FB */ EVEX_GROUP_ERR,
    /* FC */ EVEX_GROUP_ERR,
    /* FD */ EVEX_GROUP_ERR,
    /* FE */ EVEX_GROUP_ERR,
    /* FF */ EVEX_GROUP_ERR,
    // ---- map 3 (0F3A) ----
    /* 00 */ EVEX_0F3A00,
    /* 01 */ EVEX_0F3A01,
    /* 02 */ EVEX_GROUP_ERR,
    /* 03 */ EVEX_0F3A03,
    /* 04 */ EVEX_0F3A04,
    /* 05 */ EVEX_0F3A05,
    /* 06 */ EVEX_GROUP_ERR,
    /* 07 */ EVEX_0F3A07,
    /* 08 */ EVEX_0F3A08,
    /* 09 */ EVEX_0F3A09,
    /* 0A */ EVEX_0F3A0A,
    /* 0B */ EVEX_0F3A0B,
    /* 0C */ EVEX_GROUP_ERR,
    /* 0D */ EVEX_GROUP_ERR,
    /* 0E */ EVEX_GROUP_ERR,
    /* 0F */ EVEX_0F3A0F,
    /* 10 */ EVEX_GROUP_ERR,
    /* 11 */ EVEX_GROUP_ERR,
    /* 12 */ EVEX_GROUP_ERR,
    /* 13 */ EVEX_GROUP_ERR,
    /* 14 */ EVEX_0F3A14,
    /* 15 */ EVEX_0F3A15,
    /* 16 */ EVEX_0F3A16,
    /* 17 */ EVEX_0F3A17,
    /* 18 */ EVEX_0F3A18,
    /* 19 */ EVEX_0F3A19,
    /* 1A */ EVEX_0F3A1A,
    /* 1B */ EVEX_0F3A1B,
    /* 1C */ EVEX_GROUP_ERR,
    /* 1D */ EVEX_0F3A1D,
    /* 1E */ EVEX_0F3A1E,
    /* 1F */ EVEX_0F3A1F,
    /* 20 */ EVEX_0F3A20,
    /* 21 */ EVEX_0F3A21,
    /* 22 */ EVEX_0F3A22,
    /* 23 */ EVEX_0F3A23,
    /* 24 */ EVEX_GROUP_ERR,
    /* 25 */ EVEX_0F3A25,
    /* 26 */ EVEX_0F3A26,
    /* 27 */ EVEX_0F3A27,
    /* 28 */ EVEX_GROUP_ERR,
    /* 29 */ EVEX_GROUP_ERR,
    /* 2A */ EVEX_GROUP_ERR,
    /* 2B */ EVEX_GROUP_ERR,
    /* 2C */ EVEX_GROUP_ERR,
    /* 2D */ EVEX_GROUP_ERR,
    /* 2E */ EVEX_GROUP_ERR,
    /* 2F */ EVEX_GROUP_ERR,
    /* 30 */ EVEX_GROUP_ERR,
    /* 31 */ EVEX_GROUP_ERR,
    /* 32 */ EVEX_GROUP_ERR,
    /* 33 */ EVEX_GROUP_ERR,
    /* 34 */ EVEX_GROUP_ERR,
    /* 35 */ EVEX_GROUP_ERR,
    /* 36 */ EVEX_GROUP_ERR,
    /* 37 */ EVEX_GROUP_ERR,
    /* 38 */ EVEX_0F3A38,
    /* 39 */ EVEX_0F3A39,
    /* 3A */ EVEX_0F3A3A,
    /* 3B */ EVEX_0F3A3B,
    /* 3C */ EVEX_GROUP_ERR,
    /* 3D */ EVEX_GROUP_ERR,
    /* 3E */ EVEX_0F3A3E,
    /* 3F */ EVEX_0F3A3F,
    /* 40 */ EVEX_GROUP_ERR,
    /* 41 */ EVEX_GROUP_ERR,
    /* 42 */ EVEX_0F3A42,
    /* 43 */ EVEX_0F3A43,
    /* 44 */ EVEX_0F3A44,
    /* 45 */ EVEX_GROUP_ERR,
    /* 46 */ EVEX_GROUP_ERR,
    /* 47 */ EVEX_GROUP_ERR,
    /* 48 */ EVEX_GROUP_ERR,
    /* 49 */ EVEX_GROUP_ERR,
    /* 4A */ EVEX_GROUP_ERR,
    /* 4B */ EVEX_GROUP_ERR,
    /* 4C */ EVEX_GROUP_ERR,
    /* 4D */ EVEX_GROUP_ERR,
    /* 4E */ EVEX_GROUP_ERR,
    /* 4F */ EVEX_GROUP_ERR,
    /* 50 */ EVEX_0F3A50,
    /* 51 */ EVEX_0F3A51,
    /* 52 */ EVEX_0F3A52,
    /* 53 */ EVEX_0F3A53,
    /* 54 */ EVEX_0F3A54,
    /* 55 */ EVEX_0F3A55,
    /* 56 */ EVEX_0F3A56,
    /* 57 */ EVEX_0F3A57,
    /* 58 */ EVEX_GROUP_ERR,
    /* 59 */ EVEX_GROUP_ERR,
    /* 5A */ EVEX_GROUP_ERR,
    /* 5B */ EVEX_GROUP_ERR,
    /* 5C */ EVEX_GROUP_ERR,
    /* 5D */ EVEX_GROUP_ERR,
    /* 5E */ EVEX_GROUP_ERR,
    /* 5F */ EVEX_GROUP_ERR,
    /* 60 */ EVEX_GROUP_ERR,
    /* 61 */ EVEX_GROUP_ERR,
    /* 62 */ EVEX_GROUP_ERR,
    /* 63 */ EVEX_GROUP_ERR,
    /* 64 */ EVEX_GROUP_ERR,
    /* 65 */ EVEX_GROUP_ERR,
    /* 66 */ EVEX_0F3A66,
    /* 67 */ EVEX_0F3A67,
    /* 68 */ EVEX_GROUP_ERR,
    /* 69 */ EVEX_GROUP_ERR,
    /* 6A */ EVEX_GROUP_ERR,
    /* 6B */ EVEX_GROUP_ERR,
    /* 6C */ EVEX_GROUP_ERR,
    /* 6D */ EVEX_GROUP_ERR,
    /* 6E */ EVEX_GROUP_ERR,
    /* 6F */ EVEX_GROUP_ERR,
    /* 70 */ EVEX_0F3A70,
    /* 71 */ EVEX_0F3A71,
    /* 72 */ EVEX_0F3A72,
    /* 73 */ EVEX_0F3A73,
    /* 74 */ EVEX_GROUP_ERR,
    /* 75 */ EVEX_GROUP_ERR,
    /* 76 */ EVEX_GROUP_ERR,
    /* 77 */ EVEX_0F3A77,
    /* 78 */ EVEX_GROUP_ERR,
    /* 79 */ EVEX_GROUP_ERR,
    /* 7A */ EVEX_GROUP_ERR,
    /* 7B */ EVEX_GROUP_ERR,
    /* 7C */ EVEX_GROUP_ERR,
    /* 7D */ EVEX_GROUP_ERR,
    /* 7E */ EVEX_GROUP_ERR,
    /* 7F */ EVEX_GROUP_ERR,
    /* 80 */ EVEX_GROUP_ERR,
    /* 81 */ EVEX_GROUP_ERR,
    /* 82 */ EVEX_GROUP_ERR,
    /* 83 */ EVEX_GROUP_ERR,
    /* 84 */ EVEX_GROUP_ERR,
    /* 85 */ EVEX_GROUP_ERR,
    /* 86 */ EVEX_GROUP_ERR,
    /* 87 */ EVEX_GROUP_ERR,
    /* 88 */ EVEX_GROUP_ERR,
    /* 89 */ EVEX_GROUP_ERR,
    /* 8A */ EVEX_GROUP_ERR,
    /* 8B */ EVEX_GROUP_ERR,
    /* 8C */ EVEX_GROUP_ERR,
    /* 8D */ EVEX_GROUP_ERR,
    /* 8E */ EVEX_GROUP_ERR,
    /* 8F */ EVEX_GROUP_ERR,
    /* 90 */ EVEX_GROUP_ERR,
    /* 91 */ EVEX_GROUP_ERR,
    /* 92 */ EVEX_GROUP_ERR,
    /* 93 */ EVEX_GROUP_ERR,
    /* 94 */ EVEX_GROUP_ERR,
    /* 95 */ EVEX_GROUP_ERR,
    /* 96 */ EVEX_GROUP_ERR,
    /* 97 */ EVEX_GROUP_ERR,
    /* 98 */ EVEX_GROUP_ERR,
    /* 99 */ EVEX_GROUP_ERR,
    /* 9A */ EVEX_GROUP_ERR,
    /* 9B */ EVEX_GROUP_ERR,
    /* 9C */ EVEX_GROUP_ERR,
    /* 9D */ EVEX_GROUP_ERR,
    /* 9E */ EVEX_GROUP_ERR,
    /* 9F */ EVEX_GROUP_ERR,
    /* A0 */ EVEX_GROUP_ERR,
    /* A1 */ EVEX_GROUP_ERR,
    /* A2 */ EVEX_GROUP_ERR,
    /* A3 */ EVEX_GROUP_ERR,
    /* A4 */ EVEX_GROUP_ERR,
    /* A5 */ EVEX_GROUP_ERR,
    /* A6 */ EVEX_GROUP_ERR,
    /* A7 */ EVEX_GROUP_ERR,
    /* A8 */ EVEX_GROUP_ERR,
    /* A9 */ EVEX_GROUP_ERR,
    /* AA */ EVEX_GROUP_ERR,
    /* AB */ EVEX_GROUP_ERR,
    /* AC */ EVEX_GROUP_ERR,
    /* AD */ EVEX_GROUP_ERR,
    /* AE */ EVEX_GROUP_ERR,
    /* AF */ EVEX_GROUP_ERR,
    /* B0 */ EVEX_GROUP_ERR,
    /* B1 */ EVEX_GROUP_ERR,
    /* B2 */ EVEX_GROUP_ERR,
    /* B3 */ EVEX_GROUP_ERR,
    /* B4 */ EVEX_GROUP_ERR,
    /* B5 */ EVEX_GROUP_ERR,
    /* B6 */ EVEX_GROUP_ERR,
    /* B7 */ EVEX_GROUP_ERR,
    /* B8 */ EVEX_GROUP_ERR,
    /* B9 */ EVEX_GROUP_ERR,
    /* BA */ EVEX_GROUP_ERR,
    /* BB */ EVEX_GROUP_ERR,
    /* BC */ EVEX_GROUP_ERR,
    /* BD */ EVEX_GROUP_ERR,
    /* BE */ EVEX_GROUP_ERR,
    /* BF */ EVEX_GROUP_ERR,
    /* C0 */ EVEX_GROUP_ERR,
    /* C1 */ EVEX_GROUP_ERR,
    /* C2 */ EVEX_0F3AC2,
    /* C3 */ EVEX_GROUP_ERR,
    /* C4 */ EVEX_GROUP_ERR,
    /* C5 */ EVEX_GROUP_ERR,
    /* C6 */ EVEX_GROUP_ERR,
    /* C7 */ EVEX_GROUP_ERR,
    /* C8 */ EVEX_GROUP_ERR,
    /* C9 */ EVEX_GROUP_ERR,
    /* CA */ EVEX_GROUP_ERR,
    /* CB */ EVEX_GROUP_ERR,
    /* CC */ EVEX_GROUP_ERR,
    /* CD */ EVEX_GROUP_ERR,
    /* CE */ EVEX_0F3ACE,
    /* CF */ EVEX_0F3ACF,
    /* D0 */ EVEX_GROUP_ERR,
    /* D1 */ EVEX_GROUP_ERR,
    /* D2 */ EVEX_GROUP_ERR,
    /* D3 */ EVEX_GROUP_ERR,
    /* D4 */ EVEX_GROUP_ERR,
    /* D5 */ EVEX_GROUP_ERR,
    /* D6 */ EVEX_GROUP_ERR,
    /* D7 */ EVEX_GROUP_ERR,
    /* D8 */ EVEX_GROUP_ERR,
    /* D9 */ EVEX_GROUP_ERR,
    /* DA */ EVEX_GROUP_ERR,
    /* DB */ EVEX_GROUP_ERR,
    /* DC */ EVEX_GROUP_ERR,
    /* DD */ EVEX_GROUP_ERR,
    /* DE */ EVEX_GROUP_ERR,
    /* DF */ EVEX_GROUP_ERR,
    /* E0 */ EVEX_GROUP_ERR,
    /* E1 */ EVEX_GROUP_ERR,
    /* E2 */ EVEX_GROUP_ERR,
    /* E3 */ EVEX_GROUP_ERR,
    /* E4 */ EVEX_GROUP_ERR,
    /* E5 */ EVEX_GROUP_ERR,
    /* E6 */ EVEX_GROUP_ERR,
    /* E7 */ EVEX_GROUP_ERR,
    /* E8 */ EVEX_GROUP_ERR,
    /* E9 */ EVEX_GROUP_ERR,
    /* EA */ EVEX_GROUP_ERR,
    /* EB */ EVEX_GROUP_ERR,
    /* EC */ EVEX_GROUP_ERR,
    /* ED */ EVEX_GROUP_ERR,
    /* EE */ EVEX_GROUP_ERR,
    /* EF */ EVEX_GROUP_ERR,
    /* F0 */ EVEX_GROUP_ERR,
    /* F1 */ EVEX_GROUP_ERR,
    /* F2 */ EVEX_GROUP_ERR,
    /* F3 */ EVEX_GROUP_ERR,
    /* F4 */ EVEX_GROUP_ERR,
    /* F5 */ EVEX_GROUP_ERR,
    /* F6 */ EVEX_GROUP_ERR,
    /* F7 */ EVEX_GROUP_ERR,
    /* F8 */ EVEX_GROUP_ERR,
    /* F9 */ EVEX_GROUP_ERR,
    /* FA */ EVEX_GROUP_ERR,
    /* FB */ EVEX_GROUP_ERR,
    /* FC */ EVEX_GROUP_ERR,
    /* FD */ EVEX_GROUP_ERR,
    /* FE */ EVEX_GROUP_ERR,
    /* FF */ EVEX_GROUP_ERR,
    // ---- map 4 (unused) ----
    /* 00 */ EVEX_GROUP_ERR,
    /* 01 */ EVEX_GROUP_ERR,
    /* 02 */ EVEX_GROUP_ERR,
    /* 03 */ EVEX_GROUP_ERR,
    /* 04 */ EVEX_GROUP_ERR,
    /* 05 */ EVEX_GROUP_ERR,
    /* 06 */ EVEX_GROUP_ERR,
    /* 07 */ EVEX_GROUP_ERR,
    /* 08 */ EVEX_GROUP_ERR,
    /* 09 */ EVEX_GROUP_ERR,
    /* 0A */ EVEX_GROUP_ERR,
    /* 0B */ EVEX_GROUP_ERR,
    /* 0C */ EVEX_GROUP_ERR,
    /* 0D */ EVEX_GROUP_ERR,
    /* 0E */ EVEX_GROUP_ERR,
    /* 0F */ EVEX_GROUP_ERR,
    /* 10 */ EVEX_MAP5_10,
    /* 11 */ EVEX_MAP5_11,
    /* 12 */ EVEX_GROUP_ERR,
    /* 13 */ EVEX_GROUP_ERR,
    /* 14 */ EVEX_GROUP_ERR,
    /* 15 */ EVEX_GROUP_ERR,
    /* 16 */ EVEX_GROUP_ERR,
    /* 17 */ EVEX_GROUP_ERR,
    /* 18 */ EVEX_MAP5_18,
    /* 19 */ EVEX_GROUP_ERR,
    /* 1A */ EVEX_GROUP_ERR,
    /* 1B */ EVEX_MAP5_1B,
    /* 1C */ EVEX_GROUP_ERR,
    /* 1D */ EVEX_MAP5_1D,
    /* 1E */ EVEX_MAP5_1E,
    /* 1F */ EVEX_GROUP_ERR,
    /* 20 */ EVEX_GROUP_ERR,
    /* 21 */ EVEX_GROUP_ERR,
    /* 22 */ EVEX_GROUP_ERR,
    /* 23 */ EVEX_GROUP_ERR,
    /* 24 */ EVEX_GROUP_ERR,
    /* 25 */ EVEX_GROUP_ERR,
    /* 26 */ EVEX_GROUP_ERR,
    /* 27 */ EVEX_GROUP_ERR,
    /* 28 */ EVEX_GROUP_ERR,
    /* 29 */ EVEX_GROUP_ERR,
    /* 2A */ EVEX_MAP5_2A,
    /* 2B */ EVEX_GROUP_ERR,
    /* 2C */ EVEX_MAP5_2C,
    /* 2D */ EVEX_MAP5_2D,
    /* 2E */ EVEX_MAP5_2E,
    /* 2F */ EVEX_MAP5_2F,
    /* 30 */ EVEX_GROUP_ERR,
    /* 31 */ EVEX_GROUP_ERR,
    /* 32 */ EVEX_GROUP_ERR,
    /* 33 */ EVEX_GROUP_ERR,
    /* 34 */ EVEX_GROUP_ERR,
    /* 35 */ EVEX_GROUP_ERR,
    /* 36 */ EVEX_GROUP_ERR,
    /* 37 */ EVEX_GROUP_ERR,
    /* 38 */ EVEX_GROUP_ERR,
    /* 39 */ EVEX_GROUP_ERR,
    /* 3A */ EVEX_GROUP_ERR,
    /* 3B */ EVEX_GROUP_ERR,
    /* 3C */ EVEX_GROUP_ERR,
    /* 3D */ EVEX_GROUP_ERR,
    /* 3E */ EVEX_GROUP_ERR,
    /* 3F */ EVEX_GROUP_ERR,
    /* 40 */ EVEX_GROUP_ERR,
    /* 41 */ EVEX_GROUP_ERR,
    /* 42 */ EVEX_GROUP_ERR,
    /* 43 */ EVEX_GROUP_ERR,
    /* 44 */ EVEX_GROUP_ERR,
    /* 45 */ EVEX_GROUP_ERR,
    /* 46 */ EVEX_GROUP_ERR,
    /* 47 */ EVEX_GROUP_ERR,
    /* 48 */ EVEX_GROUP_ERR,
    /* 49 */ EVEX_GROUP_ERR,
    /* 4A */ EVEX_GROUP_ERR,
    /* 4B */ EVEX_GROUP_ERR,
    /* 4C */ EVEX_GROUP_ERR,
    /* 4D */ EVEX_GROUP_ERR,
    /* 4E */ EVEX_GROUP_ERR,
    /* 4F */ EVEX_GROUP_ERR,
    /* 50 */ EVEX_GROUP_ERR,
    /* 51 */ EVEX_MAP5_51,
    /* 52 */ EVEX_GROUP_ERR,
    /* 53 */ EVEX_GROUP_ERR,
    /* 54 */ EVEX_GROUP_ERR,
    /* 55 */ EVEX_GROUP_ERR,
    /* 56 */ EVEX_GROUP_ERR,
    /* 57 */ EVEX_GROUP_ERR,
    /* 58 */ EVEX_MAP5_58,
    /* 59 */ EVEX_MAP5_59,
    /* 5A */ EVEX_MAP5_5A,
    /* 5B */ EVEX_MAP5_5B,
    /* 5C */ EVEX_MAP5_5C,
    /* 5D */ EVEX_MAP5_5D,
    /* 5E */ EVEX_MAP5_5E,
    /* 5F */ EVEX_MAP5_5F,
    /* 60 */ EVEX_GROUP_ERR,
    /* 61 */ EVEX_GROUP_ERR,
    /* 62 */ EVEX_GROUP_ERR,
    /* 63 */ EVEX_GROUP_ERR,
    /* 64 */ EVEX_GROUP_ERR,
    /* 65 */ EVEX_GROUP_ERR,
    /* 66 */ EVEX_GROUP_ERR,
    /* 67 */ EVEX_GROUP_ERR,
    /* 68 */ EVEX_MAP5_68,
    /* 69 */ EVEX_MAP5_69,
    /* 6A */ EVEX_MAP5_6A,
    /* 6B */ EVEX_MAP5_6B,
    /* 6C */ EVEX_MAP5_6C,
    /* 6D */ EVEX_MAP5_6D,
    /* 6E */ EVEX_MAP5_6E,
    /* 6F */ EVEX_MAP5_6F,
    /* 70 */ EVEX_GROUP_ERR,
    /* 71 */ EVEX_GROUP_ERR,
    /* 72 */ EVEX_GROUP_ERR,
    /* 73 */ EVEX_GROUP_ERR,
    /* 74 */ EVEX_MAP5_74,
    /* 75 */ EVEX_GROUP_ERR,
    /* 76 */ EVEX_GROUP_ERR,
    /* 77 */ EVEX_GROUP_ERR,
    /* 78 */ EVEX_MAP5_78,
    /* 79 */ EVEX_MAP5_79,
    /* 7A */ EVEX_MAP5_7A,
    /* 7B */ EVEX_MAP5_7B,
    /* 7C */ EVEX_MAP5_7C,
    /* 7D */ EVEX_MAP5_7D,
    /* 7E */ EVEX_MAP5_7E,
    /* 7F */ EVEX_GROUP_ERR,
    /* 80 */ EVEX_GROUP_ERR,
    /* 81 */ EVEX_GROUP_ERR,
    /* 82 */ EVEX_GROUP_ERR,
    /* 83 */ EVEX_GROUP_ERR,
    /* 84 */ EVEX_GROUP_ERR,
    /* 85 */ EVEX_GROUP_ERR,
    /* 86 */ EVEX_GROUP_ERR,
    /* 87 */ EVEX_GROUP_ERR,
    /* 88 */ EVEX_GROUP_ERR,
    /* 89 */ EVEX_GROUP_ERR,
    /* 8A */ EVEX_GROUP_ERR,
    /* 8B */ EVEX_GROUP_ERR,
    /* 8C */ EVEX_GROUP_ERR,
    /* 8D */ EVEX_GROUP_ERR,
    /* 8E */ EVEX_GROUP_ERR,
    /* 8F */ EVEX_GROUP_ERR,
    /* 90 */ EVEX_GROUP_ERR,
    /* 91 */ EVEX_GROUP_ERR,
    /* 92 */ EVEX_GROUP_ERR,
    /* 93 */ EVEX_GROUP_ERR,
    /* 94 */ EVEX_GROUP_ERR,
    /* 95 */ EVEX_GROUP_ERR,
    /* 96 */ EVEX_GROUP_ERR,
    /* 97 */ EVEX_GROUP_ERR,
    /* 98 */ EVEX_GROUP_ERR,
    /* 99 */ EVEX_GROUP_ERR,
    /* 9A */ EVEX_GROUP_ERR,
    /* 9B */ EVEX_GROUP_ERR,
    /* 9C */ EVEX_GROUP_ERR,
    /* 9D */ EVEX_GROUP_ERR,
    /* 9E */ EVEX_GROUP_ERR,
    /* 9F */ EVEX_GROUP_ERR,
    /* A0 */ EVEX_GROUP_ERR,
    /* A1 */ EVEX_GROUP_ERR,
    /* A2 */ EVEX_GROUP_ERR,
    /* A3 */ EVEX_GROUP_ERR,
    /* A4 */ EVEX_GROUP_ERR,
    /* A5 */ EVEX_GROUP_ERR,
    /* A6 */ EVEX_GROUP_ERR,
    /* A7 */ EVEX_GROUP_ERR,
    /* A8 */ EVEX_GROUP_ERR,
    /* A9 */ EVEX_GROUP_ERR,
    /* AA */ EVEX_GROUP_ERR,
    /* AB */ EVEX_GROUP_ERR,
    /* AC */ EVEX_GROUP_ERR,
    /* AD */ EVEX_GROUP_ERR,
    /* AE */ EVEX_GROUP_ERR,
    /* AF */ EVEX_GROUP_ERR,
    /* B0 */ EVEX_GROUP_ERR,
    /* B1 */ EVEX_GROUP_ERR,
    /* B2 */ EVEX_GROUP_ERR,
    /* B3 */ EVEX_GROUP_ERR,
    /* B4 */ EVEX_GROUP_ERR,
    /* B5 */ EVEX_GROUP_ERR,
    /* B6 */ EVEX_GROUP_ERR,
    /* B7 */ EVEX_GROUP_ERR,
    /* B8 */ EVEX_GROUP_ERR,
    /* B9 */ EVEX_GROUP_ERR,
    /* BA */ EVEX_GROUP_ERR,
    /* BB */ EVEX_GROUP_ERR,
    /* BC */ EVEX_GROUP_ERR,
    /* BD */ EVEX_GROUP_ERR,
    /* BE */ EVEX_GROUP_ERR,
    /* BF */ EVEX_GROUP_ERR,
    /* C0 */ EVEX_GROUP_ERR,
    /* C1 */ EVEX_GROUP_ERR,
    /* C2 */ EVEX_GROUP_ERR,
    /* C3 */ EVEX_GROUP_ERR,
    /* C4 */ EVEX_GROUP_ERR,
    /* C5 */ EVEX_GROUP_ERR,
    /* C6 */ EVEX_GROUP_ERR,
    /* C7 */ EVEX_GROUP_ERR,
    /* C8 */ EVEX_GROUP_ERR,
    /* C9 */ EVEX_GROUP_ERR,
    /* CA */ EVEX_GROUP_ERR,
    /* CB */ EVEX_GROUP_ERR,
    /* CC */ EVEX_GROUP_ERR,
    /* CD */ EVEX_GROUP_ERR,
    /* CE */ EVEX_GROUP_ERR,
    /* CF */ EVEX_GROUP_ERR,
    /* D0 */ EVEX_GROUP_ERR,
    /* D1 */ EVEX_GROUP_ERR,
    /* D2 */ EVEX_GROUP_ERR,
    /* D3 */ EVEX_GROUP_ERR,
    /* D4 */ EVEX_GROUP_ERR,
    /* D5 */ EVEX_GROUP_ERR,
    /* D6 */ EVEX_GROUP_ERR,
    /* D7 */ EVEX_GROUP_ERR,
    /* D8 */ EVEX_GROUP_ERR,
    /* D9 */ EVEX_GROUP_ERR,
    /* DA */ EVEX_GROUP_ERR,
    /* DB */ EVEX_GROUP_ERR,
    /* DC */ EVEX_GROUP_ERR,
    /* DD */ EVEX_GROUP_ERR,
    /* DE */ EVEX_GROUP_ERR,
    /* DF */ EVEX_GROUP_ERR,
    /* E0 */ EVEX_GROUP_ERR,
    /* E1 */ EVEX_GROUP_ERR,
    /* E2 */ EVEX_GROUP_ERR,
    /* E3 */ EVEX_GROUP_ERR,
    /* E4 */ EVEX_GROUP_ERR,
    /* E5 */ EVEX_GROUP_ERR,
    /* E6 */ EVEX_GROUP_ERR,
    /* E7 */ EVEX_GROUP_ERR,
    /* E8 */ EVEX_GROUP_ERR,
    /* E9 */ EVEX_GROUP_ERR,
    /* EA */ EVEX_GROUP_ERR,
    /* EB */ EVEX_GROUP_ERR,
    /* EC */ EVEX_GROUP_ERR,
    /* ED */ EVEX_GROUP_ERR,
    /* EE */ EVEX_GROUP_ERR,
    /* EF */ EVEX_GROUP_ERR,
    /* F0 */ EVEX_GROUP_ERR,
    /* F1 */ EVEX_GROUP_ERR,
    /* F2 */ EVEX_GROUP_ERR,
    /* F3 */ EVEX_GROUP_ERR,
    /* F4 */ EVEX_GROUP_ERR,
    /* F5 */ EVEX_GROUP_ERR,
    /* F6 */ EVEX_GROUP_ERR,
    /* F7 */ EVEX_GROUP_ERR,
    /* F8 */ EVEX_GROUP_ERR,
    /* F9 */ EVEX_GROUP_ERR,
    /* FA */ EVEX_GROUP_ERR,
    /* FB */ EVEX_GROUP_ERR,
    /* FC */ EVEX_GROUP_ERR,
    /* FD */ EVEX_GROUP_ERR,
    /* FE */ EVEX_GROUP_ERR,
    /* FF */ EVEX_GROUP_ERR,
    // ---- map 5 (MAP5) ----
    /* 00 */ EVEX_GROUP_ERR,
    /* 01 */ EVEX_GROUP_ERR,
    /* 02 */ EVEX_GROUP_ERR,
    /* 03 */ EVEX_GROUP_ERR,
    /* 04 */ EVEX_GROUP_ERR,
    /* 05 */ EVEX_GROUP_ERR,
    /* 06 */ EVEX_GROUP_ERR,
    /* 07 */ EVEX_GROUP_ERR,
    /* 08 */ EVEX_GROUP_ERR,
    /* 09 */ EVEX_GROUP_ERR,
    /* 0A */ EVEX_GROUP_ERR,
    /* 0B */ EVEX_GROUP_ERR,
    /* 0C */ EVEX_GROUP_ERR,
    /* 0D */ EVEX_GROUP_ERR,
    /* 0E */ EVEX_GROUP_ERR,
    /* 0F */ EVEX_GROUP_ERR,
    /* 10 */ EVEX_GROUP_ERR,
    /* 11 */ EVEX_GROUP_ERR,
    /* 12 */ EVEX_GROUP_ERR,
    /* 13 */ EVEX_MAP6_13,
    /* 14 */ EVEX_GROUP_ERR,
    /* 15 */ EVEX_GROUP_ERR,
    /* 16 */ EVEX_GROUP_ERR,
    /* 17 */ EVEX_GROUP_ERR,
    /* 18 */ EVEX_GROUP_ERR,
    /* 19 */ EVEX_GROUP_ERR,
    /* 1A */ EVEX_GROUP_ERR,
    /* 1B */ EVEX_GROUP_ERR,
    /* 1C */ EVEX_GROUP_ERR,
    /* 1D */ EVEX_GROUP_ERR,
    /* 1E */ EVEX_GROUP_ERR,
    /* 1F */ EVEX_GROUP_ERR,
    /* 20 */ EVEX_GROUP_ERR,
    /* 21 */ EVEX_GROUP_ERR,
    /* 22 */ EVEX_GROUP_ERR,
    /* 23 */ EVEX_GROUP_ERR,
    /* 24 */ EVEX_GROUP_ERR,
    /* 25 */ EVEX_GROUP_ERR,
    /* 26 */ EVEX_GROUP_ERR,
    /* 27 */ EVEX_GROUP_ERR,
    /* 28 */ EVEX_GROUP_ERR,
    /* 29 */ EVEX_GROUP_ERR,
    /* 2A */ EVEX_GROUP_ERR,
    /* 2B */ EVEX_GROUP_ERR,
    /* 2C */ EVEX_MAP6_2C,
    /* 2D */ EVEX_MAP6_2D,
    /* 2E */ EVEX_GROUP_ERR,
    /* 2F */ EVEX_GROUP_ERR,
    /* 30 */ EVEX_GROUP_ERR,
    /* 31 */ EVEX_GROUP_ERR,
    /* 32 */ EVEX_GROUP_ERR,
    /* 33 */ EVEX_GROUP_ERR,
    /* 34 */ EVEX_GROUP_ERR,
    /* 35 */ EVEX_GROUP_ERR,
    /* 36 */ EVEX_GROUP_ERR,
    /* 37 */ EVEX_GROUP_ERR,
    /* 38 */ EVEX_GROUP_ERR,
    /* 39 */ EVEX_GROUP_ERR,
    /* 3A */ EVEX_GROUP_ERR,
    /* 3B */ EVEX_GROUP_ERR,
    /* 3C */ EVEX_GROUP_ERR,
    /* 3D */ EVEX_GROUP_ERR,
    /* 3E */ EVEX_GROUP_ERR,
    /* 3F */ EVEX_GROUP_ERR,
    /* 40 */ EVEX_GROUP_ERR,
    /* 41 */ EVEX_GROUP_ERR,
    /* 42 */ EVEX_MAP6_42,
    /* 43 */ EVEX_MAP6_43,
    /* 44 */ EVEX_GROUP_ERR,
    /* 45 */ EVEX_GROUP_ERR,
    /* 46 */ EVEX_GROUP_ERR,
    /* 47 */ EVEX_GROUP_ERR,
    /* 48 */ EVEX_GROUP_ERR,
    /* 49 */ EVEX_GROUP_ERR,
    /* 4A */ EVEX_GROUP_ERR,
    /* 4B */ EVEX_GROUP_ERR,
    /* 4C */ EVEX_MAP6_4C,
    /* 4D */ EVEX_MAP6_4D,
    /* 4E */ EVEX_MAP6_4E,
    /* 4F */ EVEX_MAP6_4F,
    /* 50 */ EVEX_GROUP_ERR,
    /* 51 */ EVEX_GROUP_ERR,
    /* 52 */ EVEX_GROUP_ERR,
    /* 53 */ EVEX_GROUP_ERR,
    /* 54 */ EVEX_GROUP_ERR,
    /* 55 */ EVEX_GROUP_ERR,
    /* 56 */ EVEX_MAP6_56,
    /* 57 */ EVEX_MAP6_57,
    /* 58 */ EVEX_GROUP_ERR,
    /* 59 */ EVEX_GROUP_ERR,
    /* 5A */ EVEX_GROUP_ERR,
    /* 5B */ EVEX_GROUP_ERR,
    /* 5C */ EVEX_GROUP_ERR,
    /* 5D */ EVEX_GROUP_ERR,
    /* 5E */ EVEX_GROUP_ERR,
    /* 5F */ EVEX_GROUP_ERR,
    /* 60 */ EVEX_GROUP_ERR,
    /* 61 */ EVEX_GROUP_ERR,
    /* 62 */ EVEX_GROUP_ERR,
    /* 63 */ EVEX_GROUP_ERR,
    /* 64 */ EVEX_GROUP_ERR,
    /* 65 */ EVEX_GROUP_ERR,
    /* 66 */ EVEX_GROUP_ERR,
    /* 67 */ EVEX_GROUP_ERR,
    /* 68 */ EVEX_GROUP_ERR,
    /* 69 */ EVEX_GROUP_ERR,
    /* 6A */ EVEX_GROUP_ERR,
    /* 6B */ EVEX_GROUP_ERR,
    /* 6C */ EVEX_GROUP_ERR,
    /* 6D */ EVEX_GROUP_ERR,
    /* 6E */ EVEX_GROUP_ERR,
    /* 6F */ EVEX_GROUP_ERR,
    /* 70 */ EVEX_GROUP_ERR,
    /* 71 */ EVEX_GROUP_ERR,
    /* 72 */ EVEX_GROUP_ERR,
    /* 73 */ EVEX_GROUP_ERR,
    /* 74 */ EVEX_GROUP_ERR,
    /* 75 */ EVEX_GROUP_ERR,
    /* 76 */ EVEX_GROUP_ERR,
    /* 77 */ EVEX_GROUP_ERR,
    /* 78 */ EVEX_GROUP_ERR,
    /* 79 */ EVEX_GROUP_ERR,
    /* 7A */ EVEX_GROUP_ERR,
    /* 7B */ EVEX_GROUP_ERR,
    /* 7C */ EVEX_GROUP_ERR,
    /* 7D */ EVEX_GROUP_ERR,
    /* 7E */ EVEX_GROUP_ERR,
    /* 7F */ EVEX_GROUP_ERR,
    /* 80 */ EVEX_GROUP_ERR,
    /* 81 */ EVEX_GROUP_ERR,
    /* 82 */ EVEX_GROUP_ERR,
    /* 83 */ EVEX_GROUP_ERR,
    /* 84 */ EVEX_GROUP_ERR,
    /* 85 */ EVEX_GROUP_ERR,
    /* 86 */ EVEX_GROUP_ERR,
    /* 87 */ EVEX_GROUP_ERR,
    /* 88 */ EVEX_GROUP_ERR,
    /* 89 */ EVEX_GROUP_ERR,
    /* 8A */ EVEX_GROUP_ERR,
    /* 8B */ EVEX_GROUP_ERR,
    /* 8C */ EVEX_GROUP_ERR,
    /* 8D */ EVEX_GROUP_ERR,
    /* 8E */ EVEX_GROUP_ERR,
    /* 8F */ EVEX_GROUP_ERR,
    /* 90 */ EVEX_GROUP_ERR,
    /* 91 */ EVEX_GROUP_ERR,
    /* 92 */ EVEX_GROUP_ERR,
    /* 93 */ EVEX_GROUP_ERR,
    /* 94 */ EVEX_GROUP_ERR,
    /* 95 */ EVEX_GROUP_ERR,
    /* 96 */ EVEX_MAP6_96,
    /* 97 */ EVEX_MAP6_97,
    /* 98 */ EVEX_MAP6_98,
    /* 99 */ EVEX_MAP6_99,
    /* 9A */ EVEX_MAP6_9A,
    /* 9B */ EVEX_MAP6_9B,
    /* 9C */ EVEX_MAP6_9C,
    /* 9D */ EVEX_MAP6_9D,
    /* 9E */ EVEX_MAP6_9E,
    /* 9F */ EVEX_MAP6_9F,
    /* A0 */ EVEX_GROUP_ERR,
    /* A1 */ EVEX_GROUP_ERR,
    /* A2 */ EVEX_GROUP_ERR,
    /* A3 */ EVEX_GROUP_ERR,
    /* A4 */ EVEX_GROUP_ERR,
    /* A5 */ EVEX_GROUP_ERR,
    /* A6 */ EVEX_MAP6_A6,
    /* A7 */ EVEX_MAP6_A7,
    /* A8 */ EVEX_MAP6_A8,
    /* A9 */ EVEX_MAP6_A9,
    /* AA */ EVEX_MAP6_AA,
    /* AB */ EVEX_MAP6_AB,
    /* AC */ EVEX_MAP6_AC,
    /* AD */ EVEX_MAP6_AD,
    /* AE */ EVEX_MAP6_AE,
    /* AF */ EVEX_MAP6_AF,
    /* B0 */ EVEX_GROUP_ERR,
    /* B1 */ EVEX_GROUP_ERR,
    /* B2 */ EVEX_GROUP_ERR,
    /* B3 */ EVEX_GROUP_ERR,
    /* B4 */ EVEX_GROUP_ERR,
    /* B5 */ EVEX_GROUP_ERR,
    /* B6 */ EVEX_MAP6_B6,
    /* B7 */ EVEX_MAP6_B7,
    /* B8 */ EVEX_MAP6_B8,
    /* B9 */ EVEX_MAP6_B9,
    /* BA */ EVEX_MAP6_BA,
    /* BB */ EVEX_MAP6_BB,
    /* BC */ EVEX_MAP6_BC,
    /* BD */ EVEX_MAP6_BD,
    /* BE */ EVEX_MAP6_BE,
    /* BF */ EVEX_MAP6_BF,
    /* C0 */ EVEX_GROUP_ERR,
    /* C1 */ EVEX_GROUP_ERR,
    /* C2 */ EVEX_GROUP_ERR,
    /* C3 */ EVEX_GROUP_ERR,
    /* C4 */ EVEX_GROUP_ERR,
    /* C5 */ EVEX_GROUP_ERR,
    /* C6 */ EVEX_GROUP_ERR,
    /* C7 */ EVEX_GROUP_ERR,
    /* C8 */ EVEX_GROUP_ERR,
    /* C9 */ EVEX_GROUP_ERR,
    /* CA */ EVEX_GROUP_ERR,
    /* CB */ EVEX_GROUP_ERR,
    /* CC */ EVEX_GROUP_ERR,
    /* CD */ EVEX_GROUP_ERR,
    /* CE */ EVEX_GROUP_ERR,
    /* CF */ EVEX_GROUP_ERR,
    /* D0 */ EVEX_GROUP_ERR,
    /* D1 */ EVEX_GROUP_ERR,
    /* D2 */ EVEX_GROUP_ERR,
    /* D3 */ EVEX_GROUP_ERR,
    /* D4 */ EVEX_GROUP_ERR,
    /* D5 */ EVEX_GROUP_ERR,
    /* D6 */ EVEX_MAP6_D6,
    /* D7 */ EVEX_MAP6_D7,
    /* D8 */ EVEX_GROUP_ERR,
    /* D9 */ EVEX_GROUP_ERR,
    /* DA */ EVEX_GROUP_ERR,
    /* DB */ EVEX_GROUP_ERR,
    /* DC */ EVEX_GROUP_ERR,
    /* DD */ EVEX_GROUP_ERR,
    /* DE */ EVEX_GROUP_ERR,
    /* DF */ EVEX_GROUP_ERR,
    /* E0 */ EVEX_GROUP_ERR,
    /* E1 */ EVEX_GROUP_ERR,
    /* E2 */ EVEX_GROUP_ERR,
    /* E3 */ EVEX_GROUP_ERR,
    /* E4 */ EVEX_GROUP_ERR,
    /* E5 */ EVEX_GROUP_ERR,
    /* E6 */ EVEX_GROUP_ERR,
    /* E7 */ EVEX_GROUP_ERR,
    /* E8 */ EVEX_GROUP_ERR,
    /* E9 */ EVEX_GROUP_ERR,
    /* EA */ EVEX_GROUP_ERR,
    /* EB */ EVEX_GROUP_ERR,
    /* EC */ EVEX_GROUP_ERR,
    /* ED */ EVEX_GROUP_ERR,
    /* EE */ EVEX_GROUP_ERR,
    /* EF */ EVEX_GROUP_ERR,
    /* F0 */ EVEX_GROUP_ERR,
    /* F1 */ EVEX_GROUP_ERR,
    /* F2 */ EVEX_GROUP_ERR,
    /* F3 */ EVEX_GROUP_ERR,
    /* F4 */ EVEX_GROUP_ERR,
    /* F5 */ EVEX_GROUP_ERR,
    /* F6 */ EVEX_GROUP_ERR,
    /* F7 */ EVEX_GROUP_ERR,
    /* F8 */ EVEX_GROUP_ERR,
    /* F9 */ EVEX_GROUP_ERR,
    /* FA */ EVEX_GROUP_ERR,
    /* FB */ EVEX_GROUP_ERR,
    /* FC */ EVEX_GROUP_ERR,
    /* FD */ EVEX_GROUP_ERR,
    /* FE */ EVEX_GROUP_ERR,
    /* FF */ EVEX_GROUP_ERR,
];

/// Number of 256-byte map blocks in [`EVEX_TABLE`] (Bochs `256*5`).
pub(crate) const EVEX_MAPS: usize = 5;
