#!/usr/bin/env python3
"""Generate the EL2 test vectors with QEMU's system emulator.

A comparison, not a reference: Arm's pseudocode decides what is right
(tests/unit/sysreg_spec.rs checks every System register access against
Arm's accessors), and where QEMU disagrees with Arm the replay follows Arm
and docs/qemu-upstream-bugs.md says so. What these vectors add is a second
implementation's view of the same cases end to end: the exception each
access or instruction takes and the state it is taken with, under EL2's
traps and routing.

Each case runs in a bare-metal EL2 program, qemu_el2_runner.S linked by
qemu_el2.ld, under `qemu-system-aarch64 -M virt,virtualization=on -cpu
cortex-a57`: the Cortex-A57 with EL2 and no EL3, which this core's
`Implementation::CORTEX_A57` with the `WithEl2` level set is. The runner
writes the case's EL2 and EL1 controls, then runs its word at EL0, EL1 or
EL2 with a sentinel after it, and reports whether the word completed (and
X0 after it) or the exception it took: the level, ESR, FAR, ELR and SPSR.

Each line of qemu_el2.txt is

    family word target hcr cptr mdcr cnthctl hstr cpacr sctlr cntkctl mdscr
        oslar spsr1 elr1 entry x0 v=<0|1> o=<0|1|2> x=<X0> c=<ESR> f=<FAR>
        e=<ELR - slot> p=<SPSR>

with every number in hex: the case (the controls in the order the runner
writes them, SPSR_EL1, ELR_EL1, the SPSR_EL2 the runner returns to EL0 or
EL1 with, and X0), whether X0 after a completed word is a value both
implementations must agree on (`v`), and the outcome, as the runner
reports it. The replay is tests/unit/el2_vectors.rs. Families are appended,
never reordered.

The cases keep to what both implementations model the same way, each
exclusion for a reason the families' comments give: no HCR_EL2.HCD (the
runner returns from EL1 with HVC), no HCR_EL2.TGE for a word at EL1 (EL1
cannot execute under it), HCR_EL2.RW set (QEMU's Cortex-A57 has AArch32 at
EL1, this core has no AArch32), no return to AArch32, no PMU, trace,
IMPLEMENTATION DEFINED or breakpoint-beyond-the-second register, no read
whose value depends on time or on an earlier case, and no value of a bit
the two may legitimately hold differently (a stateful RES0 or RES1 bit,
an AArch32 trap). DBGPRCR_EL1 is left out: QEMU does not have it
(docs/qemu-upstream-bugs.md, entry 17).

Needs an AArch64 bare-metal toolchain (CC, default aarch64-linux-gnu-gcc)
and QEMU's system emulator (QEMU, default qemu-system-aarch64). The vectors
in the tree were made with QEMU 11.1.2.

Usage: python3 -I gen_qemu_el2.py OUT_FILE [CASES_PER_FAMILY]
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from gen_unicorn import MASK64, SplitMix64  # noqa: E402

# qemu_el2.ld's layout.
EL2_VECTORS = 0x4018_0000
EL1_VECTORS = 0x4018_1000
SLOT = 0x4020_0000
XDATA = 0x4040_0000

MIDR = 0x411F_D070
MPIDR = 0x8000_0000
PSCI_VERSION = 0x8400_0000
COUNTER_HZ = 62_500_000

# HCR_EL2: RW (31) always; the trap and routing bits a case may set.
HCR_RW = 1 << 31
HCR_TGE = 1 << 27
HCR_BITS = [3, 4, 5, 13, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 28, 30]
# CPTR_EL2 (E2H = 0): its RES1 bits, and TFP, TTA, TCPAC.
CPTR_RES1 = 0x33FF
CPTR_BITS = [10, 20, 31]
# MDCR_EL2: HPMN as QEMU's Cortex-A57 resets it (six counters), and TPMCR,
# TPM, TDE, TDA, TDOSA, TDRA.
MDCR_HPMN = 6
MDCR_BITS = [5, 6, 8, 9, 10, 11]
# CNTHCTL_EL2: EL1PCTEN, EL1PCEN.
CNTHCTL_BITS = [0, 1]
# CPACR_EL1: FPEN 00, 01 or 11; TTA.
CPACR_FPEN = [0, 1 << 20, 3 << 20]
CPACR_TTA = 1 << 28
# SCTLR_EL1: the Cortex-A57's reset value with nTWI and nTWE clear, and the
# EL0 controls a case may set: UMA, EnRCTX, DZE, UCT, nTWI, nTWE, UCI.
SCTLR_BASE = 0x00C5_0838 & ~((1 << 16) | (1 << 18))
SCTLR_BITS = [9, 14, 15, 16, 18, 26]
SCTLR_EL2_RESET = 0x30C5_0830
# CNTKCTL_EL1: EL0PCTEN, EL0VCTEN, EL0VTEN, EL0PTEN.
CNTKCTL_BITS = [0, 1, 8, 9]
# MDSCR_EL1: TDCC, KDE, MDE (never SS).
MDSCR_BITS = [12, 13, 15]

# SPSR_EL2 for the return to the word's level: D, A, I and F masked.
DAIF = 0x3C0
ENTRY = {0: DAIF | 0b0000, 1: DAIF | 0b0101}


def bits(r, pool, p=4):
    """A random subset of `pool`: each bit with probability 1/p."""
    v = 0
    for b in pool:
        if r.next() % p == 0:
            v |= 1 << b
    return v


class Case:
    def __init__(self, r, target):
        self.target = target
        hcr = HCR_RW | bits(r, HCR_BITS)
        if target != 1 and r.next() % 4 == 0:
            hcr |= HCR_TGE
        self.hcr = hcr
        self.cptr = CPTR_RES1 | bits(r, CPTR_BITS)
        self.mdcr = MDCR_HPMN | bits(r, MDCR_BITS)
        self.cnthctl = bits(r, CNTHCTL_BITS, 2)
        self.hstr = r.next() & 0xBFEF
        self.cpacr = CPACR_FPEN[r.next() % 3] | (CPACR_TTA if r.next() % 4 == 0 else 0)
        self.sctlr = SCTLR_BASE | bits(r, SCTLR_BITS, 2)
        self.cntkctl = bits(r, CNTKCTL_BITS, 2)
        self.mdscr = bits(r, MDSCR_BITS)
        self.oslar = 1 if r.next() % 8 == 0 else 0
        self.spsr1 = DAIF | 0b0101
        self.elr1 = SLOT + 4
        self.x0 = r.next()
        self.word = 0
        self.value = 0

    def entry(self):
        return ENTRY.get(self.target, 0)

    def quads(self):
        return [self.hcr, self.cptr, self.mdcr, self.cnthctl, self.hstr, self.cpacr, self.sctlr, self.cntkctl,
                self.mdscr, self.target, self.x0, self.spsr1, self.elr1, self.word, self.oslar, self.entry()]


# ----- encodings -----

def sysreg(l, op0, op1, crn, crm, op2, rt=0):
    return 0xD500_0000 | (l << 21) | (op0 << 19) | (op1 << 16) | (crn << 12) | (crm << 8) | (op2 << 5) | rt


def sys_insn(op1, crn, crm, op2, rt=0):
    return sysreg(0, 1, op1, crn, crm, op2, rt)


def exc(opc_ll, imm):
    """SVC (ll 01), HVC (10), SMC (11)."""
    return 0xD400_0000 | (imm << 5) | opc_ll


# A register: its encoding; what an MSR writes ("case:<field>" for the
# case's own value, so that nothing later cases depend on changes, an
# integer, "rand", or None for no MSR); whether an MRS's value is one both
# implementations must agree on; the levels the family runs it at.
R = lambda enc, write, compare: (enc, write, compare)  # noqa: E731

EL1_REGISTERS = {
    "SCTLR_EL1": R((3, 0, 1, 0, 0), "case:sctlr", True),
    "ACTLR_EL1": R((3, 0, 1, 0, 1), 0, True),
    # CPACR_EL1.TTA, and CPTR_EL2's, are RES0 without FEAT_TRC_SR; QEMU's
    # Cortex-A57 keeps what is written there, a RES0 bit may, and this core
    # does not.
    "CPACR_EL1": R((3, 0, 1, 0, 2), "case:cpacr", False),
    "TTBR0_EL1": R((3, 0, 2, 0, 0), "rand", False),
    "TTBR1_EL1": R((3, 0, 2, 0, 1), "rand", False),
    "TCR_EL1": R((3, 0, 2, 0, 2), "rand", False),
    "AFSR0_EL1": R((3, 0, 5, 1, 0), "rand", False),
    "AFSR1_EL1": R((3, 0, 5, 1, 1), "rand", False),
    "ESR_EL1": R((3, 0, 5, 2, 0), "rand", False),
    "FAR_EL1": R((3, 0, 6, 0, 0), "rand", False),
    "PAR_EL1": R((3, 0, 7, 4, 0), "rand", False),
    "MAIR_EL1": R((3, 0, 10, 2, 0), "rand", False),
    "AMAIR_EL1": R((3, 0, 10, 3, 0), "rand", False),
    "VBAR_EL1": R((3, 0, 12, 0, 0), EL1_VECTORS, True),
    "CONTEXTIDR_EL1": R((3, 0, 13, 0, 1), "rand", False),
    "TPIDR_EL1": R((3, 0, 13, 0, 4), "rand", False),
    "TPIDR_EL0": R((3, 3, 13, 0, 2), "rand", False),
    "TPIDRRO_EL0": R((3, 3, 13, 0, 3), "rand", False),
    "CNTKCTL_EL1": R((3, 0, 14, 1, 0), "case:cntkctl", True),
    "CSSELR_EL1": R((3, 2, 0, 0, 0), 0, False),
    "MIDR_EL1": R((3, 0, 0, 0, 0), None, True),
    "MPIDR_EL1": R((3, 0, 0, 0, 5), None, True),
    "REVIDR_EL1": R((3, 0, 0, 0, 6), None, True),
    "AIDR_EL1": R((3, 1, 0, 0, 7), None, True),
    "CLIDR_EL1": R((3, 1, 0, 0, 1), None, True),
    "CTR_EL0": R((3, 3, 0, 0, 1), None, True),
    "DCZID_EL0": R((3, 3, 0, 0, 7), None, True),
    "ID_AA64PFR1_EL1": R((3, 0, 0, 4, 1), None, True),
    "ID_AA64ISAR0_EL1": R((3, 0, 0, 6, 0), None, True),
    "ID_AA64ISAR1_EL1": R((3, 0, 0, 6, 1), None, True),
    # This core implements the 4 KiB granule alone, and neither mixed
    # endianness nor Secure state, as its ID_AA64MMFR0_EL1 says; QEMU's
    # Cortex-A57 has all three.
    "ID_AA64MMFR0_EL1": R((3, 0, 0, 7, 0), None, False),
    "ID_AA64MMFR1_EL1": R((3, 0, 0, 7, 1), None, True),
    "ID_AA64MMFR2_EL1": R((3, 0, 0, 7, 2), None, True),
    "ID_AA64ISAR2_EL1": R((3, 0, 0, 6, 2), None, True),
    "ID_RESERVED_3_3": R((3, 0, 0, 3, 3), None, True),
    "NZCV": R((3, 3, 4, 2, 0), "rand", True),
    "DAIF": R((3, 3, 4, 2, 1), DAIF, True),
    "CurrentEL": R((3, 0, 4, 2, 2), None, True),
    "SPSel": R((3, 0, 4, 2, 0), None, True),
    "SP_EL0": R((3, 0, 4, 1, 0), "rand", False),
    "SPSR_EL1": R((3, 0, 4, 0, 0), "case:spsr1", True),
    "ELR_EL1": R((3, 0, 4, 0, 1), "case:elr1", True),
    "FPCR": R((3, 3, 4, 4, 0), 0, False),
    "FPSR": R((3, 3, 4, 4, 1), 0, False),
    "CNTFRQ_EL0": R((3, 3, 14, 0, 0), COUNTER_HZ, True),
    "CNTPCT_EL0": R((3, 3, 14, 0, 1), None, False),
    "CNTVCT_EL0": R((3, 3, 14, 0, 2), None, False),
    "CNTP_CTL_EL0": R((3, 3, 14, 2, 1), 0, False),
    "CNTP_CVAL_EL0": R((3, 3, 14, 2, 2), "rand", False),
    "CNTP_TVAL_EL0": R((3, 3, 14, 2, 0), "rand", False),
    "CNTV_CTL_EL0": R((3, 3, 14, 3, 1), 0, False),
    "CNTV_CVAL_EL0": R((3, 3, 14, 3, 2), "rand", False),
    "CNTV_TVAL_EL0": R((3, 3, 14, 3, 0), "rand", False),
    "MDSCR_EL1": R((2, 0, 0, 2, 2), "case:mdscr", True),
    "MDCCSR_EL0": R((2, 3, 0, 1, 0), None, True),
    "DBGDTR_EL0": R((2, 3, 0, 4, 0), 0, True),
    "DBGDTRRX_EL0": R((2, 3, 0, 5, 0), 0, True),
    "OSLAR_EL1": R((2, 0, 1, 0, 4), "case:oslar", False),
    "OSLSR_EL1": R((2, 0, 1, 1, 4), None, True),
    "OSDLR_EL1": R((2, 0, 1, 3, 4), None, True),
    "DBGBVR0_EL1": R((2, 0, 0, 0, 4), 0, True),
    "DBGBCR0_EL1": R((2, 0, 0, 0, 5), 0, True),
    "DBGWVR1_EL1": R((2, 0, 0, 1, 6), 0, True),
    "DBGWCR1_EL1": R((2, 0, 0, 1, 7), 0, True),
    "DBGCLAIMSET_EL1": R((2, 0, 7, 8, 6), 0, False),
    "DBGCLAIMCLR_EL1": R((2, 0, 7, 9, 6), 0, False),
    "MDRAR_EL1": R((2, 0, 1, 0, 0), None, True),
    # An IMPLEMENTATION DEFINED encoding neither implementation gives a
    # register: QEMU's Cortex-A57 has its L2CTLR_EL1, CPUACTLR_EL1 and the
    # rest at S3_1_C11_* and S3_1_C15_*, which this core does not model.
    "S3_0_C15_C0_0": R((3, 0, 15, 0, 0), 0, True),
    "HCR_EL2": R((3, 4, 1, 1, 0), "case:hcr", True),
    "VBAR_EL2": R((3, 4, 12, 0, 0), EL2_VECTORS, True),
    "SP_EL1": R((3, 4, 4, 1, 0), "rand", False),
}

EL2_REGISTERS = {
    "HCR_EL2": R((3, 4, 1, 1, 0), "case:hcr", True),
    # QEMU resets SCTLR_EL2's RES1 bits to 0 and keeps them as written,
    # which a RES1 bit may; this core holds them at 1.
    "SCTLR_EL2": R((3, 4, 1, 0, 0), SCTLR_EL2_RESET, False),
    "ACTLR_EL2": R((3, 4, 1, 0, 1), 0, True),
    "CPTR_EL2": R((3, 4, 1, 1, 2), "case:cptr", False),
    # HSTR_EL2's traps are of AArch32 EL1 and EL0, which QEMU's Cortex-A57
    # has and this core does not (without them the register is RES0).
    "HSTR_EL2": R((3, 4, 1, 1, 3), "case:hstr", False),
    "HACR_EL2": R((3, 4, 1, 1, 7), 0, True),
    "MDCR_EL2": R((3, 4, 1, 1, 1), "case:mdcr", False),
    "TTBR0_EL2": R((3, 4, 2, 0, 0), "rand", False),
    "TCR_EL2": R((3, 4, 2, 0, 2), "rand", False),
    "VTTBR_EL2": R((3, 4, 2, 1, 0), "rand", False),
    "VTCR_EL2": R((3, 4, 2, 1, 2), "rand", False),
    "AFSR0_EL2": R((3, 4, 5, 1, 0), "rand", False),
    "AFSR1_EL2": R((3, 4, 5, 1, 1), "rand", False),
    "ESR_EL2": R((3, 4, 5, 2, 0), "rand", False),
    "FAR_EL2": R((3, 4, 6, 0, 0), "rand", False),
    "HPFAR_EL2": R((3, 4, 6, 0, 4), "rand", False),
    "MAIR_EL2": R((3, 4, 10, 2, 0), "rand", False),
    "AMAIR_EL2": R((3, 4, 10, 3, 0), "rand", False),
    "VBAR_EL2": R((3, 4, 12, 0, 0), EL2_VECTORS, True),
    "TPIDR_EL2": R((3, 4, 13, 0, 2), "rand", False),
    "VPIDR_EL2": R((3, 4, 0, 0, 0), MIDR, True),
    "VMPIDR_EL2": R((3, 4, 0, 0, 5), MPIDR, True),
    "CNTHCTL_EL2": R((3, 4, 14, 1, 0), "case:cnthctl", True),
    "CNTVOFF_EL2": R((3, 4, 14, 0, 3), "rand", False),
    "CNTHP_CTL_EL2": R((3, 4, 14, 2, 1), 0, False),
    "CNTHP_CVAL_EL2": R((3, 4, 14, 2, 2), "rand", False),
    "CNTHP_TVAL_EL2": R((3, 4, 14, 2, 0), "rand", False),
    "SP_EL1": R((3, 4, 4, 1, 0), "rand", False),
    "SP_EL2": R((3, 6, 4, 1, 0), "rand", False),
    "SPSR_EL1": R((3, 0, 4, 0, 0), "case:spsr1", True),
    "MIDR_EL1": R((3, 0, 0, 0, 0), None, True),
    "MPIDR_EL1": R((3, 0, 0, 0, 5), None, True),
    "CPACR_EL1": R((3, 0, 1, 0, 2), "case:cpacr", False),
    "CNTKCTL_EL1": R((3, 0, 14, 1, 0), "case:cntkctl", True),
    "CurrentEL": R((3, 0, 4, 2, 2), None, True),
}


def access(r, case, registers):
    """An MRS (into X0) or MSR (from X0) of one of `registers`."""
    names = sorted(registers)
    name = names[r.next() % len(names)]
    enc, write, compare = registers[name]
    if write is None or r.next() % 2 == 0:
        case.word = sysreg(1, *enc)
        case.value = 1 if compare else 0
        return case
    case.word = sysreg(0, *enc)
    if write == "rand":
        case.x0 = r.next()
    elif isinstance(write, str):
        case.x0 = getattr(case, write.split(":")[1])
    else:
        case.x0 = write
    case.value = 0
    return case


def fam_sysreg_el1(r):
    return access(r, Case(r, 1), EL1_REGISTERS)


def fam_sysreg_el0(r):
    return access(r, Case(r, 0), EL1_REGISTERS)


def fam_sysreg_el2(r):
    return access(r, Case(r, 2), EL2_REGISTERS)


# SYS instructions: (op1, CRn, CRm, op2), X0 an address in the data region
# where one is taken.
CACHE_AND_TLB = [
    (3, 7, 14, 1),  # DC CIVAC
    (3, 7, 10, 1),  # DC CVAC
    (3, 7, 11, 1),  # DC CVAU
    (0, 7, 6, 1),   # DC IVAC
    (0, 7, 6, 2),   # DC ISW
    (0, 7, 10, 2),  # DC CSW
    (0, 7, 14, 2),  # DC CISW
    (0, 7, 5, 0),   # IC IALLU
    (0, 7, 1, 0),   # IC IALLUIS
    (3, 7, 5, 1),   # IC IVAU
    (0, 8, 7, 0),   # TLBI VMALLE1
    (0, 8, 7, 1),   # TLBI VAE1
    (0, 8, 3, 0),   # TLBI VMALLE1IS
    (4, 8, 7, 0),   # TLBI ALLE2
    (4, 8, 7, 4),   # TLBI ALLE1
    (0, 7, 8, 0),   # AT S1E1R
    (0, 7, 8, 1),   # AT S1E1W
    (0, 7, 8, 2),   # AT S1E0R
    (0, 7, 8, 3),   # AT S1E0W
    (4, 7, 8, 0),   # AT S1E2R
]

WFI = 0xD503_207F
WFE = 0xD503_205F
FMOV_D0_X0 = 0x9E67_0000
ERET = 0xD69F_03E0


def fam_insn(r):
    case = Case(r, r.next() % 3)
    kind = r.next() % 4
    if kind == 0:
        # HVC, SMC or SVC with an immediate the sentinels do not use; SMC's
        # X0 is a PSCI call the firmware answers.
        ll = 1 + r.next() % 3
        imm = r.next() % 0xFFFE
        case.word = exc(ll, imm)
        if ll == 3:
            case.x0 = PSCI_VERSION
            case.value = 1
    elif kind == 1:
        # WFI or WFE, only where something traps it: at EL0 SCTLR_EL1.nTWx
        # clear (as the base value has it) or HCR_EL2.TWx, at EL1
        # HCR_EL2.TWx. A WFI nothing traps would wait for an interrupt that
        # never comes.
        wfi = r.next() % 2 == 0
        case.word = WFI if wfi else WFE
        if case.target == 2:
            case.target = 1
            case.hcr &= ~HCR_TGE
        case.sctlr &= ~((1 << 16) | (1 << 18))
        if case.target == 1:
            case.hcr |= 1 << (13 if wfi else 14)
    else:
        op1, crn, crm, op2 = CACHE_AND_TLB[r.next() % len(CACHE_AND_TLB)]
        case.word = sys_insn(op1, crn, crm, op2)
        case.x0 = XDATA + 64 * (r.next() % 16)
    return case


def fam_fp(r):
    """A SIMD&FP instruction and a SIMD&FP register access, under
    CPACR_EL1.FPEN, CPTR_EL2.TFP and HCR_EL2.TGE."""
    case = Case(r, r.next() % 3)
    if r.next() % 2 == 0:
        case.word = FMOV_D0_X0
    else:
        case.word = sysreg(1, 3, 3, 4, 4, 0)
    return case


# The SPSR_EL1.M values of an exception return from EL1: every AArch64
# encoding (bit 4 clear: QEMU's Cortex-A57 has AArch32 at EL0 and EL1, this
# core has none).
ERET_MODES = list(range(16))


def fam_eret(r):
    """ERET at EL1 (or EL0, where it is UNDEFINED) to SPSR_EL1's level, at
    ELR_EL1 = slot + 4, the sentinel: a legal return to EL1 reaches it; one
    to EL0 finds HVC UNDEFINED there; an illegal one stays at EL1 with
    PSTATE.IL set, which the sentinel's Illegal Execution State exception
    shows."""
    case = Case(r, 1 if r.next() % 8 else 0)
    case.word = ERET
    case.spsr1 = DAIF | ERET_MODES[r.next() % len(ERET_MODES)] | ((r.next() & 0xF) << 28)
    return case


FAMILIES = [
    ("sysreg_el1", fam_sysreg_el1),
    ("sysreg_el0", fam_sysreg_el0),
    ("sysreg_el2", fam_sysreg_el2),
    ("insn", fam_insn),
    ("fp", fam_fp),
    ("eret", fam_eret),
]


# ----- the program -----

def cases_source(cases):
    out = ["    .section .cases, \"a\"", "    .balign 16", "    .global cases", "    .global cases_end", "cases:"]
    for c in cases:
        out += [f"    .quad {q & MASK64:#x}" for q in c.quads()]
    out.append("cases_end:")
    return "\n".join(out) + "\n"


def run(cases, workdir):
    cc = os.environ.get("CC", "aarch64-linux-gnu-gcc")
    qemu = os.environ.get("QEMU", "qemu-system-aarch64")
    src = os.path.join(workdir, "cases.S")
    elf = os.path.join(workdir, "el2.elf")
    with open(src, "w") as f:
        f.write(cases_source(cases))
    subprocess.run(
        [cc, "-nostdlib", "-static", "-Wl,--build-id=none", "-Wl,--no-warn-rwx-segments",
         "-T", os.path.join(HERE, "qemu_el2.ld"),
         os.path.join(HERE, "qemu_el2_runner.S"), src, "-o", elf],
        check=True,
    )
    out = subprocess.run(
        [qemu, "-M", "virt,virtualization=on", "-cpu", "cortex-a57", "-m", "256M", "-display", "none",
         "-monitor", "none", "-nic", "none", "-serial", "stdio", "-kernel", elf],
        check=True, capture_output=True, text=True, timeout=600,
    ).stdout
    return out.splitlines()


def vector_line(name, case, record):
    outcome, x0, esr, far, elr, spsr = record
    fields = [case.word, case.target, case.hcr, case.cptr, case.mdcr, case.cnthctl, case.hstr, case.cpacr,
              case.sctlr, case.cntkctl, case.mdscr, case.oslar, case.spsr1, case.elr1, case.entry(), case.x0]
    head = " ".join(f"{v & MASK64:x}" for v in fields)
    return (f"{name} {head} v={case.value} o={outcome:x} x={x0:x} c={esr:x} f={far:x} e={elr:x} p={spsr:x}")


def main():
    out_file = sys.argv[1]
    per_family = int(sys.argv[2]) if len(sys.argv) > 2 else 400
    r = SplitMix64(0xE12_5EED)
    named = []
    for name, make in FAMILIES:
        named += [(name, make(r)) for _ in range(per_family)]
    with tempfile.TemporaryDirectory() as workdir:
        lines = run([c for _, c in named], workdir)
    if not lines or not lines[0].startswith("S ") or lines[-1] != "END":
        raise SystemExit(f"the runner did not finish: {lines[:3]} .. {lines[-3:]}")
    reset = [int(v, 16) for v in lines[0].split()[1:]]
    print("reset: MIDR {:x} PFR0 {:x} DFR0 {:x} HCR {:x} SCTLR_EL2 {:x} CPTR {:x} MDCR {:x} CNTHCTL {:x} "
          "SCTLR_EL1 {:x}".format(*reset), file=sys.stderr)
    if reset[0] != MIDR or (reset[1] >> 8) & 0xF == 0:
        raise SystemExit("not a Cortex-A57 with EL2")
    body = lines[1:-1]
    if len(body) != len(named):
        raise SystemExit(f"{len(body)} lines for {len(named)} cases: {body[-2:]}")
    vectors = []
    tally = {}
    for (name, case), line in zip(named, body):
        parts = line.split()
        if parts[0] != "R":
            raise SystemExit(f"{name}: {line}")
        record = [int(v, 16) for v in parts[1:]]
        vectors.append(vector_line(name, case, record))
        t = tally.setdefault(name, [0, 0, 0])
        t[record[0]] += 1
    for name, (n, el1, el2) in tally.items():
        print(f"{name}: {n} completed, {el1} took an exception to EL1, {el2} to EL2", file=sys.stderr)
    with open(out_file, "w") as f:
        f.write("\n".join(vectors) + "\n")


if __name__ == "__main__":
    main()
