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
dropped.

QEMU is wrong too in places, and a family steers clear of what it is
known to get wrong, against the Arm ARM's pseudocode: QEMU 8.2.2 adds the
`imm9` of LDAPUR and STLUR unsigned, where Arm's `SignExtend(imm9)` makes
it signed, so `rcpc_unscaled` keeps the offset non-negative.

The program runs at EL0 with the MMU on, over Normal memory: families keep
every access aligned (the replay runs at EL1 with the MMU off, over Device
memory), and are non-branching and not PC-relative (the word runs at the
program's own address; the vector records PC = CODE + 4). Families are
appended, never reordered, as in gen_unicorn.py.

Needs an AArch64 Linux C compiler that links statically (CC, default
aarch64-linux-gnu-gcc) and QEMU's user-mode emulator (QEMU, default
qemu-aarch64). The vectors in the tree were made with QEMU 8.2.2.

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
    """LDAPUR, STLUR: a non-negative offset that keeps the access
    size-aligned."""
    size = w >> 30
    return w & ~(((1 << size) - 1) << 12) & ~(1 << 20)


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
]


def pack_v(v):
    return b"".join((lane & MASK128).to_bytes(16, "little") for lane in v)


def unpack_v(raw):
    return [int.from_bytes(raw[16 * i : 16 * i + 16], "little") for i in range(32)]


class Harness:
    """The harness program, built into a temporary directory and running
    under QEMU until closed."""

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
        self.proc = subprocess.Popen(
            [qemu, "-cpu", "max", exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE
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
            raise RuntimeError(f"harness exited with {self.proc.wait()}")
        return got

    def close(self):
        self.proc.stdin.close()
        code = self.proc.wait()
        if code != 0:
            raise RuntimeError(f"harness exited with {code}")


def run_case(harness, insn, seed, kind):
    x, sp, nzcv, v, data, fpcr = initial_state(seed, kind, insn)
    result = harness.run(insn, x, sp & MASK64, nzcv, v, data, fpcr)
    if result is None:
        return None
    xs, out_sp, out_nzcv, fpsr, vs, out_data = result
    before = (x, sp, v, data)
    after = (xs, out_sp, vs, out_data)
    return vector_line(kind, insn, seed, CODE + 4, out_nzcv, fpsr, before, after)


def main():
    out = sys.argv[1]
    per_family = int(sys.argv[2]) if len(sys.argv) > 2 else 400
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
        harness.close()
    with open(out, "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
