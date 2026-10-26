#!/usr/bin/env python3
"""Generate single-instruction test vectors for the features after ARMv8.0
with QEMU's user-mode emulator.

The vectors are gen_unicorn.py's: a word from a template family, a seed
that both this script and the Rust test (tests/unit/difftest.rs) turn into
the same initial state, and what one step changed. They differ in the
reference. Unicorn 2.1.4's max processor lacks features this core is
built to (I8MM, BF16, SME) and gets some of the ones it has wrong (it
compares LDSMAX and LDSMIN narrower than 64 bits unsigned, where Arm's
`MemAtomicInt` compares `SInt`). This script runs each word in a
static program, qemu_harness.c with qemu_trampoline.S, under `qemu-aarch64
-cpu max`, into qemu_max.txt, which the Rust replay runs on a processor
with every feature this core implements. A word the program could not run
(it raised a signal: unallocated, or a feature QEMU's max lacks) is
dropped, and so is one QEMU itself aborts on; the harness restarts after
it.

QEMU's max implements features this core does not, in the encoding space
of three families: FEAT_FPRCVT's conversions between half precision and a
SIMD&FP register (`fp16_int`), FEAT_FAMINMAX's FAMAX and FAMIN and
FEAT_FP8's FSCALE (`fp16_same`), and FEAT_FP8DOT4's FDOT and FEAT_FP8FMA's
FMLALL by element (`fp16_elem`). Those families' fixups turn such a word
into an instruction this core has. QEMU 8.2.2 gets instructions of five
families wrong against the Arm ARM's pseudocode, which 11.1.2 gets right
(docs/qemu-upstream-bugs.md lists them).

The program runs at EL0 with the MMU on, over Normal memory: families keep
every access aligned (the replay runs at EL1 with the MMU off, over Device
memory), and are non-branching and not PC-relative (the word runs at the
program's own address; the vector records PC = CODE + 4). Families are
appended, never reordered, as in gen_unicorn.py.

Needs an AArch64 Linux C compiler that links statically (CC, default
aarch64-linux-gnu-gcc) and QEMU's user-mode emulator (QEMU, default
qemu-aarch64). The vectors in the tree were made with QEMU 11.1.2.

Usage: python3 -I gen_qemu.py OUT_FILE [CASES_PER_FAMILY]
"""
import os
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from gen_unicorn import CODE, DATA_SIZE, MASK64, SplitMix64, initial_state, vector_line  # noqa: E402

MASK128 = (1 << 128) - 1
# A record as qemu_harness.c lays it out: a 16-byte header (the word or the
# status, then padding), X0-X30, SP, NZCV, FPCR, FPSR, padding, V0-V31.
RECORD = struct.Struct("<I12x31Q5Q512s")


def even_pairs(w):
    """CASP: an even Rs and Rt (odd ones are UNDEFINED)."""
    return w & ~((1 << 16) | 1)


def aligned_imm9(w):
    """LDAPUR, STLUR: an offset that keeps the access size-aligned."""
    size = w >> 30
    return w & ~(((1 << size) - 1) << 12)


def dot_elem_size(w):
    """SDOT, UDOT, SUDOT, USDOT (by element): `size<0>` = 0. SDOT and UDOT
    need size 10, SUDOT and USDOT have `US 0` there; with `size<0>` = 1,
    opcode 1111 is BFDOT and BFMLAL (FEAT_BF16)."""
    return w & ~(1 << 22)


def fcmp_zero_rm(w):
    """FCMP, FCMPE: the compares with zero keep `Rm = 00000`, a should-be
    field QEMU ignores (README, "Where the references differ")."""
    return w & ~(0x1F << 16) if w & 0x8 else w


def fmov_imm5(w):
    """FMOV (scalar, immediate): `imm5 = 00000`."""
    return w & ~(0x1F << 5)


