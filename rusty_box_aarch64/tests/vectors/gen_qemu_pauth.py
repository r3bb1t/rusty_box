#!/usr/bin/env python3
"""Generate the pointer authentication test vectors with QEMU's system
emulator.

User-mode QEMU runs a program at EL0 with keys Linux chose, so it cannot
show what an instruction does under keys, enables and TCR_EL1 fields of
the test's choosing. This script runs each case in a bare-metal EL1
program instead, qemu_pauth_runner.S linked by qemu_pauth.ld, under
`qemu-system-aarch64 -M virt -cpu max,pauth-qarma5=on` (QARMA5, with
FEAT_PAuth2, FEAT_FPAC and FEAT_FPACCOMBINE: ID_AA64ISAR1_EL1.APA = 5,
which the script checks; since QEMU 10.0 `max` defaults to QEMU's own
algorithm, and the property selects the architected one).
A case sets the keys, SCTLR_EL1.{EnIA, EnIB, EnDA, EnDB}, TCR_EL1,
ELR_EL1, NZCV, SP and X0-X30, runs up to three setup words (typically a
PAC instruction that signs what the word under test authenticates, then a
bit flip that may break it), then the word. The vector records the state
just before the word, as the runner reports it, and what the word did:
fell through, branched to the landing pad, or took an exception.

Each line of qemu_pauth.txt is

    family word seed s=<SCTLR_EL1> t=<TCR_EL1> n=<NZCV> i=<reg:value,...>
        o=<N|L|X> r=<reg:value,...> f=<NZCV> c=<ESR_EL1>

with every number in hex. The seed gives the keys (APIAKeyLo_EL1 to
APGAKeyHi_EL1), X0-X30, ELR_EL1 and SP, in that order, each the next
value of a SplitMix64 the seed starts (`Case`); `i` lists the registers the
setup words changed from those (SP as 31, ELR_EL1 as 32). `o` is N for a
word that fell through (the PC is the next word), L for one that branched
to the landing pad, X for one that took an exception (`c` is its syndrome;
no register changed). `r` lists the registers the word changed, `f` is
NZCV after it. The replay is tests/unit/pointer_auth.rs. Families are
appended, never reordered.

A family keeps every case one QEMU and the replay can both run: a branch
lands on the landing pad or faults on the authentication, a load reads
the data region or faults on it, and the bit flips that break a pointer
are made only under an enabled key (a disabled key leaves the flipped
pointer, which would branch or load somewhere else). A flip that leaves
a valid code sends the word elsewhere; `kept` drops such a case.

QEMU is wrong in two places, against the Arm ARM's pseudocode, and the
families steer clear of both: it signs a pointer whose bits 55 and 63
differ with the extension and bottom PAC bit of bit 55's half where
InsertPAC takes bit 63's (`qemu_selects_differently`; tests/unit/
pointer_auth.rs pins Arm's), and it makes no SP alignment check
(`fam_ldra`).

Needs an AArch64 bare-metal toolchain (CC, default aarch64-linux-gnu-gcc)
and QEMU 10.0 or later's system emulator (QEMU, default
qemu-system-aarch64). The vectors in the tree were made with QEMU 11.1.2.

Usage: python3 -I gen_qemu_pauth.py OUT_FILE [CASES_PER_FAMILY]
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from gen_unicorn import MASK64, SplitMix64  # noqa: E402

# qemu_pauth.ld's layout.
SLOT = 0x4020_0000
TEST_PC = SLOT + 16
LAND = 0x4030_0000
DATA = 0x4040_0000
DATA_WORDS = 0x400

ISAR1_APA_FPACCOMBINE = 5
SPSR = 0x3C5
NOP = 0xD503201F
EN_IA, EN_IB, EN_DA, EN_DB = 1 << 31, 1 << 30, 1 << 27, 1 << 13


def data_word(i):
    """The data region's doubleword `i`; `pointer_auth::data_word` in
    Rust."""
    return ((i + 1) * 0x9E3779B97F4A7C15) & MASK64


# ----- encodings -----

def pac_dp1(opcode, rn, rd):
    """PACIA .. AUTDZB, XPACI, XPACD: `opcode` is `00 Z AUT D B` or
    `01000 D`."""
    return 0xDAC10000 | (opcode << 10) | (rn << 5) | rd


def hint(crm, op2):
    return 0xD503201F | (crm << 8) | (op2 << 5)


def pacga(rd, rn, rm):
    return 0x9AC03000 | (rm << 16) | (rn << 5) | rd


def eor_bit(rd, bit):
    """EOR Xd, Xd, #(1 << bit)."""
    return 0xD2400000 | (((64 - bit) % 64) << 16) | (rd << 5) | rd


