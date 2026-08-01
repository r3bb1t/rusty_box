#!/usr/bin/env python3
"""Generate the EVEX per-opcode operand tables.

Two things about an EVEX instruction cannot be derived from its encoding
alone, only from the opcode's operand list:

  * which ModRM field names the register it writes. Most EVEX opcodes write
    the reg field, but the store forms (VEXTRACT*, the truncating VPMOV*
    stores, VCOMPRESS*, VPEXTR*, VSCATTER*) write rm, and the
    shift/rotate-by-immediate groups write EVEX.vvvv.

  * the size of the memory element it touches, which is the N in EVEX's
    compressed displacement: a mod=01 memory operand stores disp8 already
    divided by N.

Upstream keeps both in ia_opcodes_evex.def, where every operand is an `OP_*`
constant defined in fetchdecode.h as `BX_FORM_SRC(type, src)`. The `src` of
the first operand gives the destination field; the `type` of the memory
operand feeds `evex_displ8_compression` (cpu/decoder/fetchdecode32.cc).

Both tables are read from upstream so neither can drift from the definitions
it describes.

Usage:  python scripts/gen_evex_operands.py
"""

import io
import os
import re
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DEC = os.path.join(ROOT, "cpp_orig", "bochs", "bench-src", "bochs", "cpu", "decoder")
HDR = os.path.join(DEC, "fetchdecode.h")
DEF = os.path.join(DEC, "ia_opcodes_evex.def")
ENUM = os.path.join(ROOT, "rusty_box_decoder", "src", "opcode.rs")
OUT = os.path.join(ROOT, "rusty_box_decoder", "src", "decoder", "evex_operands.rs")

OP_RE = re.compile(r"\b(OP_\w+)\b")
DEF_RE = re.compile(r"\s*bx_define_opcode\(\s*BX_IA_(EVEX_\w+)\s*,(.*)$")

# Sources that can name a memory reference; only these carry a disp8 scale.
MEM_SRC = {"BX_SRC_RM", "BX_SRC_VECTOR_RM", "BX_SRC_VSIB"}

# The first operand's source origin says which field holds the destination.
DST_SRC = {
    "BX_SRC_NNN": "Nnn",
    "BX_SRC_RM": "Rm",
    "BX_SRC_VECTOR_RM": "Rm",
    "BX_SRC_VSIB": "Rm",
    "BX_SRC_VVV": "Vvvv",
}

VMM_KIND = {
    "BX_VMM_FULL_VECTOR": "FullVector",
    "BX_VMM_FULL_VECTOR_W": "FullVectorW",
    "BX_VMM_SCALAR_BYTE": "ScalarByte",
    "BX_VMM_SCALAR_WORD": "ScalarWord",
    "BX_VMM_SCALAR_DWORD": "ScalarDword",
    "BX_VMM_SCALAR_QWORD": "ScalarQword",
    "BX_VMM_SCALAR": "Scalar",
    "BX_VMM_HALF_VECTOR": "HalfVector",
    "BX_VMM_HALF_VECTOR_W": "HalfVectorW",
    "BX_VMM_QUARTER_VECTOR": "QuarterVector",
    "BX_VMM_QUARTER_VECTOR_W": "QuarterVectorW",
    "BX_VMM_EIGHTH_VECTOR": "EighthVector",
    "BX_VMM_VEC128": "Vec128",
    "BX_VMM_VEC256": "Vec256",
}

# Bochs handles GPR memory operands before its vector switch; anything else
# falls through to 1, which is what None yields.
GPR_KIND = {
    "BX_GPR8": "None",
    "BX_GPR16": "Gpr16",
    "BX_GPR32": "Gpr32",
    "BX_GPR64": "Gpr64",
}

KINDS = [
    "FullVector", "FullVectorW", "ScalarByte", "ScalarWord", "ScalarDword",
    "ScalarQword", "Scalar", "HalfVector", "HalfVectorW", "QuarterVector",
    "QuarterVectorW", "EighthVector", "Vec128", "Vec256",
    "Gpr16", "Gpr32", "Gpr64", "None",
]


def read(path):
    with io.open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def parse_op_constants(text):
    """OP_Wdq = BX_FORM_SRC(BX_VMM_FULL_VECTOR, BX_SRC_VECTOR_RM)."""
    out = {}
    for m in re.finditer(
        r"const\s+Bit8u\s+(OP_\w+)\s*=\s*BX_FORM_SRC\(\s*(\w+)\s*,\s*(\w+)\s*\)", text
    ):
        out[m.group(1)] = (m.group(2), m.group(3))
    if not out:
        sys.exit("could not parse any OP_* constants from fetchdecode.h")
    return out


def rust_opcode_names():
    names = re.findall(r"^\s+(Evex[A-Za-z0-9]*)\s*,\s*$", read(ENUM), re.M)
    by_ci = {}
    for n in names:
        by_ci.setdefault(n.lower(), []).append(n)
    collisions = {k: v for k, v in by_ci.items() if len(v) > 1}
    if collisions:
        sys.exit(f"opcode enum has case-collisions: {collisions}")
    return {k: v[0] for k, v in by_ci.items()}