def without_fprcvt(w):
    """Half precision to and from a general register: with `rmode` other
    than 00, opcodes 01x and 1xx are FEAT_FPRCVT's conversions to and from
    a SIMD&FP register (or unallocated); opcode<0> alone remains, FCVTPS,
    FCVTPU, FCVTMS, FCVTMU, FCVTZS or FCVTZU to a general register."""
    if w >> 19 & 3 and w >> 16 & 0b110:
        return w & ~(0b110 << 16)
    return w


def without_faminmax_fscale(w):
    """Three-same half precision: FAMAX and FAMIN (FEAT_FAMINMAX, `a = 1`,
    opcode 011) become FMULX and FMUL, FSCALE (FEAT_FP8, `U = 1`, `a = 1`,
    opcode 111) becomes FDIV (`a = 0`)."""
    opcode = w >> 11 & 7
    if w >> 23 & 1 and (opcode == 0b011 or (w >> 29 & 1 and opcode == 0b111)):
        return w & ~(1 << 23)
    return w


def without_fp8_elem(w):
    """By element at `size = 00`: FDOT (FEAT_FP8DOT4, `U = 0`, opcode 0000)
    becomes FMLA, FMLALLBB and the rest (FEAT_FP8FMA, `U = 1`, opcode 1000)
    FMULX, by setting opcode<0>."""
    if (w >> 29 & 1, w >> 12 & 0xF) in ((0, 0b0000), (1, 0b1000)):
        return w | (1 << 12)
    return w


def fcadd_opcode(w):
    """FCMLA has opcode (bits 14:11) 10xx, FCADD 11x0: with `opcode<2>` set,
    `opcode<0>` clear (1101 is BFMMLA and later matrix multiplies, 1111
    BFDOT and BFMLAL)."""
    return w & ~(1 << 11) if w >> 13 & 1 else w


def flag_op2(w):
    """CFINV, XAFLAG, AXFLAG: `op2` 000, 001 or 010; 011 is MSR UAO."""
    return w & ~(1 << 5) if w >> 5 & 3 == 3 else w


