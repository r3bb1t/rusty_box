#!/usr/bin/env python3
"""Generate single-instruction test vectors for rusty_box_aarch64 with Unicorn.

Each vector is one instruction word from a template family plus a seed. The
seed drives a splitmix64 stream that both this script and the Rust test
(tests/unit/difftest.rs) turn into the same initial state: X0-X30, SP, NZCV,
V0-V31 and a 64 KiB data region (FPSR and FPCR start at zero). Unicorn
executes the one instruction; the vector records the PC, NZCV and FPSR after
it and only what changed of the registers and data bytes, so the file stays
small and the Rust side needs no Unicorn to check against it.

Unicorn runs as QEMU's cortex-a57 model, so it rejects every post-ARMv8.0
encoding (FP16, RDM, LSE, ...), and encodings it rejects are dropped: every
vector is an instruction a Cortex-A57 executes. QEMU's cortex-a57 also has
the optional crypto and CRC32 extensions, which this core does not advertise;
the Rust replay skips exactly those. Families that touch memory pin their
base register into the data region and keep every access aligned, because
with the MMU off data accesses are Device memory, where an unaligned access
is an alignment fault on this core (Unicorn's older QEMU does not check).

Usage: python3 -I gen_unicorn.py OUT_FILE [CASES_PER_FAMILY]
"""
import sys

from unicorn import Uc, UcError, UC_ARCH_ARM64, UC_MODE_ARM
from unicorn import arm64_const as A

MASK64 = (1 << 64) - 1
CODE = 0x10000
DATA = 0x100000
DATA_SIZE = 0x10000
LOW_NIBBLES = int("0F" * 16, 16)
ONE_PER_BYTE = int("01" * 16, 16)


class SplitMix64:
    def __init__(self, seed):
        self.state = seed & MASK64

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK64
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK64
        return z ^ (z >> 31)


