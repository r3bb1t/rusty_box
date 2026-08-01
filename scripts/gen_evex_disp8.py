#!/usr/bin/env python3
"""Generate the EVEX compressed-displacement (disp8*N) tuple table.

An EVEX instruction with a mod=01 memory operand encodes its displacement
scaled: the byte is multiplied by N, where N is the size of the memory
element the instruction actually touches. Without the scaling the effective
address is wrong for every such instruction, which shows up as a spurious
#GP on the aligned moves and as silently wrong data everywhere else.

Bochs derives N in `evex_displ8_compression` (cpu/decoder/fetchdecode32.cc)
from the *type* of the memory operand, plus the vector length, EVEX.b
(broadcast) and EVEX.W. The type comes from the opcode's operand list in
ia_opcodes_evex.def, where each `OP_*` is `BX_FORM_SRC(type, src)` packed as
`(type << 4) | src`.

This script reads all three of those from upstream — the `OP_*` constants and
both enums from fetchdecode.h, the operand lists from ia_opcodes_evex.def —
and emits, for each EVEX opcode rusty implements, the tuple type of its
memory operand. decode64 turns that into N at decode time.

Usage:  python scripts/gen_evex_disp8.py
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DEC = os.path.join(ROOT, "cpp_orig", "bochs", "bench-src", "bochs", "cpu", "decoder")
HDR = os.path.join(DEC, "fetchdecode.h")
DEF = os.path.join(DEC, "ia_opcodes_evex.def")
ENUM = os.path.join(ROOT, "rusty_box_decoder", "src", "opcode.rs")
OUT = os.path.join(ROOT, "rusty_box_decoder", "src", "decoder", "evex_disp8.rs")

# Sources that can name a memory reference; only these carry a scale.
MEM_SRC = {"BX_SRC_RM", "BX_SRC_VECTOR_RM", "BX_SRC_VSIB"}


def read(path):
    with io.open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def parse_enum(text, first_member):
    """Read a C enum whose first member is `first_member` -> {name: value}."""
    m = re.search(r"enum\s*\{([^}]*\b" + first_member + r"\b[^}]*)\}", text, re.S)
    if not m:
        sys.exit(f"could not find the enum containing {first_member}")
    out, nxt = {}, 0
    for line in m.group(1).split(","):
        line = re.sub(r"//.*", "", line).strip()
        if not line:
            continue
        mm = re.match(r"(\w+)\s*(?:=\s*(0x[0-9A-Fa-f]+|\d+))?$", line)
        if not mm:
            continue
        nxt = int(mm.group(2), 0) if mm.group(2) else nxt
        out[mm.group(1)] = nxt
        nxt += 1
    return out


def parse_op_constants(text):
    """OP_Wdq = BX_FORM_SRC(BX_VMM_FULL_VECTOR, BX_SRC_VECTOR_RM) -> (type, src)."""
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
    return {n.lower(): n for n in names}


def main():
    hdr = read(HDR)
    ops = parse_op_constants(hdr)
    vmm = parse_enum(hdr, "BX_VMM_FULL_VECTOR")
    rust_names = rust_opcode_names()

    # Rust-side tuple kinds, named after the Bochs constants they mirror.
    # No BX_GPR8 kind: Bochs's BX_SRC_RM switch has no case for it and falls
    # through to `return 1`, which is what `None` already yields, and no EVEX
    # opcode has a byte-sized GPR memory operand anyway.
    kinds = [
        "FullVector", "FullVectorW", "ScalarByte", "ScalarWord", "ScalarDword",
        "ScalarQword", "Scalar", "HalfVector", "HalfVectorW", "QuarterVector",
        "QuarterVectorW", "EighthVector", "Vec128", "Vec256",
        "Gpr16", "Gpr32", "Gpr64", "None",
    ]
    vmm_to_kind = {
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
    # BX_SRC_RM keeps the GPR sizes, which Bochs handles before the vector
    # switch: 8 for GPR64, 4 for GPR32, 2 for GPR16, else 1.
    gpr_to_kind = {
        "BX_GPR8": "None", "BX_GPR16": "Gpr16",
        "BX_GPR32": "Gpr32", "BX_GPR64": "Gpr64",
    }

    entries, unmapped = [], set()
    for line in read(DEF).splitlines():
        m = re.match(r"\s*bx_define_opcode\(\s*BX_IA_(EVEX_\w+)\s*,(.*)$", line)
        if not m:
            continue
        bx_name, rest = m.group(1), m.group(2)
        rust = rust_names.get(bx_name.replace("_", "").lower())
        if rust is None:
            continue  # not implemented here; decodes to IaError anyway

        kind = None
        for opname in re.findall(r"\b(OP_\w+)\b", rest):
            if opname not in ops:
                continue
            typ, src = ops[opname]
            if src not in MEM_SRC:
                continue
            if src == "BX_SRC_RM" and typ in gpr_to_kind:
                kind = gpr_to_kind[typ]
            elif typ in vmm_to_kind:
                kind = vmm_to_kind[typ]
            else:
                # BX_SRC_RM with a non-GPR type (x87, MMX, kmask): Bochs falls
                # through its GPR switch and returns 1.
                kind = "None"
            break
        if kind is None:
            continue
        entries.append((rust, kind))

    entries.sort()
    seen, deduped = set(), []
    for rust, kind in entries:
        if rust in seen:
            continue
        seen.add(rust)
        deduped.append((rust, kind))

    out = []
    out.append("//! EVEX compressed displacement (disp8*N) — generated, do not edit.")
    out.append("//!")
    out.append("//! Regenerate with `python scripts/gen_evex_disp8.py`.")
    out.append("//!")
    out.append("//! An EVEX instruction with a mod=01 memory operand stores its")
    out.append("//! displacement divided by N, where N is the size of the memory element")
    out.append("//! it touches. Bochs recovers N in `evex_displ8_compression`")
    out.append("//! (cpu/decoder/fetchdecode32.cc) from the memory operand's type, the")
    out.append("//! vector length, EVEX.b and EVEX.W. This table carries the type; the")
    out.append("//! arithmetic lives in `EvexTuple::scale`.")
    out.append("")
    out.append("use crate::opcode::Opcode;")
    out.append("")
    out.append("/// Memory-operand tuple kind, mirroring Bochs's `BX_VMM_*` constants.")
    out.append("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    out.append("pub(crate) enum EvexTuple {")
    for k in kinds:
        out.append(f"    {k},")
    out.append("}")
    out.append("")
    out.append("impl EvexTuple {")
    out.append("    /// N for this operand. Mirrors `evex_displ8_compression`.")
    out.append("    ///")
    out.append("    /// `vl` is 0/1/2 for 128/256/512-bit, so Bochs's `len` (1/2/4) is")
    out.append("    /// `1 << vl`. `broadcast` is EVEX.b with a memory operand.")
    out.append("    pub(crate) const fn scale(self, vl: u8, broadcast: bool, w: bool) -> u32 {")
    out.append("        let len = 1u32 << vl;")
    out.append("        let w4 = if w { 8 } else { 4 };")
    out.append("        match self {")
    out.append("            Self::Gpr64 => 8,")
    out.append("            Self::Gpr32 => 4,")
    out.append("            Self::Gpr16 => 2,")
    out.append("            Self::None => 1,")
    out.append("            Self::FullVector => {")
    out.append("                if broadcast { w4 } else { 16 * len }")
    out.append("            }")
    out.append("            Self::FullVectorW => {")
    out.append("                if broadcast { 2 } else { 16 * len }")
    out.append("            }")
    out.append("            Self::ScalarByte => 1,")
    out.append("            Self::ScalarWord => 2,")
    out.append("            Self::ScalarDword => 4,")
    out.append("            Self::ScalarQword => 8,")
    out.append("            Self::Scalar => w4,")
    out.append("            Self::HalfVector => {")
    out.append("                if broadcast { w4 } else { 8 * len }")
    out.append("            }")
    out.append("            Self::HalfVectorW => {")
    out.append("                if broadcast { 2 } else { 8 * len }")
    out.append("            }")
    out.append("            Self::QuarterVector => 4 * len,")
    out.append("            Self::QuarterVectorW => {")
    out.append("                if broadcast { 2 } else { 4 * len }")
    out.append("            }")
    out.append("            Self::EighthVector => 2 * len,")
    out.append("            Self::Vec128 => 16,")
    out.append("            Self::Vec256 => 32,")
    out.append("        }")
    out.append("    }")
    out.append("}")
    out.append("")
    out.append("/// Tuple kind of an EVEX opcode's memory operand, or `None` if it has")
    out.append("/// none (register-only forms never carry a scaled displacement).")
    out.append("pub(crate) const fn evex_tuple(op: Opcode) -> EvexTuple {")
    out.append("    match op {")
    for rust, kind in deduped:
        out.append(f"        Opcode::{rust} => EvexTuple::{kind},")
    out.append("        _ => EvexTuple::None,")
    out.append("    }")
    out.append("}")
    out.append("")

    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out))

    from collections import Counter
    hist = Counter(k for _, k in deduped)
    print(f"opcodes with a memory operand : {len(deduped)}")
    for k, n in hist.most_common():
        print(f"    {k:16s} {n}")
    if unmapped:
        print(f"unmapped operand types: {sorted(unmapped)[:10]}")
    print(f"wrote {os.path.relpath(OUT, ROOT)}")


if __name__ == "__main__":
    main()
