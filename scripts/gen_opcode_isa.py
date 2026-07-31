#!/usr/bin/env python3
"""Generate `rusty_box_decoder/src/opcode_isa.rs` from the vendored Bochs tree.

Bochs gates every instruction on a CPUID ISA feature: `ia_opcodes.def` carries
the feature as field 6 of each `bx_define_opcode(...)`, and
`init_FetchDecodeTables()` rewrites the handler to `BxError` when the running
CPU model lacks it. rusty_box needs the same mapping, so this script derives it
from Bochs rather than hand-maintaining ~2900 entries.

Matching is by name: a Bochs `BX_IA_<NAME>` and a rusty `Opcode::<Name>` are the
same instruction when `name.replace('_','').lower()` agrees. Likewise
`BX_ISA_<FEAT>` matches `X86Feature::Isa<Feat>`.

Run from the repo root:

    python scripts/gen_opcode_isa.py

It rewrites the generated file in place and prints a summary. Re-run it after
syncing `cpp_orig/bochs/` or after adding `Opcode` / `X86Feature` variants; the
`opcode_isa_table_matches_bochs` test fails if the file drifts out of date.
"""

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "rusty_box_decoder/src/opcode_isa.rs"
ALWAYS = 0xFFFF

# Opcodes with no Bochs counterpart. They stay ungated, which preserves the
# behaviour rusty_box had before the gate existed — a missing gate is the status
# quo, a wrong gate would #UD a working guest. Listed explicitly so that new
# drift shows up as a diff rather than silently widening this set.
KNOWN_UNMATCHED = {
    # rusty-internal pseudo-opcodes with no Bochs BX_IA_* counterpart.
    "IaError",
    "InsertedOpcode",
    "Int0",
    "Tmmultf32psTnnnTrmTreg",
    # Bochs defines only the masked form BX_IA_EVEX_VPMULTISHIFTQB_..._Kmask;
    # this unmasked variant is a rusty-side invention.
    "EvexVpmultishiftqbVdqHdqWdq",
    # No BX_IA_EVEX_VMINPBF16/VMAXPBF16 exist upstream under any suffix.
    "EvexVminpbf16VphHphWph",
    "EvexVminpbf16VphHphWphKmask",
    "EvexVmaxpbf16VphHphWph",
    "EvexVmaxpbf16VphHphWphKmask",
}


def split_top(s):
    """Split on commas that are not nested inside parens or angle brackets."""
    parts, depth, cur = [], 0, ""
    for ch in s:
        if ch in "(<":
            depth += 1
        elif ch in ")>":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(cur.strip())
            cur = ""
        else:
            cur += ch
    parts.append(cur.strip())
    return parts


def norm(s):
    return s.replace("_", "").lower()


def read(path):
    return (REPO / path).read_text(encoding="utf-8", errors="replace")