FAMILIES = [
    # FEAT_LSE: LDADD, LDCLR, LDEOR, LDSET, LDSMAX, LDSMIN, LDUMAX, LDUMIN
    # and SWP of every size and ordering (o3 = 1 with another opc is
    # unallocated, LDAPR apart), CAS and CASP.
    ("lse_memop", "M", 0x38200000, 0xC0DFF3FF, None),
    ("lse_cas", "M", 0x08A07C00, 0xC05F83FF, None),
    ("lse_casp", "M", 0x08207C00, 0x405F83FF, even_pairs),
    # FEAT_LRCPC: LDAPR, LDAPRB, LDAPRH. FEAT_LRCPC2: LDAPUR* and STLUR*.
    ("rcpc_ldapr", "M", 0x38BFC000, 0xC00003FF, None),
    ("rcpc_unscaled", "M", 0x19000000, 0xC0DFF3FF, aligned_imm9),
    # FEAT_RDM: SQRDMLAH and SQRDMLSH, vector, scalar and by element, of
    # every size (16 and 32 bits are allocated).
    ("rdm_vector", "V", 0x2E008400, 0x40DF0BFF, None),
    ("rdm_scalar", "V", 0x7E008400, 0x00DF0BFF, None),
    ("rdm_elem", "V", 0x2F00D000, 0x40FF2BFF, None),
    ("rdm_scalar_elem", "V", 0x7F00D000, 0x00FF2BFF, None),
    # FEAT_DotProd and FEAT_I8MM: SDOT, UDOT and USDOT (vector), SDOT, UDOT,
    # SUDOT and USDOT (by element), SMMLA, USMMLA and UMMLA.
    ("dot_vector", "V", 0x0E009400, 0x60DF0BFF, None),
    ("dot_elem", "V", 0x0F00E000, 0x60FF1BFF, dot_elem_size),
    ("mmla", "V", 0x4E80A400, 0x201F0BFF, None),
    # FEAT_FP16, scalar: data processing with one, two and three sources,
    # compares, conditional compare and select, FMOV (immediate), and the
    # conversions to and from general registers, fixed point included.
    ("fp16_dp1", "H", 0x1EE04000, 0x001F83FF, None),
    ("fp16_dp2", "H", 0x1EE00800, 0x001FF3FF, None),
    ("fp16_dp3", "H", 0x1FC00000, 0x003FFFFF, None),
    ("fp16_cmp", "H", 0x1EE02000, 0x001F03F8, fcmp_zero_rm),
    ("fp16_ccmp", "H", 0x1EE00400, 0x001FF3FF, None),
    ("fp16_csel", "H", 0x1EE00C00, 0x001FF3FF, None),
    ("fp16_imm", "H", 0x1EE01000, 0x001FE01F, fmov_imm5),
    ("fp16_int", "H", 0x1EE00000, 0x801F03FF, without_fprcvt),
    ("fp16_fixed", "H", 0x1EC00000, 0x801FFFFF, None),
    # FEAT_FP16, Advanced SIMD: three-same and two-register miscellaneous,
    # vector and scalar, by element, across lanes, pairwise, FMOV (vector,
    # immediate) and the fixed-point conversions.
    ("fp16_same", "H", 0x0E400400, 0x609F3BFF, without_faminmax_fscale),
    ("fp16_scalar_same", "H", 0x5E400400, 0x209F3BFF, None),
    ("fp16_misc", "H", 0x0E780800, 0x6081F3FF, None),
    ("fp16_scalar_misc", "H", 0x5E780800, 0x2081F3FF, None),
    ("fp16_elem", "H", 0x0F000000, 0x603FFBFF, without_fp8_elem),
    ("fp16_scalar_elem", "H", 0x5F000000, 0x203FFBFF, None),
    ("fp16_across", "H", 0x0E300800, 0x4081F3FF, None),
    ("fp16_pair", "H", 0x5E300800, 0x0081F3FF, None),
    ("fp16_movi", "H", 0x0F00FC00, 0x400703FF, None),
    ("fp16_shift_fixed", "H", 0x0F10E400, 0x600F1BFF, None),
    ("fp16_scalar_shift_fixed", "H", 0x5F10E400, 0x200F1BFF, None),
    # FEAT_FHM: FMLAL, FMLSL, FMLAL2, FMLSL2, vector and by element.
    ("fhm_vector", "H", 0x0E20CC00, 0x60BF23FF, None),
    ("fhm_elem", "H", 0x0F800000, 0x603FCBFF, None),
    # FEAT_FCMA: FCMLA and FCADD, vector, and FCMLA by element, of single
    # and double precision, and of half precision (FEAT_FP16).
    ("fcma_vector", "F", 0x2E80C400, 0x405F3BFF, fcadd_opcode),
    ("fcma_vector_half", "H", 0x2E40C400, 0x401F3BFF, fcadd_opcode),
    ("fcma_elem", "F", 0x2F801000, 0x403F6BFF, None),
    ("fcma_elem_half", "H", 0x2F401000, 0x403F6BFF, None),
    # FEAT_JSCVT: FJCVTZS.
    ("jscvt", "F", 0x1E7E0000, 0x000003FF, None),
    # FEAT_FRINTTS: FRINT32Z, FRINT32X, FRINT64Z, FRINT64X, scalar and
    # vector.
    ("frintts_scalar", "F", 0x1E284000, 0x004183FF, None),
    ("frintts_vector", "F", 0x0E21E800, 0x604013FF, None),
    # FEAT_BF16: BFCVT, BFCVTN, BFDOT (vector and by element), BFMLALB and
    # BFMLALT (vector and by element), BFMMLA.
    ("bf16_cvt", "F", 0x1E634000, 0x000003FF, None),
    ("bf16_cvtn", "F", 0x0EA16800, 0x400003FF, None),
    ("bf16_dot", "B", 0x2E40FC00, 0x401F03FF, None),
    ("bf16_dot_elem", "B", 0x0F40F000, 0x403F0BFF, None),
    ("bf16_mlal", "B", 0x2EC0FC00, 0x401F03FF, None),
    ("bf16_mlal_elem", "B", 0x0FC0F000, 0x403F0BFF, None),
    ("bf16_mmla", "B", 0x6E40EC00, 0x001F03FF, None),
    # FEAT_FlagM and FEAT_FlagM2: CFINV, XAFLAG, AXFLAG; RMIF; SETF8 and
    # SETF16.
    ("flagm_pstate", "D", 0xD500401F, 0x00000060, flag_op2),
    ("flagm_rmif", "D", 0xBA000400, 0x001F83EF, None),
    ("flagm_setf", "D", 0x3A00080D, 0x000043E0, None),
    # FEAT_SB: SB.
    ("sb", "D", 0xD50330FF, 0x00000000, None),
    # FEAT_LOR: LDLAR and STLLR of every size.
    ("lor", "M", 0x089F7C00, 0xC04003FF, None),
    # FEAT_XS: DSB nXS.
    ("xs_dsb", "D", 0xD503323F, 0x00000C00, None),
]


