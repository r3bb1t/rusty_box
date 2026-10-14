#!/usr/bin/env python3
"""Generate the EVEX per-opcode operand tables.

Two things about an EVEX instruction cannot be derived from its encoding
alone, only from the opcode's operand list:

  * which ModRM fields name the register it writes and the register it reads
    through ModRM. Most EVEX opcodes write the reg field and read rm, but the
    store forms (VEXTRACT*, the truncating VPMOV* stores, VCOMPRESS*, VPEXTR*,
    VSCATTER*) write rm and read reg, the shift/rotate-by-immediate groups
    write EVEX.vvvv, and TILEMOVROW/TILEMOVCOL `Trm, Wdq` write and read rm.

  * the size of the memory element it touches, which is the N in EVEX's
    compressed displacement: a mod=01 memory operand stores disp8 already
    divided by N.

Upstream keeps both in ia_opcodes_evex.def, where every operand is an `OP_*`
constant defined in fetchdecode.h as `BX_FORM_SRC(type, src)`, or an alias of
one (`OP_Mb = OP_Eb`), which every table here follows. The `src` of the first
operand gives the destination field, the `src` of the first later ModRM
operand gives the source field, and the `type` of the memory operand feeds
`evex_displ8_compression` (cpu/decoder/fetchdecode32.cc).

Both tables are read from upstream so neither can drift from the definitions
it describes.

Three more facts feed only the test that holds `Instruction::typed()` to the
same definitions: how many distinct register and memory operands an entry
lists, how many immediates, and which ModRM forms it executes in (the entry's
`execute1` handler is its memory form, `execute2` its register form; a form
Bochs never executes has `NULL` or `BxError` there). All are emitted under
`#[cfg(test)]`.

Usage:  python scripts/gen_evex_operands.py
"""

import io
import os
import re
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DEC = os.path.join(ROOT, "cpp_orig", "bochs", "bochs", "cpu", "decoder")
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

# Sources that name a register through a ModRM field. The decoder fills the
# one register source that comes from ModRM; the vvvv source has its own slot.
MODRM_SRC = {
    "BX_SRC_NNN": "Nnn",
    "BX_SRC_RM": "Rm",
    "BX_SRC_VECTOR_RM": "Rm",
    "BX_SRC_VSIB": "Rm",
}

