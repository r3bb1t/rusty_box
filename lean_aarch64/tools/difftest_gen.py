#!/usr/bin/env python3
"""Random single-instruction tests for `oracle bare` and `leanbox bare`.

    python3 -I difftest_gen.py GROUP COUNT SEED > tests.txt

GROUP is dpimm, dpreg, ldst, branch, simd or all. Each line is an
instruction word of that top-level encoding group and a start state:
`insn sp nzcv x0..x30 v0lo v0hi .. v31lo v31hi`, in hex. Registers hold
either a random value or an address in the 8 KiB scratch window at RAM base
+ 4 KiB (half the time), so loads and stores reach memory as well as fault.
"""
import random
import sys

RAM = 0x4000_0000
GROUPS = {
    # (mask, value) over bits 28:25 (op1), and a bias of the other bits
    "dpimm": [0b1000, 0b1001],
    "dpreg": [0b0101, 0b1101],
    "ldst": [0b0100, 0b0110, 0b1100, 0b1110],
    "branch": [0b1010, 0b1011],
    "simd": [0b0111, 0b1111],
}


def word(rng, group):
    op1 = rng.choice(GROUPS[group])
    while True:
        w = rng.getrandbits(32)
        w = (w & ~(0xF << 25)) | (op1 << 25)
        # no exception generation, system instructions or ERET-class branches
        if group == "branch" and (w >> 24) & 0xFF in (0xD4, 0xD5):
            continue
        return w & 0xFFFF_FFFF


def value(rng):
    r = rng.random()
    if r < 0.5:
        return RAM + 0x1000 + rng.randrange(0, 0x2000 - 64) & ~(rng.choice([0, 0, 7, 15]))
    if r < 0.6:
        return rng.choice([0, 1, 0xFFFF_FFFF, 0x8000_0000, 0x7FFF_FFFF, 2**64 - 1, 2**63, 2**63 - 1])
    return rng.getrandbits(64)


def main():
    group, count, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    rng = random.Random(seed)
    for _ in range(count):
        g = rng.choice(list(GROUPS)) if group == "all" else group
        f = [word(rng, g), (RAM + 0x1000 + rng.randrange(0x100, 0x1F00)) & ~(rng.choice([15, 15, 15, 7])),
             rng.getrandbits(4)]
        f += [value(rng) for _ in range(31)]
        f += [rng.getrandbits(64) for _ in range(64)]
        print(" ".join(f"{x:x}" for x in f))


if __name__ == "__main__":
    main()
