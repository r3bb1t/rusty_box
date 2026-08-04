#!/usr/bin/env python3
"""Generate the EVEX opcode maps from Bochs's own decoder tables.

Reads cpu/decoder/fetchdecode_opmap_evex.cc — which *is* the table, so the
generated Rust is a transcription rather than a re-derivation — and emits
rusty_box_decoder/src/decoder/opmap_evex.rs.

Bochs shape, reproduced exactly:

  * one `BxOpcodeGroup_EVEX_<name>[]` per (map, opcode byte) that has any
    encoding, each entry a `form_opcode(attrs, opcode)` with the last one
    marked by `last_opcode`;
  * a master `BxOpcodeTableEVEX[256*5]` indexed `(map - 1) * 256 + opcode`,
    with `BxOpcodeGroup_ERR` wherever nothing is defined.

Opcode names are matched to rusty's `Opcode` enum case-insensitively after
dropping underscores: rusty renders `BX_IA_EVEX_VPADDD_VdqHdqWdq` as
`EvexVpadddVdqHdqWdq`, and the two differ only in case, with no collisions.
Names rusty does not have (FP16/BF16/FP8 forms, which Skylake-X does not
advertise) become `Opcode::IaError`, i.e. a guest #UD — the same thing Bochs
produces with those ISA bits off.

Usage:  python scripts/gen_opmap_evex.py
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SRC = os.path.join(
    ROOT, "cpp_orig", "bochs", "bench-src", "bochs", "cpu", "decoder",
    "fetchdecode_opmap_evex.cc",
)
ENUM = os.path.join(ROOT, "rusty_box_decoder", "src", "opcode.rs")
OUT = os.path.join(ROOT, "rusty_box_decoder", "src", "decoder", "opmap_evex.rs")

# Bochs indexes the master table by (map - 1); map 1 = 0F, 2 = 0F38, 3 = 0F3A,
# 5 = MAP5, 6 = MAP6. Slot 3 (map 4) is unused but present, so the table is 5
# blocks of 256.
MAPS = 5

# Bochs attribute names that rusty's tables.rs spells differently.
ATTR_RENAME = {
    "MODC0": "MOD_REG",   # tables.rs: MOD_REG = attr(1, 1, MODC0_OFFSET)
}


def read(path):
    with io.open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def resolve_conditionals(text):
    """Flatten `#if FEATURE / #else / #endif` by keeping the feature-on branch.

    The only conditionals inside these tables are `#if BX_SUPPORT_AMX`, where
    the #else arm is `BxOpcodeGroup_ERR`. We keep the #if arm on purpose: in
    rusty, gating an opcode a model does not advertise is the ISA gate's job
    (it rewrites to IaError at icache fill), so the decoder table should carry
    the encoding. Baking ERR in here instead would make the opcode
    unreachable for any future cpudb model that does advertise AMX — the same
    unreachable-handler bug this table exists to fix.
    """
    out, skipping, depth = [], False, 0
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("#if"):
            depth += 1
            continue
        if s.startswith("#else") and depth:
            skipping = True
            continue
        if s.startswith("#endif"):
            if depth:
                depth -= 1
                skipping = False
            continue
        if not skipping:
            out.append(line)
    return "\n".join(out)


def rust_opcode_names():
    names = re.findall(r"^\s+(Evex[A-Za-z0-9]*)\s*,\s*$", read(ENUM), re.M)
    by_ci = {}
    for n in names:
        by_ci.setdefault(n.lower(), []).append(n)
    collisions = {k: v for k, v in by_ci.items() if len(v) > 1}
    if collisions:
        sys.exit(f"opcode enum has case-collisions, mapping would be ambiguous: {collisions}")
    return {k: v[0] for k, v in by_ci.items()}


def parse_groups(text):
    """-> {group_name: [(attr_expr, bx_opcode_name), ...]} in source order."""
    groups = {}
    pattern = re.compile(
        r"static\s+const\s+Bit64u\s+(BxOpcodeGroup_EVEX_\w+)\s*\[\s*\]\s*=\s*\{(.*?)\};",
        re.S,
    )
    entry = re.compile(
        r"\b(?:form_opcode|last_opcode)\s*\((.*?),\s*BX_IA_(\w+)\s*\)", re.S
    )
    for m in pattern.finditer(text):
        name, body = m.group(1), m.group(2)
        entries = []
        for e in entry.finditer(body):
            attrs = " ".join(e.group(1).split())
            entries.append((attrs, e.group(2)))
        if entries:
            groups[name] = entries
    return groups


def parse_master(text):
    """-> list of 256*MAPS group names ('BxOpcodeGroup_ERR' where undefined)."""
    m = re.search(
        r"const\s+Bit64u\s*\*\s*BxOpcodeTableEVEX\s*\[[^\]]*\]\s*=\s*\{(.*?)\};",
        text, re.S,
    )
    if not m:
        sys.exit("could not find BxOpcodeTableEVEX in the Bochs source")
    slots = re.findall(r"\b(BxOpcodeGroup_\w+)\b", m.group(1))
    if len(slots) != 256 * MAPS:
        sys.exit(f"BxOpcodeTableEVEX has {len(slots)} slots, expected {256 * MAPS}")
    return slots


def rust_attrs(expr):
    """ATTR_VEX_W0 | ATTR_MASK_K0 -> A::VEX_W0.union(A::MASK_K0)

    `union` rather than `|` because `form_opcode` is a const fn taking a typed
    `OpcodeAttrs`, and bitflags' `BitOr` is not const.
    """
    names = [ATTR_RENAME.get(n, n) for n in re.findall(r"ATTR_([A-Z0-9_]+)", expr)]
    if not names:
        return "A::empty()"
    out = f"A::{names[0]}"
    for n in names[1:]:
        out += f".union(A::{n})"
    return out


def main():
    text = resolve_conditionals(strip_comments(read(SRC)))
    groups = parse_groups(text)
    master = parse_master(text)
    rust_names = rust_opcode_names()

    used = sorted({g for g in master if g != "BxOpcodeGroup_ERR"})
    unknown = [g for g in used if g not in groups]
    if unknown:
        sys.exit(f"master table references groups that were not parsed: {unknown[:5]}")

    missing = set()
    total_entries = 0
    out = []
    out.append("//! EVEX opcode maps — generated, do not edit by hand.")
    out.append("//!")
    out.append("//! Regenerate with `python scripts/gen_opmap_evex.py`.")
    out.append("//!")
    out.append("//! Transcribed from Bochs `cpu/decoder/fetchdecode_opmap_evex.cc`,")
    out.append("//! which is itself the table: one group per (map, opcode byte), each")
    out.append("//! entry a `form_opcode(attrs, opcode)`, selected by the same decmask")
    out.append("//! machinery `tables.rs` already implements. The master table is")
    out.append("//! indexed `(map - 1) * 256 + opcode`, matching `BxOpcodeTableEVEX`.")
    out.append("//!")
    out.append("//! Opcodes rusty does not implement (FP16/BF16/FP8 forms, which")
    out.append("//! Skylake-X does not advertise) resolve to `Opcode::IaError`, i.e. a")
    out.append("//! guest #UD — what Bochs produces with those ISA bits off.")
    out.append("")
    out.append("use super::form_opcode;")
    out.append("use super::tables::OpcodeAttrs as A;")
    out.append("use crate::opcode::Opcode;")
    out.append("")
    out.append("/// Empty slot — every encoding for this byte is undefined.")
    out.append("pub(crate) static EVEX_GROUP_ERR: &[u64] = &[];")
    out.append("")

    emitted = {}
    for g in used:
        entries = groups[g]
        ident = "EVEX_" + g[len("BxOpcodeGroup_EVEX_"):].upper()
        emitted[g] = ident
        out.append(f"static {ident}: &[u64] = &[")
        for attrs, bx in entries:
            total_entries += 1
            key = bx.replace("_", "").lower()
            if key in rust_names:
                op = f"Opcode::{rust_names[key]}"
            else:
                missing.add(bx)
                op = "Opcode::IaError"
            out.append(f"    form_opcode({rust_attrs(attrs)}, {op}),")
        out.append("];")
        out.append("")

    out.append("/// Master EVEX table, indexed `(map - 1) * 256 + opcode`.")
    out.append("///")
    out.append("/// Bochs `BxOpcodeTableEVEX[256*5]`. Map 1 = 0F, 2 = 0F38, 3 = 0F3A,")
    out.append("/// 5 = MAP5, 6 = MAP6; the map-4 block is unused but kept so the")
    out.append("/// indexing matches upstream exactly.")
    out.append(f"pub(crate) static EVEX_TABLE: [&[u64]; {256 * MAPS}] = [")
    map_label = {0: "0F", 1: "0F38", 2: "0F3A", 3: "unused", 4: "MAP5"}
    for i, g in enumerate(master):
        if i % 256 == 0:
            out.append(f"    // ---- map {i // 256 + 1} ({map_label[i // 256]}) ----")
        ident = emitted.get(g, "EVEX_GROUP_ERR")
        out.append(f"    /* {i % 256:02X} */ {ident},")
    out.append("];")
    out.append("")

    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out))

    defined = sum(1 for g in master if g != "BxOpcodeGroup_ERR")
    print(f"groups emitted        : {len(used)}")
    print(f"table entries         : {total_entries}")
    print(f"master slots defined  : {defined} / {256 * MAPS}")
    print(f"opcodes -> IaError    : {len(missing)} distinct (not in rusty's enum)")
    if missing:
        for n in sorted(missing)[:10]:
            print(f"    {n}")
    print(f"wrote {os.path.relpath(OUT, ROOT)}")


if __name__ == "__main__":
    main()
