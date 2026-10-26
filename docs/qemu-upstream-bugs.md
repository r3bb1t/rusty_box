# QEMU Upstream Bugs (AArch64)

Bugs in upstream QEMU found while building `rusty_box_aarch64`, which ports
QEMU's `virt` board and checks its processor against both QEMU and the Arm
ARM. Each entry is written to be sent upstream, harmless ones included.
Every entry cites the QEMU file and function, never a line number, and the
Arm pseudocode function or document section the behaviour is judged by.

The references are QEMU 8.2.2 (Ubuntu 24.04's package), QEMU 11.1.2 (built
from the `v11.1.2` tag), and QEMU master as of 2026-10-26 for the open
entries. The architectural reference is the Arm ARM's pseudocode as Arm
publishes it in AARCHMRS (release 2026-09), and a device's own TRM.

QEMU takes changes as patches mailed to `qemu-devel@nongnu.org` (and, for
Arm, `qemu-arm@nongnu.org`), with the maintainers
`scripts/get_maintainer.pl` names in copy, each patch carrying a
`Signed-off-by:` line (see QEMU's `docs/devel/submitting-a-patch.rst`). It
does not take GitHub or GitLab pull requests. The patches here apply to
`v11.1.2` and were built and tested there; they are diffs, to be committed
with a message and sign-off of the submitter's own.

Reproducers:
- `docs/qemu-aarch64-probe.S`: a bare-metal EL1 program for
  `qemu-system-aarch64 -M virt`, one line of output per case (A to L). Its
  header gives the commands and the expected and observed output.
- `docs/qemu-aarch64-probe-user.c`: a static Linux program for
  `qemu-aarch64 -cpu max`, one case per argument, for the entries fixed
  upstream.

What `rusty_box_aarch64` does about each entry is in its README, "Where the
references differ".

---

## Open in QEMU 11.1.2 and master

### 1. PACIA and the other PAC instructions code the wrong half of the address space for a pointer whose bits 55 and 63 differ

**Impact**: low. A pointer code differs from hardware's (and from QEMU's
own AUT* expectations on hardware-signed pointers) for pointers whose bits
55 and 63 differ, where no top-byte-ignore applies to bit 55's half but
one applies to the other half, or none applies at all. Such pointers are
non-canonical or carry an unusual tag, so ordinary software rarely signs
one.

**Location**: `target/arm/tcg/pauth_helper.c`, `pauth_addpac()`.

**What QEMU does**: `pauth_addpac()` takes `param =
aa64_va_parameters(env, ptr, ...)`, which selects the TTBR0 or TTBR1 half
by `ptr<55>`, and computes the code's bottom bit from that half's TxSZ
(`bot_bit = 64 - param.tsz`). The extension is `ptr<55>` if that half's
TBI applies and `ptr<63>` otherwise.

**What Arm specifies**: `InsertPAC()` (shared pseudocode,
`aarch64/functions/pac/addpac/InsertPAC`) computes `selbit` first: in a
regime with two ranges it is `ptr<55>` when TBI applies to the access in
either half (TCR_ELx.TBI1 or TBI0, minus TBID for an instruction address),
and `ptr<63>` otherwise; in a regime with one range it is `ptr<55>` with
TBI and `ptr<63>` without. `bottom_PAC_bit = CalculateBottomPACBit(selbit)`
uses the TxSZ of the half `selbit` names, and the extension field is
`Replicate(selbit)`. (With FEAT_PAuth2 and FEAT_CONSTPACFIELD `selbit` is
`ptr<55>` always; QEMU implements FEAT_CONSTPACFIELD for no CPU.)