MSR_ELR_X30 = 0xD518403E


def mov_sp(rn):
    return 0x91000000 | (rn << 5) | 31


def branch(opc, m, rn, rm):
    """The authenticating register branches: `1101011 opc 11111 00001 M Rn
    Rm`."""
    return 0xD61F0800 | (opc << 21) | (m << 10) | (rn << 5) | rm


def ldra(m, imm10, w, rn, rt):
    return 0xF8200400 | (m << 23) | ((imm10 >> 9) << 22) | ((imm10 & 0x1FF) << 12) | (w << 11) | (rn << 5) | rt


# ----- cases -----

def seeded(seed):
    """The keys, X0-X30, ELR_EL1 and SP of a case's seed;
    `pointer_auth::Seeded` in Rust."""
    s = SplitMix64(seed)
    keys = [s.next() for _ in range(10)]
    x = [s.next() for _ in range(31)]
    elr = s.next()
    sp = s.next() & ~0xF & MASK64
    return keys, x, elr, sp


class Case:
    def __init__(self, r):
        self.seed = r.next()
        self.keys, self.x, self.elr, self.sp = seeded(self.seed)
        en = 0
        for bit in (EN_IA, EN_IB, EN_DA, EN_DB):
            if r.next() % 8:
                en |= bit
        self.sctlr = en
        self.tcr = random_tcr(r, r.next() % 4 == 0)
        self.nzcv = (r.next() % 16) << 28
        self.words = [NOP, NOP, NOP, NOP]

    def enabled(self, bit):
        return self.sctlr & bit != 0


def random_tcr(r, any_size):
    """T0SZ, T1SZ, TBI0, TBI1, TBID0, TBID1. Unless `any_size`, T0SZ keeps
    bit 30 (the landing pad's and the data region's) an address bit."""
    t0sz = 16 + r.next() % 24 if any_size else 16 + r.next() % 18
    t1sz = 16 + r.next() % 24
    flags = r.next()
    tbi0, tbi1, tbid0, tbid1 = flags & 1, (flags >> 1) & 1, (flags >> 2) & 1, (flags >> 3) & 1
    return t0sz | (t1sz << 16) | (tbi0 << 37) | (tbi1 << 38) | (tbid0 << 51) | (tbid1 << 52)


def pointer(r):
    """A pointer of either half of the address space, sometimes tagged,
    sometimes anything."""
    kind = r.next() % 4
    v = r.next()
    if kind == 0:
        return v & 0x0000_FFFF_FFFF_FFFF
    if kind == 1:
        return v | 0xFFFF_0000_0000_0000
    if kind == 2:
        return (v & 0x00FF_FFFF_FFFF_FFFF) | ((r.next() & 0xFF) << 56)
    return v


def flip(r, case, rd, bit_enabled, at):
    """With an enabled key, half the time flip a bit of `rd` in slot word
    `at`: one of bits 32-63, which hold the code or the tag, or address
    bits no code or data is at, so that a pointer the flip leaves valid
    cannot reach the runner's code."""
    if case.enabled(bit_enabled) and r.next() % 2:
        case.words[at] = eor_bit(rd, 32 + r.next() % 32)


def reg(r, avoid=()):
    while True:
        n = r.next() % 31
        if n not in avoid:
            return n