def pack_v(v):
    return b"".join((lane & MASK128).to_bytes(16, "little") for lane in v)


def unpack_v(raw):
    return [int.from_bytes(raw[16 * i : 16 * i + 16], "little") for i in range(32)]


class EmulatorAborted(Exception):
    """QEMU itself stopped on a word, killed by a signal (it asserts on
    some unallocated ones). Any other early exit is the harness's fault and
    stops the run."""


class Harness:
    """The harness program, built into a temporary directory and running
    under QEMU until closed; restarted after QEMU aborts on a word."""

    def __init__(self, workdir):
        cc = os.environ.get("CC", "aarch64-linux-gnu-gcc")
        qemu = os.environ.get("QEMU", "qemu-aarch64")
        exe = os.path.join(workdir, "qemu_harness")
        subprocess.run(
            [
                cc,
                "-static",
                "-O2",
                "-fno-stack-protector",
                "-Wl,--no-warn-rwx-segments",
                "-o",
                exe,
                os.path.join(HERE, "qemu_harness.c"),
                os.path.join(HERE, "qemu_trampoline.S"),
            ],
            check=True,
        )
        self.command = [qemu, "-cpu", "max", exe]
        self.start()

    def start(self):
        self.proc = subprocess.Popen(
            self.command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
        )

    def run(self, insn, x, sp, nzcv, v, data, fpcr):
        """Run `insn` from the state; `None` if it raised a signal, else the
        state after it as `(x, sp, nzcv, fpsr, v, data)`."""
        request = RECORD.pack(insn, *x, sp, nzcv, fpcr, 0, 0, pack_v(v))
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
        return xs, out_sp, out_nzcv, fpsr, unpack_v(regs[36]), out_data

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


def run_case(harness, insn, seed, kind):
    x, sp, nzcv, v, data, fpcr = initial_state(seed, kind, insn)
    try:
        result = harness.run(insn, x, sp & MASK64, nzcv, v, data, fpcr)
    except EmulatorAborted as aborted:
        print(f"{insn:08x}: {aborted}; dropped", file=sys.stderr)
        return None
    if result is None:
        return None
    xs, out_sp, out_nzcv, fpsr, vs, out_data = result
    before = (x, sp, v, data)
    after = (xs, out_sp, vs, out_data)
    return vector_line(kind, insn, seed, CODE + 4, out_nzcv, fpsr, before, after)


def main():
    out = sys.argv[1]
    per_family = int(sys.argv[2]) if len(sys.argv) > 2 else 200
    picker = SplitMix64(0xA64_D1FF)
    lines = []
    with tempfile.TemporaryDirectory() as workdir:
        harness = Harness(workdir)
        for name, kind, base, mask, fixup in FAMILIES:
            wanted = per_family * (3 if kind == "F" else 1)
            kept = 0
            attempts = 0
            while kept < wanted and attempts < wanted * 20:
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
            if kept < wanted:
                raise SystemExit(f"{name}: {kept} of {wanted} vectors; is the family or the harness broken?")
        harness.close()
    with open(out, "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