**Reproduce**: probe case C (`-cpu max,pauth-qarma5=on`). PACDA of
`0x7f80000000001000` with T0SZ = 25, T1SZ = 20 and no TBI: `selbit` is
`ptr<63>` = 0, so the code occupies bits 54:39.

    Arm:          C 3106208000001000
    QEMU 11.1.2:  C 3106200000001000     bits 43:39 left zero (T1SZ's 54:44)

**Fix**: `docs/qemu-insertpac-selbit.patch`: a `pauth_selbit()` helper
following `InsertPAC()`, and the bottom bit and extension from `selbit`'s
half. `AuthPAC()` needs no change: it uses `ptr<55>`, which `InsertPAC()`
has set to `selbit`. Tested: probe case C prints Arm's value, and
`rusty_box_aarch64/tests/vectors/gen_qemu_pauth.py` run without its
`qemu_selects_differently` filter (2,400 cases of PAC*, AUT*, XPAC*, the
hints, PACGA, the authenticating branches, ERETAA/ERETAB and LDRAA/LDRAB
under random keys and TCR_EL1 TBI, TBID and TxSZ fields) agrees with
`rusty_box_aarch64`'s implementation of `InsertPAC()` on all 2,400 with
the patch, and on 2,344 without it. An alternative for `-cpu max` alone is
to advertise FEAT_CONSTPACFIELD (ID_AA64ISAR2_EL1.PAC_frac = 1), which
makes QEMU's current behaviour the architected one there, but not for a
named CPU with FEAT_PAuth.

### 2. An illegal exception return leaves PSTATE.PAN alone

**Impact**: low. Only an illegal exception return, which is a software
bug, shows it; the Illegal Execution State exception that follows then
saves PAN as it was instead of as SPSR_ELx asked.

**Location**: `target/arm/tcg/helper-a64.c`, `HELPER(exception_return)`,
the `illegal_return` path.

**What QEMU does**: restores NZCV, DAIF and ALLINT from SPSR_ELx and keeps
every other PSTATE field.

**What Arm specifies**: `SetPSTATEFromPSR()` restores NZCV, PAN (with
FEAT_PAN), PM (with FEAT_EBEP, FEAT_SPE_EXC or FEAT_TRBE_EXC), ALLINT
(with FEAT_NMI) and DAIF "regardless of illegal exception return"; only
the other fields are kept or made UNKNOWN.

**Reproduce**: probe case A: an ERET to EL2h (no EL2) with SPSR_EL1.PAN =
1 and PSTATE.PAN = 0. The Illegal Execution State exception's SPSR_EL1
holds the PAN the return set.

    Arm:          A 000000003a000000 00000000005003c5
    QEMU 11.1.2:  A 000000003a000000 00000000001003c5     bit 22 clear

**Fix**: `docs/qemu-illegal-return-pan.patch` restores PAN too when the
CPU has FEAT_PAN. Tested with probe case A. (QEMU implements none of
FEAT_EBEP, FEAT_SPE_EXC and FEAT_TRBE_EXC, so PM needs nothing yet.)

### 3. An atomic read-modify-write reports its alignment, translation and read-permission faults as writes

**Impact**: low to medium. ESR_ELx.WnR is set where Arm and hardware
clear it. An OS that decides from WnR how to resolve a fault (Linux's
arm64 `do_page_fault()` does) handles an atomic to a missing page as a
write fault on QEMU and as a read fault followed by a write fault on
hardware; the outcome is the same, the fault sequence is not.

**Location**: `accel/tcg/cputlb.c`, `atomic_mmu_lookup()`: the TLB fill
and the alignment check use `MMU_DATA_STORE`; read permission is checked
after write permission (`tlb_fill_align(..., MMU_DATA_LOAD, ...)` only once
the page is known writable).

**What Arm specifies**: `CreateAccDescAtomicOp()` makes an atomic access
both a read and a write. A fault record starts as `NoFault(accdesc)`,
whose `write` is `!accdesc.read && accdesc.write`, false for an atomic, so
an alignment, translation, access flag or address size fault reports WnR
= 0. `AArch64_S1CheckPermissions()` checks read permission before write
permission, and only a refused write sets `write`. Linux's 2024 arm64
change forcing a write fault for atomic RMW instructions describes the
same two faults on hardware (a read fault on a missing page, then a write
fault).

**Reproduce**: probe case B: LDADD of 8 bytes crossing a 16-byte boundary
(an alignment fault).

    Arm:          B 0000000096000021 ...     WnR (bit 6) clear
    QEMU 11.1.2:  B 0000000096000061 ...

**Fix**: no patch. `atomic_mmu_lookup()` is target-independent, and the
store-first order is right for targets whose locked RMW reports a write
(x86's page fault error code W/R bit). A fix needs a target hook or MemOp
flag for "reports an atomic's faults as Arm does": fill and check
alignment as `MMU_DATA_LOAD` first, then check write permission and report
only that refusal as a write.

### 4. Cortex-A57, Cortex-A72, Cortex-A53 and Cortex-A35 report the wrong cache geometry

**Impact**: harmless to most software. `CCSIDR_EL1` and, since 11.1.0,
the `virt` device tree's CPU cache properties (`d-cache-sets` and the
rest, which Linux's cacheinfo reports in
`/sys/devices/system/cpu/cpu*/cache/`) describe caches these CPUs do not
have.

**Location**: `target/arm/cpu-max.c` `aarch64_aa32_a57_init()` (shared by
`cortex-a57` and `max`), `target/arm/tcg/cpu64.c` `aarch64_a72_initfn()`
and `aarch64_a35_initfn()`, `target/arm/cpu64.c` `aarch64_a53_initfn()`.

**What QEMU does**: commit 676624d757ab ("target/arm/tcg: refine cache
descriptions with a wrapper", QEMU 9.2) rewrote the raw CCSIDR values as
`make_ccsidr(format, assoc, linesize, cachesize, flags)` calls, and the
associativity of five caches changed:

| CPU | Cache | Before 9.2 (raw value) | 9.2 to 11.1.2 and master |
| --- | --- | --- | --- |
| Cortex-A57 | L1 data, 32 KiB | 2-way, 256 sets (`0x701fe00a`) | 4-way, 128 sets |
| Cortex-A72 | L1 data, 32 KiB | 2-way, 256 sets (`0x701fe00a`) | 4-way, 128 sets |
| Cortex-A53 | L1 instruction, 32 KiB | 2-way, 256 sets (`0x201fe00a`) | 1-way, 512 sets |
| Cortex-A35 | L1 instruction, 32 KiB | 2-way, 256 sets (`0x201fe00a`) | 4-way, 128 sets |
| Cortex-A35 | L2 | 8-way, 512 sets, 256 KiB (`0x703fe03a`) | 16-way, 512 sets, 512 KiB |

The Cortex-A72's instruction cache comment also reads "48KB L1 dcache".

**What the CPUs have**: the Cortex-A57 TRM (ARM DDI 0488C, 6.1 "About the
L1 memory system") gives a "32KB 2-way set-associative data cache" and a
"48KB 3-way set-associative instruction cache", with 64-byte lines. For
the A72, A53 and A35 the values before 9.2 are QEMU's own, from when each
CPU was added; they should be confirmed against those CPUs' TRMs when the
patch is sent. The A35 L2's size is a configuration choice; before 9.2 the
raw value said 256 KiB and its comment 512 KiB.

**Reproduce**: `qemu-system-aarch64 -M virt,dumpdtb=virt.dtb -cpu
cortex-a57 -m 256M -nic none` writes `d-cache-sets = <0x80>` (4 ways); a
guest reads the same from `CCSIDR_EL1` with `CSSELR_EL1` = 0.

**Fix**: `docs/qemu-cortex-cache-geometry.patch` restores the values from
before 9.2 (and the A72 comment). Tested: the `virt` device tree has
`d-cache-sets = <0x100>` for `cortex-a57`.

### 5. `virt` names phandle 0 as a CPU's next-level cache

**Impact**: harmless. Phandle 0 names no node (libfdt treats 0 as "no
phandle", and Linux's `of_find_node_by_phandle(0)` returns NULL), so
software finds no next level, which is what it would find without the
property.

**Location**: `hw/arm/virt.c`, `fdt_add_cpu_nodes()`.

**What QEMU does**: since commit f669de700111 ("hw/arm/virt: add cache
hierarchy to device tree", QEMU 11.1.0), every CPU node gets
`next-level-cache = <core_offset>`. When no cache node is created for a
level above L1 (no `-machine smp-cache` description, the default),
`core_offset` stays 0.

**Reproduce**: `qemu-system-aarch64 -M virt,dumpdtb=virt.dtb -cpu
cortex-a57 -m 256M -nic none`, then `dtc -I dtb virt.dtb`:
`next-level-cache = <0x00>;` in `/cpus/cpu@0`.

**Fix**: `docs/qemu-virt-next-level-cache.patch` writes the property only
when a next-level node exists. Tested with the same dump.

### 6. The PL011 in loopback writes past its receive FIFO

**Impact**: harmless outside loopback tests. With UARTCR.LBE set, the
17th character written while 16 wait in the receive FIFO overwrites the
oldest one, and `read_count` grows past the FIFO's depth, so the guest
reads one character too many and loses the first.

**Location**: `hw/char/pl011.c`, `pl011_loopback_tx()`, which calls
`pl011_fifo_rx_put()` without the room check `pl011_can_receive()` gives
the chardev path.

**What the PL011 does**: the PL011 TRM (ARM DDI 0183) describes an
overrun: with the receive FIFO full, a received character is lost, the
FIFO's contents stay valid, and the overrun error is flagged (UARTRSR.OE,
UARTDR.OE and the overrun interrupt UARTRIS.OERIS).

**Reproduce**: probe case D: FEN and LBE set, 17 characters written, then
UARTDR read until UARTFR.RXFE.

    PL011:        D abcdefghijklmnop
    QEMU 11.1.2:  D qbcdefghijklmnopq

**Fix**: in `pl011_loopback_tx()`, drop the character when `read_count`
has reached `pl011_get_fifo_depth()` (and flag the overrun, which QEMU's
PL011 does not model on its chardev path either).

**To check against the TRM before sending**: `pl011_loopback_mdmctrl()`
sets a modem status interrupt (UARTRIS.RIRMIS, DCDRMIS, CTSRMIS, DSRRMIS)
while the looped line is asserted and clears it when the line drops. The
PL011's modem status interrupt, as this author recalls the TRM's
description of UARTMSINTR, is asserted when a modem status line changes,
either way, and cleared only through UARTICR. `rusty_box_aarch64` follows
QEMU here until the TRM settles it.

### 7. DC CVAP and DC CVADP fault as loads

**Impact**: small. A fault on DC CVAP or DC CVADP (FEAT_DPB, FEAT_DPB2)
reaches the guest as a load's: ESR_ELx.CM and WnR clear, and FAR_ELx the
address rounded down to the cache line rather than the register's value.
At EL1 with PSTATE.PAN set, one to memory EL0 can reach takes a permission
fault, which Arm's rules never give a clean from EL1. Linux tells a cache
maintenance fault from a write by CM (`is_write_abort()` in
`arch/arm64/mm/fault.c`), so it handles QEMU's as a read, which is what
it would decide anyway; a guest that keys anything else on CM, or cleans
user memory with PAN set, sees the difference.

**Location**: `target/arm/helper.c`, `dccvap_writefn()`, which calls
`probe_read(env, vaddr, dline_size, mem_idx, GETPC())` with `vaddr` the
register rounded down to `CTR_EL0.DminLine`'s line and `mem_idx` the
current one (`arm_env_mmu_index()`, the PAN index under PAN).

**What the Arm ARM says**: `AArch64_DC()` translates the register's value
with `CreateAccDescDC()`, an `AccessType_DC` access whose `pan` is
FALSE. `AArch64_S1CheckPermissions()` gives a clean (`CacheOp_Clean`) a
permission fault only at EL0, for want of read permission ("DC from
privileged context which clean cannot generate a Permission fault"). On a
fault, `AArch64_DC()` sets `fault.vaddress` to the register's value, and
`AArch64_FaultSyndrome()` sets ISS bit 8 (CM) and bit 6 (WnR) for
`AccessType_DC`.

**Reproduce**: probe cases F (DC CVAP of an unmapped `0x80000123`) and G
(DC CVAP of memory EL0 can reach, at EL1 with PAN set).

    Arm:          F 0000000096000145 0000000080000123
    QEMU 11.1.2:  F 0000000096000005 0000000080000100
    Arm:          G none
    QEMU 11.1.2:  G 000000009600000d 00000000c0000100

**Fix**: no patch yet. The probe needs a translation with an
`AccessType_DC`-like access type, which QEMU's `MMUAccessType` lacks:
one that skips the PAN index and the EL1 permission check, keeps EL0's
read check, and makes `arm_deliver_fault()` set CM and WnR and report the
unrounded address. `arm_deliver_fault()` (`target/arm/tcg/tlb_helper.c`)
passes `cm = 0` to the syndrome builders on every path today; only the AT
instructions' faults on a stage 2 walk set CM (`target/arm/tcg/cpregs-at.c`).

### 8. An exception return does not send an event

**Impact**: harmless to Linux, whose EL0 runs with SCTLR_EL1.nTWE set.
The Event Register stays clear across an ERET, so a WFE soon after one
waits for the next event instead of completing, and at EL0 with nTWE
clear it traps. A wait loop whose condition an interrupt handler changed
between the loop's check and its WFE sleeps until the next event (the
event stream's, if on) instead of going round again at once, which is
what the architecture's event on exception return is for.

**Location**: `target/arm/tcg/helper-a64.c`, `HELPER(exception_return)`.
QEMU 11 sets the Event Register for SEV and SEVL only (`trans_SEV`,
`trans_SEVL` in `target/arm/tcg/translate-a64.c`).

**What the Arm ARM says**: `AArch64_ExceptionReturn()` calls
`SendEventLocal()` after `SetPSTATEFromPSR()`, on a legal and an illegal
return alike. (`AArch32_ExceptionReturn()` does the same, for
`HELPER(cpsr_write_eret)`.)

**Reproduce**: probe case J: WFE as the first EL0 instruction after the
ERET to EL0, with SCTLR_EL1.nTWE clear.

    Arm:          J 0000000056000000 ...   (the SVC after the WFE)
    QEMU 11.1.2:  J 0000000007e00001 ...   (the WFE trapped)

**Fix**: `docs/qemu-eret-event.patch` sets the Event Register in
`HELPER(exception_return)`, ahead of the illegal-return branch so both
paths send it. Tested: case J prints the SVC's syndrome; Alpine 3.23
boots to its login prompt.

### 9. The event stream sets the Event Register only while a WFE waits

**Impact**: harmless. QEMU arms the event stream's timer only from inside
`HELPER(wfe)` and `HELPER(wfet)`, and its callback sets the Event Register
only if the CPU is halted in WFE. An event the stream generates while the
CPU runs is lost, so the next WFE waits for the stream's following event
instead of completing at once, and at EL0 with SCTLR_EL1.nTWE clear it
traps. A loop that polls with WFE sees at most one stream period of extra
latency.

**Location**: `target/arm/tcg/op_helper.c`, `gt_calc_next_event_stream()`
and its callers; `target/arm/cpu.c`, `arm_wfxt_timer_cb()`.

**What the Arm ARM says**: `TestEventCNTV()` (and `TestEventCNTP()` for
CNTHCTL_EL2) runs on every count of the generic counter and calls
`SetEventRegister()` at each selected transition of the count's bit
`EVNTI`, whatever the PE is doing.

**Reproduce**: probe case K: SEVL and WFE at EL0 (an event, consumed),
a delay with CNTKCTL_EL1.EVNTEN set and EVNTI = 0, then WFE.

    Arm:          K 0000000056000000 ...
    QEMU 11.1.2:  K 0000000007e00001 ...   (the WFE trapped)

**Fix**: no patch yet. WFE and WFET would need to know whether the stream
has passed a selected transition since the Event Register was last
cleared: the count at that moment kept in `CPUARMState` and compared at
the next WFE or WFET (the same bit test `gt_recalc_one_evt()` makes), and
brought up to date when CNTKCTL_EL1 or CNTHCTL_EL2 is written, so each
stretch of the count is judged by the controls that held then.
`rusty_box_aarch64` does exactly this (`cpu/wait.rs`).

### 10. WFET does not trap when an interrupt is pending

**Impact**: harmless. At EL0 with SCTLR_EL1.nTWE clear, a WFET whose
timeout has not passed completes instead of trapping when an interrupt is
pending, masked or not.

**Location**: `target/arm/tcg/op_helper.c`, `HELPER(wfet)`, which returns
on `cpu_has_work(cs)` before `check_wfx_trap()`.

**What the Arm ARM says**: `Hint_WFET()` completes ahead of
`AArch64_CheckForWFxTrap()` only when the Event Register is set or the
local timeout has passed (`LocalTimeoutEvent()`). Interrupts come before
the trap check only in `Hint_WFI()` and `Hint_WFIT()`, which QEMU follows.

**Reproduce**: probe case L: SEVL and WFE at EL0, then WFET with a
timeout of all ones while the virtual timer's interrupt is pending
(masked by PSTATE.I).

    Arm:          L 0000000007e00027 ...   (the WFET trapped: TI = 3, RV, RN = 1)
    QEMU 11.1.2:  L 0000000056000000 ...

With the timer's interrupt not pending, QEMU 11.1.2 traps as Arm does
(`0000000007e00027`).

**Fix**: `docs/qemu-wfet-trap-order.patch` moves the `cpu_has_work()`
return after the trap check. Tested: case L prints the WFET's syndrome;
Alpine 3.23 boots to its login prompt.

---

## Deliberate QEMU omissions

Not bugs: QEMU's comments say these are chosen. Each is a possible
feature patch.

- **No SP alignment check.** `gen_check_sp_alignment()` in
  `target/arm/tcg/translate-a64.c` is an empty hook "for the sake of guest
  program performance"; the Arm ARM checks SP's alignment on every
  SP-based load or store when SCTLR_ELx.SA or SA0 says so
  (`CheckSPAlignment()`). An `ldraa x0, [sp]` with SP 8 bytes off and
  SCTLR_EL1.SA set loads on QEMU.
- **DC ZVA does not fault on Device memory.** `HELPER(dc_zva)` in
  `target/arm/tcg/helper-a64.c` zeroes it; Arm's
  `AArch64_S1HasAlignmentFaultDueToMemType()` returns TRUE for
  `AccessType_DCZero` on Device memory, an Alignment fault.
- **Cache maintenance by address is a NOP.** `v8_cp_reginfo` in
  `target/arm/helper.c` makes IC IVAU, DC IVAC, DC CVAC, DC CVAU and DC
  CIVAC `ARM_CP_NOP` ("Cache ops: all NOPs since we don't emulate
  caches"). Arm's `AArch64_IC()` and `AArch64_DC()` translate the address
  first and abort on a fault, with CM and WnR set, and DC IVAC needs write
  permission (`AArch64_S1CheckPermissions()`): probe cases E (DC CIVAC), H
  (IC IVAU) and I (DC IVAC of read-only memory) fault on Arm and print
  "none" on QEMU. A kernel's cache maintenance of a bad user address, which
  faults on hardware and goes to its exception fixup, completes on QEMU.
- **Should-be fields.** QEMU executes the exclusive loads and stores
  (`STXR`, `LDXR`, `STXP`, `LDXP`) with `Rs` or `Rt2` other than their
  should-be values, and FCMP/FCMPE with zero whatever `Rm` holds. Such a
  word is CONSTRAINED UNPREDICTABLE, and executing it as if the field held
  its should-be value is among the permitted behaviours.

---

## Fixed upstream since 8.2.2

Each was reproduced on 8.2.2 and checked fixed on 11.1.2 (the user-mode
cases with `docs/qemu-aarch64-probe-user.c`). The release is the first to
contain the fix.

| Bug in 8.2.2 | Fixed by | Release |
| --- | --- | --- |
| LDAPUR and STLUR add `imm9` unsigned: `ldapursb x1, [x3, #-166]` reads at +346 (`ldapur`) | 5669d26ec614 "Fix handling of LDAPR/STLR with negative offset" | 9.1.0 |
| FCVTZS (scalar, fixed-point) to half precision sign-extends the result into 32 bits (`fcvtzs_half`) | fe84877ed440 "Zero-extend writeback for fp16 FCVTZS (scalar, integer)" | 9.1.0 |
| FJCVTZS sets Z for a denormal FPCR.FZ flushes (`fjcvtzs_fz`) | 7619129f0d4a "Fix FJCVTZS vs flush-to-zero" | 9.1.0 |
| LDAPR with no offset ignores SCTLR_ELx.nAA under FEAT_LSE2 | 25489b521b61 "LDAPR should honour SCTLR_ELx.nAA" | 9.1.0 |
| A 64-bit FCMLA (by element) of half precision raises the exceptions of the four elements above the vector (`fcmla_elem_q0`) | between 8.2.2 and 11.1.2 (likely the decodetree conversion, 80b02a565e51, 9.1.0) | by 11.1.2 |
| An alignment fault from a Device memory type is reported after a permission fault on the same page | e530581ee065 "Fix alignment fault priority in get_phys_addr_lpae" | 9.2.0 |
| The scalar half-precision FABS and FNEG, which Arm does not allocate: FNEG (0x7ef8f820) executes, FABS (0x5ef8f820) aborts QEMU in `disas_simd_two_reg_misc_fp16()` (`fabs_scalar_half`) | 7261f2a7a75a "Fix decode of fp16 vector fabs, fneg, fsqrt" | 10.0.0 |
| Unallocated two-register miscellaneous half-precision words abort QEMU in `disas_simd_two_reg_misc_fp16()` (`misc_fp16_abort`) | that function's conversion to decodetree | by 10.0.0 |
| The Debug Communications Channel registers sit at the wrong encodings (DBGDTR_EL0 is missing) | 655659a74a36 "Correct encoding of Debug Communications Channel registers" | 10.1.0 |

Two of these were in `rusty_box_aarch64` too until this audit: the
alignment-fault priority, and the missing DBGDTR_EL0, DBGDTRRX_EL0 and
DBGDTRTX_EL0 (with their MDSCR_EL1.TDCC trap at EL0).

Not a bug, but a change a test harness notices: since 10.0.0 (commit
132f8ec799ce) `-cpu max` uses QEMU's own pointer authentication algorithm
by default, not QARMA5; `-cpu max,pauth-qarma5=on` selects the
architected one.
