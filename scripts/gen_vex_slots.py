#!/usr/bin/env python3
"""Generate (or verify) the VEX slot bitmap in decoder/vex_shared.rs.

Bochs resolves a VEX-encoded instruction against ``BxOpcodeTableVEX`` and
nothing else: a slot holding ``BxOpcodeGroup_ERR`` is a guest #UD. rusty_box
instead *shares* the legacy SSE opcode tables with the VEX path, which means a
legacy entry can catch a VEX encoding that Bochs rejects outright — that is how
``VEX.0F 80`` once decoded as ``JO rel32`` and the guest took the branch.

``vex_shared::vex_slot_populated`` restores upstream's shape by testing the slot
before the shared-table lookup. This script derives that bitmap straight from
the Bochs source so it can be regenerated when the upstream snapshot rebases,
rather than hand-transcribed.

Usage:
    python scripts/gen_vex_slots.py --verify    # exit 1 if vex_shared.rs drifted
    python scripts/gen_vex_slots.py             # print the Rust table
    python scripts/gen_vex_slots.py --emit-groups PATH
        # write Bochs's VEX groups and master table as a Rust module at PATH

``--emit-groups`` transcribes every ``BxOpcodeGroup_VEX_*`` row the master
table reaches — attributes and ``BX_IA_*`` opcode, in Bochs order — in the
entry encoding ``opmap_evex.rs`` uses (``form_opcode(attrs, opcode)``, built by
``gen_opmap_evex.rust_attrs``), so ``decoder::find_opcode_in_table`` resolves
them exactly as ``findOpcode`` does. The master table ``VEX_TABLE`` holds maps
1, 2, 3 and 7 in that order, as ``BxOpcodeTableVEX`` does with
``BX_SUPPORT_AMX`` 0. Beside it, ``bochs_vex_sources`` states, for each opcode
the table holds, what its ia_opcodes.def entry tells the decoder (aliases
followed as ``gen_evex_operands`` follows them): the fields its destination and
its ModRM source come from; whether fetchdecode32.cc ``assign_srcs`` finds a
``BX_SRC_VVV`` source, a ``BX_SRC_VSIB`` source, or an opmask named by
ModRM.reg or by VEX.vvvv; and whether ``assignHandler`` finds a handler for the
memory form and for the register form (``BxError`` or ``NULL`` is a guest #UD).

Table layout, with BX_SUPPORT_AMX = 0 (rusty_box does not implement AMX):

    entries    0..255   VEX map 1  (0F)
    entries  256..511   VEX map 2  (0F38)
    entries  512..767   VEX map 3  (0F3A)
    entries  768..1023  VEX map 7  (MSR immediate forms)

map 4 and map 6 are "for now empty" in upstream and emit no entries at all;
map 5 sits inside ``#if BX_SUPPORT_AMX``. Only maps 1-3 are generated here.
Map 7 is not shared with any legacy table: its two populated slots, F6
(WRMSRNS/RDMSR) and F8 (UWRMSR/URDMSR), decode through groups of their own,
``VEX_MAP7_F6`` and ``VEX_MAP7_F8``, and ``vex_slot_populated`` names those two
bytes beside them by hand.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

import gen_evex_operands
import gen_opmap_evex

REPO = Path(__file__).resolve().parent.parent
BOCHS_DECODER = REPO / "cpp_orig/bochs/bochs/cpu/decoder"
BOCHS_TABLE = BOCHS_DECODER / "fetchdecode_opmap_avx.cc"
BOCHS_DEFS = [BOCHS_DECODER / "ia_opcodes.def", BOCHS_DECODER / "ia_opcodes_evex.def"]
RUST_SOURCE = REPO / "rusty_box_decoder/src/decoder/vex_shared.rs"

MAPS = 3
SLOTS_PER_MAP = 256

# BxOpcodeTableVEX with BX_SUPPORT_AMX 0: maps 1, 2, 3 and 7, in that order.
TABLE_MAPS = (1, 2, 3, 7)

# How the reference build resolves the conditionals of fetchdecode_opmap_avx.cc
# (gen_opmap_evex.REFERENCE_BUILD, plus the file's own BX_SUPPORT_AVX guard).
VEX_REFERENCE_BUILD = {
    "#if BX_SUPPORT_AVX": True,
    "#ifndef BX_STANDALONE_DECODER": True,
    "#if BX_SUPPORT_AMX": False,
}

DEF_LINE_RE = re.compile(r"^\s*bx_define_opcode\(\s*BX_IA_(\w+)\s*,(.*)\)\s*$")

# The instruction field an operand's `BX_SRC_*` origin names.
FIELD = {
    "BX_SRC_NONE": "None",
    "BX_SRC_EAX": "Eax",
    "BX_SRC_NNN": "Nnn",
    "BX_SRC_RM": "Rm",
    "BX_SRC_VECTOR_RM": "Rm",
    "BX_SRC_VSIB": "Rm",
    "BX_SRC_VVV": "Vvv",
    "BX_SRC_VIB": "Vib",
}

ENTRY_RE = re.compile(r"\bBxOpcodeGroup_(\w+)")
TABLE_START_RE = re.compile(r"const\s+Bit64u\s*\*\s*BxOpcodeTableVEX\s*\[")


def collect_entries(source: str) -> list[str]:
    """Return the table's group names in order, with BX_SUPPORT_AMX = 0.

    Bochs writes one entry per line, so a line-oriented scan is exact. The only
    conditional inside the table is BX_SUPPORT_AMX; any other ``#if`` would be
    new upstream and must be handled explicitly rather than guessed at, so it
    raises instead of being silently ignored.
    """
    lines = source.splitlines()
    start = next(
        (n for n, line in enumerate(lines) if TABLE_START_RE.search(line)), None
    )
    if start is None:
        raise SystemExit(f"BxOpcodeTableVEX not found in {BOCHS_TABLE}")

    entries: list[str] = []
    skip_depth = 0          # >0 while inside a region we are excluding
    cond_stack: list[str] = []

    for line in lines[start + 1 :]:
        stripped = line.strip()

        if stripped.startswith("#if"):
            cond = stripped[3:].lstrip("defined").strip(" ()")
            if "BX_SUPPORT_AMX" in stripped:
                cond_stack.append("AMX")
                skip_depth += 1
            elif "BX_SUPPORT_AVX" in stripped:
                cond_stack.append("AVX")     # always true for this port
            else:
                raise SystemExit(
                    f"unhandled preprocessor condition inside BxOpcodeTableVEX: "
                    f"{stripped!r} — teach this script what it means before "
                    f"regenerating (cond={cond!r})"
                )
            continue

        if stripped.startswith("#else"):
            if not cond_stack:
                raise SystemExit("#else outside any #if inside BxOpcodeTableVEX")
            if cond_stack[-1] == "AMX":
                skip_depth -= 1              # the #else branch is the AMX=0 one
            else:
                skip_depth += 1
            cond_stack[-1] = "!" + cond_stack[-1]
            continue

        if stripped.startswith("#endif"):
            if not cond_stack:
                raise SystemExit("#endif outside any #if inside BxOpcodeTableVEX")
            cond = cond_stack.pop()
            if cond in ("AMX", "!AVX"):
                skip_depth -= 1
            continue

        if stripped.startswith("};"):
            break

        if skip_depth:
            continue

        for match in ENTRY_RE.finditer(line):
            entries.append(match.group(1))

    return entries


def build_bitmap(entries: list[str]) -> list[list[int]]:
    needed = MAPS * SLOTS_PER_MAP
    if len(entries) < needed:
        raise SystemExit(
            f"expected at least {needed} VEX table entries, parsed {len(entries)}"
        )

    bitmap = [[0] * 4 for _ in range(MAPS)]
    for index in range(needed):
        if entries[index] == "ERR":
            continue
        opcode_map, opcode_byte = divmod(index, SLOTS_PER_MAP)
        word, bit = divmod(opcode_byte, 64)
        bitmap[opcode_map][word] |= 1 << bit
    return bitmap


def render(bitmap: list[list[int]], entries: list[str]) -> str:
    names = ["0F", "0F38", "0F3A"]
    out = ["const VEX_POPULATED_SLOTS: [[u64; 4]; 3] = ["]
    for m, words in enumerate(bitmap):
        block = entries[m * SLOTS_PER_MAP : (m + 1) * SLOTS_PER_MAP]
        count = sum(1 for e in block if e != "ERR")
        out.append(f"    // map {m + 1} ({names[m]}) — {count} slots")
        rendered = ", ".join(f"0x{w:016X}" for w in words)
        out.append(f"    [{rendered}],")
    out.append("];")
    return "\n".join(out)


def bochs_sources() -> dict[str, dict[str, object]]:
    """For every Bochs opcode, its destination and ModRM-source fields, what
    ``assign_srcs`` checks of its sources, and the forms it executes in.

    Read from the four source operands of each ``bx_define_opcode`` entry,
    every ``OP_*`` resolved through fetchdecode.h with aliases followed. A name
    that is neither a ``BX_FORM_SRC`` constant nor an alias of one stops the
    run.
    """
    header = (BOCHS_DECODER / "fetchdecode.h").read_text(encoding="utf-8", errors="replace")
    ops = gen_evex_operands.parse_op_constants(header)
    aliases = gen_evex_operands.parse_op_aliases(header)

    facts: dict[str, dict[str, object]] = {}
    for path in BOCHS_DEFS:
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            m = DEF_LINE_RE.match(line.split("//")[0])
            if not m:
                continue
            fields = [f.strip() for f in m.group(2).split(",")]
            if len(fields) != 10:
                raise SystemExit(f"{m.group(1)}: expected 10 fields after the name, got {len(fields)}")
            sources = []
            for name in fields[5:9]:
                if name == "OP_NONE":
                    continue
                resolved = gen_evex_operands.resolve_operand(name, ops, aliases)
                if resolved is None:
                    raise SystemExit(
                        f"{m.group(1)}: operand {name} is neither a BX_FORM_SRC constant "
                        f"nor an alias of one in fetchdecode.h"
                    )
                sources.append(resolved)
            # The destination is the first operand (`OP_NONE` there: none), and
            # the ModRM source the first later operand that names a register
            # through ModRM and is not the destination read back — the rule
            # gen_evex_operands applies to the EVEX entries.
            first = None
            if fields[5] != "OP_NONE":
                first = gen_evex_operands.resolve_operand(fields[5], ops, aliases)
            modrm_source = "None"
            for operand in sources[1 if first is not None else 0 :]:
                if operand != first and operand[1] in gen_evex_operands.MODRM_SRC:
                    modrm_source = FIELD[operand[1]]
                    break
            kmask = ("BX_KMASK_REG", "BX_KMASK_REG_PAIR")
            facts[m.group(1)] = {
                "dst": "None" if first is None else FIELD[first[1]],
                "modrm_source": modrm_source,
                # execute1 runs the memory form and execute2 the register
                # form; `assignHandler` installs whichever the ModRM form
                # selects, and a `BxError` (or `NULL`) there is a guest #UD.
                "memory_form": fields[2] not in gen_evex_operands.NO_HANDLER,
                "register_form": fields[3] not in gen_evex_operands.NO_HANDLER,
                "vvv": any(src == "BX_SRC_VVV" for _, src in sources),
                "vsib": any(src == "BX_SRC_VSIB" for _, src in sources),
                # assign_srcs: an opmask in ModRM.reg (BX_KMASK_REG or
                # BX_KMASK_REG_PAIR) or in VEX.vvvv (BX_KMASK_REG) must be one
                # of k0..k7.
                "kmask_nnn": any(src == "BX_SRC_NNN" and typ in kmask for typ, src in sources),
                "kmask_vvv": any(
                    src == "BX_SRC_VVV" and typ == "BX_KMASK_REG" for typ, src in sources
                ),
            }
    return facts


def render_groups(entries: list[str]) -> str:
    """The VEX groups and master table as a Rust module (see ``--emit-groups``)."""
    text = gen_opmap_evex.resolve_conditionals(
        gen_opmap_evex.strip_comments(BOCHS_TABLE.read_text(encoding="utf-8", errors="replace")),
        VEX_REFERENCE_BUILD,
        "VEX",
    )
    groups = gen_opmap_evex.parse_groups(text, "VEX")
    rust_names = gen_opmap_evex.rust_opcode_names("[A-Z]")
    sources = bochs_sources()

    needed = len(TABLE_MAPS) * SLOTS_PER_MAP
    if len(entries) != needed:
        raise SystemExit(f"expected {needed} VEX table entries, parsed {len(entries)}")
    used = sorted({e for e in entries if e != "ERR"})
    unknown = [g for g in used if "BxOpcodeGroup_" + g not in groups]
    if unknown:
        raise SystemExit(f"master table references groups that were not parsed: {unknown[:5]}")

    out = [
        "//! Bochs `BxOpcodeTableVEX` — generated, do not edit by hand.",
        "//!",
        "//! Regenerate with `python scripts/gen_vex_slots.py --emit-groups <path>`.",
        "//!",
        "//! Transcribed from Bochs `cpu/decoder/fetchdecode_opmap_avx.cc` with",
        "//! `BX_SUPPORT_AMX` 0: one group per (map, opcode byte), each entry a",
        "//! `form_opcode(attrs, opcode)` in Bochs order. The master table holds",
        "//! maps 1, 2, 3 and 7, indexed `block * 256 + opcode`.",
        "",
        "use super::form_opcode;",
        "use super::tables::OpcodeAttrs as A;",
        "use crate::opcode::Opcode;",
        "",
        "/// Empty slot — every encoding for this byte is undefined.",
        "pub(crate) static VEX_GROUP_ERR: &[u64] = &[];",
        "",
    ]
    missing: set[str] = set()
    opcodes: list[str] = []
    rows = 0
    for g in used:
        out.append(f"static {g.upper()}: &[u64] = &[")
        for attrs, bx in groups["BxOpcodeGroup_" + g]:
            rows += 1
            key = bx.replace("_", "").lower()
            if key in rust_names:
                op = rust_names[key]
                if op not in opcodes:
                    opcodes.append(op)
                if bx not in sources:
                    raise SystemExit(f"BX_IA_{bx} has no bx_define_opcode entry")
            else:
                missing.add(bx)
                op = "IaError"
            out.append(f"    form_opcode({gen_opmap_evex.rust_attrs(attrs)}, Opcode::{op}),")
        out.append("];")
        out.append("")

    out.append("/// Master VEX table: blocks 0..3 hold maps 1, 2, 3 and 7.")
    out.append(f"pub(crate) static VEX_TABLE: [&[u64]; {needed}] = [")
    for i, g in enumerate(entries):
        if i % SLOTS_PER_MAP == 0:
            out.append(f"    // ---- map {TABLE_MAPS[i // SLOTS_PER_MAP]} ----")
        ident = "VEX_GROUP_ERR" if g == "ERR" else g.upper()
        out.append(f"    /* {i % SLOTS_PER_MAP:02X} */ {ident},")
    out.append("];")
    out.append("")

    by_rust = {}
    for bx, fact in sources.items():
        key = bx.replace("_", "").lower()
        if key in rust_names:
            by_rust[rust_names[key]] = fact
    # The fields the emitted opcodes name, in FIELD's order; `None` is the
    # default arm's.
    named = {"None"} | {by_rust[op][k] for op in opcodes for k in ("dst", "modrm_source")}
    out += [
        "/// The instruction field a Bochs operand's `BX_SRC_*` origin names.",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub(crate) enum BochsVexField {",
    ]
    out += [f"    {field}," for field in dict.fromkeys(FIELD.values()) if field in named]
    out += [
        "}",
        "",
        "/// What Bochs checks of a decoded VEX opcode, from its ia_opcodes.def",
        "/// entry: fetchdecode32.cc `assign_srcs` checks its sources, and",
        "/// `assignHandler` installs the handler of its ModRM form.",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub(crate) struct BochsVexSources {",
        "    /// The field the first operand names (`None` when it is `OP_NONE`).",
        "    pub(crate) dst: BochsVexField,",
        "    /// The field of the first later operand that names a register",
        "    /// through ModRM and is not the destination read back.",
        "    pub(crate) modrm_source: BochsVexField,",
        "    /// A source is `BX_SRC_VVV`; without one, VEX.vvvv must be 1111b.",
        "    pub(crate) vvv: bool,",
        "    /// A source is `BX_SRC_VSIB`: ModRM.rm must be 100b.",
        "    pub(crate) vsib: bool,",
        "    /// ModRM.reg names an opmask, which must be k0..k7.",
        "    pub(crate) kmask_nnn: bool,",
        "    /// VEX.vvvv names an opmask, which must be k0..k7.",
        "    pub(crate) kmask_vvv: bool,",
        "    /// The memory form has a handler (`execute1` is neither `NULL` nor",
        "    /// `BxError`).",
        "    pub(crate) memory_form: bool,",
        "    /// The register form has a handler (`execute2`).",
        "    pub(crate) register_form: bool,",
        "}",
        "",
        "/// [`BochsVexSources`] of every opcode [`VEX_TABLE`] holds; none for the rest.",
        "pub(crate) const fn bochs_vex_sources(op: Opcode) -> BochsVexSources {",
        "    match op {",
    ]
    flags = ("vvv", "vsib", "kmask_nnn", "kmask_vvv", "memory_form", "register_form")
    for op in sorted(opcodes):
        f = by_rust[op]
        values = ", ".join(
            [f"dst: BochsVexField::{f['dst']}", f"modrm_source: BochsVexField::{f['modrm_source']}"]
            + [f"{name}: {str(f[name]).lower()}" for name in flags]
        )
        out.append(f"        Opcode::{op} => BochsVexSources {{ {values} }},")
    none = ", ".join(
        ["dst: BochsVexField::None", "modrm_source: BochsVexField::None"]
        + [f"{name}: false" for name in flags]
    )
    out += [
        f"        _ => BochsVexSources {{ {none} }},",
        "    }",
        "}",
        "",
    ]

    print(f"groups emitted        : {len(used)}", file=sys.stderr)
    print(f"table rows            : {rows}", file=sys.stderr)
    print(f"distinct opcodes      : {len(opcodes)}", file=sys.stderr)
    print(f"master slots defined  : {len(used)} / {needed}", file=sys.stderr)
    print(f"opcodes -> IaError    : {len(missing)} distinct (not in rusty's enum)", file=sys.stderr)
    for n in sorted(missing):
        print(f"    {n}", file=sys.stderr)
    return "\n".join(out)


def parse_rust_bitmap() -> list[list[int]]:
    text = RUST_SOURCE.read_text(encoding="utf-8")
    match = re.search(
        r"const VEX_POPULATED_SLOTS: \[\[u64; 4\]; 3\] = \[(.*?)\n\];",
        text,
        re.DOTALL,
    )
    if not match:
        raise SystemExit(f"VEX_POPULATED_SLOTS not found in {RUST_SOURCE}")

    rows = re.findall(r"\[([^\]]*)\]", match.group(1))
    if len(rows) != MAPS:
        raise SystemExit(f"expected {MAPS} rows in VEX_POPULATED_SLOTS, found {len(rows)}")

    return [[int(v.strip().replace("_", ""), 16) for v in row.split(",") if v.strip()]
            for row in rows]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--verify",
        action="store_true",
        help="compare vex_shared.rs against Bochs and exit non-zero on drift",
    )
    parser.add_argument(
        "--emit-groups",
        metavar="PATH",
        help="write Bochs's VEX groups and master table as a Rust module at PATH",
    )
    args = parser.parse_args()

    entries = collect_entries(BOCHS_TABLE.read_text(encoding="utf-8", errors="replace"))

    if args.emit_groups:
        Path(args.emit_groups).write_text(render_groups(entries), encoding="utf-8", newline="\n")
        print(f"wrote {args.emit_groups}", file=sys.stderr)
        return 0

    expected = build_bitmap(entries)

    if not args.verify:
        print(render(expected, entries))
        return 0

    actual = parse_rust_bitmap()
    drifted = False
    for m in range(MAPS):
        if actual[m] != expected[m]:
            drifted = True
            print(f"map {m + 1}: DRIFT")
            for opcode_byte in range(SLOTS_PER_MAP):
                word, bit = divmod(opcode_byte, 64)
                want = (expected[m][word] >> bit) & 1
                have = (actual[m][word] >> bit) & 1
                if want != have:
                    verb = "missing from" if want else "not in Bochs but set in"
                    print(f"  opcode {opcode_byte:02X} {verb} vex_shared.rs")

    if drifted:
        print("\nRegenerate with: python scripts/gen_vex_slots.py")
        return 1

    total = sum(bin(w).count("1") for row in expected for w in row)
    print(f"VEX_POPULATED_SLOTS matches Bochs ({total} populated slots across 3 maps)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
