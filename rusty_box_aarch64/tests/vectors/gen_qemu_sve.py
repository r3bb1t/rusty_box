#!/usr/bin/env python3
"""Generate single-instruction test vectors for the SVE instructions FEAT_SME
makes legal in Streaming SVE mode, with QEMU's user-mode emulator.

Each vector is gen_qemu.py's, run in Streaming SVE mode at a 512-bit
vector length with ZA storage enabled: a word from a template family, a
seed that both this script and the Rust test (tests/unit/difftest.rs,
`replay_sve`) turn into the same initial state, and what one step
changed. The state is gen_unicorn.py's (`initial_state`), with Z0-Z31 and
P0-P15 in place of the SIMD&FP registers (`sve_state`), and ZA and ZT0,
zero but for the ZA instructions' kinds (`za_state`, `zt0_state`). The
program,
qemu_sve_harness.c with qemu_sve_trampoline.S, runs each word under
`qemu-aarch64 -cpu max,sme_fa64=off,sme-default-vector-length=64`:
Streaming SVE mode without FEAT_SME_FA64, as this core implements it, so
a word illegal in it raises a signal there too. A word the program could
not run is dropped, and so is one QEMU itself aborts on; the harness
restarts after it.

A family is an Arm encoding class of the A64 `sve` group, or of the `sme`
group for the instructions on ZA and ZT0 (the kinds beginning with Z) and
the multi-vector loads and stores, its word the class's fixed bits with
the rest random (unallocated words are dropped). Families are appended,
never reordered.

A vector line is gen_unicorn.py's with `z=` and `p=` (the registers that
changed, each as one big-endian hex number, 128 and 16 digits wide) in
place of `v=`, and for the Z kinds `za=` (ZA's horizontal vectors that
changed, by number, 128 digits wide) and `zt=` (ZT0 as `0:` and 128
digits, if it changed):

    family kind word seed pc nzcv r=... z=... p=... m=... f=fpsr [za=... zt=...]

Needs an AArch64 Linux C compiler that links statically (CC, default
aarch64-linux-gnu-gcc) and QEMU's user-mode emulator (QEMU, default
qemu-aarch64). The vectors in the tree were made with QEMU 11.1.2 with
docs/patches/qemu/qemu-afp-vector-fabs-fneg.patch applied, which changes nothing in
Streaming SVE mode, docs/patches/qemu/qemu-sve-fcmla-indexed-ah.patch,
docs/patches/qemu/qemu-sve-fmad-size00.patch and docs/patches/qemu/qemu-sme-ld1-vertical-tail.patch,
which bring FCMLA (indexed), FMAD and the SME LD1 to a vertical tile slice
to the Arm ARM (docs/qemu-upstream-bugs.md, entries 13 to 15), and
docs/patches/qemu/qemu-sme2-movt-debug-state.patch, which changes no family's word
(entry 16). QEMU is a
second implementation, not the reference: where it and this core differ,
the Arm ARM's pseudocode decides.

Usage: python3 -I gen_qemu_sve.py OUT_FILE [CASES_PER_FAMILY]
"""
import os
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from gen_unicorn import (  # noqa: E402
    BF16_SPECIAL,
    CODE,
    DATA_SIZE,
    F16_SPECIAL,
    F32_SPECIAL,
    F64_SPECIAL,
    MASK64,
    SplitMix64,
    initial_state,
    ordinary_bf16,
    ordinary_f16,
    ordinary_f32,
    ordinary_f64,
)

# The Streaming SVE vector length in bytes, and the predicate length.
VLB = 64
PLB = VLB // 8
# ZT0's length in bytes.
ZT0B = 64
# A record as qemu_sve_harness.c lays it out: a 16-byte header (the word or
# the status, then padding), X0-X30, SP, NZCV, FPCR, FPSR, padding, Z0-Z31,
# P0-P15, ZA's horizontal vectors, ZT0.
RECORD = struct.Struct(f"<I12x31Q5Q{32 * VLB}s{16 * PLB}s{VLB * VLB}s{ZT0B}s")

# 64-bit lanes a Z register may be filled with: zeros, ones, the sign bits
# and the extremes of each element size.
Z_SPECIAL = [
    0x0000000000000000,
    0xFFFFFFFFFFFFFFFF,
    0x8000000000000000,
    0x7FFFFFFFFFFFFFFF,
    0x8000000080000000,
    0x7FFFFFFF7FFFFFFF,
    0x8000800080008000,
    0x7FFF7FFF7FFF7FFF,
    0x8080808080808080,
    0x7F7F7F7F7F7F7F7F,
    0x0000000000000001,
    0x0101010101010101,
    0xFFFFFFFF00000000,
    0x00000000FFFFFFFF,
]

# Canonical predicates of each element size: one bit per element.
P_ELEMENTS = [0xFFFFFFFFFFFFFFFF, 0x5555555555555555, 0x1111111111111111, 0x0101010101010101]

# The formats a kind SF register's lanes may take, by the `size` field that
# names them (00 standing for BFloat16): each lane's width, its special
# values and its ordinary values.
FP_FORMATS = [
    (16, BF16_SPECIAL, ordinary_bf16),
    (16, F16_SPECIAL, ordinary_f16),
    (32, F32_SPECIAL, ordinary_f32),
    (64, F64_SPECIAL, ordinary_f64),
]


