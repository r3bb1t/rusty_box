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
syncing `cpp_orig/bochs/` or after adding `Opcode` / `X86Feature` variants. In
`rusty_box_decoder/src/tests.rs`, `opcode_isa_table_is_in_sync_with_the_opcode_enum`
fails when the file no longer covers the enum (`OPCODE_VARIANT_COUNT`), and
`opcode_isa_counts_are_pinned_to_the_reference_build` fails when a
regeneration moves `GATED_OPCODE_COUNT`, `EVEX_FLAGGED_OPCODE_COUNT`,
`STATE_AVX_OPCODE_COUNT` or `STATE_EVEX_OPCODE_COUNT`.
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
    # Substituted by the icache fill path when the guest has not enabled the
    # CPU state the decoded instruction needs. Bochs expresses the same thing
    # as a handler swap to BxNoAVX / BxNoEVEX, so there is no BX_IA_* to match.
    "NoAvxState",
    "NoEvexState",
}

# Opcodes whose rusty name does not normalise to their Bochs name. Bochs's
# BX_IA_ERROR loses its IA_ with the prefix every other name sheds, and rusty
# keeps it, since a bare `Opcode::Error` would read as an error type.
BOCHS_SPELLING = {
    "IaError": "BX_IA_ERROR",
}

# An opcode with no Bochs counterpart gets no BX_PREPARE_* either, and
# defaulting it to STATE_NONE would exempt it from the state gate entirely.
# Every unmatched opcode that is nevertheless a real VEX/EVEX encoding needs its
# class stated here. The invariant below makes forgetting one an error rather
# than a silently ungated instruction.
STATE_OVERRIDES = {}

FETCHDECODE_H = "cpp_orig/bochs/bochs/cpu/decoder/fetchdecode.h"

# BX_PREPARE_* bits a def may carry that this generator deliberately takes no
# meaning from, each named by the fetchdecode.h token that introduces it. Its
# value is read from fetchdecode.h like every other token's; only the bit it
# adds on top of the tokens it is built from is exempt.
#
# BX_PREPARE_SCALEDATA is `(0x800 | BX_PREPARE_AMX)` in fetchdecode.h. Its AMX
# bit classifies the opcode like any BX_PREPARE_AMX neighbour. The 0x800 bit
# feeds only `assignHandler`'s BX_FETCH_MODE_SCALEDATA_OK test
# (fetchdecode32.cc), which sits inside `#if BX_SUPPORT_AMX`; the reference
# build compiles it out, and rusty_box models no scale-data state.
PREPARE_BITS_IGNORED = {
    "BX_PREPARE_SCALEDATA": "BX_PREPARE_AMX",
}


def prepare_values(header):
    """Every `#define BX_PREPARE_<X> (<expr>)` in fetchdecode.h, evaluated.

    Each expression ORs hex literals and other BX_PREPARE_* names, so the
    names resolve recursively. An expression with any other term stops the
    run: a value guessed here would classify opcodes wrongly without a trace.
    """
    raw = dict(re.findall(r"^#define\s+(BX_PREPARE_\w+)\s+\((.*?)\)\s*$", header, re.M))
    if not raw:
        sys.exit(f"no BX_PREPARE_* defines found in {FETCHDECODE_H}")
    values = {}

    def value(name, chain):
        if name in values:
            return values[name]
        if name in chain:
            sys.exit(f"{FETCHDECODE_H}: {name} is defined in terms of itself")
        total = 0
        for term in (t.strip() for t in raw[name].split("|")):
            if re.fullmatch(r"0[xX][0-9A-Fa-f]+|[0-9]+", term):
                total |= int(term, 0)
            elif term in raw:
                total |= value(term, chain + (name,))
            else:
                sys.exit(f"{FETCHDECODE_H}: cannot evaluate {name}: unknown term {term!r}")
        values[name] = total
        return total

    for name in raw:
        value(name, ())
    return values


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


