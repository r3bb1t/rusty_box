#!/usr/bin/env python3
"""Generate the EVEX opcode maps from Bochs's own decoder tables.

Reads cpu/decoder/fetchdecode_opmap_evex.cc — which *is* the table, so the
generated Rust is a transcription rather than a re-derivation — and emits
rusty_box_decoder/src/decoder/opmap_evex.rs.

Bochs shape, reproduced exactly:

  * one `BxOpcodeGroup_EVEX_<name>[]` per (map, opcode byte) that has any
    encoding, each entry a `form_opcode(attrs, opcode)` with the last one
    marked by `last_opcode`;
  * a master `BxOpcodeTableEVEX[256*5]` indexed `(block - 1) * 256 + opcode`,
    with `BxOpcodeGroup_ERR` wherever nothing is defined.

Opcode names are matched to rusty's `Opcode` enum case-insensitively after
dropping underscores: rusty renders `BX_IA_EVEX_VPADDD_VdqHdqWdq` as
`EvexVpadddVdqHdqWdq`, and the two differ only in case, with no collisions.
A name the enum lacks becomes `Opcode::IaError` and is reported by the run,
so a gap is visible rather than silent.

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

# Bochs's master table holds 5 blocks of 256: 0F, 0F38, 0F3A, MAP5 and MAP6.
# There is no EVEX map 4, so the decoder renumbers maps 5 and 6 down into
# blocks 4 and 5 before indexing (Bochs fetchdecode64.cc decoder_evex64,
# `if (evex_opc_map >= 4) evex_opc_map--`).
MAPS = 5
BLOCK_LABEL = {0: "0F", 1: "0F38", 2: "0F3A", 3: "MAP5", 4: "MAP6"}

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


# How the Bochs builds this port is checked against (cpp_orig/bochs/build-mingw
# and build-bench-nosmp, both `--enable-evex` without `--enable-amx`) resolve
# each preprocessor condition in the EVEX table file: True keeps the #if arm.
REFERENCE_BUILD = {
    "#if BX_SUPPORT_EVEX": True,
    "#ifndef BX_STANDALONE_DECODER": True,
    "#if BX_SUPPORT_AMX": False,
}


def resolve_conditionals(text):
    """Flatten `#if / #else / #endif` the way the reference build does.

    Its BX_SUPPORT_AMX is 0, and rusty_box implements no AMX state, so an AMX
    group is not emitted and its master-table slot is the #else arm's
    `BxOpcodeGroup_ERR` — a guest #UD, as upstream. scripts/gen_vex_slots.py
    resolves the VEX table the same way. A condition missing from
    REFERENCE_BUILD stops the run rather than guess a branch, and so does an
    `#elif`, whose arm a single REFERENCE_BUILD lookup cannot choose.
    """
    out, skipping = [], []
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("#elif"):
            sys.exit(f"unexpected conditional in the EVEX table: {s}")
        if s.startswith("#if"):
            condition = s.split("//")[0].strip()
            if condition not in REFERENCE_BUILD:
                sys.exit(f"unexpected conditional in the EVEX table: {s}")
            skipping.append(not REFERENCE_BUILD[condition])
            continue
        if s.startswith("#else"):
            if not skipping:
                sys.exit("#else outside any #if in the EVEX table")
            skipping[-1] = not skipping[-1]
            continue
        if s.startswith("#endif"):
            if not skipping:
                sys.exit("#endif outside any #if in the EVEX table")
            skipping.pop()
            continue
        if not any(skipping):
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
    out.append("//! machinery `tables.rs` already implements. Preprocessor conditions")
    out.append("//! are resolved as the reference build resolves them: BX_SUPPORT_AMX")
    out.append("//! is 0 there, so no AMX encoding is present. The master table is")
    out.append("//! indexed `(block - 1) * 256 + opcode`, matching `BxOpcodeTableEVEX`.")
    out.append("//!")
    out.append("//! An opcode the CPU model does not advertise still decodes here; the")
    out.append("//! ISA gate rewrites it to `Opcode::IaError` at icache fill, a guest")
    out.append("//! #UD — what Bochs produces with that ISA bit off.")
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

    out.append("/// Master EVEX table, indexed `(block - 1) * 256 + opcode`.")
    out.append("///")
    out.append("/// Bochs `BxOpcodeTableEVEX[256*5]`: blocks 1-5 hold 0F, 0F38, 0F3A,")
    out.append("/// MAP5 and MAP6. There is no EVEX map 4, so maps 5 and 6 are blocks")
    out.append("/// 4 and 5 (Bochs fetchdecode64.cc decoder_evex64).")
    out.append(f"pub(crate) static EVEX_TABLE: [&[u64]; {256 * MAPS}] = [")
    for i, g in enumerate(master):
        if i % 256 == 0:
            out.append(f"    // ---- block {i // 256 + 1} ({BLOCK_LABEL[i // 256]}) ----")
        ident = emitted.get(g, "EVEX_GROUP_ERR")
        out.append(f"    /* {i % 256:02X} */ {ident},")
    out.append("];")
    out.append("")
    out.append("/// Number of 256-byte blocks in [`EVEX_TABLE`] (Bochs `256*5`).")
    out.append(f"pub(crate) const EVEX_MAPS: usize = {MAPS};")
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