def main():
    # Bochs opcode name -> BX_ISA_* (or "0" for base-ISA instructions)
    bochs = {}
    # Bochs opcode name -> the BX_PREPARE_EVEX* encoding-restriction bits
    evex_flags = {}
    for name in ("ia_opcodes.def", "ia_opcodes_evex.def"):
        text = read("cpp_orig/bochs/bochs/cpu/decoder/" + name)
        # Drop trailing line comments first. The entry regex anchors on the
        # closing paren at end of line, and several defs carry a trailing
        # `// ignore the SAE` that would otherwise make the whole entry
        # invisible and silently leave that opcode ungated. No def field
        # contains a literal '//' (the mnemonics are plain strings), so this
        # is safe to strip wholesale.
        text = re.sub(r"//[^\n]*", "", text)
        for m in re.finditer(r"bx_define_opcode\((.*?)\)\s*$", text, re.M):
            fields = split_top(m.group(1))
            if len(fields) < 6:
                continue
            bochs[norm(fields[0].replace("BX_IA_", ""))] = fields[5].split("/*")[0].strip()
            # Field 10 carries the BX_PREPARE_* attributes. Only the EVEX
            # encoding-restriction bits matter here; the rest are decode hints
            # rusty_box does not model.
            attrs = fields[10] if len(fields) > 10 else ""
            flags = 0
            if "BX_PREPARE_EVEX_NO_BROADCAST" in attrs:
                flags |= 0x280
            if "BX_PREPARE_EVEX_NO_SAE" in attrs:
                flags |= 0x180
            if "BX_PREPARE_EVEX" in attrs:
                flags |= 0x080
            evex_flags[norm(fields[0].replace("BX_IA_", ""))] = flags

    # rusty X86Feature variants, declaration order == discriminant
    feat_src = read("rusty_box_decoder/src/features.rs")
    feat_src = feat_src[feat_src.index("pub enum X86Feature"):]
    features = re.findall(r"^\s{4}([A-Z]\w+),\s*$", feat_src, re.M)
    feat_index = {norm(v): i for i, v in enumerate(features)}

    # rusty Opcode variants, declaration order == discriminant
    op_src = read("rusty_box_decoder/src/opcode.rs")
    op_src = op_src[op_src.index("pub enum Opcode"):]
    opcodes = re.findall(r"^\s{8}([A-Z][A-Za-z0-9_]*),\s*$", op_src, re.M)

    table, unmatched, missing_feature, gated = [], [], {}, 0
    for op in opcodes:
        feature = bochs.get(norm(op))
        if feature is None:
            unmatched.append(op)
            table.append((op, ALWAYS, None))
            continue
        if feature == "0":
            table.append((op, ALWAYS, None))
            continue
        key = "isa" + norm(feature.replace("BX_ISA_", ""))
        if key not in feat_index:
            missing_feature[feature] = missing_feature.get(feature, 0) + 1
            table.append((op, ALWAYS, None))
            continue
        table.append((op, feat_index[key], features[feat_index[key]]))
        gated += 1

    if missing_feature:
        print("ERROR: Bochs features with no X86Feature variant:", file=sys.stderr)
        for k, v in sorted(missing_feature.items()):
            print(f"  {v:5d} {k}", file=sys.stderr)
        return 1

    new_unmatched = set(unmatched) - KNOWN_UNMATCHED
    gone = KNOWN_UNMATCHED - set(unmatched)
    if new_unmatched or gone:
        print("ERROR: KNOWN_UNMATCHED is stale.", file=sys.stderr)
        for op in sorted(new_unmatched):
            print(f"  newly unmatched: {op}", file=sys.stderr)
        for op in sorted(gone):
            print(f"  no longer unmatched: {op}", file=sys.stderr)
        return 1

    lines = [
        "//! Per-opcode CPUID/ISA feature gate — GENERATED, DO NOT EDIT BY HAND.",
        "//!",
        "//! Regenerate with `python scripts/gen_opcode_isa.py` after syncing",
        "//! `cpp_orig/bochs/` or adding `Opcode` / `X86Feature` variants.",
        "//!",
        "//! Mirrors the ISA field of Bochs `cpu/decoder/ia_opcodes.def`, which",
        "//! `init_FetchDecodeTables()` uses to point unsupported opcodes at",
        "//! `BxError`. Indexed by `Opcode as usize`; `ISA_ALWAYS` marks an",
        "//! instruction with no feature gate (base ISA), which is also the",
        "//! conservative fallback for the few opcodes Bochs does not define.",
        "",
        "use crate::features::X86Feature;",
        "use crate::opcode::Opcode;",
        "",
        "/// Sentinel: this opcode is not gated on any CPUID feature.",
        "pub const ISA_ALWAYS: u16 = 0xFFFF;",
        "",
        f"/// `X86Feature as u16` required by each opcode ({gated} of {len(opcodes)} are gated).",
        f"pub static OPCODE_ISA: [u16; {len(opcodes)}] = [",
    ]
    for op, value, feat in table:
        if value == ALWAYS:
            lines.append(f"    ISA_ALWAYS, // {op}")
        else:
            lines.append(f"    {value}, // {op} -> X86Feature::{feat}")
    lines += [
        "];",
        "",
        "/// Feature required to execute `opcode`, or `ISA_ALWAYS` if ungated.",
        "#[inline]",
        "pub fn opcode_isa_feature(opcode: Opcode) -> u16 {",
        "    OPCODE_ISA[opcode as usize]",
        "}",
        "",
        "/// Number of opcodes carrying a real feature gate. Asserted by tests so",
        "/// that a silent regeneration drop is caught.",
        f"pub const GATED_OPCODE_COUNT: usize = {gated};",
        "",
        "/// Number of `Opcode` variants the table was generated against. A",
        "/// mismatch with the enum means the table needs regenerating.",
        f"pub const OPCODE_VARIANT_COUNT: usize = {len(opcodes)};",
        "",
        "// EVEX encoding restrictions — Bochs cpu/decoder/fetchdecode.h.",
        "// `EVEX.b` means embedded broadcast on a memory operand and SAE /",
        "// embedded rounding on a register operand; an opcode that supports",
        "// neither must #UD rather than silently ignore the bit.",
        "/// Opcode participates in the EVEX prepare checks at all.",
        "pub const PREPARE_EVEX: u16 = 0x080;",
        "/// `EVEX.b` with a register operand (SAE) is illegal for this opcode.",
        "pub const PREPARE_EVEX_NO_SAE: u16 = 0x180;",
        "/// `EVEX.b` with a memory operand (broadcast) is illegal for this opcode.",
        "pub const PREPARE_EVEX_NO_BROADCAST: u16 = 0x280;",
        "",
        "/// BX_PREPARE_EVEX* attribute bits per opcode, from field 10 of",
        "/// `bx_define_opcode`.",
        "// A `const` rather than a `static`: the EVEX decode path is a",
        "// `const fn`, and const evaluation may read consts but not statics.",
        f"pub const OPCODE_EVEX_FLAGS: [u16; {len(opcodes)}] = [",
    ]
    evex_gated = 0
    for op in opcodes:
        flags = evex_flags.get(norm(op), 0)
        if flags:
            evex_gated += 1
        lines.append(f"    {flags:#05x}, // {op}")
    lines += [
        "];",
        "",
        "/// EVEX prepare attributes for `opcode` (0 when it has none).",
        "#[inline]",
        "pub const fn opcode_evex_flags(opcode: Opcode) -> u16 {",
        "    OPCODE_EVEX_FLAGS[opcode as usize]",
        "}",
        "",
        "/// Number of opcodes carrying EVEX prepare attributes, pinned by tests.",
        f"pub const EVEX_FLAGGED_OPCODE_COUNT: usize = {evex_gated};",
        "",
        "#[allow(dead_code)]",
        "fn _feature_type_is_used(f: X86Feature) -> u16 {",
        "    // Keeps the X86Feature import meaningful: the table stores raw",
        "    // discriminants of exactly this enum.",
        "    f as u16",
        "}",
        "",
    ]
    OUT.write_text("\n".join(lines), encoding="utf-8", newline="")
    print(f"wrote {OUT.relative_to(REPO)}")
    print(f"  opcodes: {len(opcodes)}   gated: {gated}   ungated: {len(opcodes) - gated}")
    print(f"  X86Feature variants: {len(features)}")
    print(f"  unmatched (left ungated): {len(unmatched)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