def bochs_key(define):
    """The match key of a Bochs opcode define: `BX_IA_INTO` and
    `BX_INSERTED_OPCODE` alike lose their prefix before normalising."""
    for prefix in ("BX_IA_", "BX_"):
        if define.startswith(prefix):
            return norm(define[len(prefix):])
    return norm(define)


def rusty_key(variant):
    """The match key of a rusty `Opcode` variant."""
    return bochs_key(BOCHS_SPELLING[variant]) if variant in BOCHS_SPELLING else norm(variant)


def read(path):
    return (REPO / path).read_text(encoding="utf-8", errors="replace")


def main():
    # Bochs opcode name -> BX_ISA_* (or "0" for base-ISA instructions)
    bochs = {}
    # Bochs opcode name -> the BX_PREPARE_EVEX* encoding-restriction bits
    evex_flags = {}
    # Bochs opcode name -> which CPU state must be enabled to execute it
    prepare_class = {}

    # Field 10's BX_PREPARE_* tokens are decoded by value, as the C++ compiler
    # decodes them, so a token defined in terms of another (OPMASK is EVEX,
    # SCALEDATA carries AMX) lands in the class its bits say.
    prepare = prepare_values(read(FETCHDECODE_H))
    for token in ("BX_PREPARE_FPU", "BX_PREPARE_MMX", "BX_PREPARE_SSE", "BX_PREPARE_AVX",
                  "BX_PREPARE_EVEX", "BX_PREPARE_EVEX_NO_SAE",
                  "BX_PREPARE_EVEX_NO_BROADCAST", "BX_PREPARE_AMX"):
        if token not in prepare:
            sys.exit(f"{FETCHDECODE_H} no longer defines {token}")
    bit_amx = prepare["BX_PREPARE_AMX"]
    bit_evex = prepare["BX_PREPARE_EVEX"]
    bit_avx = prepare["BX_PREPARE_AVX"]
    bit_sse = prepare["BX_PREPARE_SSE"]
    bit_mmx = prepare["BX_PREPARE_MMX"]
    bit_fpu = prepare["BX_PREPARE_FPU"]
    # The EVEX encoding-restriction bits, as the Rust table stores them
    # (PREPARE_EVEX / PREPARE_EVEX_NO_SAE / PREPARE_EVEX_NO_BROADCAST).
    evex_restrictions = (
        prepare["BX_PREPARE_EVEX_NO_SAE"] | prepare["BX_PREPARE_EVEX_NO_BROADCAST"]
    )
    understood = bit_amx | bit_evex | bit_avx | bit_sse | bit_mmx | bit_fpu | evex_restrictions
    for token, base in PREPARE_BITS_IGNORED.items():
        if token not in prepare or base not in prepare:
            sys.exit(f"PREPARE_BITS_IGNORED names {token} / {base}, which {FETCHDECODE_H} lacks")
        understood |= prepare[token] & ~prepare[base]

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
            bochs[bochs_key(fields[0])] = fields[5].split("/*")[0].strip()
            # Field 10 carries the BX_PREPARE_* attributes, ORed with flags of
            # other families (BX_LOCKABLE, BX_TRACE_END, ...) this table does
            # not record. Every BX_PREPARE_* token must be one fetchdecode.h
            # defines, and every bit it sets must be one this generator either
            # uses or lists in PREPARE_BITS_IGNORED; anything else stops the
            # run rather than quietly classifying the opcode as Base.
            attrs = fields[10] if len(fields) > 10 else ""
            attr_value = 0
            for token in re.findall(r"\bBX_PREPARE_\w+\b", attrs):
                if token not in prepare:
                    sys.exit(f"{fields[0]}: {token} is not defined in {FETCHDECODE_H}")
                attr_value |= prepare[token]
            if attr_value & ~understood:
                sys.exit(
                    f"{fields[0]}: BX_PREPARE_* bits {attr_value & ~understood:#x} carry a "
                    f"meaning this generator does not know; classify them or add the "
                    f"token to PREPARE_BITS_IGNORED"
                )

            # The EVEX encoding-restriction bits, kept as Bochs's values.
            evex_flags[bochs_key(fields[0])] = attr_value & evex_restrictions

            # The same field also names the CPU state the instruction needs
            # enabled. Bochs turns this into a BxNo* handler substitution in
            # `assignHandler`; rusty_box applies it at icache fill. The classes
            # are mutually exclusive except AMX, which some opcodes carry
            # alongside EVEX — AMX is the stricter of the two, so it wins.
            if attr_value & bit_amx:
                cls = STATE_AMX
            elif attr_value & bit_evex:
                cls = STATE_EVEX
            elif attr_value & bit_avx:
                cls = STATE_AVX
            elif attr_value & bit_sse:
                cls = STATE_SSE
            elif attr_value & bit_mmx:
                cls = STATE_MMX
            elif attr_value & bit_fpu:
                cls = STATE_FPU
            else:
                cls = STATE_NONE
            prepare_class[bochs_key(fields[0])] = cls

    # rusty X86Feature variants, declaration order == discriminant
    feat_src = read("rusty_box_decoder/src/features.rs")
    feat_src = feat_src[feat_src.index("pub enum X86Feature"):]
    features = re.findall(r"^\s{4}([A-Z]\w+),\s*$", feat_src, re.M)
    feat_index = {norm(v): i for i, v in enumerate(features)}

    # Bochs's features, declaration order. A commented-out `//x86_feature(...)`
    # line is not one, so the pattern anchors on the start of the line. The
    # table's numbers are X86Feature discriminants, so the two lists must agree
    # entry for entry or a number would name a different feature than Bochs's.
    bochs_features = re.findall(
        r"^\s*x86_feature\(BX_ISA_(\w+),",
        read("cpp_orig/bochs/bochs/cpu/decoder/features.h"),
        re.M,
    )
    ours = [norm(v) for v in features]
    theirs = ["isa" + norm(f) for f in bochs_features]
    if ours != theirs:
        print("ERROR: X86Feature differs from Bochs cpu/decoder/features.h:", file=sys.stderr)
        for extra in [v for v in ours if v not in theirs]:
            print(f"  only in X86Feature: {extra}", file=sys.stderr)
        for missing in [v for v in theirs if v not in ours]:
            print(f"  only in Bochs: {missing}", file=sys.stderr)
        if sorted(ours) == sorted(theirs):
            print("  the same features in a different order", file=sys.stderr)
        return 1

    # rusty Opcode variants, declaration order == discriminant
    op_src = read("rusty_box_decoder/src/opcode.rs")
    op_src = op_src[op_src.index("pub enum Opcode"):]
    opcodes = re.findall(r"^\s{8}([A-Z][A-Za-z0-9_]*),\s*$", op_src, re.M)

    table, unmatched, missing_feature, gated = [], [], {}, 0
    for op in opcodes:
        feature = bochs.get(rusty_key(op))
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
        flags = evex_flags.get(rusty_key(op), 0)
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
        and STATE_OVERRIDES.get(op, prepare_class.get(rusty_key(op), STATE_NONE)) == STATE_NONE
    ]
    if ungated_vector:
        print("ERROR: VEX/EVEX opcodes with no state class:", file=sys.stderr)
        for op in sorted(ungated_vector):
            print(f"  {op} — add it to STATE_OVERRIDES", file=sys.stderr)
        return 1

    prepare_counts = {}
    for op in opcodes:
        cls = STATE_OVERRIDES.get(op, prepare_class.get(rusty_key(op), STATE_NONE))
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
        "/// Bochs `cpu/decoder/features.h`'s features in declaration order, the",
        "/// `BX_ISA_` prefix dropped. `X86Feature` keeps exactly this order, so",
        "/// the numbers above mean what Bochs's do; a decoder test pins it.",
        "#[cfg(test)]",
        f"pub(crate) const BOCHS_ISA_FEATURES: [&str; {len(bochs_features)}] = [",
    ]
    for f in bochs_features:
        lines.append(f'    "{f}",')
    lines += [
        "];",
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