# (destination field, ModRM source field) -> the `EvexDst` variant that tells
# the decoder both. `None` is an opcode with no ModRM register source. A pair
# missing here stops the run, since the decoder has no rule for it.
LAYOUT = {
    ("Nnn", "Rm"): "Nnn",
    ("Nnn", None): "Nnn",
    ("Rm", "Nnn"): "Rm",
    ("Rm", None): "Rm",
    # fetchdecode.h: OP_Trm = BX_FORM_SRC(BX_TMM_REG, BX_SRC_RM) and
    # OP_Wdq = BX_FORM_SRC(BX_VMM_FULL_VECTOR, BX_SRC_VECTOR_RM).
    ("Rm", "Rm"): "RmSourceRm",
    ("Vvvv", "Rm"): "Vvvv",
    ("Vvvv", None): "Vvvv",
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


def parse_op_aliases(text):
    """OP_Mb = OP_Eb."""
    return dict(re.findall(r"const\s+Bit8u\s+(OP_\w+)\s*=\s*(OP_\w+)\s*;", text))


def resolve_operand(name, ops, aliases):
    """The (type, src) an operand constant names, following aliases; None when
    it names neither a `BX_FORM_SRC` constant nor an alias of one."""
    seen = set()
    while name in aliases and name not in seen:
        seen.add(name)
        name = aliases[name]
    return ops.get(name)


# A def entry's handler for a form Bochs never executes.
NO_HANDLER = {"NULL", "&BX_CPU_C::BxError"}

# (memory form executes, register form executes) -> the `EvexForms` variant.
FORMS = {
    (True, True): "RegisterAndMemory",
    (False, True): "Register",
    (True, False): "Memory",
}


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
    hdr = read(HDR)
    ops = parse_op_constants(hdr)
    aliases = parse_op_aliases(hdr)
    rust_names = rust_opcode_names()

    dsts, tuples, counts, immediates, forms = {}, {}, {}, {}, {}
    for line in read(DEF).splitlines():
        m = DEF_RE.match(line)
        if not m:
            continue
        rust = rust_names.get(m.group(1).replace("_", "").lower())
        if rust is None:
            continue  # not implemented here; decodes to IaError anyway

        # bx_define_opcode(name, disasm, disasm, execute1, execute2, isa,
        # src1, src2, src3, src4, prepare): execute1 runs the memory form,
        # execute2 the register form.
        fields = [f.strip() for f in m.group(2).split(",")]
        if len(fields) != 10:
            sys.exit(f"{m.group(1)}: expected 10 fields after the name, got {len(fields)}")
        executes = (fields[2] not in NO_HANDLER, fields[3] not in NO_HANDLER)
        if executes not in FORMS:
            sys.exit(f"{m.group(1)}: neither form has a handler")
        forms.setdefault(rust, FORMS[executes])

        # Register and memory operands: every operand but OP_NONE and the
        # immediates, which Bochs forms as `BX_FORM_SRC(BX_IMM*, BX_SRC_NONE)`.
        # One that repeats an earlier operand's type and source names the same
        # register (the FMA forms list their destination again as a source),
        # so it counts once.
        # OP_NONE is `BX_SRC_NONE` itself, an empty slot; any other name the
        # header does not define stops the run rather than going uncounted.
        resolved = []
        for name in OP_RE.findall(m.group(2)):
            if name == "OP_NONE":
                continue
            r = resolve_operand(name, ops, aliases)
            if r is None:
                sys.exit(
                    f"{m.group(1)}: operand {name} is neither a BX_FORM_SRC constant "
                    f"nor an alias of one in fetchdecode.h"
                )
            resolved.append(r)
        for typ, src in resolved:
            if src == "BX_SRC_NONE" and not typ.startswith("BX_IMM"):
                sys.exit(f"{m.group(1)}: operand ({typ}, {src}) is neither an immediate nor sourced")
        registers = []
        for r in resolved:
            if r[1] != "BX_SRC_NONE" and r not in registers:
                registers.append(r)
        counts.setdefault(rust, len(registers))
        immediates.setdefault(rust, sum(1 for r in resolved if r[1] == "BX_SRC_NONE"))

        if not resolved:
            continue

        # Destination: the first operand, with aliases followed, so the
        # `OP_Mb`/`OP_Mw` stores of VPEXTRB/VPEXTRW take `OP_Eb`/`OP_Ew`'s
        # `BX_SRC_RM`. Its ModRM register source: the first later operand that
        # names a register through ModRM and is not the destination read back
        # (the FMA forms list `Vps` twice).
        _typ, src = resolved[0]
        if src in DST_SRC:
            modrm_src = None
            for operand in resolved[1:]:
                if operand != resolved[0] and operand[1] in MODRM_SRC:
                    modrm_src = MODRM_SRC[operand[1]]
                    break
            pair = (DST_SRC[src], modrm_src)
            if pair not in LAYOUT:
                sys.exit(
                    f"{m.group(1)}: destination {pair[0]} with ModRM source {pair[1]} "
                    f"has no decoder layout; add one to LAYOUT and to EvexDst"
                )
            dsts.setdefault(rust, LAYOUT[pair])

        # disp8 scale: the first operand that can name memory, aliases
        # followed (`OP_Mw` is `OP_Ew`, a `BX_GPR16` memory operand: N = 2).
        for typ, src in resolved:
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
    a("//! The destination and disp8 tables come from the operand lists in Bochs's")
    a("//! `cpu/decoder/ia_opcodes_evex.def`, where each `OP_*` is")
    a("//! `BX_FORM_SRC(type, src)` or an alias of one (`OP_Mb = OP_Eb`), which")
    a("//! is followed. The first operand's `src` gives the")
    a("//! destination field and the first later ModRM operand's `src` the")
    a("//! source field; the memory operand's `type` gives the disp8 scale")
    a("//! that `evex_displ8_compression` computes upstream. The test-only")
    a("//! operand and immediate counts and executed forms come from the same")
    a("//! entries.")
    a("")
    a("use crate::opcode::Opcode;")
    a("")
    a("/// Which ModRM fields name the register an EVEX opcode writes and the")
    a("/// register it reads through ModRM.")
    a("///")
    a("/// Most write the reg field and read rm. The store forms — VEXTRACT*, the")
    a("/// truncating VPMOV* stores, VCOMPRESS*, VPEXTR*, VSCATTER* — write rm and")
    a("/// read reg, and the shift/rotate-by-immediate groups write EVEX.vvvv and")
    a("/// read rm.")
    a("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    a("pub(crate) enum EvexDst {")
    a("    Nnn,")
    a("    Rm,")
    a("    /// Writes rm and reads rm: Bochs TILEMOVROW/TILEMOVCOL `Trm, Wdq`, whose")
    a("    /// fetchdecode.h operands `OP_Trm` (`BX_SRC_RM`) and `OP_Wdq`")
    a("    /// (`BX_SRC_VECTOR_RM`) both name rm.")
    a("    RmSourceRm,")
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
    a("/// Distinct register and memory operands in an EVEX opcode's def entry:")
    a("/// every `OP_*` it lists but `OP_NONE` and the immediates (`BX_SRC_NONE`),")
    a("/// with aliases such as `OP_Mb = OP_Eb` followed, and an operand that")
    a("/// repeats an earlier one's type and source (the FMA forms' destination,")
    a("/// listed again as a source) counted once; 0 for an opcode with no EVEX")
    a("/// def entry. Read by the typed-view test.")
    a("#[cfg(test)]")
    a("pub(crate) const fn evex_operand_count(op: Opcode) -> u8 {")
    a("    match op {")
    for rust in sorted(counts):
        a(f"        Opcode::{rust} => {counts[rust]},")
    a("        _ => 0,")
    a("    }")
    a("}")
    a("")
    a("/// Immediates (`BX_IMM*` operands) in an EVEX opcode's def entry; none")
    a("/// unless listed. Read by the typed-view test.")
    a("#[cfg(test)]")
    a("pub(crate) const fn evex_immediate_count(op: Opcode) -> u8 {")
    a("    match op {")
    for rust in sorted(k for k, v in immediates.items() if v != 0):
        a(f"        Opcode::{rust} => {immediates[rust]},")
    a("        _ => 0,")
    a("    }")
    a("}")
    a("")
    a("/// The ModRM forms an EVEX opcode executes in. Bochs's def entry names a")
    a("/// handler per form, `execute1` for memory and `execute2` for register,")
    a("/// and leaves it `NULL` or `BxError` for a form that never executes.")
    a("#[cfg(test)]")
    a("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    a("pub(crate) enum EvexForms {")
    a("    RegisterAndMemory,")
    a("    Register,")
    a("    Memory,")
    a("}")
    a("")
    a("/// The forms an EVEX opcode executes in; both unless listed. Read by the")
    a("/// typed-view test.")
    a("#[cfg(test)]")
    a("pub(crate) const fn evex_forms(op: Opcode) -> EvexForms {")
    a("    match op {")
    for rust in sorted(k for k, v in forms.items() if v != "RegisterAndMemory"):
        a(f"        Opcode::{rust} => EvexForms::{forms[rust]},")
    a("        _ => EvexForms::RegisterAndMemory,")
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
    print(f"opcodes with an operand count    : {len(counts)}")
    for k, n in sorted(Counter(counts.values()).items()):
        print(f"    {k:<14d} {n}")
    print(f"opcodes with an immediate        : {sum(1 for v in immediates.values() if v)}")
    for k, n in sorted(Counter(immediates.values()).items()):
        print(f"    {k:<14d} {n}")
    print(f"opcodes by executed forms        : {len(forms)}")
    for k, n in Counter(forms.values()).most_common():
        print(f"    {k:17s} {n}")
    print(f"wrote {os.path.relpath(OUT, ROOT)}")


if __name__ == "__main__":
    main()
