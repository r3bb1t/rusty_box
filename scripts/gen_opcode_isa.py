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

# Prepare classes, mirroring the BX_PREPARE_* attributes of Bochs
# `cpu/decoder/fetchdecode.h`. Numbered densely rather than reusing Bochs's bit
# values because exactly one applies per opcode.
STATE_NONE, STATE_FPU, STATE_MMX, STATE_SSE = 0, 1, 2, 3
STATE_AVX, STATE_EVEX, STATE_AMX = 4, 5, 6
PREPARE_NAMES = {
    STATE_NONE: "Base",
    STATE_FPU: "Fpu",
    STATE_MMX: "Mmx",
    STATE_SSE: "Sse",
    STATE_AVX: "Avx",
    STATE_EVEX: "Evex",
    STATE_AMX: "Amx",
}

# Opcodes with no Bochs counterpart. They stay ungated, which preserves the
# behaviour rusty_box had before the gate existed — a missing gate is the status
# quo, a wrong gate would #UD a working guest. Listed explicitly so that new
# drift shows up as a diff rather than silently widening this set.
KNOWN_UNMATCHED = {
    # rusty-internal pseudo-opcodes with no Bochs BX_IA_* counterpart.
    "IaError",
    "InsertedOpcode",
    "Int0",
    # Substituted by the icache fill path when the guest has not enabled the
    # CPU state the decoded instruction needs. Bochs expresses the same thing
    # as a handler swap to BxNoAVX / BxNoEVEX, so there is no BX_IA_* to match.
    "NoAvxState",
    "NoEvexState",
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

# An opcode with no Bochs counterpart gets no BX_PREPARE_* either, and
# defaulting it to STATE_NONE would exempt it from the state gate entirely.
# Every unmatched opcode that is nevertheless a real VEX/EVEX encoding needs its
# class stated here. The invariant below makes forgetting one an error rather
# than a silently ungated instruction.
STATE_OVERRIDES = {
    "EvexVpmultishiftqbVdqHdqWdq": STATE_EVEX,
    "EvexVminpbf16VphHphWph": STATE_EVEX,
    "EvexVminpbf16VphHphWphKmask": STATE_EVEX,
    "EvexVmaxpbf16VphHphWph": STATE_EVEX,
    "EvexVmaxpbf16VphHphWphKmask": STATE_EVEX,
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
    # Bochs opcode name -> which CPU state must be enabled to execute it
    prepare_class = {}
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

            # The same field also names the CPU state the instruction needs
            # enabled. Bochs turns this into a BxNo* handler substitution in
            # `assignHandler`; rusty_box applies it at icache fill. The classes
            # are mutually exclusive except AMX, which some opcodes carry
            # alongside EVEX — AMX is the stricter of the two, so it wins.
            # BX_PREPARE_OPMASK is #defined to BX_PREPARE_EVEX, so the two are
            # the same class and the EVEX test below catches both.
            if "BX_PREPARE_AMX" in attrs:
                cls = STATE_AMX
            elif "BX_PREPARE_EVEX" in attrs or "BX_PREPARE_OPMASK" in attrs:
                cls = STATE_EVEX
            elif "BX_PREPARE_AVX" in attrs:
                cls = STATE_AVX
            elif "BX_PREPARE_SSE" in attrs:
                cls = STATE_SSE
            elif "BX_PREPARE_MMX" in attrs:
                cls = STATE_MMX
            elif "BX_PREPARE_FPU" in attrs:
                cls = STATE_FPU
            else:
                cls = STATE_NONE
            prepare_class[norm(fields[0].replace("BX_IA_", ""))] = cls

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
    ]

    # ---- prepare class (which CPU state must be enabled) ----
    lines += [
        "/// The CPU state an instruction needs enabled before it may execute —",
        "/// the `BX_PREPARE_*` attribute of Bochs `bx_define_opcode`.",
        "///",
        "/// Bochs consults it in `assignHandler` and swaps the handler for",
        "/// `BxNoFPU` / `BxNoMMX` / `BxNoSSE` / `BxNoAVX` / `BxNoEVEX` when the",
        "/// state is unavailable; rusty_box applies it at icache fill, so the",
        "/// dispatch loop pays nothing and no individual handler can forget it.",
        "///",
        "/// Exactly one applies per opcode. This is an enum rather than a set of",
        "/// integer constants so that a `match` over it is exhaustive: adding a",
        "/// class breaks every consumer at compile time instead of silently",
        "/// falling through a catch-all arm and leaving instructions ungated.",
        "#[derive(Clone, Copy, PartialEq, Eq, Debug)]",
        "#[repr(u8)]",
        "pub enum CpuState {",
        "    /// Base ISA — no state beyond an ordinary integer instruction.",
        "    Base,",
        "    /// x87 state (CR0.EM, CR0.TS).",
        "    Fpu,",
        "    /// MMX state.",
        "    Mmx,",
        "    /// SSE state (CR0.EM, CR4.OSFXSR, CR0.TS).",
        "    Sse,",
        "    /// AVX state (protected mode, CR4.OSXSAVE, XCR0.SSE|YMM, CR0.TS).",
        "    Avx,",
        "    /// AVX-512 state (AVX plus XCR0.OPMASK|ZMM_HI256|HI_ZMM).",
        "    Evex,",
        "    /// AMX tile state.",
        "    Amx,",
        "}",
        "",
        "/// CPU state each opcode requires, from field 10 of `bx_define_opcode`.",
        "// A `const` for the same reason as OPCODE_EVEX_FLAGS.",
        f"pub const OPCODE_STATE: [CpuState; {len(opcodes)}] = [",
    ]
    # A VEX/EVEX-encoded opcode that ends up needing no state is almost always a
    # missing mapping rather than a real ungated instruction, and the failure
    # mode is silent: the icache state gate would wave it through for a guest
    # that never enabled AVX. Catch it here instead.
    ungated_vector = [
        op
        for op in opcodes
        if op.startswith(("Evex", "V128", "V256", "V512"))
        and STATE_OVERRIDES.get(op, prepare_class.get(norm(op), STATE_NONE)) == STATE_NONE
    ]
    if ungated_vector:
        print("ERROR: VEX/EVEX opcodes with no state class:", file=sys.stderr)
        for op in sorted(ungated_vector):
            print(f"  {op} — add it to STATE_OVERRIDES", file=sys.stderr)
        return 1

    prepare_counts = {}
    for op in opcodes:
        cls = STATE_OVERRIDES.get(op, prepare_class.get(norm(op), STATE_NONE))
        prepare_counts[cls] = prepare_counts.get(cls, 0) + 1
        lines.append(f"    CpuState::{PREPARE_NAMES[cls]}, // {op}")
    lines += [
        "];",
        "",
        "/// CPU state `opcode` requires before it may execute.",
        "#[inline]",
        "pub const fn opcode_state(opcode: Opcode) -> CpuState {",
        "    OPCODE_STATE[opcode as usize]",
        "}",
        "",
        "/// Opcodes requiring AVX state, pinned by tests so a regeneration that",
        "/// silently drops the gate is caught.",
        f"pub const STATE_AVX_OPCODE_COUNT: usize = {prepare_counts.get(STATE_AVX, 0)};",
        "",
        "/// Opcodes requiring AVX-512 state.",
        f"pub const STATE_EVEX_OPCODE_COUNT: usize = {prepare_counts.get(STATE_EVEX, 0)};",
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