def fp_register(r, size):
    """A Z register of lanes of the format `size` names, each a special value
    or an ordinary one, half and half."""
    width, special, ordinary = FP_FORMATS[size]
    value = 0
    for k in range(VLB * 8 // width):
        w = r.next()
        lane = special[(w >> 1) % len(special)] if w & 1 else ordinary(w)
        value |= lane << (width * k)
    return value


def int_register(r, size, extreme):
    """A Z register of elements of `size` (`8 << size` bits): with `extreme`
    each zero, one, minus one, the largest or the smallest value, or one
    above the smallest; else each a shift amount from -(esize+2) to
    esize+2."""
    n = 8 << size
    mask = (1 << n) - 1
    value = 0
    for k in range(VLB * 8 // n):
        w = r.next()
        if extreme:
            element = [0, 1, mask, mask >> 1, (mask >> 1) + 1, (mask >> 1) + 2][w % 6]
        else:
            element = ((w % (2 * n + 5)) - (n + 2)) & mask
        value |= element << (n * k)
    return value


def sve_state(seed, kind, insn):
    """Z0-Z31 and P0-P15 (as integers, little-endian over their bytes) and
    the FPCR bits the kind adds. Must match `difftest::sve_state` in Rust."""
    r = SplitMix64(seed ^ 0x5356_455F_5354_4154)
    z = []
    for _ in range(32):
        sel = r.next() & 7
        lanes = []
        for _ in range(VLB // 8):
            w = r.next()
            if sel == 0:
                # Every byte in -16..15: shift amounts and indexes in range.
                signs = r.next()
                lanes.append((w & 0x0F0F0F0F0F0F0F0F) | ((signs & 0x0101010101010101) * 0xF0))
            elif sel == 1:
                lanes.append(w & 0x0707070707070707)
            elif sel == 2:
                lanes.append(Z_SPECIAL[w % len(Z_SPECIAL)])
            elif sel == 3:
                lanes.append(0)
            else:
                lanes.append(w)
        z.append(sum(lane << (64 * i) for i, lane in enumerate(lanes)))
    p = []
    for _ in range(16):
        sel = r.next() & 7
        w = r.next()
        if sel == 0:
            p.append(w)
        elif sel == 1:
            p.append(MASK64)
        elif sel == 2:
            p.append(0)
        elif sel < 7:
            p.append(P_ELEMENTS[sel - 3])
        else:
            p.append(w & P_ELEMENTS[r.next() & 3])
    fpcr = 0
    if kind == "SA":
        # FPCR.AH, which FABS and FNEG read.
        fpcr = (r.next() & 1) << 1
    if kind in ("SF", "SS", "ZF"):
        # Floating-point inputs: per register, the state above, or lanes of
        # floating-point values, half of the registers in the format the
        # word names and the rest in half, single or double precision; then
        # a random FPCR (RMode, FZ, DN, AHP, FZ16, and FEAT_AFP's FIZ, AH
        # and NEP). The format is the one the `size` field names, single
        # precision for the conversions of words (SS), or for an outer
        # product (ZF) its sources' (`za_fp_format`).
        own = (insn >> 22) & 3 if kind == "SF" else 2 if kind == "SS" else za_fp_format(insn)
        for i in range(32):
            sel = r.next() & 7
            if sel != 0:
                z[i] = fp_register(r, own if sel <= 4 else sel - 4)
        fpcr = ((r.next() & 0x1F) << 22) | ((r.next() & 1) << 19) | (r.next() & 7)
    if kind == "SI":
        # SVE2 integer inputs: per register, the state above, or elements
        # of the size the word's `size` field names, each a shift amount
        # in range or an extreme, or elements half that size (bytes for
        # `size = 00`), each an extreme: a long operation's sources.
        own = (insn >> 22) & 3
        for i in range(32):
            sel = r.next() & 3
            if sel == 1:
                z[i] = int_register(r, own, False)
            elif sel == 2:
                z[i] = int_register(r, own, True)
            elif sel == 3:
                z[i] = int_register(r, max(own - 1, 0), True)
    if kind == "SN":
        # A narrow's sources, words or doublewords: per register, the state
        # above, or extremes of words or of doublewords, or words each a
        # shift amount in range.
        for i in range(32):
            sel = r.next() & 3
            if sel == 1:
                z[i] = int_register(r, 2, True)
            elif sel == 2:
                z[i] = int_register(r, 3, True)
            elif sel == 3:
                z[i] = int_register(r, 2, False)
    return z, p, fpcr


def za_fp_format(insn):
    """The `FP_FORMATS` index of a floating-point ZA instruction's source
    elements. An outer product's (bit 30 clear): single precision (bit 24
    clear), BFloat16 (bit 24 set, bit 21 clear) or half precision. A
    multi-vector instruction's on ZA array vectors (bit 30 set): single
    precision for FMLA, FMLS, FADD and FSUB (bits 12:11 = 11 by vectors, bit
    21 set; bits 23:22 = 01 with bits 12, 5 and 3 clear by an indexed
    element), else BFloat16 (bit 4 set) or half precision."""
    if (insn >> 30) & 1:
        by_vectors = (insn >> 21) & 1
        if (by_vectors and (insn >> 11) & 3 == 3) or (not by_vectors and (insn >> 22) & 3 == 1 and insn & 0x1028 == 0):
            return 2
        return 0 if (insn >> 4) & 1 else 1
    if not (insn >> 24) & 1:
        return 2
    return 1 if (insn >> 21) & 1 else 0


def za_state(seed, kind):
    """ZA's horizontal vectors (as integers, little-endian over their bytes):
    zero but for the Z kinds, whose each is zero, lanes of single-precision
    values (the floating-point outer products' accumulators), lanes of
    `Z_SPECIAL` or random. Must match `difftest::za_state` in Rust."""
    za = [0] * VLB
    if not kind.startswith("Z"):
        return za
    r = SplitMix64(seed ^ 0x5A41_5F53_5441_5445)
    for i in range(VLB):
        sel = r.next() & 3
        if sel == 1:
            za[i] = fp_register(r, 2)
        elif sel == 2:
            za[i] = sum(Z_SPECIAL[r.next() % len(Z_SPECIAL)] << (64 * k) for k in range(VLB // 8))
        elif sel == 3:
            za[i] = sum(r.next() << (64 * k) for k in range(VLB // 8))
    return za


def zt0_state(seed, kind):
    """ZT0 (as an integer, little-endian over its bytes): zero but for the Z
    kinds, whose ZT0 is random. Must match `difftest::zt0_state` in Rust."""
    if not kind.startswith("Z"):
        return 0
    r = SplitMix64(seed ^ 0x5A54_305F_5354_4154)
    return sum(r.next() << (64 * k) for k in range(ZT0B // 8))


def movprfx_unpredicated(w):
    """MOVPRFX (unpredicated): `opc` and `opc2` zero, its only encoding."""
    return w & ~0x00DF0000


def read_vl(w):
    """RDVL, RDSVL: `op = 0`, `opc2 = 11111`."""
    return (w & ~0x00400000) | 0x001F0000


def ptest(w):
    """PTEST: `op = 0`, `S = 1`, `opc2 = 0000`."""
    return (w & ~0x00C0000F) | 0x00400000


def structures(w):
    """LD2-LD4, ST2-ST4: `opc` (bits 22:21) nonzero, 00 being LDNT1 and
    STNT1."""
    return w | 0x00200000 if (w >> 21) & 3 == 0 else w


def not_spill(w):
    """ST1 (scalar plus scalar): `opc = 110` is STR (vector), so ST1H's
    010 in its place."""
    return w & ~0x01000000 if (w >> 22) & 7 == 0b110 else w


def fp_size(w):
    """`size = 00`, where a class's floating-point encodings carry Arm's
    `size != '00'`, is FEAT_SVE_B16B16's or FEAT_SVE_BFSCALE's BFloat16
    form, and is UNDEFINED in the rest: half precision in its place."""
    return w | 0x00400000 if (w >> 22) & 3 == 0 else w


def fp_arith_pred(w):
    """FADD to FDIV (vectors, predicated), `fp_size`'s: and `opc = 111x`,
    FAMAX and FAMIN (FEAT_FAMINMAX), as FDIVR and FDIV."""
    w = fp_size(w)
    return w & ~0x00020000 if (w >> 17) & 7 == 0b111 else w


def fp_indexed(w):
    """FMLA, FMLS, FMUL (indexed): `o2 = 1` is FEAT_SVE_B16B16's BFMLA,
    BFMLS and BFMUL."""
    return w & ~0x00000800


def bfdot_indexed(w):
    """BFDOT (indexed): `op = 1`, `opc2 = 00`; FDOT (`op = 0`), FEAT_SME2's,
    has a family of its own, and the rest is FP8's."""
    return (w | 0x00400000) & ~0x00000C00


def bfdot(w):
    """BFDOT (vectors): `op = 1`, `o2 = 0`; FDOT (`op = 0`), FEAT_SME2's,
    has a family of its own, and `o2 = 1` is FP8's."""
    return (w | 0x00400000) & ~0x00000400


def not_bfmlsl(w):
    """FMLALB to BFMLALT: `o2 = 1` with `op = 1`, BFMLSLB and BFMLSLT,
    FEAT_SME2's, have a family of their own, so BFMLALB and BFMLALT in
    their place."""
    return w & ~0x00002000 if (w >> 22) & 1 and (w >> 13) & 1 else w


def size_1x(w):
    """SDOT and UDOT, by vector and indexed: `size<1> = 0` is the
    halfword-to-word forms of FEAT_SVE2p3 and FEAT_SME2p3, or unallocated,
    so `size<1>` set."""
    return w | 0x00800000


def size_10(w):
    """USDOT and SUDOT: `size = 10` alone."""
    return (w & ~0x00400000) | 0x00800000


def merging(w):
    """URECPE, URSQRTE, SQABS, SQNEG: `Z = 1` is the zeroing forms of
    FEAT_SVE2p2 and FEAT_SME2p2, so `Z` clear."""
    return w & ~0x00020000


def not_subp(w):
    """ADDP to UMINP: `opc = 00` with `U = 0` is SUBP, FEAT_SVE2p3's and
    FEAT_SME2p3's, so ADDP in its place."""
    return w | 0x00010000 if (w >> 17) & 3 == 0 else w


def fp_minmax_groups(w):
    """FMAX to FMINNM of a group by a group, `fp_size`'s: and `opc = 1x`,
    FAMAX and FAMIN (FEAT_FAMINMAX) or unallocated, as `opc = 0x`."""
    return fp_size(w) & ~0x00000040


def not_pmull128(w):
    """PMULLB, PMULLT (`op = 0`, `U = 1`): `size = 00` is FEAT_SVE_PMULL128's
    128-bit form, so `size = 01` in its place."""
    pmull = (w >> 12) & 1 == 0 and (w >> 11) & 1 == 1
    return w | 0x00400000 if pmull and (w >> 22) & 3 == 0 else w


def not_64bit(w):
    """`sz = 1`, FEAT_SME_I16I64's or FEAT_SME_F64F64's 64-bit form of an
    instruction on ZA array vectors: `sz = 0` in its place."""
    return w & ~0x00400000


def not_fp8_long(w):
    """FMLAL of a group by one vector with `o2 = 1`, FEAT_SME_F8F16's form
    of FP8 values or unallocated, as `o2 = 0`."""
    return w & ~0x00000004


def not_fp8_dot(w):
    """FDOT and BFDOT of a group by one vector, `opc = x1` FEAT_SME_F8F16's
    and FEAT_SME_F8F32's forms of FP8 values, as `opc = x0`."""
    return w & ~0x00000008


def not_fp8_indexed_dot(w):
    """FDOT of FP8 values by an indexed group, FEAT_SME_F8F32's `op = 0,
    opc2 = 111` of two vectors and `opc2 = 001` of four, as `opc2 = 100`
    and `010`: SVDOT and FMLS, which the classes' samples reach least."""
    if (w >> 12) & 1:
        return w
    if (w >> 15) & 1:
        return (w & ~0x00000008) | 0x00000010 if (w >> 3) & 7 == 0b001 else w
    return w & ~0x00000018 if (w >> 3) & 7 == 0b111 else w


def not_fp8_dot_groups(w):
    """FDOT and BFDOT of a group by a group, `opc = 1x` FEAT_SME_F8F16's and
    FEAT_SME_F8F32's forms of FP8 values, as `opc = 0x`."""
    return w & ~0x00000020


# (name, kind, fixed bits, free bits, fixup): Arm's encoding classes of the
# A64 `sve` group whose encodings FEAT_SME makes legal, with `op0 = 000`
# and `001`, then the loads, stores and prefetches (`op0 = 100`, `101` and
# `111`, kind M, or R for scalar plus scalar), then the floating-point
# instructions (`op0 = 011`, kind SF), then SVE2's integer instructions
# (`op0 = 010`, kind SI), `op1<7> = 0` first, then the ZA instructions of
# the A64 `sme` group (kind Z, ZF for the floating-point outer products,
# ZR and ZM for the loads and stores as R and M), then FEAT_SME2's: the
# predicate-as-counter classes of the `sve` group (kind S), the
# multi-vector loads and stores of the `sme` group (R and M), ZERO, LDR
# and STR of ZT0 (Z and ZM), and the multi-vector SVE operations of the
# `sme` group (S for SEL and the permutes, SI for the integer operations,
# SN for the narrows, SF for the floating-point operations and SS for the
# conversions of words), its multi-vector instructions on ZA array vectors
# (Z, ZF for the floating-point ones), and BMOPA and BMOPS, the outer
# products of halfwords, MOVA of a group and LUTI2 and LUTI4 (Z); then the
# SVE2.1 instructions FEAT_SME2 makes legal: the two-way dot products
# (SI), the narrows of two vectors (SN) and FCLAMP (SF), each a class, and
# FDOT and BFMLSLB and BFMLSLT (SF), named for their first encoding, each
# its class's fixed bits and its own, as their classes' families move
# them to BFDOT and BFMLALB; their fixed bits from AARCHMRS 2026-09. FMAX to FCLAMP's words whose encodings this processor
# does not implement, the BFloat16 forms and FAMAX and FAMIN, are moved to
# implemented ones as the SVE floating-point classes' are, and so are the
# ZA array vector instructions' 64-bit forms and forms of FP8 values. A
# word of a class that is unallocated (MOVA's `Q = 1` with a `size` other
# than 11, PEXT's `opc = 11x`) raises a signal and is dropped. MOVT
# between ZT0 and a general register, which is defined only in Debug
# state, has no family.
FAMILIES = [
    ("sve_int_bin_pred_arit_0", "S", 0x04000000, 0x00C71FFF, None),
    ("sve_int_bin_pred_arit_1", "S", 0x04080000, 0x00C71FFF, None),
    ("sve_int_bin_pred_arit_2", "S", 0x04100000, 0x00C31FFF, None),
    ("sve_int_bin_pred_div", "S", 0x04140000, 0x00C31FFF, None),
    ("sve_int_bin_pred_log", "S", 0x04180000, 0x00C71FFF, None),
    ("sve_int_reduce_0", "S", 0x04002000, 0x00C31FFF, None),
    ("sve_int_reduce_1", "S", 0x04082000, 0x00C31FFF, None),
    ("sve_int_movprfx_pred", "S", 0x04102000, 0x00C71FFF, None),
    ("sve_int_reduce_2", "S", 0x04182000, 0x00C31FFF, None),
    ("sve_int_bin_pred_shift_0", "S", 0x04008000, 0x00CF1FFF, None),
    ("sve_int_bin_pred_shift_1", "S", 0x04108000, 0x00C71FFF, None),
    ("sve_int_bin_pred_shift_2", "S", 0x04188000, 0x00C71FFF, None),
    ("sve_int_un_pred_arit_0", "S", 0x0400A000, 0x00D71FFF, None),
    ("sve_int_un_pred_arit_1", "SA", 0x0408A000, 0x00D71FFF, None),
    ("sve_int_mlas_vvv_pred", "S", 0x04004000, 0x00DF3FFF, None),
    ("sve_int_mladdsub_vvv_pred", "S", 0x0400C000, 0x00DF3FFF, None),
    ("sve_int_bin_cons_arit_0", "S", 0x04200000, 0x00DF1FFF, None),
    ("sve_int_bin_cons_log", "S", 0x04203000, 0x00DF03FF, None),
    ("sve_int_rotate_imm", "S", 0x04203400, 0x00DF03FF, None),
    ("sve_int_tern_log", "S", 0x04203800, 0x00DF07FF, None),
    ("sve_int_bin_cons_shift_a", "S", 0x04208000, 0x00DF0FFF, None),
    ("sve_int_bin_cons_shift_b", "S", 0x04209000, 0x00DF0FFF, None),
    ("sve_int_bin_cons_misc_0_d", "S", 0x0420BC00, 0x00DF03FF, movprfx_unpredicated),
    ("sve_int_index_ii", "S", 0x04204000, 0x00DF03FF, None),
    ("sve_int_index_ri", "S", 0x04204400, 0x00DF03FF, None),
    ("sve_int_index_ir", "S", 0x04204800, 0x00DF03FF, None),
    ("sve_int_index_rr", "S", 0x04204C00, 0x00DF03FF, None),
    ("sve_int_arith_vl", "S", 0x04205000, 0x005F07FF, None),
    ("sve_int_arith_svl", "S", 0x04205800, 0x005F07FF, None),
    ("sve_int_read_vl_a", "S", 0x04A05000, 0x005F07FF, read_vl),
    ("sve_int_read_svl_a", "S", 0x04A05800, 0x005F07FF, read_vl),
    ("sve_int_mul_b", "S", 0x04206000, 0x00DF0FFF, None),
    ("sve_int_sqdmulh", "S", 0x04207000, 0x00DF07FF, None),
    ("sve_int_countvlv0", "S", 0x0420C000, 0x00CF0FFF, None),
    ("sve_int_countvlv1", "S", 0x0430C000, 0x00CF07FF, None),
    ("sve_int_count", "S", 0x0420E000, 0x00CF07FF, None),
    ("sve_int_pred_pattern_a", "S", 0x0430E000, 0x00CF07FF, None),
    ("sve_int_pred_pattern_b", "S", 0x0420F000, 0x00DF0FFF, None),
    ("sve_int_log_imm", "S", 0x05000000, 0x00C3FFFF, None),
    ("sve_int_dup_mask_imm", "S", 0x05C00000, 0x0003FFFF, None),
    ("sve_int_dup_imm_pred", "S", 0x05100000, 0x00CF7FFF, None),
    ("sve_int_dup_fpimm_pred", "S", 0x0510C000, 0x00CF1FFF, None),
    ("sve_int_perm_extract_i", "S", 0x05200000, 0x001F1FFF, None),
    ("sve_intx_perm_extract_i", "S", 0x05600000, 0x001F1FFF, None),
    ("sve_int_perm_dup_i", "S", 0x05202000, 0x00DF03FF, None),
    ("sve_int_perm_tbl_3src", "S", 0x05202800, 0x00DF07FF, None),
    ("sve_int_perm_tbl", "S", 0x05203000, 0x00DF03FF, None),
    ("sve_int_perm_dup_r", "S", 0x05203800, 0x00C003FF, None),
    ("sve_int_perm_insrs", "S", 0x05243800, 0x00C003FF, None),
    ("sve_int_perm_unpk", "S", 0x05303800, 0x00C303FF, None),
    ("sve_int_perm_insrv", "S", 0x05343800, 0x00C003FF, None),
    ("sve_int_perm_reverse_z", "S", 0x05383800, 0x00C003FF, None),
    ("sve_int_perm_bin_perm_pp", "S", 0x05204000, 0x00CF1DEF, None),
    ("sve_int_perm_punpk", "S", 0x05304000, 0x000101EF, None),
    ("sve_int_perm_reverse_p", "S", 0x05344000, 0x00C001EF, None),
    ("sve_int_perm_bin_perm_zz", "S", 0x05206000, 0x00DF1FFF, None),
    ("sve_int_perm_cpy_v", "S", 0x05208000, 0x00C01FFF, None),
    ("sve_int_perm_last_v", "S", 0x05228000, 0x00C11FFF, None),
    ("sve_int_perm_rev", "S", 0x05248000, 0x00C33FFF, None),
    ("sve_int_perm_clast_zz", "S", 0x05288000, 0x00C11FFF, None),
    ("sve_int_perm_clast_vz", "S", 0x052A8000, 0x00C11FFF, None),
    ("sve_int_perm_splice", "S", 0x052C8000, 0x00C01FFF, None),
    ("sve_intx_perm_splice", "S", 0x052D8000, 0x00C01FFF, None),
    ("sve_int_perm_last_r", "S", 0x0520A000, 0x00C11FFF, None),
    ("sve_int_perm_cpy_r", "S", 0x0528A000, 0x00C01FFF, None),
    ("sve_int_perm_clast_rz", "S", 0x0530A000, 0x00C11FFF, None),
    ("sve_int_perm_revd", "S", 0x052E8000, 0x00C03FFF, None),
    ("sve_int_sel_vvv", "S", 0x0520C000, 0x00DF3FFF, None),
    ("sve_int_cmp_0", "S", 0x24000000, 0x00DFBFFF, None),
    ("sve_int_cmp_1", "S", 0x24004000, 0x00DFBFFF, None),
    ("sve_int_ucmp_vi", "S", 0x24200000, 0x00DFFFFF, None),
    ("sve_int_scmp_vi", "S", 0x25000000, 0x00DFBFFF, None),
    ("sve_int_pred_log", "S", 0x25004000, 0x00CF3FFF, None),
    ("sve_int_brkp", "S", 0x2500C000, 0x00CF3DFF, None),
    ("sve_int_break", "S", 0x25104000, 0x00C03DFF, None),
    ("sve_int_brkn", "S", 0x25184000, 0x00403DEF, None),
    ("sve_int_ptest", "S", 0x2510C000, 0x00C03DEF, ptest),
    ("sve_int_pfirst", "S", 0x2518C000, 0x00C001EF, None),
    ("sve_int_pnext", "S", 0x2519C400, 0x00C001EF, None),
    ("sve_int_ptrue", "S", 0x2518E000, 0x00C103EF, None),
    ("sve_int_pfalse", "S", 0x2518E400, 0x00C0000F, None),
    ("sve_int_pred_dup", "S", 0x25204000, 0x00DF3FEF, None),
    ("sve_int_while_rr", "S", 0x25200000, 0x00DF1FFF, None),
    ("sve_int_cterm", "S", 0x25202000, 0x00DF03F0, None),
    ("sve_int_whilenc", "S", 0x25203000, 0x00DF03FF, None),
    ("sve_int_pcount_pred", "S", 0x25208000, 0x00C73DFF, None),
    ("sve_int_count_v_sat", "S", 0x25288000, 0x00C307FF, None),
    ("sve_int_count_v", "S", 0x252C8000, 0x00C307FF, None),
    ("sve_int_count_r_sat", "S", 0x25288800, 0x00C307FF, None),
    ("sve_int_count_r", "S", 0x252C8800, 0x00C307FF, None),
    ("sve_int_arith_imm0", "S", 0x2520C000, 0x00C73FFF, None),
    ("sve_int_arith_imm1", "S", 0x2528C000, 0x00C73FFF, None),
    ("sve_int_arith_imm2", "S", 0x2530C000, 0x00C73FFF, None),
    ("sve_int_dup_imm", "S", 0x2538C000, 0x00C63FFF, None),
    ("sve_int_dup_fpimm", "S", 0x2539C000, 0x00C63FFF, None),
    ("sve_mem_32b_pfill", "M", 0x85800000, 0x003F1FEF, None),
    ("sve_mem_32b_fill", "M", 0x85804000, 0x003F1FFF, None),
    ("sve_mem_prfm_si", "M", 0x85C00000, 0x003F7FEF, None),
    ("sve_mem_prfm_ss", "R", 0x8400C000, 0x019F1FEF, None),
    ("sve_mem_ld_dup", "M", 0x84408000, 0x01BF7FFF, None),
    ("sve_mem_ldqr_ss", "R", 0xA4000000, 0x01FF1FFF, None),
    ("sve_mem_ldqr_si", "M", 0xA4002000, 0x01EF1FFF, None),
    ("sve_mem_cld_ss", "R", 0xA4004000, 0x01FF1FFF, None),
    ("sve_mem_cld_si", "M", 0xA400A000, 0x01EF1FFF, None),
    ("sve_mem_cldnt_ss", "R", 0xA400C000, 0x019F1FFF, None),
    ("sve_mem_eld_ss", "R", 0xA400C000, 0x01FF1FFF, structures),
    ("sve_mem_cldnt_si", "M", 0xA400E000, 0x018F1FFF, None),
    ("sve_mem_eld_si", "M", 0xA400E000, 0x01EF1FFF, structures),
    ("sve_mem_pspill", "M", 0xE5800000, 0x003F1FEF, None),
    ("sve_mem_cst_ss", "R", 0xE4004000, 0x01FF1FFF, not_spill),
    ("sve_mem_spill", "M", 0xE5804000, 0x003F1FFF, None),
    ("sve_mem_cstnt_ss", "R", 0xE4006000, 0x019F1FFF, None),
    ("sve_mem_est_ss", "R", 0xE4006000, 0x01FF1FFF, structures),
    ("sve_mem_cst_si", "M", 0xE400E000, 0x01EF1FFF, None),
    ("sve_mem_cstnt_si", "M", 0xE410E000, 0x018F1FFF, None),
    ("sve_mem_est_si", "M", 0xE410E000, 0x01EF1FFF, structures),
    ("sve_fp_fcmla", "SF", 0x64000000, 0x00DF7FFF, None),
    ("sve_fp_fcadd", "SF", 0x64008000, 0x00C11FFF, None),
    ("sve_fp_fcvt2", "SF", 0x6408A000, 0x00C31FFF, None),
    ("sve_fp_pairwise", "SF", 0x64108000, 0x00C71FFF, None),
    ("sve_fp_fma_by_indexed_elem", "SF", 0x64200000, 0x00DF0FFF, fp_indexed),
    ("sve_fp_fcmla_by_indexed_elem", "SF", 0x64201000, 0x00DF0FFF, None),
    ("sve_fp_fmul_by_indexed_elem", "SF", 0x64202000, 0x00DF0BFF, fp_indexed),
    ("sve_fp_fdot_by_indexed_elem", "SF", 0x64204000, 0x005F0FFF, bfdot_indexed),
    ("sve_fp_fma_long_by_indexed_elem", "SF", 0x64A04000, 0x005F2FFF, not_bfmlsl),
    ("sve_fp_fdot", "SF", 0x64208000, 0x005F07FF, bfdot),
    ("sve_fp_fma_long", "SF", 0x64A08000, 0x005F27FF, not_bfmlsl),
    ("sve_fp_3op_u_zd", "SF", 0x65000000, 0x00DF1FFF, fp_size),
    ("sve_fp_fast_red", "SF", 0x65002000, 0x00C71FFF, None),
    ("sve_fp_3op_p_pd", "SF", 0x65004000, 0x00DFBFFF, None),
    ("sve_fp_2op_p_zds", "SF", 0x65008000, 0x00CF1FFF, fp_arith_pred),
    ("sve_fp_2op_i_p_zds", "SF", 0x65188000, 0x00C71C3F, None),
    ("sve_fp_2op_p_zd_a", "SF", 0x6500A000, 0x00C71FFF, None),
    ("sve_fp_2op_p_zd_b_0", "SF", 0x6508A000, 0x00C31FFF, None),
    ("sve_fp_2op_p_zd_b_1", "SF", 0x650CA000, 0x00C31FFF, None),
    ("sve_fp_2op_p_zd_c", "SF", 0x6510A000, 0x00C71FFF, None),
    ("sve_fp_2op_p_zd_d", "SF", 0x6518A000, 0x00C71FFF, None),
    ("sve_fp_2op_u_zd", "SF", 0x650E3000, 0x00C103FF, None),
    ("sve_fp_2op_p_pd", "SF", 0x65102000, 0x00C31FFF, None),
    ("sve_fp_3op_p_zds_a", "SF", 0x65200000, 0x00DF7FFF, fp_size),
    ("sve_fp_3op_p_zds_b", "SF", 0x65208000, 0x00DF7FFF, fp_size),
    ("sve_intx_dot", "SI", 0x44000000, 0x00DF07FF, size_1x),
    ("sve_intx_qdmlalbt", "SI", 0x44000800, 0x00DF07FF, None),
    ("sve_intx_cdot", "SI", 0x44001000, 0x00DF0FFF, None),
    ("sve_intx_cmla", "SI", 0x44002000, 0x00DF1FFF, None),
    ("sve_intx_mlal_long", "SI", 0x44004000, 0x00DF1FFF, None),
    ("sve_intx_qdmlal_long", "SI", 0x44006000, 0x00DF0FFF, None),
    ("sve_intx_qrdmlah", "SI", 0x44007000, 0x00DF07FF, None),
    ("sve_intx_mixed_dot", "SI", 0x44007800, 0x00DF03FF, size_10),
    ("sve_intx_bin_pred_shift_sat_round", "SI", 0x44008000, 0x00CF1FFF, None),
    ("sve_intx_pred_arith_unary", "SI", 0x4400A000, 0x00CB1FFF, merging),
    ("sve_intx_accumulate_long_pairs", "SI", 0x4404A000, 0x00C11FFF, None),
    ("sve_intx_pred_arith_binary", "SI", 0x44108000, 0x00C71FFF, None),
    ("sve_intx_arith_binary_pairs", "SI", 0x4410A000, 0x00C71FFF, not_subp),
    ("sve_intx_pred_arith_binary_sat", "SI", 0x44188000, 0x00C71FFF, None),
    ("sve_intx_clamp", "SI", 0x4400C000, 0x00DF07FF, None),
    ("sve_intx_dot_by_indexed_elem", "SI", 0x44200000, 0x00DF07FF, size_1x),
    ("sve_intx_mla_by_indexed_elem", "SI", 0x44200800, 0x00DF07FF, None),
    ("sve_intx_qrdmlah_by_indexed_elem", "SI", 0x44201000, 0x00DF07FF, None),
    ("sve_intx_mixed_dot_by_indexed_elem", "SI", 0x44201800, 0x00DF07FF, size_10),
    ("sve_intx_qdmla_long_by_indexed_elem", "SI", 0x44202000, 0x00DF1FFF, None),
    ("sve_intx_cdot_by_indexed_elem", "SI", 0x44204000, 0x00DF0FFF, None),
    ("sve_intx_cmla_by_indexed_elem", "SI", 0x44206000, 0x00DF0FFF, None),
    ("sve_intx_qrdcmla_by_indexed_elem", "SI", 0x44207000, 0x00DF0FFF, None),
    ("sve_intx_mla_long_by_indexed_elem", "SI", 0x44208000, 0x00DF3FFF, None),
    ("sve_intx_mul_long_by_indexed_elem", "SI", 0x4420C000, 0x00DF1FFF, None),
    ("sve_intx_qdmul_long_by_indexed_elem", "SI", 0x4420E000, 0x00DF0FFF, None),
    ("sve_intx_qdmulh_by_indexed_elem", "SI", 0x4420F000, 0x00DF07FF, None),
    ("sve_intx_mul_by_indexed_elem", "SI", 0x4420F800, 0x00DF03FF, None),
    ("sve_intx_cons_arith_long", "SI", 0x45000000, 0x00DF3FFF, None),
    ("sve_intx_cons_arith_wide", "SI", 0x45004000, 0x00DF1FFF, None),
    ("sve_intx_cons_mul_long", "SI", 0x45006000, 0x00DF1FFF, not_pmull128),
    ("sve_intx_clong", "SI", 0x45008000, 0x00DF0FFF, None),
    ("sve_intx_eorx", "SI", 0x45009000, 0x00DF07FF, None),
    ("sve_intx_shift_long", "SI", 0x4500A000, 0x005F0FFF, None),
    ("sve_intx_aba_long", "SI", 0x4500C000, 0x00DF0FFF, None),
    ("sve_intx_adc_long", "SI", 0x4500D000, 0x00DF07FF, None),
    ("sve_intx_cadd", "SI", 0x4500D800, 0x00C107FF, None),
    ("sve_intx_sra", "SI", 0x4500E000, 0x00DF0FFF, None),
    ("sve_intx_shift_insert", "SI", 0x4500F000, 0x00DF07FF, None),
    ("sve_intx_aba", "SI", 0x4500F800, 0x00DF07FF, None),
    ("sve_intx_shift_narrow", "SI", 0x45200000, 0x005F3FFF, None),
    ("sve_intx_extract_narrow", "SI", 0x45204000, 0x00581FFF, None),
    ("sve_intx_arith_narrow", "SI", 0x45206000, 0x00DF1FFF, None),
    ("mortlach_f32f32_prod", "ZF", 0x80800000, 0x001FFFF3, None),
    ("mortlach_b16f32_prod", "ZF", 0x81800000, 0x001FFFF3, None),
    ("mortlach_f16f32_prod", "ZF", 0x81A00000, 0x001FFFF3, None),
    ("mortlach_i8i32_prod", "Z", 0xA0800000, 0x013FFFF3, None),
    ("mortlach_insert_pred", "Z", 0xC0000000, 0x00C1FFEF, None),
    ("mortlach_extract_pred", "Z", 0xC0020000, 0x00C1FDFF, None),
    ("mortlach_zero", "Z", 0xC0080000, 0x000000FF, None),
    ("mortlach_addhv", "Z", 0xC0900000, 0x0001FFE3, None),
    ("mortlach_contig_load", "ZR", 0xE0000000, 0x00DFFFEF, None),
    ("mortlach_contig_store", "ZR", 0xE0200000, 0x00DFFFEF, None),
    ("mortlach_ctxt_ldst", "ZM", 0xE1000000, 0x002063EF, None),
    ("mortlach_contig_qload", "ZR", 0xE1C00000, 0x001FFFEF, None),
    ("mortlach_contig_qstore", "ZR", 0xE1E00000, 0x001FFFEF, None),
    ("sve_int_while_rr_pn", "S", 0x25204010, 0x00DF2FEF, None),
    ("sve_int_while_rr_pair", "S", 0x25205010, 0x00DF0FEF, None),
    ("sve_int_ctr_to_mask", "S", 0x25207010, 0x00C007EF, None),
    ("sve_int_pn_ptrue", "S", 0x25207810, 0x00C00007, None),
    ("sve_int_pcount_pn", "S", 0x25208200, 0x00C005FF, None),
    ("mortlach_multi2_cld_cldnt_ss_ctg", "R", 0xA0000000, 0x001F7FFF, None),
    ("mortlach_multi4_cld_cldnt_ss_ctg", "R", 0xA0008000, 0x001F7FFD, None),
    ("mortlach_multi2_cst_cstnt_ss_ctg", "R", 0xA0200000, 0x001F7FFF, None),
    ("mortlach_multi4_cst_cstnt_ss_ctg", "R", 0xA0208000, 0x001F7FFD, None),
    ("mortlach_multi2_cld_cldnt_si_ctg", "M", 0xA0400000, 0x000F7FFF, None),
    ("mortlach_multi4_cld_cldnt_si_ctg", "M", 0xA0408000, 0x000F7FFD, None),
    ("mortlach_multi2_cst_cstnt_si_ctg", "M", 0xA0600000, 0x000F7FFF, None),
    ("mortlach_multi4_cst_cstnt_si_ctg", "M", 0xA0608000, 0x000F7FFD, None),
    ("mortlach_multi2_cld_cldnt_ss_nctg", "R", 0xA1000000, 0x001F7FFF, None),
    ("mortlach_multi4_cld_cldnt_ss_nctg", "R", 0xA1008000, 0x001F7FFB, None),
    ("mortlach_multi2_cst_cstnt_ss_nctg", "R", 0xA1200000, 0x001F7FFF, None),
    ("mortlach_multi4_cst_cstnt_ss_nctg", "R", 0xA1208000, 0x001F7FFB, None),
    ("mortlach_multi2_cld_cldnt_si_nctg", "M", 0xA1400000, 0x000F7FFF, None),
    ("mortlach_multi4_cld_cldnt_si_nctg", "M", 0xA1408000, 0x000F7FFB, None),
    ("mortlach_multi2_cst_cstnt_si_nctg", "M", 0xA1600000, 0x000F7FFF, None),
    ("mortlach_multi4_cst_cstnt_si_nctg", "M", 0xA1608000, 0x000F7FFB, None),
    ("mortlach_zero_zt", "Z", 0xC0480001, 0x00000000, None),
    ("mortlach_zt_ldst", "ZM", 0xE11F8000, 0x002003E0, None),
    ("mortlach_multi2_select_int", "S", 0xC1208000, 0x00DE1FDE, None),
    ("mortlach_multi4_select_int", "S", 0xC1218000, 0x00DC1F9C, None),
    ("mortlach_multi2_z_z_minmax_sm", "SI", 0xC120A000, 0x00CF003F, None),
    ("mortlach_multi2_z_z_fminmax_sm", "SF", 0xC120A100, 0x00CF003F, fp_size),
    ("mortlach_multi2_z_z_shift_sm", "SI", 0xC120A200, 0x00CF00FF, None),
    ("mortlach_multi2_z_z_add_sm", "SI", 0xC120A300, 0x00CF001F, None),
    ("mortlach_multi2_z_z_sqdmulh_sm", "SI", 0xC120A400, 0x00CF001F, None),
    ("mortlach_multi4_z_z_minmax_sm", "SI", 0xC120A800, 0x00CF003D, None),
    ("mortlach_multi4_z_z_fminmax_sm", "SF", 0xC120A900, 0x00CF003D, fp_size),
    ("mortlach_multi4_z_z_shift_sm", "SI", 0xC120AA00, 0x00CF00FD, None),
    ("mortlach_multi4_z_z_add_sm", "SI", 0xC120AB00, 0x00CF001D, None),
    ("mortlach_multi4_z_z_sqdmulh_sm", "SI", 0xC120AC00, 0x00CF001D, None),
    ("mortlach_multi2_z_z_minmax_mm", "SI", 0xC120B000, 0x00DE007F, None),
    ("mortlach_multi2_z_z_fminmax_mm", "SF", 0xC120B100, 0x00DE007F, fp_minmax_groups),
    ("mortlach_multi2_z_z_shift_mm", "SI", 0xC120B200, 0x00DE00FF, None),
    ("mortlach_multi2_z_z_sqdmulh_mm", "SI", 0xC120B400, 0x00DE001F, None),
    ("mortlach_multi4_z_z_minmax_mm", "SI", 0xC120B800, 0x00DC007D, None),
    ("mortlach_multi4_z_z_fminmax_mm", "SF", 0xC120B900, 0x00DC007D, fp_minmax_groups),
    ("mortlach_multi4_z_z_shift_mm", "SI", 0xC120BA00, 0x00DC00FD, None),
    ("mortlach_multi4_z_z_sqdmulh_mm", "SI", 0xC120BC00, 0x00DC001D, None),
    ("mortlach_multi2_fclamp", "SF", 0xC120C000, 0x00DF03FF, fp_size),
    ("mortlach_multi2_clamp_int", "SI", 0xC120C400, 0x00DF03FF, None),
    ("mortlach_multi4_fclamp", "SF", 0xC120C800, 0x00DF03FD, fp_size),
    ("mortlach_multi4_clamp_int", "SI", 0xC120CC00, 0x00DF03FD, None),
    ("mortlach_multi2_z_z_zip", "S", 0xC120D000, 0x00DF03FF, None),
    ("mortlach_multi2_z_z_long_zip", "S", 0xC120D400, 0x001F03FF, None),
    ("mortlach_multi2_qrshr", "SN", 0xC1E0D400, 0x001F03FF, None),
    ("mortlach_multi4_qrshr", "SN", 0xC120D800, 0x00DF07FF, None),
    ("mortlach_multi2_narrow_fp_cvrt", "SS", 0xC120E000, 0x004003FF, None),
    ("mortlach_multi2_fpint_cvrt", "SS", 0xC121E000, 0x000003FE, None),
    ("mortlach_multi2_intfp_cvrt", "SS", 0xC122E000, 0x000003FE, None),
    ("mortlach_multi2_narrow_int_cvrt", "SN", 0xC123E000, 0x004003FF, None),
    ("mortlach_multi2_wide_int", "SI", 0xC125E000, 0x00C003FF, None),
    ("mortlach_multi2_frint", "SF", 0xC128E000, 0x00C703DE, None),
    ("mortlach_multi4_fpint_cvrt", "SS", 0xC131E000, 0x000003BC, None),
    ("mortlach_multi4_intfp_cvrt", "SS", 0xC132E000, 0x000003BC, None),
    ("mortlach_multi4_narrow_int_cvrt", "SN", 0xC133E000, 0x00C003FF, None),
    ("mortlach_multi4_wide_int", "SI", 0xC135E000, 0x00C003DD, None),
    ("mortlach_multi4_z_z_zip", "S", 0xC136E000, 0x00C0039E, None),
    ("mortlach_multi4_z_z_long_zip", "S", 0xC137E000, 0x0000039E, None),
    ("mortlach_multi4_frint", "SF", 0xC138E000, 0x00C7039C, None),
    ("mortlach_multi2_zz_za_mla_long_long_sm", "Z", 0xC1200000, 0x004F63FD, not_64bit),
    ("mortlach_multi1_zz_za_mla_long_long_sm", "Z", 0xC1200400, 0x004F63FF, not_64bit),
    ("mortlach_multi2_zz_za_fma_long_sm", "ZF", 0xC1200800, 0x000F63FF, not_fp8_long),
    ("mortlach_multi2_zz_za_mla_long_sm", "Z", 0xC1600800, 0x000F63FF, None),
    ("mortlach_multi1_zz_za_fma_long_sm", "ZF", 0xC1200C00, 0x000F63FF, None),
    ("mortlach_multi1_zz_za_mla_long_sm", "Z", 0xC1600C00, 0x000F63FF, None),
    ("mortlach_multi2_z_za_fpdot_sm", "ZF", 0xC1201000, 0x000F63FF, not_fp8_dot),
    ("mortlach_multi2_z_za_4way_dot_sm", "Z", 0xC1201400, 0x004F63F7, not_64bit),
    ("mortlach_multi2_z_za_mixed_dot_sm", "Z", 0xC1201408, 0x000F63F7, None),
    ("mortlach_multi2_z_za_2way_dot_sm", "Z", 0xC1601408, 0x000F63F7, None),
    ("mortlach_multi2_zz_za_float_sm", "ZF", 0xC1201800, 0x004F63EF, not_64bit),
    ("mortlach_multi2_zz_za_int_sm", "Z", 0xC1201810, 0x004F63EF, not_64bit),
    ("mortlach_multi4_zz_za_mla_long_long_sm", "Z", 0xC1300000, 0x004F63FD, not_64bit),
    ("mortlach_multi4_zz_za_fma_long_sm", "ZF", 0xC1300800, 0x000F63FF, not_fp8_long),
    ("mortlach_multi4_zz_za_mla_long_sm", "Z", 0xC1700800, 0x000F63FF, None),
    ("mortlach_multi4_z_za_fpdot_sm", "ZF", 0xC1301000, 0x000F63FF, not_fp8_dot),
    ("mortlach_multi4_z_za_4way_dot_sm", "Z", 0xC1301400, 0x004F63F7, not_64bit),
    ("mortlach_multi4_z_za_mixed_dot_sm", "Z", 0xC1301408, 0x000F63F7, None),
    ("mortlach_multi4_z_za_2way_dot_sm", "Z", 0xC1701408, 0x000F63F7, None),
    ("mortlach_multi4_zz_za_float_sm", "ZF", 0xC1301800, 0x004F63EF, not_64bit),
    ("mortlach_multi4_zz_za_int_sm", "Z", 0xC1301810, 0x004F63EF, not_64bit),
    ("mortlach_multi2_zz_za_mla_long_long_mm", "Z", 0xC1A00000, 0x005E63DD, not_64bit),
    ("mortlach_multi2_zz_za_fma_long_mm", "ZF", 0xC1A00800, 0x001E63DB, None),
    ("mortlach_multi2_zz_za_mla_long_mm", "Z", 0xC1E00800, 0x001E63DB, None),
    ("mortlach_multi2_z_za_fpdot_mm", "ZF", 0xC1A01000, 0x001E63F7, not_fp8_dot_groups),
    ("mortlach_multi2_z_za_4way_dot_mm", "Z", 0xC1A01400, 0x005E63D7, not_64bit),
    ("mortlach_multi2_z_za_mixed_dot_mm", "Z", 0xC1A01408, 0x001E63C7, None),
    ("mortlach_multi2_z_za_2way_dot_mm", "Z", 0xC1E01408, 0x001E63D7, None),
    ("mortlach_multi2_zz_za_float_mm", "ZF", 0xC1A01800, 0x005E63CF, not_64bit),
    ("mortlach_multi2_zz_za_int_mm", "Z", 0xC1A01810, 0x005E63CF, not_64bit),
    ("mortlach_multi2_z_za_float_mm", "ZF", 0xC1A01C00, 0x004063CF, not_64bit),
    ("mortlach_multi2_z_za_int_mm", "Z", 0xC1A01C10, 0x004063CF, not_64bit),
    ("mortlach_multi4_zz_za_mla_long_long_mm", "Z", 0xC1A10000, 0x005C639D, not_64bit),
    ("mortlach_multi4_zz_za_fma_long_mm", "ZF", 0xC1A10800, 0x001C639B, None),
    ("mortlach_multi4_zz_za_mla_long_mm", "Z", 0xC1E10800, 0x001C639B, None),
    ("mortlach_multi4_z_za_fpdot_mm", "ZF", 0xC1A11000, 0x001C63B7, not_fp8_dot_groups),
    ("mortlach_multi4_z_za_4way_dot_mm", "Z", 0xC1A11400, 0x005C6397, not_64bit),
    ("mortlach_multi4_z_za_mixed_dot_mm", "Z", 0xC1A11408, 0x001C6387, None),
    ("mortlach_multi4_z_za_2way_dot_mm", "Z", 0xC1E11408, 0x001C6397, None),
    ("mortlach_multi4_zz_za_float_mm", "ZF", 0xC1A11800, 0x005C638F, not_64bit),
    ("mortlach_multi4_zz_za_int_mm", "Z", 0xC1A11810, 0x005C638F, not_64bit),
    ("mortlach_multi4_z_za_float_mm", "ZF", 0xC1A11C00, 0x0040638F, not_64bit),
    ("mortlach_multi4_z_za_int_mm", "Z", 0xC1A11C10, 0x0040638F, not_64bit),
    ("mortlach_multi1_mla_long_long_idx_s", "Z", 0xC1000000, 0x000FFFFF, None),
    ("mortlach_multi1_fma_long_idx", "ZF", 0xC1801000, 0x000FEFFF, None),
    ("mortlach_multi1_mla_long_idx", "Z", 0xC1C01000, 0x000FEFFF, None),
    ("mortlach_multi2_mla_long_long_idx_s", "Z", 0xC1100000, 0x000F6FFF, None),
    ("mortlach_multi2_zza_idx_s", "ZF", 0xC1500000, 0x000F7FFF, not_fp8_indexed_dot),
    ("mortlach_multi2_fma_long_idx", "ZF", 0xC1901000, 0x000F6FDF, None),
    ("mortlach_multi2_mla_long_idx", "Z", 0xC1D01000, 0x000F6FDF, None),
    ("mortlach_multi4_mla_long_long_idx_s", "Z", 0xC1108000, 0x000F6FBF, None),
    ("mortlach_multi4_zza_idx_s", "ZF", 0xC1508000, 0x000F7FBF, not_fp8_indexed_dot),
    ("mortlach_multi4_fma_long_idx", "ZF", 0xC1909000, 0x000F6F9F, None),
    ("mortlach_multi4_mla_long_idx", "Z", 0xC1D09000, 0x000F6F9F, None),
    ("mortlach_bini32_prod", "Z", 0x80800008, 0x001FFFF3, None),
    ("mortlach_i16i32_prod", "Z", 0xA0800008, 0x011FFFF3, None),
    ("mortlach_multi2_insert_ctg", "Z", 0xC0040000, 0x00C0E3C7, None),
    ("mortlach_multi4_insert_ctg", "Z", 0xC0040400, 0x00C0E387, None),
    ("mortlach_multi2_extract_ctg", "Z", 0xC0060000, 0x00C0E0FE, None),
    ("mortlach_multi4_extract_ctg", "Z", 0xC0060400, 0x00C0E0FC, None),
    ("mortlach_multi2_za_insert_ctg", "Z", 0xC0040800, 0x000063C7, None),
    ("mortlach_multi4_za_insert_ctg", "Z", 0xC0040C00, 0x00006387, None),
    ("mortlach_multi2_za_extract_ctg", "Z", 0xC0060800, 0x000060FE, None),
    ("mortlach_multi4_za_extract_ctg", "Z", 0xC0060C00, 0x000060FC, None),
    ("mortlach_expand_1dst", "Z", 0xC0C80000, 0x0007FFFF, None),
    ("mortlach_expand_2dst_ctg", "Z", 0xC0884000, 0x0007BFFE, None),
    ("mortlach_expand_4dst_ctg", "Z", 0xC0888000, 0x00073FFC, None),
    ("sve_intx_dot2", "SI", 0x4400C800, 0x001F07FF, None),
    ("sve_intx_dot2_by_indexed_elem", "SI", 0x4480C800, 0x001F07FF, None),
    ("sve_intx_multi_shift_narrow", "SN", 0x45A00000, 0x005F3BDF, None),
    ("sve_intx_multi_extract_narrow", "SN", 0x45214000, 0x00581BDF, None),
    ("sve_fp_clamp", "SF", 0x64202400, 0x00DF03FF, fp_size),
    ("fdot_z_zzz_", "SF", 0x64208000, 0x001F03FF, None),
    ("fdot_z_zzzi_", "SF", 0x64204000, 0x001F03FF, None),
    ("bfmlslb_z_zzz_", "SF", 0x64E0A000, 0x001F07FF, None),
    ("bfmlslb_z_zzzi_", "SF", 0x64E06000, 0x001F0FFF, None),
]


def to_bytes(values, width):
    return b"".join(v.to_bytes(width, "little") for v in values)


def from_bytes(raw, width, count):
    return [int.from_bytes(raw[width * i : width * i + width], "little") for i in range(count)]


class EmulatorAborted(Exception):
    """QEMU itself stopped on a word, killed by a signal. Any other early
    exit is the harness's fault and stops the run."""


class Harness:
    """The harness program, built into a temporary directory and running
    under QEMU until closed; restarted after QEMU aborts on a word."""

    def __init__(self, workdir):
        cc = os.environ.get("CC", "aarch64-linux-gnu-gcc")
        qemu = os.environ.get("QEMU", "qemu-aarch64")
        exe = os.path.join(workdir, "qemu_sve_harness")
        subprocess.run(
            [
                cc,
                "-static",
                "-O2",
                "-fno-stack-protector",
                "-Wl,--no-warn-rwx-segments",
                "-o",
                exe,
                os.path.join(HERE, "qemu_sve_harness.c"),
                os.path.join(HERE, "qemu_sve_trampoline.S"),
            ],
            check=True,
        )
        self.command = [qemu, "-cpu", f"max,sme_fa64=off,sme-default-vector-length={VLB}", exe]
        self.start()

    def start(self):
        self.proc = subprocess.Popen(
            self.command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
        )

    def run(self, insn, x, sp, nzcv, fpcr, z, p, za, zt0, data):
        """Run `insn` from the state; `None` if it raised a signal, else the
        state after it as `(x, sp, nzcv, fpsr, z, p, za, zt0, data)`."""
        request = RECORD.pack(
            insn, *x, sp, nzcv, fpcr, 0, 0, to_bytes(z, VLB), to_bytes(p, PLB), to_bytes(za, VLB), to_bytes([zt0], ZT0B)
        )
        self.proc.stdin.write(request + bytes(data))
        self.proc.stdin.flush()
        reply = self.read(RECORD.size)
        out_data = self.read(DATA_SIZE)
        fields = RECORD.unpack(reply)
        status, regs = fields[0], fields[1:]
        if status != 0:
            return None
        xs = list(regs[:31])
        out_sp, out_nzcv, _fpcr, fpsr, _pad = regs[31:36]
        zs, ps, zas = from_bytes(regs[36], VLB, 32), from_bytes(regs[37], PLB, 16), from_bytes(regs[38], VLB, VLB)
        out_zt0 = from_bytes(regs[39], ZT0B, 1)[0]
        return xs, out_sp, out_nzcv, fpsr, zs, ps, zas, out_zt0, out_data

    def read(self, n):
        got = self.proc.stdout.read(n)
        if len(got) != n:
            code = self.proc.wait()
            if code >= 0:
                raise RuntimeError(f"harness exited with {code} mid-record")
            self.start()
            raise EmulatorAborted(f"QEMU exited with {code}")
        return got

    def close(self):
        self.proc.stdin.close()
        code = self.proc.wait()
        if code != 0:
            raise RuntimeError(f"harness exited with {code}")


def vector_line(kind, insn, seed, out_nzcv, fpsr, before, after):
    """A vector's fields after its family name, from `before` and `after`,
    each `(x, sp, z, p, za, zt0, data)`."""
    x, sp, z, p, za, zt0, data = before
    xs, out_sp, zs, ps, zas, out_zt0, out_data = after
    regs = [f"{i}:{xs[i]:x}" for i in range(31) if xs[i] != x[i]]
    if out_sp != sp:
        regs.append(f"31:{out_sp:x}")
    zregs = [f"{i}:{zs[i]:0{2 * VLB}x}" for i in range(32) if zs[i] != z[i]]
    pregs = [f"{i}:{ps[i]:0{2 * PLB}x}" for i in range(16) if ps[i] != p[i]]
    out_data = bytes(out_data)
    data = bytes(data)
    mem = []
    if out_data != data:
        mem = [f"{i:x}:{out_data[i]:x}" for i in range(DATA_SIZE) if out_data[i] != data[i]]
    line = (
        f"{kind} {insn:08x} {seed:x} {CODE + 4:x} {out_nzcv:x} "
        f"r={','.join(regs) or '-'} z={','.join(zregs) or '-'} p={','.join(pregs) or '-'} "
        f"m={','.join(mem) or '-'} f={fpsr:x}"
    )
    if kind.startswith("Z"):
        rows = [f"{i}:{zas[i]:0{2 * VLB}x}" for i in range(VLB) if zas[i] != za[i]]
        line += f" za={','.join(rows) or '-'}"
        zt = f"0:{out_zt0:0{2 * ZT0B}x}" if out_zt0 != zt0 else "-"
        line += f" zt={zt}"
    return line


def run_case(harness, insn, seed, kind):
    x, sp, nzcv, _v, data, base_fpcr = initial_state(seed, kind, insn)
    z, p, fpcr = sve_state(seed, kind, insn)
    za = za_state(seed, kind)
    zt0 = zt0_state(seed, kind)
    fpcr |= base_fpcr
    try:
        result = harness.run(insn, x, sp & MASK64, nzcv, fpcr, z, p, za, zt0, data)
    except EmulatorAborted as aborted:
        print(f"{insn:08x}: {aborted}; dropped", file=sys.stderr)
        return None
    if result is None:
        return None
    xs, out_sp, out_nzcv, fpsr, zs, ps, zas, out_zt0, out_data = result
    before = (x, sp, z, p, za, zt0, data)
    return vector_line(kind, insn, seed, out_nzcv, fpsr, before, (xs, out_sp, zs, ps, zas, out_zt0, out_data))


def main():
    out = sys.argv[1]
    per_family = int(sys.argv[2]) if len(sys.argv) > 2 else 40
    picker = SplitMix64(0x5E5E_D1FF)
    lines = []
    with tempfile.TemporaryDirectory() as workdir:
        harness = Harness(workdir)
        for name, kind, base, mask, fixup in FAMILIES:
            kept = 0
            attempts = 0
            while kept < per_family and attempts < per_family * 40:
                attempts += 1
                insn = base | (picker.next() & mask)
                if fixup:
                    insn = fixup(insn)
                seed = picker.next()
                line = run_case(harness, insn, seed, kind)
                if line is not None:
                    lines.append(f"{name} {line}")
                    kept += 1
            print(f"{name}: {kept} vectors from {attempts} encodings", file=sys.stderr)
            if kept < per_family:
                raise SystemExit(f"{name}: {kept} of {per_family} vectors; is the family or the harness broken?")
        harness.close()
    with open(out, "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
