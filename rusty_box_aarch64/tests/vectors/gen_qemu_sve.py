#!/usr/bin/env python3
"""Generate single-instruction test vectors for the SVE instructions FEAT_SME
makes legal in Streaming SVE mode, with QEMU's user-mode emulator.

Each vector is gen_qemu.py's, run in Streaming SVE mode at a 512-bit
vector length with ZA storage enabled: a word from a template family, a
seed that both this script and the Rust test (tests/unit/difftest.rs,
`replay_sve`) turn into the same initial state, and what one step
changed. The state is gen_unicorn.py's (`initial_state`), with Z0-Z31 and
P0-P15 in place of the SIMD&FP registers (`sve_state`), and ZA, zero but
for the ZA instructions' kinds (`za_state`). The program,
qemu_sve_harness.c with qemu_sve_trampoline.S, runs each word under
`qemu-aarch64 -cpu max,sme_fa64=off,sme-default-vector-length=64`:
Streaming SVE mode without FEAT_SME_FA64, as this core implements it, so
a word illegal in it raises a signal there too. A word the program could
not run is dropped, and so is one QEMU itself aborts on; the harness
restarts after it.

A family is an Arm encoding class of the A64 `sve` group, or of the `sme`
group for the ZA instructions (the kinds beginning with Z), its word the
class's fixed bits with the rest random (unallocated words are dropped).
Families are appended, never reordered.

A vector line is gen_unicorn.py's with `z=` and `p=` (the registers that
changed, each as one big-endian hex number, 128 and 16 digits wide) in
place of `v=`, and for the Z kinds `za=` (ZA's horizontal vectors that
changed, by number, 128 digits wide):

    family kind word seed pc nzcv r=... z=... p=... m=... f=fpsr [za=...]

Needs an AArch64 Linux C compiler that links statically (CC, default
aarch64-linux-gnu-gcc) and QEMU's user-mode emulator (QEMU, default
qemu-aarch64). The vectors in the tree were made with QEMU 11.1.2 with
docs/qemu-afp-vector-fabs-fneg.patch applied, which changes nothing in
Streaming SVE mode, and docs/qemu-sve-fcmla-indexed-ah.patch,
docs/qemu-sve-fmad-size00.patch and docs/qemu-sme-ld1-vertical-tail.patch,
which bring FCMLA (indexed), FMAD and the SME LD1 to a vertical tile slice
to the Arm ARM (docs/qemu-upstream-bugs.md, entries 13 to 15). QEMU is a
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
# A record as qemu_sve_harness.c lays it out: a 16-byte header (the word or
# the status, then padding), X0-X30, SP, NZCV, FPCR, FPSR, padding, Z0-Z31,
# P0-P15, ZA's horizontal vectors.
RECORD = struct.Struct(f"<I12x31Q5Q{32 * VLB}s{16 * PLB}s{VLB * VLB}s")

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
    if kind in ("SF", "ZF"):
        # Floating-point inputs: per register, the state above, or lanes of
        # floating-point values, half of the registers in the format the
        # word names and the rest in half, single or double precision; then
        # a random FPCR (RMode, FZ, DN, AHP, FZ16, and FEAT_AFP's FIZ, AH
        # and NEP). The format is the one the `size` field names, or for an
        # outer product (ZF) its sources' (`za_fp_format`).
        own = (insn >> 22) & 3 if kind == "SF" else za_fp_format(insn)
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
    return z, p, fpcr


def za_fp_format(insn):
    """The `FP_FORMATS` index of a floating-point outer product's source
    elements: single precision (bit 24 clear), BFloat16 (bit 24 set, bit
    21 clear) or half precision."""
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
    """BFDOT (indexed): `op = 1`, `opc2 = 00`; FDOT (`op = 0`) is FEAT_SME2's
    and the rest FP8's."""
    return (w | 0x00400000) & ~0x00000C00


def bfdot(w):
    """BFDOT (vectors): `op = 1`, `o2 = 0`; FDOT (`op = 0`) is FEAT_SME2's
    and `o2 = 1` FP8's."""
    return (w | 0x00400000) & ~0x00000400


def not_bfmlsl(w):
    """FMLALB to BFMLALT: `o2 = 1` with `op = 1`, BFMLSLB and BFMLSLT, is
    FEAT_SME2's, so BFMLALB and BFMLALT in its place."""
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


def not_pmull128(w):
    """PMULLB, PMULLT (`op = 0`, `U = 1`): `size = 00` is FEAT_SVE_PMULL128's
    128-bit form, so `size = 01` in its place."""
    pmull = (w >> 12) & 1 == 0 and (w >> 11) & 1 == 1
    return w | 0x00400000 if pmull and (w >> 22) & 3 == 0 else w


# (name, kind, fixed bits, free bits, fixup): Arm's encoding classes of the
# A64 `sve` group whose encodings FEAT_SME makes legal, with `op0 = 000`
# and `001`, then the loads, stores and prefetches (`op0 = 100`, `101` and
# `111`, kind M, or R for scalar plus scalar), then the floating-point
# instructions (`op0 = 011`, kind SF), then SVE2's integer instructions
# (`op0 = 010`, kind SI), `op1<7> = 0` first, then the ZA instructions of
# the A64 `sme` group (kind Z, ZF for the floating-point outer products,
# ZR and ZM for the loads and stores as R and M), their fixed bits from
# AARCHMRS 2026-09. A word of a class that is unallocated (MOVA's `Q = 1`
# with a `size` other than 11) raises a signal and is dropped.
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

    def run(self, insn, x, sp, nzcv, fpcr, z, p, za, data):
        """Run `insn` from the state; `None` if it raised a signal, else the
        state after it as `(x, sp, nzcv, fpsr, z, p, za, data)`."""
        request = RECORD.pack(insn, *x, sp, nzcv, fpcr, 0, 0, to_bytes(z, VLB), to_bytes(p, PLB), to_bytes(za, VLB))
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
        return xs, out_sp, out_nzcv, fpsr, zs, ps, zas, out_data

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
    each `(x, sp, z, p, za, data)`."""
    x, sp, z, p, za, data = before
    xs, out_sp, zs, ps, zas, out_data = after
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
    return line


def run_case(harness, insn, seed, kind):
    x, sp, nzcv, _v, data, base_fpcr = initial_state(seed, kind, insn)
    z, p, fpcr = sve_state(seed, kind, insn)
    za = za_state(seed, kind)
    fpcr |= base_fpcr
    try:
        result = harness.run(insn, x, sp & MASK64, nzcv, fpcr, z, p, za, data)
    except EmulatorAborted as aborted:
        print(f"{insn:08x}: {aborted}; dropped", file=sys.stderr)
        return None
    if result is None:
        return None
    xs, out_sp, out_nzcv, fpsr, zs, ps, zas, out_data = result
    before = (x, sp, z, p, za, data)
    return vector_line(kind, insn, seed, out_nzcv, fpsr, before, (xs, out_sp, zs, ps, zas, out_data))


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