def qemu_selects_differently(tcr, ptr, data):
    """Whether QEMU (to 11.1.2) signs `ptr` with another extension or
    bottom PAC bit than Arm's InsertPAC. Arm takes the half of the address
    space from bit 55 where a TBI bit applies to the access, of either
    half, and from bit 63 otherwise (the case InsertPAC's comment calls
    awkward); QEMU's `pauth_addpac` takes the bottom bit from bit 55's half
    always, and the extension from bit 55 only where that half's TBI
    applies. A pointer whose bits 55 and 63 agree cannot tell them apart."""
    b55, b63 = (ptr >> 55) & 1, ptr >> 63
    if b55 == b63:
        return False
    applies = [(tcr >> (37 + h)) & 1 and (data or not (tcr >> (51 + h)) & 1) for h in (0, 1)]
    sel = b55 if any(applies) else b63
    qemu_ext = b55 if applies[b55] else b63

    def txsz(h):
        return min(39, max(16, (tcr >> (16 * h)) & 0x3F))

    return (sel, txsz(sel)) != (qemu_ext, txsz(b55))


def signable(r, case, data):
    """A pointer QEMU signs as Arm does (`qemu_selects_differently`)."""
    while True:
        p = pointer(r)
        if not qemu_selects_differently(case.tcr, p, data):
            return p


def fam_pac(r):
    """PACIA, PACIB, PACDA, PACDB (Rn = 31 is SP) and the zero-modifier
    PACIZA .. PACDZB."""
    c = Case(r)
    z = r.next() % 2
    rd = reg(r)
    rn = 31 if z else r.next() % 32
    opcode = (z << 3) | (r.next() % 4)
    c.x[rd] = signable(r, c, opcode & 2 != 0)
    c.words[3] = pac_dp1(opcode, rn, rd)
    return c


def fam_aut(r):
    """AUTIA .. AUTDZB of a pointer the same key and modifier signed, or a
    different one, with a bit flipped or not, or of an unsigned pointer."""
    c = Case(r)
    z = r.next() % 2
    db = r.next() % 4
    rd = reg(r)
    rn = 31 if z else r.next() % 32
    if rn == rd:
        rn = 31
    c.x[rd] = pointer(r)
    if r.next() % 8:
        sign_db = db if r.next() % 8 else r.next() % 4
        c.words[0] = pac_dp1((z << 3) | sign_db, rn, rd)
        enable = (EN_IA, EN_IB, EN_DA, EN_DB)[db]
        flip(r, c, rd, enable, 1)
    c.words[3] = pac_dp1((z << 3) | 4 | db, rn, rd)
    return c


def fam_xpac(r):
    """XPACI, XPACD, of signed pointers and others."""
    c = Case(r)
    d = r.next() % 2
    rd = reg(r)
    c.x[rd] = pointer(r)
    if r.next() % 2:
        c.words[0] = pac_dp1(r.next() % 4, reg(r, (rd,)), rd)
    c.words[3] = pac_dp1(0b010000 | d, 31, rd)
    return c


HINT_PAC = [(0b0001, 0b000), (0b0001, 0b010), (0b0011, 0b000), (0b0011, 0b001), (0b0011, 0b010), (0b0011, 0b011)]


def fam_pac_hint(r):
    """PACIA1716, PACIB1716, PACIAZ, PACIASP, PACIBZ, PACIBSP, AUTIA1716
    .. AUTIBSP (after the matching sign, perhaps with a bit flipped) and
    XPACLRI."""
    c = Case(r)
    c.x[16] = r.next()
    c.x[17] = signable(r, c, False)
    c.x[30] = signable(r, c, False)
    choice = r.next() % 13
    if choice == 12:
        if r.next() % 2:
            c.words[0] = hint(0b0011, r.next() % 4)
        c.words[3] = hint(0b0000, 0b111)
        return c
    crm, op2 = HINT_PAC[choice % 6]
    if choice >= 6:
        rd = 17 if crm == 0b0001 else 30
        if r.next() % 4:
            c.words[0] = hint(crm, op2)
            flip(r, c, rd, EN_IB if op2 & 2 else EN_IA, 1)
        op2 |= 4
    c.words[3] = hint(crm, op2)
    return c


def fam_pacga(r):
    c = Case(r)
    c.words[3] = pacga(r.next() % 32, r.next() % 32, r.next() % 32)
    return c


def tagged(r, case, at, instruction):
    """`at`, an address of the lower half, half the time tagged where TBI0
    (and, for an instruction address, not TBID0) ignores the tag."""
    tbi = (case.tcr >> 37) & 1
    tbid = (case.tcr >> 51) & 1
    if tbi and not (instruction and tbid) and r.next() % 2:
        return at | ((r.next() & 0xFF) << 56)
    return at