def initial_state(seed, kind, insn):
    """Must match `difftest::initial_state` in Rust exactly."""
    r = SplitMix64(seed)
    x = []
    for _ in range(31):
        sel = r.next() & 7
        v = r.next()
        if sel == 0:
            x.append(0)
        elif sel == 1:
            x.append(v & 0xFF)
        elif sel == 2:
            x.append((-(v & 0xFF)) & MASK64)
        elif sel == 3:
            x.append(v & 0xFFFFFFFF)
        elif sel == 4:
            w = v & 0xFFFFFFFF
            x.append(w | (0xFFFFFFFF00000000 if w & 0x80000000 else 0))
        elif sel == 5:
            x.append(1 << (v & 63))
        else:
            x.append(v)
    sp = r.next()
    nzcv = (r.next() & 0xF) << 28
    v = []
    for _ in range(32):
        lo = r.next()
        hi = r.next()
        v.append(lo | (hi << 64))
    data = bytearray()
    for _ in range(DATA_SIZE // 8):
        data += r.next().to_bytes(8, "little")
    if kind == "V":
        # Half the vector registers get every byte in -16..15, so that shift
        # amounts taken from a register's low byte are often in range.
        for i in range(32):
            sel = r.next() & 1
            signs = r.next() | (r.next() << 64)
            if sel == 1:
                v[i] = (v[i] & LOW_NIBBLES) | ((signs & ONE_PER_BYTE) * 0xF0)
    fpcr = 0
    if kind == "F":
        # Floating-point inputs: per register, random bits, or lanes of
        # special and ordinary single or double values; then a random FPCR
        # (RMode, FZ, DN, AHP).
        for i in range(32):
            sel = r.next() & 7
            if sel >= 2:
                single = sel in (2, 4, 5)
                mixed = sel in (2, 3)
                lanes = []
                for _ in range(4 if single else 2):
                    w = r.next()
                    if mixed and w & 1:
                        table = F32_SPECIAL if single else F64_SPECIAL
                        lanes.append(table[(w >> 1) % len(table)])
                    elif single:
                        lanes.append(ordinary_f32(w))
                    else:
                        lanes.append(ordinary_f64(w))
                width = 32 if single else 64
                v[i] = sum(lane << (width * k) for k, lane in enumerate(lanes))
        fpcr = (r.next() & 0x1F) << 22
    if kind == "T":
        rn = (insn >> 5) & 31
        if rn != 31:
            x[rn] = CODE + ((r.next() & 0x3FFF) << 2)
    if kind in ("M", "R"):
        rn = (insn >> 5) & 31
        base = DATA + 0x4000 + (r.next() & 0x3F0)
        if rn == 31:
            sp = base
        else:
            x[rn] = base
        if kind == "R":
            rm = (insn >> 16) & 31
            off = (r.next() & 0xF) << 4
            if rm != 31 and rm != rn:
                x[rm] = off
    return x, sp, nzcv, v, data, fpcr


# Zeros, infinities, quiet and signalling NaNs, denormals, the extreme
# normals, and values on integer, tie and conversion-range boundaries.
F32_SPECIAL = [
    0x00000000, 0x80000000, 0x7F800000, 0xFF800000, 0x7FC00000, 0xFFC12345,
    0x7F800001, 0xFFA00000, 0x00000001, 0x807FFFFF, 0x00800000, 0x80800000,
    0x7F7FFFFF, 0xFF7FFFFF, 0x3F800000, 0xBF800000, 0x3F000000, 0xBF000000,
    0x3FC00000, 0x40200000, 0xC0200000, 0x40600000, 0x4B000000, 0x4B7FFFFF,
    0x4F000000, 0xCF000000, 0x4F800000, 0x5F000000, 0xDF000000, 0x5F800000,
    0x4EFFFFFF, 0x477FE000, 0x33800000, 0x387FC000,
]
F64_SPECIAL = [
    0x0000000000000000, 0x8000000000000000, 0x7FF0000000000000,
    0xFFF0000000000000, 0x7FF8000000000000, 0xFFF8000000012345,
    0x7FF0000000000001, 0xFFF4000000000000, 0x0000000000000001,
    0x800FFFFFFFFFFFFF, 0x0010000000000000, 0x8010000000000000,
    0x7FEFFFFFFFFFFFFF, 0xFFEFFFFFFFFFFFFF, 0x3FF0000000000000,
    0xBFF0000000000000, 0x3FE0000000000000, 0xBFE0000000000000,
    0x3FF8000000000000, 0x4004000000000000, 0xC004000000000000,
    0x400C000000000000, 0x4330000000000000, 0x41E0000000000000,
    0xC1E0000000000000, 0x41DFFFFFFFC00000, 0x41EFFFFFFFE00000,
    0x41F0000000000000, 0x43E0000000000000, 0xC3E0000000000000,
    0x43F0000000000000, 0x47EFFFFFE0000000, 0x36A0000000000000,
    0x3810000000000000, 0x380FFFFFFFFFFFFF,
]


def ordinary_f32(w):
    """A normal single with exponent within 2^±10 and a random significand."""
    return ((w >> 63) << 31) | ((117 + ((w >> 32) % 21)) << 23) | (w & 0x7FFFFF)


def ordinary_f64(w):
    """A normal double with exponent within 2^±20 and a random significand."""
    return ((w >> 63) << 63) | ((1003 + ((w >> 52) & 0x3FF) % 41) << 52) | (w & ((1 << 52) - 1))


# (name, kind, base, random-bit mask, fixup)
def clear(bits):
    return lambda w: w & ~bits


def nonzero_immh(w):
    if (w >> 19) & 0xF == 0:
        w |= 1 << 19
    return w


def fp_immh(w):
    """immh for a single (01xx) or double (1xxx) fixed-point conversion."""
    if (w >> 21) & 3 == 0:
        w |= 1 << 21
    return w


def fp_elem_opcode(w):
    """FMLA, FMLS, FMUL or FMULX by element, single or double."""
    opcode = (0b0001, 0b0101, 0b1001)[((w >> 12) & 0xF) % 3]
    return (w & ~(0xF << 12)) | (opcode << 12) | (1 << 23)


def fp_across_opcode(w):
    """FMAXNMV, FMINNMV (01100) or FMAXV, FMINV (01111)."""
    opcode = 0b01111 if (w >> 12) & 1 else 0b01100
    return (w & ~(0x1F << 12)) | (opcode << 12)


def fp_pair_opcode(w):
    """FMAXNMP, FADDP, FMAXP (and the MIN forms by size<1>)."""
    opcode = (0b01100, 0b01101, 0b01111)[((w >> 12) & 0xF) % 3]
    return (w & ~(0x1F << 12)) | (opcode << 12)


FAMILIES = [
    ("addsub_imm", "D", 0x11000000, 0xE07FFFFF, None),
    ("addsub_shift", "D", 0x0B000000, 0xE0DFFFFF, None),
    ("addsub_ext", "D", 0x0B200000, 0xE01FFFFF, None),
    ("addsub_carry", "D", 0x1A000000, 0xE01F03FF, None),
    ("logic_imm", "D", 0x12000000, 0xE07FFFFF, None),
    ("logic_shift", "D", 0x0A000000, 0xE0FFFFFF, None),
    ("movewide", "D", 0x12800000, 0xE07FFFFF, None),
    ("pcrel", "D", 0x10000000, 0xE0FFFFFF, None),
    ("bitfield", "D", 0x13000000, 0xE07FFFFF, None),
    ("extract", "D", 0x13800000, 0x805FFFFF, None),
    ("condsel", "D", 0x1A800000, 0xC01FF7FF, None),
    ("condcmp", "D", 0x3A400000, 0xC01FFBEF, None),
    ("dp1", "D", 0x5AC00000, 0x80001FFF, None),
    ("dp2", "D", 0x1AC00000, 0x801F3FFF, None),
    ("dp3", "D", 0x1B000000, 0x80FFFFFF, None),
    ("bcond", "D", 0x54000000, 0x0003FFEF, None),
    ("cbz", "D", 0x34000000, 0x8103FFFF, None),
    ("tbz", "D", 0x36000000, 0x81FFFFFF, None),
    ("b_bl", "D", 0x14000000, 0x80003FFF, None),
    ("br_blr_ret", "T", 0xD61F0000, 0x006003E0, None),
    ("ldst_pos", "M", 0x39000000, 0xC0FFFFFF, None),
    ("ldst_pos_simd", "M", 0x3D000000, 0xC0FFFFFF, None),
    ("ldst_regoff", "R", 0x38200800, 0xC0DFF3FF, None),
    ("ldst_unscaled", "M", 0x38000000, 0xC0DFF3FF, clear(0x7 << 12 | 0x3 << 10)),
    ("ldst_post", "M", 0x38000400, 0xC0DFF3FF, clear(0x7 << 12 | 0x2 << 10)),
    ("ldst_pre", "M", 0x38000C00, 0xC0DFF3FF, clear(0x7 << 12)),
    ("ldst_pair", "M", 0x28000000, 0xC1FFFFFF, None),
    ("ldst_pair_simd", "M", 0x2C000000, 0xC1FFFFFF, None),
    ("ldst_multiple", "M", 0x0C000000, 0x4040FFFF, None),
    ("ldst_multiple_post", "R", 0x0C800000, 0x405FFFFF, None),
    ("ldst_single_struct", "M", 0x0D000000, 0x4060FFFF, None),
    ("simd_imm", "D", 0x0F000400, 0x6007F3FF, None),
    ("simd_ins", "D", 0x0E000400, 0x601F7BFF, None),
    ("simd_same", "V", 0x0E200400, 0x60DFFBFF, None),
    ("simd_misc", "V", 0x0E200800, 0x60C1F3FF, None),
    ("simd_across", "D", 0x0E300800, 0x60C1F3FF, None),
    ("simd_shift_imm", "D", 0x0F000400, 0x607FFBFF, nonzero_immh),
    ("simd_ext", "D", 0x2E000000, 0x401F7BFF, None),
    ("simd_perm", "D", 0x0E000800, 0x40DF73FF, None),
    ("simd_tbl", "D", 0x0E000000, 0x401F73FF, None),
    ("simd_addp_scalar", "D", 0x5EF1B800, 0x000003FF, None),
    ("fmov_general", "D", 0x1E260000, 0x80C103FF, None),
    ("scalar_same", "V", 0x5E200400, 0x20DFFBFF, None),
    ("scalar_misc", "V", 0x5E200800, 0x20C1F3FF, None),
    ("scalar_shift_imm", "V", 0x5F000400, 0x207FFBFF, nonzero_immh),
    ("simd_diff", "V", 0x0E200000, 0x60DFF3FF, None),
    ("scalar_diff", "V", 0x5E200000, 0x20DFF3FF, None),
    ("simd_elem", "V", 0x0F000000, 0x60FFF3FF, None),
    ("scalar_elem", "V", 0x5F000000, 0x20FFF3FF, None),
    ("scalar_dup", "V", 0x5E000400, 0x001F03FF, None),
    ("fp_dp1", "F", 0x1E204000, 0x00DF83FF, None),
    ("fp_dp2", "F", 0x1E200800, 0x00DFF3FF, None),
    ("fp_dp3", "F", 0x1F000000, 0x00FFFFFF, None),
    ("fp_cmp", "F", 0x1E202000, 0x00DF03F8, None),
    ("fp_ccmp", "F", 0x1E200400, 0x00DFF3FF, None),
    ("fp_csel", "F", 0x1E200C00, 0x00DFF3FF, None),
    ("fp_imm", "F", 0x1E201000, 0x00DFE01F, None),
    ("fp_to_int", "F", 0x1E200000, 0x80DF03FF, None),
    ("fp_to_fixed", "F", 0x1E000000, 0x80DFFFFF, None),
    ("simd_same_fp", "F", 0x0E20C400, 0x60DF3BFF, None),
    ("simd_misc_fp", "F", 0x0E210800, 0x60C0F3FF, None),
    ("simd_misc_fpcmp", "F", 0x0EA0C800, 0x604033FF, None),
    ("simd_shift_fixed", "F", 0x0F00E400, 0x607F1BFF, fp_immh),
    ("simd_elem_fp", "F", 0x0F000000, 0x60FFFBFF, fp_elem_opcode),
    ("simd_across_fp", "F", 0x2E300800, 0x40C013FF, fp_across_opcode),
    ("scalar_same_fp", "F", 0x5E20C400, 0x20DF3BFF, None),
    ("scalar_misc_fp", "F", 0x5E210800, 0x20C0F3FF, None),
    ("scalar_misc_fpcmp", "F", 0x5EA0C800, 0x204033FF, None),
    ("scalar_shift_fixed", "F", 0x5F00E400, 0x207F1BFF, fp_immh),
    ("scalar_elem_fp", "F", 0x5F000000, 0x20FFFBFF, fp_elem_opcode),
    ("scalar_pair_fp", "F", 0x7E300800, 0x00C013FF, fp_pair_opcode),
    ("fp_cvt_precision", "F", 0x1E224000, 0x00C183FF, None),
]


def run_case(insn, seed, kind):
    x, sp, nzcv, v, data, fpcr = initial_state(seed, kind, insn)
    uc = Uc(UC_ARCH_ARM64, UC_MODE_ARM)
    uc.ctl_set_cpu_model(A.UC_CPU_ARM64_A57)
    uc.mem_map(0, DATA)
    uc.mem_map(DATA, DATA_SIZE)
    uc.mem_write(CODE, insn.to_bytes(4, "little"))
    uc.mem_write(DATA, bytes(data))
    uc.reg_write(A.UC_ARM64_REG_CPACR_EL1, 3 << 20)
    uc.reg_write(A.UC_ARM64_REG_FPCR, fpcr)
    for i in range(29):
        uc.reg_write(A.UC_ARM64_REG_X0 + i, x[i])
    uc.reg_write(A.UC_ARM64_REG_X29, x[29])
    uc.reg_write(A.UC_ARM64_REG_X30, x[30])
    uc.reg_write(A.UC_ARM64_REG_SP, sp)
    uc.reg_write(A.UC_ARM64_REG_NZCV, nzcv)
    for i in range(32):
        uc.reg_write(A.UC_ARM64_REG_Q0 + i, v[i])
    try:
        uc.emu_start(CODE, CODE + 4, count=1)
    except UcError:
        return None
    pc = uc.reg_read(A.UC_ARM64_REG_PC)
    out_nzcv = uc.reg_read(A.UC_ARM64_REG_NZCV)
    xs = [uc.reg_read(A.UC_ARM64_REG_X0 + i) for i in range(29)]
    xs.append(uc.reg_read(A.UC_ARM64_REG_X29))
    xs.append(uc.reg_read(A.UC_ARM64_REG_X30))
    out_sp = uc.reg_read(A.UC_ARM64_REG_SP)
    vs = [uc.reg_read(A.UC_ARM64_REG_Q0 + i) for i in range(32)]
    out_data = uc.mem_read(DATA, DATA_SIZE)
    fpsr = uc.reg_read(A.UC_ARM64_REG_FPSR)
    regs =[f"{i}:{xs[i]:x}" for i in range(31) if xs[i] != x[i]]
    if out_sp != sp:
        regs.append(f"31:{out_sp:x}")
    vregs = [f"{i}:{vs[i]:x}" for i in range(32) if vs[i] != v[i]]
    out_data = bytes(out_data)
    data = bytes(data)
    mem = []
    if out_data != data:
        for chunk in range(0, DATA_SIZE, 256):
            if out_data[chunk : chunk + 256] != data[chunk : chunk + 256]:
                mem += [
                    f"{i:x}:{out_data[i]:x}"
                    for i in range(chunk, chunk + 256)
                    if out_data[i] != data[i]
                ]
    return (
        f"{kind} {insn:08x} {seed:x} {pc:x} {out_nzcv:x} "
        f"r={','.join(regs) or '-'} v={','.join(vregs) or '-'} m={','.join(mem) or '-'} "
        f"f={fpsr:x}"
    )


def main():
    out = sys.argv[1]
    per_family = int(sys.argv[2]) if len(sys.argv) > 2 else 400
    picker = SplitMix64(0xA64_D1FF)
    lines = []
    for name, kind, base, mask, fixup in FAMILIES:
        # Floating point has many more corner cases per encoding.
        wanted = per_family * (3 if kind == "F" else 1)
        kept = 0
        attempts = 0
        while kept < wanted and attempts < wanted * 20:
            attempts += 1
            insn = base | (picker.next() & mask)
            if fixup:
                insn = fixup(insn)
            seed = picker.next()
            line = run_case(insn, seed, kind)
            if line is not None:
                lines.append(f"{name} {line}")
                kept += 1
        print(f"{name}: {kept} vectors from {attempts} encodings", file=sys.stderr)
    with open(out, "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