def main():
    ops = parse_op_constants(read(HDR))
    rust_names = rust_opcode_names()

    dsts, tuples = {}, {}
    for line in read(DEF).splitlines():
        m = DEF_RE.match(line)
        if not m:
            continue
        rust = rust_names.get(m.group(1).replace("_", "").lower())
        if rust is None:
            continue  # not implemented here; decodes to IaError anyway
        operands = [o for o in OP_RE.findall(m.group(2)) if o in ops]
        if not operands:
            continue

        # Destination: the first operand.
        _typ, src = ops[operands[0]]
        if src in DST_SRC:
            dsts.setdefault(rust, DST_SRC[src])

        # disp8 scale: the first operand that can name memory.
        for opname in operands:
            typ, src = ops[opname]
            if src not in MEM_SRC:
                continue
            if src == "BX_SRC_RM" and typ in GPR_KIND:
                tuples.setdefault(rust, GPR_KIND[typ])
            else:
                tuples.setdefault(rust, VMM_KIND.get(typ, "None"))
            break

    if not dsts or not tuples:
        sys.exit("parsed no operands — the def file or OP_* format changed")

    out = []
    a = out.append
    a("//! EVEX per-opcode operand tables — generated, do not edit.")
    a("//!")
    a("//! Regenerate with `python scripts/gen_evex_operands.py`.")
    a("//!")
    a("//! Both tables come from the operand lists in Bochs's")
    a("//! `cpu/decoder/ia_opcodes_evex.def`, where each `OP_*` is")
    a("//! `BX_FORM_SRC(type, src)`. The first operand's `src` gives the")
    a("//! destination field; the memory operand's `type` gives the disp8 scale")
    a("//! that `evex_displ8_compression` computes upstream.")
    a("")
    a("use crate::opcode::Opcode;")
    a("")
    a("/// Which ModRM field names the register an EVEX opcode writes.")
    a("///")
    a("/// Most write the reg field. The store forms — VEXTRACT*, the truncating")
    a("/// VPMOV* stores, VCOMPRESS*, VPEXTR*, VSCATTER* — write rm, and the")
    a("/// shift/rotate-by-immediate groups write EVEX.vvvv.")
    a("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    a("pub(crate) enum EvexDst {")
    a("    Nnn,")
    a("    Rm,")
    a("    Vvvv,")
    a("}")
    a("")
    a("/// Destination field for an EVEX opcode; the reg field unless listed.")
    a("pub(crate) const fn evex_dst(op: Opcode) -> EvexDst {")
    a("    match op {")
    for rust in sorted(k for k, v in dsts.items() if v != "Nnn"):
        a(f"        Opcode::{rust} => EvexDst::{dsts[rust]},")
    a("        _ => EvexDst::Nnn,")
    a("    }")
    a("}")
    a("")
    a("/// Memory-operand tuple kind, mirroring Bochs's `BX_VMM_*` constants.")
    a("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    a("pub(crate) enum EvexTuple {")
    for k in KINDS:
        a(f"    {k},")
    a("}")
    a("")
    a("impl EvexTuple {")
    a("    /// N for this operand. Mirrors `evex_displ8_compression`.")
    a("    ///")
    a("    /// `vl` is 0/1/2 for 128/256/512-bit, so Bochs's `len` (1/2/4) is")
    a("    /// `1 << vl`. `broadcast` is EVEX.b with a memory operand.")
    a("    pub(crate) const fn scale(self, vl: u8, broadcast: bool, w: bool) -> u32 {")
    a("        let len = 1u32 << vl;")
    a("        let w4 = if w { 8 } else { 4 };")
    a("        match self {")
    a("            Self::Gpr64 => 8,")
    a("            Self::Gpr32 => 4,")
    a("            Self::Gpr16 => 2,")
    a("            Self::None => 1,")
    a("            Self::FullVector => {")
    a("                if broadcast { w4 } else { 16 * len }")
    a("            }")
    a("            Self::FullVectorW => {")
    a("                if broadcast { 2 } else { 16 * len }")
    a("            }")
    a("            Self::ScalarByte => 1,")
    a("            Self::ScalarWord => 2,")
    a("            Self::ScalarDword => 4,")
    a("            Self::ScalarQword => 8,")
    a("            Self::Scalar => w4,")
    a("            Self::HalfVector => {")
    a("                if broadcast { w4 } else { 8 * len }")
    a("            }")
    a("            Self::HalfVectorW => {")
    a("                if broadcast { 2 } else { 8 * len }")
    a("            }")
    a("            Self::QuarterVector => 4 * len,")
    a("            Self::QuarterVectorW => {")
    a("                if broadcast { 2 } else { 4 * len }")
    a("            }")
    a("            Self::EighthVector => 2 * len,")
    a("            Self::Vec128 => 16,")
    a("            Self::Vec256 => 32,")
    a("        }")
    a("    }")
    a("}")
    a("")
    a("/// Tuple kind of an EVEX opcode's memory operand, or `None` if it has")
    a("/// none (register-only forms never carry a scaled displacement).")
    a("pub(crate) const fn evex_tuple(op: Opcode) -> EvexTuple {")
    a("    match op {")
    for rust in sorted(tuples):
        a(f"        Opcode::{rust} => EvexTuple::{tuples[rust]},")
    a("        _ => EvexTuple::None,")
    a("    }")
    a("}")
    a("")

    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out))

    print(f"opcodes with a destination field : {len(dsts)}")
    for k, n in Counter(dsts.values()).most_common():
        print(f"    {k:14s} {n}")
    print(f"opcodes with a memory operand    : {len(tuples)}")
    for k, n in Counter(tuples.values()).most_common():
        print(f"    {k:14s} {n}")
    print(f"wrote {os.path.relpath(OUT, ROOT)}")


if __name__ == "__main__":
    main()