def fam_branch(r):
    """BRAA, BRAB, BLRAA, BLRAB (modifier Xm or SP), BRAAZ .. BLRABZ (zero
    modifier), RETAA, RETAB (X30 under SP) to a pointer to the landing
    pad the matching key signed, perhaps with a bit flipped."""
    c = Case(r)
    m = r.next() % 2
    form = r.next() % 3
    if form == 2:
        c.x[30] = tagged(r, c,LAND, True)
        c.words[0] = hint(0b0011, 0b001 | (m << 1))
        flip(r, c, 30, EN_IB if m else EN_IA, 1)
        c.words[3] = branch(0b0010, m, 31, 31)
        return c
    link = r.next() % 2
    rn = reg(r)
    c.x[rn] = tagged(r, c,LAND, True)
    if form == 0:
        rm = r.next() % 32
        if rm == rn:
            rm = 31
        c.words[0] = pac_dp1(m, rm, rn)
        c.words[3] = branch(0b1000 | link, m, rn, rm)
    else:
        c.words[0] = pac_dp1(0b1000 | m, 31, rn)
        c.words[3] = branch(link, m, rn, 31)
    flip(r, c, rn, EN_IB if m else EN_IA, 1)
    return c


def fam_eret(r):
    """ERETAA, ERETAB of ELR_EL1 = a pointer to the landing pad PACIASP or
    PACIBSP signed, perhaps with a bit flipped, to EL1h."""
    c = Case(r)
    m = r.next() % 2
    c.x[30] = tagged(r, c,LAND, True)
    c.words[0] = hint(0b0011, 0b001 | (m << 1))
    flip(r, c, 30, EN_IB if m else EN_IA, 1)
    c.words[2] = MSR_ELR_X30
    c.words[3] = 0xD69F0BFF | (m << 10)
    return c


def fam_ldra(r):
    """LDRAA, LDRAB, with and without writeback, of a pointer the offset
    takes into the data region, PACDZA or PACDZB signed, perhaps with a bit
    flipped; the base is SP a fifth of the time, then 16-byte aligned
    (QEMU makes no SP alignment check, `gen_check_sp_alignment`)."""
    c = Case(r)
    m = r.next() % 2
    w = r.next() % 2
    sp = r.next() % 5 == 0
    imm = r.next() % 1024 - 512
    target = DATA + 8 * (r.next() % DATA_WORDS)
    base = target - 8 * imm
    if sp and base % 16:
        imm += 1 if imm < 511 else -1
        base = target - 8 * imm
    rn = reg(r)
    c.x[rn] = tagged(r, c,base, False)
    c.words[0] = pac_dp1(0b1010 | m, 31, rn)
    flip(r, c, rn, EN_DB if m else EN_DA, 1)
    if sp:
        c.words[2] = mov_sp(rn)
        rn = 31
    rt = reg(r) if r.next() % 8 or rn == 31 else rn
    c.words[3] = ldra(m, imm & 0x3FF, w, rn, rt)
    return c


FAMILIES = [
    ("pac", fam_pac),
    ("aut", fam_aut),
    ("xpac", fam_xpac),
    ("pac_hint", fam_pac_hint),
    ("pacga", fam_pacga),
    ("branch", fam_branch),
    ("eret", fam_eret),
    ("ldra", fam_ldra),
]


# ----- the program -----

def cases_source(cases):
    out = ["    .section .pdata, \"a\""]
    out += [f"    .quad {data_word(i):#x}" for i in range(DATA_WORDS)]
    out += ["    .section .cases, \"a\"", "    .balign 16", "    .global cases", "    .global cases_end", "cases:"]
    for c in cases:
        quads = c.keys + [c.sctlr, c.tcr, c.elr, c.nzcv, c.sp] + c.x
        out += [f"    .quad {q:#x}" for q in quads]
        out += [f"    .word {w:#x}" for w in c.words]
    out.append("cases_end:")
    return "\n".join(out) + "\n"


