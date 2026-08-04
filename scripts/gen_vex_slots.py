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

Table layout, with BX_SUPPORT_AMX = 0 (rusty_box does not implement AMX):

    entries    0..255   VEX map 1  (0F)
    entries  256..511   VEX map 2  (0F38)
    entries  512..767   VEX map 3  (0F3A)
    entries  768..1023  VEX map 7  (MSR immediate forms)

map 4 and map 6 are "for now empty" in upstream and emit no entries at all;
map 5 sits inside ``#if BX_SUPPORT_AMX``. Only maps 1-3 are mirrored here —
map 7 holds WRMSRNS/RDMSR/UWRMSR/URDMSR, gated on BX_ISA_MSR_IMM and
BX_ISA_USER_MSR, which Corei7SkylakeX does not advertise, so rejecting the map
at decode is observationally identical to Bochs's #UD on the ISA check.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BOCHS_TABLE = REPO / "cpp_orig/bochs/bochs/cpu/decoder/fetchdecode_opmap_avx.cc"
RUST_SOURCE = REPO / "rusty_box_decoder/src/decoder/vex_shared.rs"

MAPS = 3
SLOTS_PER_MAP = 256

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
    args = parser.parse_args()

    entries = collect_entries(BOCHS_TABLE.read_text(encoding="utf-8", errors="replace"))
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