def run(cases, workdir):
    cc = os.environ.get("CC", "aarch64-linux-gnu-gcc")
    qemu = os.environ.get("QEMU", "qemu-system-aarch64")
    src = os.path.join(workdir, "cases.S")
    elf = os.path.join(workdir, "pauth.elf")
    with open(src, "w") as f:
        f.write(cases_source(cases))
    subprocess.run(
        [cc, "-nostdlib", "-static", "-Wl,--build-id=none", "-Wl,--no-warn-rwx-segments",
         "-T", os.path.join(HERE, "qemu_pauth.ld"),
         os.path.join(HERE, "qemu_pauth_runner.S"), src, "-o", elf],
        check=True,
    )
    out = subprocess.run(
        [qemu, "-M", "virt", "-cpu", "max,pauth-qarma5=on", "-m", "256M", "-display", "none", "-monitor", "none", "-nic", "none",
         "-serial", "stdio", "-kernel", elf],
        check=True, capture_output=True, text=True, timeout=600,
    ).stdout
    return out.splitlines()


def vector_line(name, case, sctlr_base, init, final):
    _, x, elr, sp = seeded(case.seed)
    base = x + [sp, elr]
    setup = [f"{i}:{init[i]:x}" for i in range(33) if init[i] != base[i]]
    outcome = "NLX"[final[35]]
    changes = [f"{i}:{final[i]:x}" for i in range(32) if final[i] != init[i]]
    esr = f"{final[36]:x}" if outcome == "X" else "-"
    return (f"{name} {case.words[3]:08x} {case.seed:x} s={sctlr_base | case.sctlr:x} t={case.tcr:x} "
            f"n={init[33]:x} i={','.join(setup) or '-'} o={outcome} r={','.join(changes) or '-'} f={final[33]:x} "
            f"c={esr}")


PAC_FAIL = 0x1C


def kept(final):
    """Whether the replay can run what QEMU did: a word falls through, lands
    on the landing pad, or fails its authentication. A bit flip that left a
    valid code sends a branch or a load elsewhere (the code is a few bits
    under a large TxSZ), which the replay does not map."""
    if final[35] != 2:
        return True
    return final[38] == TEST_PC and final[36] >> 26 == PAC_FAIL


def main():
    out_file = sys.argv[1]
    per_family = int(sys.argv[2]) if len(sys.argv) > 2 else 300
    r = SplitMix64(0x9AC_A5)
    named = []
    for name, make in FAMILIES:
        named += [(name, make(r)) for _ in range(per_family)]
    with tempfile.TemporaryDirectory() as workdir:
        lines = run([c for _, c in named], workdir)
    if not lines or not lines[0].startswith("S ") or lines[-1] != "END":
        raise SystemExit(f"the runner did not finish: {lines[:3]} .. {lines[-3:]}")
    sctlr_base = int(lines[0].split()[1], 16)
    isar1 = int(lines[0].split()[2], 16)
    if (isar1 >> 4) & 0xF != ISAR1_APA_FPACCOMBINE:
        raise SystemExit(f"ID_AA64ISAR1_EL1 = {isar1:#x}: APA is not FEAT_FPACCOMBINE")
    body = lines[1:-1]
    if len(body) != 2 * len(named):
        raise SystemExit(f"{len(body)} lines for {len(named)} cases")
    vectors = []
    tally = {}
    for i, (name, case) in enumerate(named):
        init = [int(v, 16) for v in body[2 * i].split()[1:]]
        final = [int(v, 16) for v in body[2 * i + 1].split()[1:]]
        if final[35] == 0 and case.words[3] & 0xFE1F_0000 == 0xD61F_0000:
            raise SystemExit(f"{name} {case.words[3]:08x}: a branch fell through")
        t = tally.setdefault(name, [0, 0, 0, 0])
        if not kept(final):
            t[3] += 1
            continue
        vectors.append(vector_line(name, case, sctlr_base, init, final))
        t[final[35]] += 1
    for name, (n, l, x, dropped) in tally.items():
        print(f"{name}: {n} fell through, {l} landed, {x} failed to authenticate, {dropped} dropped", file=sys.stderr)
        if dropped * 50 > per_family:
            raise SystemExit(f"{name}: {dropped} of {per_family} cases dropped; is the family or the runner broken?")
    with open(out_file, "w") as f:
        f.write("\n".join(vectors) + "\n")


if __name__ == "__main__":
    main()
