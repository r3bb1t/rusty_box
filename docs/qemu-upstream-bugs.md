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
does not take GitHub or GitLab pull requests. Each entry's **Fix** names
its patch, or says it has none yet (entries 3, the first of the two
numbered 17, 20, 21 and 24 onward). The patches apply to `v11.1.2`, each on its own and all
of them together, and were built and tested there, but for entries 22 and
23, written against master as of 2026-10-28 and checked to apply there,
not yet built or tested; they are diffs, to be committed with a message
and sign-off of the submitter's own. An entry whose **Reproduce** says it
was found by reading the source has not been run on QEMU; each says which
master it was checked against.

Reproducers:
- `docs/qemu-aarch64-probe.S`: a bare-metal EL1 program for
  `qemu-system-aarch64 -M virt`, one line of output per case (A to T). Its
  header gives the commands and the expected and observed output.
- `docs/qemu-aarch64-probe-user.c`: a static Linux program for
  `qemu-aarch64 -cpu max`, one case per argument, for the entries fixed
  upstream and for entries 12 to 16.

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

**Fix**: `docs/patches/qemu/qemu-insertpac-selbit.patch`: a `pauth_selbit()` helper
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

**Fix**: `docs/patches/qemu/qemu-illegal-return-pan.patch` restores PAN too when the
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

**Fix**: `docs/patches/qemu/qemu-cortex-cache-geometry.patch` restores the values from
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

**Fix**: `docs/patches/qemu/qemu-virt-next-level-cache.patch` writes the property only
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

**Fix**: `docs/patches/qemu/qemu-pl011-loopback-overrun.patch`: `pl011_fifo_rx_put()`
drops a character that finds `read_count` at the FIFO's depth and flags
the overrun in UARTRSR.OE and the overrun interrupt (INT_OE); a read of
UARTDR keeps UARTRSR.OE, which only a write of UARTECR clears. Loopback
and a break from the chardev (`pl011_event()`, which puts it in the FIFO
without asking `pl011_can_receive()`) can overrun; the chardev's
characters cannot, since `pl011_can_receive()` holds them back. UARTDR.OE,
which the TRM reports from an overrun until a character next enters the
FIFO, is not modelled. Tested: case D prints the PL011's line.

**Before sending**: QEMU's Rust PL011, which a build with Rust uses for
`virt` in place of the C one (`hw/char/Kconfig`), has the same unchecked
write in `PL011Registers::fifo_rx_put()`
(`rust/hw/char/pl011/src/device.rs`), and upstream will want both models
changed together. The patch covers the C model only: building QEMU's Rust
needs `bindgen-cli`, which this environment could not install, so a Rust
change could not be built or tested here.

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

**Fix**: `docs/patches/qemu/qemu-dc-cvap-fault.patch`. `dccvap_writefn()` probes the
line with the current regime's index without PAN (`dc_mmu_idx()`) and
without faulting (`probe_access_flags()` with `nonfault`). On a miss,
`dc_clean_fault()` translates the register's value with `get_phys_addr()`,
completes on a Permission fault above EL0 (but not on one of the stage 1
walk's own stage 2 translation, which is the walk's and taken at any
EL), and otherwise takes the fault
through `arm_deliver_fault()` (no longer static) as a write at the
register's value, with a new `ARMMMUFaultInfo.cm` that
`arm_deliver_fault()` passes to the syndrome builders for ISS.CM. The
template syndrome it merges into is cleared first: `GETPC()` in a
`writefn` is not always a return address into translated code (it is only
when `HELPER(set_cp_reg64)` tail-calls the `writefn`), so
`cpu_restore_state()` may leave a stale template whose ISV or VNCR bit
would otherwise reach the abort; a cache maintenance instruction's
template is empty, since only a load or store of one register sets ISV.
The probe no longer checks watchpoints, which `AArch64_DC()` gives a clean
none (it passes `AArch64_TranslateAddress()` a watchpoint size only for
an invalidate). In `qemu-aarch64` an
unreadable line raises SIGSEGV as before, now at the register's value.
Tested: cases F and G print Arm's lines; in `qemu-aarch64 -cpu max`, a
DC CVAP of a readable line completes and one of a `PROT_NONE` page at
offset 0x123 raises SIGSEGV with `si_addr` at 0x123 (0x120 before); an
Alpine 3.23 kernel boots to `login:`.

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

**Fix**: `docs/patches/qemu/qemu-eret-event.patch` sets the Event Register in
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

**Fix**: `docs/patches/qemu/qemu-event-stream-fold.patch`. A new
`gt_fold_event_stream()` sets the Event Register when a stream has made a
selected transition (`gt_evt_between()`) between the system counter value
it was last brought up to, a new `CPUARMState.evt_folded_to`, and now,
and then moves that value to now. The streams are Arm's: CNTHCTL_EL2's on
the physical count (`TestEventCNTP()`), and CNTKCTL_EL1's on the physical
count less CNTVOFF_EL2 unless the effective E2H and TGE are both 1
(`TestEventCNTV()`), whatever the current EL. WFE and WFET fold before
they read the register while either stream is on (with both off there is
nothing to fold, and the write that turns one on folds first). Writes of
CNTKCTL_EL1, CNTHCTL_EL2 and CNTVOFF_EL2 fold first, as do a write of
HCR_EL2 that changes E2H or TGE and a write of SCR_EL3 that changes NS or
EEL2 (which decide whether EL2 is enabled, and with it the effective
TGE), so each stretch of the count is judged by the controls that held
then. CNTKCTL_EL1 keeps EVNTIS only with FEAT_ECV: without it the bit is
RES0, and `TestEventCNTV()` reads it only with FEAT_ECV. A reset and an
incoming migration start at the current value. The timer that wakes a
waiting WFE is left alone (entries 18 and 19 fix it).
`rusty_box_aarch64` does the same (`cpu/wait.rs`). Tested: case K prints
the SVC's syndrome, and an Alpine 3.23 kernel boots to `login:` on
`-cpu max` with one, two and four CPUs and on `-cpu cortex-a57`.

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

**Fix**: `docs/patches/qemu/qemu-wfet-trap-order.patch` moves the `cpu_has_work()`
return after the trap check. Tested: case L prints the WFET's syndrome;
Alpine 3.23 boots to its login prompt.

### 11. DBGAUTHSTATUS_EL1 is missing

**Impact**: small. An `MRS` of DBGAUTHSTATUS_EL1 at EL1 or EL2 is
UNDEFINED, where every AArch64 processor has the register, so software
that asks how debug is authenticated takes an Undefined Instruction
exception instead of an answer. Linux reads it in one place: KVM's
`trap_dbgauthstatus_el1()` (`arch/arm64/kvm/sys_regs.c`, 6.18) answers a
guest's trapped read with the host's own `read_sysreg(dbgauthstatus_el1)`,
which on QEMU with EL2 would be an Undefined Instruction exception in the
host kernel. That path is read from the source, not reproduced here.

**Location**: `target/arm/debug_helper.c`, `debug_cp_reginfo`, which
defines the other debug registers (OSLAR_EL1, DBGCLAIMSET_EL1 and the
rest) but not this one; only its fine-grained trap bit,
`HDFGRTR_EL2.DBGAUTHSTATUS_EL1`, appears (`target/arm/cpregs.h`).

**What the Arm ARM says**: Arm's register data gives DBGAUTHSTATUS_EL1 to
every processor with AArch64 (its condition is `FEAT_AA64`), readable at
EL1 and above. `UpdateDbgAuthStatus()` sets it from the IMPLEMENTATION
DEFINED authentication interface: on a processor whose security state is
always Non-secure, NSID = 0b10 or 0b11 (`ExternalInvasiveDebugEnabled()`,
DBGEN), NSNID = 0b10 or 0b11 (`ExternalNoninvasiveDebugEnabled()`, which
is TRUE with FEAT_Debugv8p4), and SID and SNID 0b00; with Secure state,
SID and SNID report `ExternalSecureInvasiveDebugEnabled()` and
`ExternalSecureNoninvasiveDebugEnabled()` likewise.

**Reproduce**: probe case M, an `MRS` of DBGAUTHSTATUS_EL1 at EL1 that
prints the register's value: NSID 0b10 and, with FEAT_Debugv8p4, NSNID
0b11 when no external debugger drives DBGEN.

    Arm:          M 000000000000000e
    QEMU 11.1.2:  M 0000000002000000 ...   (UNDEFINED)

**Fix**: `docs/patches/qemu/qemu-dbgauthstatus.patch` adds DBGAUTHSTATUS_EL1 to
`debug_cp_reginfo` (op0 2, op1 0, CRn 7, CRm 14, op2 6; for AArch32,
DBGAUTHSTATUS on cp14): read-only from EL1, trapped by MDCR_EL2.TDA and
MDCR_EL3.TDA (`access_tda()`) and by HDFGRTR_EL2.DBGAUTHSTATUS_EL1, whose
`DO_BIT` entry it adds. QEMU has no external debugger, so DBGEN, NIDEN,
SPIDEN and SPNIDEN are LOW: NSID 0b10; NSNID 0b11 with FEAT_Debugv8p4,
else 0b10; SID and SNID 0b10 when the CPU has EL3, else 0b00; RLID,
RLNID, RTID and RTNID 0b10 with FEAT_RME, else 0b00. Tested: case M
prints Arm's line.

### 12. FABS and FNEG (vector) change a NaN's sign under FPCR.AH = 1

**Impact**: small. With FEAT_AFP's FPCR.AH set, the Advanced SIMD vector
FABS and FNEG clear or flip the sign bit of a NaN element, which the
architecture leaves alone; the scalar forms, and SVE's, get it right, so
the same value negated as a scalar and as a vector element differs. Only
software that sets FPCR.AH (x86 emulators running on Arm, for x86's NaN
rules) and then inspects a NaN's sign sees it.

**Location**: `target/arm/tcg/translate-a64.c`, `do_fabs_fneg_v()`, which
expands both instructions as one bitwise AND or XOR over the vector
(`gen_gvec_fabs()`, `gen_gvec_fneg()`) whatever `s->fpcr_ah` is. The scalar
forms choose between `f_scalar_fabs` and `f_scalar_ah_fabs` (and FNEG's
pair) by FPCR.AH, and SVE has `sve_ah_fabs_*` and `sve_ah_fneg_*`.

**What the Arm ARM says**: FABS (vector) and FNEG (vector) compute
`FPAbs{esize}(element, FPCR())` and `FPNeg{esize}(element, FPCR())` for
each element, and both return the operand unchanged when it is a NaN and
`IsFeatureImplemented(FEAT_AFP) && !UsingAArch32() && fpcr.AH == '1'`
("When fpcr.AH=1, sign of NaN has no consequence").

**Reproduce**: `docs/qemu-aarch64-probe-user.c`, case `fneg_vector_ah`.

    Arm:          fneg v2.4s of 0x7fc00001 = 0x7fc00001, fabs v5.4s of 0xffc00001 = 0xffc00001
    QEMU 11.1.2:  fneg v2.4s of 0x7fc00001 = 0xffc00001, fabs v5.4s of 0xffc00001 = 0x7fc00001

**Fix**: `docs/patches/qemu/qemu-afp-vector-fabs-fneg.patch`: under FPCR.AH, expand
each element with the scalar forms' `gen_vfp_ah_abs*` and
`gen_vfp_ah_neg*` (an element at a time; a gvec helper like SVE's would
be faster). Built and tested on `v11.1.2`: the probe agrees, and
`rusty_box_aarch64`'s QEMU vectors, the FEAT_AFP families included, were
generated with it.

### 13. SVE FCMLA (indexed) changes a NaN's sign under FPCR.AH = 1

**Impact**: small. With FEAT_AFP's FPCR.AH set, the SVE FCMLA (indexed)
flips the sign of a NaN element of Zm that its rotation negates, which
the architecture leaves alone; the Advanced SIMD FCMLA (by element) and
the predicated SVE FCMLA get it right. As with entry 12, only software
that sets FPCR.AH and then inspects a NaN's sign sees it.

**Location**: `target/arm/tcg/translate-sve.c`, `trans_FCMLA_zzxz()` (the
`TRANS_FEAT(FCMLA_zzxz, ...)` line). Its helpers,
`HELPER(gvec_fcmlah_idx)` and `HELPER(gvec_fcmlas_idx)` in
`target/arm/tcg/vec_helper.c`, read FPCR.AH from bit 4 of their data and
negate with `float_muladd_negate_product` under it (which leaves a NaN
alone), else by flipping the element's sign bit. The SVE translation
passes `a->index * 4 + a->rot`, whose bit 4 is always 0; the Advanced SIMD
one (`trans_FCMLA_vi()` in `translate-a64.c`) passes `(s->fpcr_ah << 4) |
(a->idx << 2) | a->rot`.

**What the Arm ARM says**: FCMLA (indexed) (`fcmla_z_zzzi`) computes
`elt2_a = FPNeg{esize}(elt2_a, FPCR())` when `neg_r` and the same of
`elt2_b` when `neg_i`, and FPNeg returns a NaN unchanged when
`IsFeatureImplemented(FEAT_AFP) && !UsingAArch32() && fpcr.AH == '1'`.

**Reproduce**: `docs/qemu-aarch64-probe-user.c`, case `sve_fcmla_idx_ah`.

    Arm:          fcmla z0.h, z1.h, z2.h[0], #90 of 1.0 by NaN 0xffc1: z0.h[0] = 0xffc1
    QEMU 11.1.2:  z0.h[0] = 0x7fc1 (Advanced SIMD fcmla v3.8h: 0xffc1)

**Fix**: `docs/patches/qemu/qemu-sve-fcmla-indexed-ah.patch`: pass FPCR.AH in bit 4, as
the Advanced SIMD form does. Built and tested on `v11.1.2`: the probe
agrees, and `rusty_box_aarch64`'s SVE vectors were generated with it.

### 14. FMAD, FMSB, FNMAD and FNMSB with size = 00 execute as BFloat16 operations

**Impact**: small. With FEAT_SVE_B16B16 (`-cpu max`), the SVE FMAD,
FMSB, FNMAD and FNMSB with `size = 00`, which the architecture makes
UNDEFINED, execute as BFloat16 fused multiply-adds instead of raising
SIGILL. No assembler produces the word; a test of the UNDEFINED encoding
space sees it.

**Location**: `target/arm/tcg/sve.decode` decodes the four as
`FMLA_zpzzz`, `FMLS_zpzzz`, `FNMLA_zpzzz` and `FNMLS_zpzzz` with the
operands reordered (`@rdn_pg_rm_ra`), and `target/arm/tcg/translate-sve.c`,
`do_fmla_zpzzz()`, accepts `esz = MO_8` as BFloat16 when FEAT_SVE_B16B16
is implemented, for BFMLA and BFMLS, the only instructions of the eight
that have a BFloat16 form.

**What the Arm ARM says**: the encodings of FMAD, FMSB, FNMAD and FNMSB
(class `sve_fp_3op_p_zds_b`) carry no `size != '00'` condition, so the
word is theirs, and each one's decode begins `if size == '00' then
EndOfDecode(Decode_UNDEF)`. FEAT_SVE_B16B16 adds BFMLA and BFMLS to
`sve_fp_3op_p_zds_a` (where FMLA and FMLS carry `size != '00'`) and
nothing to `sve_fp_3op_p_zds_b`.

QEMU 11.1.2 does the same for FSUBR (vectors, predicated) with `size =
00`, which it decodes as FSUB with the operands swapped; master fixed
that with f2bfb1cf ("Split decode of BFloat SVE FADD, FSUB, FMUL, FMIN*,
FMAX*", 2026-09-01), which is in no release yet.

**Reproduce**: `docs/qemu-aarch64-probe-user.c`, cases `sve_fmad_size00`
and `sve_fsubr_size00`.

    Arm:          SIGILL (UNDEFINED), both
    QEMU 11.1.2:  0x65228020 (fmad, size = 00) executed; 0x65038020 (fsubr, size = 00) executed

**Fix**: `docs/patches/qemu/qemu-sve-fmad-size00.patch`: decode the four as their own
names and refuse `esz = MO_8` for them. Built and tested on `v11.1.2`:
`sve_fmad_size00` raises SIGILL, and `rusty_box_aarch64`'s SVE vectors
were generated with it.

### 15. SME LD1 to a vertical tile slice leaves the inactive elements after the last active one

**Impact**: moderate. The SME LD1B, LD1H, LD1W, LD1D and LD1Q to a
vertical tile slice zero the inactive elements up to the last active one
and leave the ones after it as they were, where the architecture zeroes
them all. Code that loads a partial column of a tile, the last iteration
of a loop under a WHILELO predicate among it, reads stale elements back
from ZA. Horizontal slices are right, and so is a load with no element
active, which clears the whole slice.

**Location**: `target/arm/tcg/sme_helper.c`, `sme_ld1()`. On its fast
path (every page RAM), a horizontal slice is cleared whole with `memset`
before the active elements are loaded; for a vertical one it clears the
elements before the first active one (`clr_fn(za, 0, reg_off)`), and the
loop over the elements clears each inactive one it meets, but the loop
runs from the first active element to the last (`info.reg_off_last`), so
the inactive elements after the last are never cleared. The same holds on
the second page of a load that crosses one. The slow path, for MMIO,
loads into a zeroed scratch vector and copies it all, and is right.

**What the Arm ARM says**: LD1W (scalar plus scalar, tile slice)
(`ld1w_za_p_rrr_`) computes `result[e*:esize] = Zeros{esize}` for every
inactive element `e` from 0 to `dim - 1`, then writes the whole slice,
`ZAslice{VL}(t, esize, vertical, slice) = result`; LD1B, LD1H, LD1D and
LD1Q are the same.

**Reproduce**: `docs/qemu-aarch64-probe-user.c`, case
`sme_ld1_vertical_tail`.

    Arm:          ld1w {za0v.s[w12, 0]}, p0/z with element 0 alone active: element 1 = 0
    QEMU 11.1.2:  element 1 = 0xffffffff, its value before the load

**Fix**: `docs/patches/qemu/qemu-sme-ld1-vertical-tail.patch`: clear the whole slice
before loading, vertical or horizontal, which makes the per-element
clears redundant. Built and tested on `v11.1.2`: the probe agrees, and
`rusty_box_aarch64`'s ZA vectors were generated with it.

---

### 16. MOVT between ZT0 and a general register executes outside Debug state

**Impact**: harmless. MOVT (table to scalar) and MOVT (scalar to table),
`movt x0, zt0[0]` and `movt zt0[0], x0`, execute in any state on QEMU,
reading or writing a doubleword of ZT0. The architecture defines them
only in Debug state, for an external debugger to reach ZT0; a program
that executes one gets the Undefined Instruction exception on hardware,
and a doubleword move on QEMU. MOVT (vector to table), FEAT_SME_LUTv2's,
is defined in any state and is not affected.

**Location**: `target/arm/tcg/translate-sme.c`, `do_movt()` through
`TRANS_FEAT(MOVT_rzt, aa64_sme2, ...)` and `TRANS_FEAT(MOVT_ztr, ...)`,
which check only FEAT_SME2 and then `sme2_zt0_enabled_check()`. QEMU
implements no Debug state for TCG, so these encodings are UNDEFINED in
every state it has.

**What the Arm ARM says**: the decode of `movt_r_zt_` and of
`movt_zt_r_` is `if !IsFeatureImplemented(FEAT_SME2) || !Halted() then
EndOfDecode(Decode_UNDEF); end;`, and `Halted()` is TRUE only in Debug
state.

**Reproduce**: `docs/qemu-aarch64-probe-user.c`, case `sme2_movt`.

    Arm:          SIGILL (UNDEFINED)
    QEMU 11.1.2:  0xc04c03e0 (movt x0, zt0[0]) executed

QEMU master as of 2026-10-27 has the same `do_movt()`.

**Fix**: `docs/patches/qemu/qemu-sme2-movt-debug-state.patch`: the two encodings'
translators return false, which raises the Undefined Instruction
exception. Built and tested on `v11.1.2`: the probe raises SIGILL, and
the other cases print what they did before.

### 17. DBGPRCR_EL1 is missing

**Impact**: small. An `MRS` or `MSR` of DBGPRCR_EL1 at EL1 or EL2 is
UNDEFINED, where every AArch64 processor has the register, and with EL2
an EL1 access that `MDCR_EL2.TDOSA` or `TDE` traps to EL2 is UNDEFINED at
EL1 instead, so a hypervisor never sees the trap it asked for.

**Location**: `target/arm/debug_helper.c`, `debug_cp_reginfo`, which
defines OSLAR_EL1, OSLSR_EL1, OSDLR_EL1 and the other OS registers that
`MDCR_EL2.TDOSA` traps, but not this one.

**What the Arm ARM says**: Arm's register data gives DBGPRCR_EL1 to every
processor with AArch64 (its condition is `FEAT_AA64`): bit 0, CORENPDRQ,
and the rest RES0; its accessor traps an EL1 access to EL2 when
`MDCR_EL2.TDE` or `MDCR_EL2.TDOSA` is set and otherwise reads or writes
the register, at EL1 and EL2.

**Reproduce**: `rusty_box_aarch64/tests/vectors/gen_qemu_el2.py` with
DBGPRCR_EL1 in `EL1_REGISTERS`, under `qemu-system-aarch64 -M
virt,virtualization=on -cpu cortex-a57`: an `MRS X0, DBGPRCR_EL1`
(`0xd5301480`) at EL1.

    Arm:          completes, X0 = 0 (MDCR_EL2.TDOSA and TDE clear); with TDOSA set, a trap to EL2, EC 0x18
    QEMU 11.1.2:  an exception to EL1, ESR 0x02000000 (UNDEFINED), with TDOSA set or not

**Fix**: no patch yet: a `debug_cp_reginfo` entry (op0 2, op1 0, CRn 1,
CRm 4, op2 4; cp14 for AArch32) keeping CORENPDRQ, with the access check
the OS registers use (`access_tdosa`).

---

### 17. `-cpu max` calls itself a Cortex-A57 in the device tree

**Impact**: harmless. The processor's device tree node says
`compatible = "arm,cortex-a57"` while its `MIDR_EL1` says it is no Arm
part. Linux's arm64 code identifies a processor by `MIDR_EL1`, not by the
node's `compatible`, so no guest known to us acts on it; a guest or tool
that reads the device tree to identify the processor is told it runs on a
Cortex-A57 with none of the features `max` has.

**Location**: `target/arm/tcg/cpu64.c`, `aarch64_max_tcg_initfn()`, which
starts from `aarch64_a57_initfn()` (`target/arm/cpu64.c`, which sets
`cpu->dtb_compatible = "arm,cortex-a57"`), resets `MIDR_EL1` and never
resets the compatible; `hw/arm/virt.c`, `fdt_add_cpu_nodes()`, writes it.

**What the references say**: QEMU's own comment on `max`'s MIDR: "Reset
MIDR so the guest doesn't mistake our 'max' CPU type for a real one",
with implementer 0, which the Arm ARM's `MIDR_EL1.Implementer` reserves
for software use. Linux's binding,
`Documentation/devicetree/bindings/arm/cpus.yaml`, lists
`arm,armv8 # Only for s/w models` among the `compatible` values.

**Reproduce**: `qemu-system-aarch64 -M virt,dumpdtb=max.dtb -cpu max -m
256M -nographic`, then `dtc -I dtb max.dtb`: `compatible =
"arm,cortex-a57";` in `/cpus/cpu@0`.

QEMU master as of 2026-10-27 has the same: `aarch64_max_v8_tcg_initfn()`
(`target/arm/tcg/cpu64.c`) starts from `aarch64_aa32_a57_init()`
(`target/arm/cpu-max.c`), which sets `arm,cortex-a57`, and resets only
the MIDR.

**Fix**: `docs/patches/qemu/qemu-max-dtb-compatible.patch` sets `arm,armv8` after the
MIDR (in master, the same line in `aarch64_max_v8_tcg_initfn()`). Built
and tested on `v11.1.2`: the dump has `compatible = "arm,armv8";`, and an
Alpine 3.23 kernel (Linux 6.18) boots to `login:` on `-cpu max` as
before.

### 18. A WFE never waits for the event stream

**Impact**: harmless to the guest, costly to the host. Arm's
`WaitForEvent()` lets a PE resume whenever "the implementation chooses to
resume execution", so a WFE that does not wait breaks no rule of Arm's.
But QEMU means a WFE to sleep until the stream's next event, and with the
stream on it does not sleep at all. The timer that should wake it is set
for a count already past, so it fires at once and sets the Event
Register, and the WFE after that completes at once too. A guest that
waits with WFE spins instead. Linux's `__delay()`
(`arch/arm64/lib/delay.c`, 6.18) waits that way on a processor without
FEAT_WFxT when the event stream is on, which Linux sets to 10 kHz;
`-cpu cortex-a57` is one such processor. For EVNTI 0 or 1 with EVNTIS 0,
the code also shifts a 64-bit value by 65 or 64 places, which is
undefined behaviour in C.

**Location**: `target/arm/tcg/op_helper.c`, `gt_recalc_one_evt()`. It
works out how many ticks remain until the selected bit's next selected
transition, then returns that distance plus the timer's offset as the
system counter value to wake at. The value to wake at is the counter's
current value plus the distance. Its mask of the bits below the selected
one, `MAKE_64BIT_MASK(0, bit - 1)`, is one bit short.
`gt_calc_next_event_stream()` takes each stream's offset from
`gt_direct_access_timer_offset()`, the offset of a counter read at the
current EL.

**What the Arm ARM says**: `GenericCounterTick()` runs `TestEventCNTP()`
and `TestEventCNTV()` on every tick of the system counter.
`TestEventCNTP()` watches bit EVNTI (plus 8 with EVNTIS) of the physical
count itself. `TestEventCNTV()` watches the same bit of the physical
count less CNTVOFF_EL2 when EL2 is implemented, at every EL.

**Reproduce**: probe case N: eight rounds of SEVL, WFE and WFE at EL1
with the stream on bit 23 of the count. It prints how many rounds took
at least half a stream period. A WFE that waits for the stream's next
event gives 8.

    Waiting:      N 0000000000000008
    QEMU 11.1.2:  N 0000000000000000

QEMU master as of 2026-10-27 has the same `gt_recalc_one_evt()`.

**Fix**: `docs/patches/qemu/qemu-event-stream-wake-timer.patch`. The wake-up count
is the system counter's current value plus the distance, and the mask is
the bits below the selected one, `target_bit - 1`. The physical stream is
judged on the physical count, and the virtual stream on the count less
CNTVOFF_EL2. With a counter that does not run at 1 GHz the WFE still
wakes early until entry 19 is fixed too. Built and tested on `v11.1.2`:
case N prints 8 on `-M virt`, and with entry 19's patch on `-M virt-9.0`.
An Alpine 3.23 kernel boots to `login:` on `-cpu max` with one, two and
four CPUs and on `-cpu cortex-a57`.

### 19. WFIT and WFET time out early when the counter does not run at 1 GHz

**Impact**: harmless to the guest, costly to the host. Arm's
`WaitForInterrupt()` and `WaitForEvent()` let a PE resume whenever the
implementation chooses, so a short wait breaks no rule of Arm's. But QEMU
means a WFIT or WFET to sleep until its timeout. With the system counter
at 62.5 MHz it sleeps for about a sixteenth of the timeout. QEMU gives
that frequency to `-M virt-9.0` and older machine types, and to the
processor types older than its 1 GHz default
(`ARM_FEATURE_BACKCOMPAT_CNTFRQ`), `-cpu cortex-a57` among them. A WFE
waiting for the event stream wakes early in the same way once entry 18 is
fixed.
Linux's `__delay()` on a processor with FEAT_WFxT loops on WFET until
the delay has passed, so it spins.

**Location**: `target/arm/cpu.c`, `arm_cpu_realizefn()`, creates
`cpu->wfxt_timer` with `timer_new_ns()`, a timer in nanoseconds. But
`HELPER(wfit)`, `HELPER(wfet)` and `HELPER(wfe)`
(`target/arm/tcg/op_helper.c`) set it with `timer_mod()` to a system
counter value. They also check that value against
`INT64_MAX / gt_cntfrq_period_ns()`, as if the timer counted in ticks of
the counter's period, which is how the generic timers (`cpu->gt_timer[]`)
are created.

**What the Arm ARM says**: `LocalTimeoutEvent()` is TRUE once
`VirtualCounterTimer()` (CNTVCT_EL0) equals or exceeds the timeout, a
count of the virtual counter, whatever its frequency.

**Reproduce**: probe case O: WFIT at EL1 with a timeout 2^24 counts
ahead. It prints how many whole timeouts the count moved by.

    Waiting:                    O 0000000000000001
    QEMU 11.1.2, -M virt:       O 0000000000000001
    QEMU 11.1.2, -M virt-9.0:   O 0000000000000000

QEMU master as of 2026-10-27 creates the timer the same way.

**Fix**: `docs/patches/qemu/qemu-wfxt-timer-scale.patch` creates the timer with
`timer_new(QEMU_CLOCK_VIRTUAL, gt_cntfrq_period_ns(cpu), ...)`, as the
generic timers are. Built and tested on `v11.1.2`: on `-M virt-9.0`,
case O prints 1, a WFIT with a timeout 2^24 counts ahead waits
0x1009f3b counts (0xd47f4 before), and with entry 18's patch case N
prints 8. An Alpine 3.23 kernel boots to `login:` on `-M virt-9.0
-cpu max` and on `-cpu cortex-a57`.

### 20. The PL031's match interrupt is up to a second late

**Impact**: low. A guest that sets an alarm and reads RTCDR and RTCRIS
can find the RTC at the match value with no interrupt raised, for up to a
second.

**Location**: `hw/rtc/pl031.c`, `pl031_set_alarm()`, arms `s->timer` for
`now + ticks * NANOSECONDS_PER_SECOND`, a whole number of seconds after
the write. `pl031_get_count()` reads `tick_offset + now /
NANOSECONDS_PER_SECOND`, which changes on the second boundaries of
`rtc_clock`, so the count reaches the match value at the first boundary
after the write, up to a second before the timer fires.

**What the PL031 does**: DDI 0224C §2.1 and Figure 2-3: RTCINTR is
asserted when the RTC value and the match value are equal, on the 1 Hz
edge at which the count reaches it.

**Reproduce**: found by reading the source against the TRM; not yet run
on QEMU. A guest at 5.4 s that writes RTCMR = RTCDR + 5 sees RTCDR reach
it at 10.0 s and RTCRIS rise at 10.4 s.

**Fix**: not yet written. `pl031_set_alarm()` would arm the timer for the
boundary, `(now / NANOSECONDS_PER_SECOND + ticks) * NANOSECONDS_PER_SECOND`.
`rusty_box_aarch64` arms its alarm there (`src/devices/pl031.rs`).

### 21. The PL061 latches level interrupts and does not see its outputs

**Impact**: low on `virt`, whose only wired line is the power button, an
input Linux sets to both edges. A guest using a level-sensitive line or
an output's edge sees a different interrupt from the PL061's.

**Location**: `hw/gpio/pl061.c`, `pl061_update()` and `pl061_write()`.
- `s->istate |= ~(s->data ^ s->iev) & s->isense` ORs a level-sensitive
  line's level into the raw status, where only GPIOIC clears it. The
  raw status stays set after the level goes.
- Edges are found in `(s->old_in_data ^ s->data) & ~s->dir`, inputs
  only.
- A GPIODATA write is masked with `s->dir`, so a write to an input line
  is lost.
- `pl061_set_irq()` drops the outside level of an output line. When the
  line becomes an input it reads its last driven value until the outside
  changes.

**What the PL061 does**, according to DDI 0190B:
- §2.3.4 and §3.3.9: a level interrupt is recognised while "the external
  source holds the level constant", and GPIOIC clears only "the interrupt
  edge detection logic register". A level-sensitive line's raw status is
  its level.
- §2.2.2, Figure 2-1 and Table 2-2: detection watches the pads (`GPIN`,
  synchronised), which loop an output's level back.
- §3.3.1: a write reaches the GPIODATA register, gated by `PADDR[9:2]`
  alone, and an input reads `GPIN`.

**Reproduce**: found by reading the source against the TRM; not yet run
on QEMU.

**Fix**: not yet written. `rusty_box_aarch64`'s `src/devices/pl061.rs`
models the TRM's behaviour: latched edges and live levels kept apart,
detection on the line levels, and the outside levels kept per line.

### 22. SME2's multi-vector narrowing shifts execute outside Streaming SVE mode

**Impact**: small. On a processor with FEAT_SVE as well as FEAT_SME2
(`-cpu max`), SQRSHR, UQRSHR and SQRSHRU of two and four vectors and
SQRSHRN, UQRSHRN and SQRSHRUN of four execute with PSTATE.SM = 0, at the
non-streaming vector length, where the architecture takes the SME trap.
Software that runs them in Streaming SVE mode, as it must, sees no
difference; a test of the trap sees the instruction execute.

**Location**: `target/arm/tcg/translate-sme.c`, `do_zz_rshr()`, which
every one of them reaches through `TRANS_FEAT(..., aa64_sme2,
do_zz_rshr, ...)`. It calls `sve_access_check()`, which with PSTATE.SM =
0 on a CPU with SVE makes the plain SVE check (`goto continue_sve`). The
file's other SME2 multi-vector helpers call `sme_sm_enabled_check()`.

**What the Arm ARM says**: the Execute section of `sqrshr_z_mz2`,
`sqrshr_z_mz4`, `sqrshrn_z_mz4` and the unsigned forms on those pages
begins `CheckStreamingSVEEnabled()`, which with PSTATE.SM = 0 takes
`SMEAccessTrap(SMEExceptionType_NotStreaming, ...)`: EC 0x1D, SMTC
0b010. Their Decode requires FEAT_SME2 alone. The two-vector SQRSHRN,
UQRSHRN and SQRSHRUN (`sqrshrn_z_mz2`), which FEAT_SVE2p1 shares, begin
`CheckSVEEnabled()` and are legal outside Streaming SVE mode, as QEMU
has them.

**Reproduce**: found by reading the source against the pages; not yet run
on QEMU.

    Arm:     sqrshr z0.h, {z0.s-z1.s}, #16 (0xC1E0D400) at EL0, SME enabled,
             PSTATE.SM = 0: SME trap, SMTC 0b010 (SIGILL under Linux)
    QEMU:    executes, at the SVE vector length

Found on QEMU 11.1.2, and in master as of 2026-10-28 (73294d0).

**Fix**: `docs/patches/qemu/qemu-sme2-rshr-streaming-check.patch`: the
SME2-only forms take `sme_sm_enabled_check()`, the three forms SVE2.1
shares keep `sve_access_check()`. It applies to master as of 2026-10-28;
not yet built or tested.

### 23. SQCVT and SQRSHR of four vectors to bytes misplace them on a big-endian host

**Impact**: small, big-endian hosts only. The SME2 SQCVT, UQCVT and
SQCVTU and SQRSHR, UQRSHR and SQRSHRU of four words to bytes write the
bytes of each destination doubleword in the wrong order on a big-endian
host; a little-endian host is right.

**Location**: `target/arm/tcg/sme_helper.c`, the instances
`SQCVT4(sme2_sqcvt_sb, ...)`, `SQCVT4(sme2_uqcvt_sb, ...)`,
`SQCVT4(sme2_sqcvtu_sb, ...)` and the same three of `SQRSHR4()`. Each
indexes its `int8_t` or `uint8_t` destination with `H2`, the halfword
index macro (`H2(x)` is `x ^ 3` on a big-endian host), where a byte
element needs `H1` (`x ^ 7`), as the interleaving forms
(`SQCVTN4(sme2_sqcvtn_sb, ...)`, `SQRSHRN4(sme2_sqrshrn_sb, ...)`) and
every other byte helper have it.

**What the Arm ARM says**: `sqcvt_z_mz4` and `sqrshr_z_mz4` write
`Elem[result, r*elements + e, esize]` of each source `r` and element
`e`: the bytes of a vector in element order.

**Reproduce**: found by reading the source; not yet run (it needs a
big-endian host).

    Arm:     sqcvt z0.b, {z4.s-z7.s} with z4.s = {1, 2, 3, 4, ...}:
             z0.b[0] = 1, z0.b[1] = 2
    QEMU on a big-endian host: z0.b[0] = 2, z0.b[1] = 1

Found on QEMU 11.1.2, and in master as of 2026-10-28.

**Fix**: `docs/patches/qemu/qemu-sme2-narrow-bytes-host-order.patch`:
`H1` for the six byte forms. It applies to master as of 2026-10-28; not
yet built or tested.

### 24. SVE contiguous loads and stores ignore SCTLR_ELx.A

**Impact**: small. With strict alignment checking on (`SCTLR_EL1.A` = 1),
an SVE contiguous load or store of halfwords or larger to a misaligned
address on Normal memory completes, where the architecture takes an
alignment fault. Only system-mode software that sets `SCTLR_EL1.A` and
relies on the fault sees it; Linux leaves it clear.

**Location**: `target/arm/tcg/translate-sve.c`, `do_mem_zpa()`, passes no
alignment requirement to the helpers, and `target/arm/tcg/sve_helper.c`
(`sve_ldN_r`, `sve_stN_r` and the first-fault and non-fault forms) access
memory through host pointers or, from `target/arm/tcg/sve_ldst_internal.h`
`DO_LD_TLB` and `DO_ST_TLB`, `cpu_ld*_data_ra` and `cpu_st*_data_ra`,
which are `MO_UNALN`. QEMU enforces `SCTLR_ELx.A` at translation time
(`align_mem`, `target/arm/tcg/hflags.c` `aprofile_require_alignment`),
which these paths never consult; only Device memory's alignment
(`TLB_CHECK_ALIGNED`, `target/arm/ptw.c`) reaches them. SVE `LDR` and
`STR`, which do use `align_mem`, are right.

**What the Arm ARM says**: each element's access is `Mem{msize}`, whose
`aligned = IsAlignedSize(address, bytes)`, and a misaligned access faults
when `AArch64_UnalignedAccessFaults` holds, which `AlignmentEnforced()`
(`SCTLR_ELx.A` = 1) makes it.

**Reproduce**: found by reading the source; not yet run on QEMU.

    Arm:     EL1, SCTLR_EL1.A = 1, Streaming SVE or SVE enabled, X0 = 0x1001
             on Normal memory, P0 all true for halfwords:
             ld1h {z0.h}, p0/z, [x0] (0xA4A0A000) takes an alignment fault
    QEMU:    loads the halfwords

Found on QEMU 11.1.2, and in master as of 2026-10-28. The SME loads and
stores of tile slices and of vector groups use the same helpers and are
likely affected too; not checked.

**Fix**: not yet written. The translator would pass `align_mem` in the
helpers' descriptor, and the helpers would raise the alignment fault for
the first active misaligned element before any access, as
`arm_cpu_do_unaligned_access()` raises it for a scalar access.
`rusty_box_aarch64` checks each element (`src/cpu/sve/mem.rs`, through
`Cpu::read_mem`).

### 25. LDP and STP of Q registers at 8 mod 16 alignment-fault on Device memory

**Impact**: small. An LDP, STP, LDNP or STNP of two Q registers at an
address 8-byte but not 16-byte aligned takes an alignment fault on Device
memory, and with the MMU off on all memory, where the architecture makes
the access. Firmware copying with Q-register pairs before the MMU is on
sees the fault.

**Location**: `target/arm/tcg/translate-a64.c`, `trans_LDP_v()` and
`trans_STP_v()`, make two 16-byte accesses with
`finalize_memop_asimd(s, MO_128)` (`target/arm/tcg/translate.h`), which
adds `MO_ALIGN`: enforced at translation time when `align_mem` is set,
which with the MMU off it always is (`aprofile_require_alignment`), and
on a Device page through `memop_tlb_alignment_bits()` in
`get_phys_addr_lpae()` (`target/arm/ptw.c`). Either way each Q register
must be 16-byte aligned.

**What the Arm ARM says**: `ldp_fpsimd.xml` and `stp_fpsimd.xml` make one
`Mem{2*datasize}` access with `ispair`. For 32 bytes, `Mem` (release
2026-09) first applies `AArch64_UnalignedAccessFaults` with `aligned`
judged by the 16-byte element (so `SCTLR_ELx.A` = 1 still faults), then:
"If a 32 byte LDP (SIMD&FP) access is not aligned to 32 bytes but aligned
to 8 bytes, it is treated as four 8 byte single-copy atomic accesses",
each passed `aligned = TRUE`, which
`AArch64_S1HasAlignmentFaultDueToMemType` does not fault on Device
memory.

**Reproduce**: found by reading the source against the pseudocode; not
yet run on QEMU.

    Arm:     MMU off, SCTLR_EL1.A = 0, X0 = RAM base + 8:
             ldp q0, q1, [x0] (0xAD400400) loads both registers
    QEMU:    alignment fault, DFSC 0x21

Found on QEMU 11.1.2, and in master as of 2026-10-28.

**Fix**: not yet written. The pair needs `MO_ALIGN_8` with
`MO_ALIGN_TLB_ONLY` when `SCTLR_ELx.A` is clear and `MO_ALIGN` (16)
when it is set, which the translator cannot tell apart today: `align_mem`
is set for either. A translation flag for `SCTLR_ELx.A` alone would.
`rusty_box_aarch64` makes the four accesses (`src/cpu/mmu.rs`,
`Cpu::read_mem_q_pair` and `Cpu::write_mem_q_pair`).

### 26. LDLAR and STLLR report AR = 0 in a stage 2 Data Abort

**Impact**: low. A hypervisor that emulates a device access made with a
LORegion load-acquire or store-release (FEAT_LOR) reads in `ESR_EL2` that
the instruction had no acquire or release semantics. KVM does not read
`AR`; a hypervisor that orders its emulation by it would order these
accesses wrongly.

**Location**: `target/arm/tcg/a64.decode` decodes LDLAR and STLLR as
`LDAR` and `STLR` ("inc LDLAR", "inc STLLR") with `lasr` taken from the
encoding's `o0` bit, which is 0 for them, and
`target/arm/tcg/translate-a64.c` `trans_LDAR()` and `trans_STLR()` pass
`a->lasr` as the syndrome's acquire/release bit.

**What the Arm ARM says**: `ldlar.xml` and `stllr.xml` build their access
with `CreateAccDescLOR()`, LDLAR with `acqsc` TRUE and STLLR with `relsc`
TRUE (shared pseudocode, `CreateAccDescLOR`); the `ESR_ELx` ISS `AR`
field, bit 14, is 1 for an "instruction [that] did have acquire/release
semantics" (`LSInstructionSyndrome()`, bit 0, in `AArch64_FaultSyndrome()`).

**Reproduce**: found by reading the source against the pseudocode; not
yet run on QEMU.

    Arm:     EL2 with HCR_EL2.VM = 1, EL1 X1 = an IPA stage 2 does not map:
             ldlar x2, [x1] (0xC8DF7C22) takes a Data Abort to EL2 with
             ESR_EL2.ISV = 1, SRT = 2, SF = 1, AR = 1
    QEMU:    the same Data Abort with AR = 0

Found on QEMU 11.1.2; master not checked.

**Fix**: not yet written. `trans_LDAR()` and `trans_STLR()` would pass
true as the syndrome's acquire/release bit for every form they decode,
LDAR, STLR, LDLAR and STLLR all having the semantics.
`rusty_box_aarch64` reports `AR` 1 for them (`src/cpu/ldst.rs`).

### 27. A stage 2 fault past a misaligned access's first byte reports an instruction syndrome

**Impact**: low. A load or store that crosses from a page stage 2 maps
into one it does not is taken to EL2 with `ISV` = 1, where Arm reports
`ISV` = 0 and the hypervisor decodes the instruction itself. A hypervisor
that emulates the access from the syndrome emulates the whole access at
the faulting address, which `FAR_EL2` and `HPFAR_EL2` give as the second
page's, though part of it lies in the first.

**Location**: `target/arm/tcg/tlb_helper.c`, `merge_syn_data_abort()`,
keeps the template syndrome's `ISV` for every stage 2 Data Abort routed
to EL2 but a stage 1 walk's, whichever byte of the access the fault is
at; the slow path of a page-crossing access (`accel/tcg/cputlb.c`) faults
on the second page with the instruction's template.

**What the Arm ARM says**: `Mem[]` makes a misaligned access byte by byte
and clears `accdesc.lowestaddress` after the first byte (shared
pseudocode, `Mem`), and `AArch64_InstructionSyndromeValid()` returns
FALSE first of all for a fault whose `accessdesc.lowestaddress` is FALSE.

**Reproduce**: found by reading the source against the pseudocode; not
yet run on QEMU.

    Arm:     EL2 with HCR_EL2.VM = 1, stage 2 maps the IPA page P but not
             P + 0x1000, EL1 X1 = P + 0xFFC:
             ldr x2, [x1] (0xF9400022) takes a Data Abort to EL2 with
             ESR_EL2.ISV = 0
    QEMU:    the same Data Abort with ISV = 1, SAS = 3, SRT = 2, SF = 1

Found on QEMU 11.1.2; master not checked.

**Fix**: not yet written. The fault would carry whether its address is
the access's lowest, as `ARMMMUFaultInfo` carries `s1ptw`, and
`merge_syn_data_abort()` would drop the instruction syndrome where it is
not. `rusty_box_aarch64` reports the syndrome only at the lowest address
(`src/cpu/mmu.rs`, `SyndromeSlot`).

### 28. PAR_EL1.SH reports the reserved 0b01

**Impact**: very low. An address translation instruction of a page whose
descriptor holds the reserved `SH` value 0b01 returns `PAR_EL1.SH` 0b01,
a value Arm's `PAR_EL1` never holds. Only software that writes the
reserved value and reads it back through `AT` sees it.

**Location**: `target/arm/ptw.c`, `get_phys_addr_lpae()`, takes the
descriptor's `SH` as `cacheattrs.shareability` as written, and
`target/arm/tcg/cpregs-at.c` `par_el1_shareability()` returns it for
Normal cacheable memory.

**What the Arm ARM says**: `DecodeShareability()` takes 0b01 to
`ConstrainUnpredictable(Unpredictable_Shareability)`, which permits
Outer Shareable, Inner Shareable or Non-shareable, and
`PAREncodeShareability()` encodes each of them as 0b10, 0b11 or 0b00.

**Reproduce**: found by reading the source against the pseudocode; not
yet run on QEMU.

    Arm:     EL1, stage 1 maps X0's page Normal Write-Back (MAIR_EL1
             attribute 0xFF) with SH = 0b01:
             at s1e1r, x0 (0xD5087800), then PAR_EL1.SH is 0b00, 0b10 or 0b11
    QEMU:    PAR_EL1.SH is 0b01

Found on QEMU 11.1.2; master not checked.

**Fix**: not yet written. `get_phys_addr_lpae()` would map 0b01 to one of
the permitted values. `rusty_box_aarch64` takes Outer Shareable
(`src/cpu/memattr.rs`, `Shareability::decode`).

### 29. `max` without EL2 advertises FEAT_ECV_POFF

**Impact**: very low. `-cpu max` with EL2 taken away (`-M virt`, whose
`virtualization` is off by default) reports `ID_AA64MMFR0_EL1.ECV` = 2,
FEAT_ECV_POFF, a feature only a processor with EL2 has. Its one register,
`CNTPOFF_EL2`, is an EL2 register, so software that believes the field
finds nothing to program; only software that checks the ID registers
against Arm's feature rules sees the contradiction.

**Location**: `target/arm/tcg/cpu64.c`, `aarch64_max_tcg_initfn()`, sets
`ID_AA64MMFR0.ECV` to 2; `target/arm/cpu.c`, `arm_cpu_realizefn()`, on
`!cpu->has_el2` unsets `ARM_FEATURE_EL2`, and its later
`!arm_feature(env, ARM_FEATURE_EL2)` block clears the hypervisor fields
of the ID registers (`ID_AA64PFR0.EL2`, `ID_PFR1.VIRTUALIZATION`) but not
`ID_AA64MMFR0.ECV`.

**What the Arm ARM says**: AARCHMRS `Features.json` constrains
`(FEAT_AA64 && FEAT_ECV_POFF) ==> FEAT_AA64EL2`, and `FEAT_ECV_POFF <=>
(UInt(ID_AA64MMFR0_EL1.ECV) >= 2)`: a processor without EL2 has
`ECV` at most 1.

**Reproduce**: probe case P (`-M virt -cpu max`, no EL2) prints
`ID_AA64MMFR0_EL1`:

    EL1:     mrs x5, id_aa64mmfr0_el1 (0xD538_0705)
    Arm:     P 1...............   (ECV, bits 63:60, at most 1)
    QEMU:    P 2100032310201126   (ECV 2)

Found on QEMU 11.1.2; master not checked.

**Fix**: not yet written. The `!arm_feature(env, ARM_FEATURE_EL2)` block
of `arm_cpu_realizefn()` would lower `ID_AA64MMFR0.ECV` to 1 where it is
above 1, beside the hypervisor fields it clears. `rusty_box_aarch64`
drops the feature from a processor without EL2 (`src/cpu/features.rs`,
`Implementation::with_levels`).

---

### 30. A 64 KiB descriptor's bits 15:12 are ignored under an output size below 52 bits

**Impact**: very low. With FEAT_LPA and the 64 KiB granule, a block,
page or table descriptor whose bits 15:12 are not zero translates where
they say nothing, when `TCR_ELx.{I}PS` or `VTCR_EL2.PS` gives fewer than
52 bits, instead of taking an Address size fault. Only software that
writes 52-bit addresses into its tables and then narrows the output size
sees it.

**Location**: `target/arm/ptw.c`, `get_phys_addr_lpae()`: `descaddr =
descriptor & descaddrmask` keeps bits 47 down to the granule; descriptor
bits 15:12 join it as `descaddr` bits 51:48 only `if (outputsize > 48)`,
and otherwise the out-of-range test (`descaddr >> outputsize`) sees an
address without them.

**What the Arm ARM says**: `AArch64_NextTableBase`,
`AArch64_S1LeafBase` and `AArch64_S2LeafBase` take descriptor bits 15:12
as address bits 51:48 for the 64 KiB granule whenever `AArch64_PAMax()
>= 52`, whatever the configured output size; `AArch64_S1Walk` and
`AArch64_S2Walk` then fault a table address (`AArch64_S1OAOutOfRange` on
the next `descaddress`) or an output address the output size cannot hold:
`Fault_AddressSize` at the level of the descriptor that holds it.

**Reproduce**: probe case Q (`-M virt -cpu max`): a level 2 block
descriptor of the 64 KiB granule with bits 15:12 = 0xa under `IPS` 48
bits, `AT S1E1R` of its address:

    EL1:     at s1e1r, x2; mrs x5, par_el1
    Arm:     Q 0000000000000805   (F, FST = level 2 Address size)
    QEMU:    Q ff00000040000b80   (translated, the bits dropped)

Found on QEMU 11.1.2; master not checked.

**Fix**: not yet written. `get_phys_addr_lpae()` would add descriptor bits
15:12 as address bits 51:48 for the 64 KiB granule whenever `arm_pamax()`
is 52, before the out-of-range test, so that test faults them under a
smaller `outputsize`. `rusty_box_aarch64` does (`src/cpu/regime.rs`,
`OaLayout::descriptor_address`).

---

### 31. A table base register's bits 5:2 are taken as BADDR[51:48] under IPS 0b111

**Impact**: very low. With FEAT_LPA, the 64 KiB granule and `TCR_ELx.IPS`
(or `PS`, `VTCR_EL2.PS`) of 0b111, 56 bits, which `PAMax` makes 52, the
walk starts at a table address whose bits 51:48 are the table base
register's bits 5:2, where Arm takes those bits as the base's own bits
5:2, below the alignment of any start table of 64 bytes or more. Software
that leaves bits 5:2 zero, as it must under 0b110 for a table in the
first 256 TiB, sees no difference.

**Location**: `target/arm/ptw.c`, `get_phys_addr_lpae()`: `descaddr |=
extract64(ttbr, 2, 4) << 48` `if (outputsize > 48)`, where `outputsize`
is the `PARange` bound of `PS`, 52 for both 0b110 and 0b111.

**What the Arm ARM says**: `AArch64_S1TTBaseAddress` and
`AArch64_S2TTBaseAddress` take bits 5:2 as base bits 51:48 when
`walkparams.ds == '1'` or when the granule is 64 KiB, `walkparams.ps ==
'110'` and FEAT_LPA is implemented (or the IMPLEMENTATION DEFINED "BADDR
expresses 52 bits for 64KB granule"); otherwise the base is
`ttbr[47:1]`, aligned down to the start table's size.

**Reproduce**: probe case R (`-M virt -cpu max`): `IPS` 0b111,
`TTBR1_EL1` bits 5:2 = 0xa, the table at its address, `AT S1E1R`:

    EL1:     at s1e1r, x2; mrs x5, par_el1
    Arm:     R ff00000040000b80   (translated)
    QEMU:    R 0000000096000156 ...   (Data Abort: a synchronous
             External abort on the level 2 walk, read at 0xa << 48 up)

Found on QEMU 11.1.2; master not checked.

**Fix**: not yet written. `get_phys_addr_lpae()` would take the bits as
`descaddr` bits 51:48 under `param.ds`, or for the 64 KiB granule when
`param.ps` is 6 and `PARange` is at least 6, rather than whenever
`outputsize > 48`. `rusty_box_aarch64` does (`src/cpu/regime.rs`,
`OaLayout::table_base`).

---

### 32. Under FEAT_LPA2's DS a descriptor's bits 9:8 are ignored under an output size below 52 bits

**Impact**: very low. With FEAT_LPA2 and `TCR_ELx.DS` or `VTCR_EL2.DS`
set (the 4 KiB and 16 KiB granules), a block, page or table descriptor
whose bits 9:8, address bits 51:50 under `DS`, are not zero translates
where they say nothing, when the output size is under 52 bits, instead of
taking an Address size fault. Entry 30's sibling for `DS`: only software
that writes 52-bit addresses into its tables and then narrows the output
size sees it.

**Location**: `target/arm/ptw.c`, `get_phys_addr_lpae()`: under
`param.ds`, `descaddrmask` keeps descriptor bits 49 down to the granule,
and bits 9:8 join `descaddr` as bits 51:50 only `if (outputsize > 48)`;
otherwise the out-of-range test (`descaddr >> outputsize`) sees an
address without them.

**What the Arm ARM says**: `AArch64_NextTableBase`,
`AArch64_S1LeafBase` and `AArch64_S2LeafBase` take descriptor bits 9:8
and 49:48 as address bits 51:48 whenever `walkparams.ds == '1'`,
whatever the configured output size; `AArch64_S1Walk` and
`AArch64_S2Walk` then fault a table address or an output address the
output size cannot hold: `Fault_AddressSize` at the level of the
descriptor that holds it.

**Reproduce**: probe case S (`-M virt -cpu max`): `TCR_EL1.DS`, the 4
KiB granule, `IPS` 48 bits, a level 1 block descriptor with bits 9:8 =
0b01, `AT S1E1R` of its address:

    EL1:     at s1e1r, x2; mrs x5, par_el1
    Arm:     S 0000000000000803   (F, FST = level 1 Address size)
    QEMU:    S ff00000040000b80   (translated, the bits dropped)

Found on QEMU 11.1.2; master not checked.

**Fix**: not yet written. `get_phys_addr_lpae()` would add descriptor bits
9:8 as address bits 51:50 under `param.ds` before the out-of-range test,
as entry 30's fix adds bits 15:12 for the 64 KiB granule.
`rusty_box_aarch64` does (`src/cpu/regime.rs`,
`OaLayout::descriptor_address`).

---

### 33. Under FEAT_LPA2's DS a table base register's bits 5:2 are not BADDR[51:48] under an output size below 52 bits

**Impact**: very low. With FEAT_LPA2 and `DS` set, a table base register
whose bits 5:2 are not zero, `BADDR[51:48]` under `DS`, starts the walk
at the base without them when the output size is under 52 bits, instead
of taking a level 0 Address size fault. Entry 31's opposite: there QEMU
takes the bits where Arm does not, here it drops them where Arm takes
them.

**Location**: `target/arm/ptw.c`, `get_phys_addr_lpae()`: `descaddr |=
extract64(ttbr, 2, 4) << 48` only `if (outputsize > 48)`; otherwise bits
5:2 stay in `descaddr` as its own bits 5:2, which `descaddr &=
~indexmask` clears for any start table of 64 bytes or more.

**What the Arm ARM says**: `AArch64_S1TTBaseAddress` and
`AArch64_S2TTBaseAddress` take bits 5:2 as base bits 51:48 whenever
`walkparams.ds == '1'`, whatever `walkparams.ps`; `AArch64_S1Walk` and
`AArch64_S2Walk` fault a base the output size cannot hold
(`AArch64_S1OAOutOfRange` on the start level's descriptor address) at
level 0.

**Reproduce**: probe case T (`-M virt -cpu max`): `TCR_EL1.DS`, the 4
KiB granule, `IPS` 48 bits, `TTBR1_EL1` bits 5:2 = 0x1, `AT S1E1R`:

    EL1:     at s1e1r, x2; mrs x5, par_el1
    Arm:     T 0000000000000801   (F, FST = level 0 Address size)
    QEMU:    T ff00000040000b80   (translated)

Found on QEMU 11.1.2; master not checked.

**Fix**: entry 31's: take the bits as `descaddr` bits 51:48 under
`param.ds`, or for the 64 KiB granule when `param.ps` is 6 and `PARange`
is at least 6, rather than whenever `outputsize > 48`.
`rusty_box_aarch64` does (`src/cpu/regime.rs`, `OaLayout::table_base`).

### 34. LDP and STP of words are one 8-byte access on every memory

**Impact**: small. A device that takes 8-byte accesses sees one where the
architecture makes two word accesses, in order. A device with only 4-byte
accesses is spared, as the memory core splits the access for it.

**Location**: `target/arm/tcg/translate-a64.c`, `trans_LDP()` and
`trans_STP()`, issue a pair of words as one `tcg_gen_qemu_ld_i64` or
`tcg_gen_qemu_st_i64`, whatever the memory type and with or without
FEAT_LSE2; their comment says one operation is issued in all cases.

**What the Arm ARM says**: `ldp_gen.xml` and `stp_gen.xml` make one
`Mem{2*datasize}` with `ispair`, and `AArch64_MemSingleRead` and
`AArch64_MemSingleWrite` make it one access only with FEAT_LSE2, in one
16-byte granule, on Normal Write-Back Shareable memory (`IsWBShareable`).
On anything else it is two accesses of the elements (Arm ARM B2.8.2.1.1).

**Reproduce**: found by reading the source against the pseudocode; not
yet run on QEMU.

    Arm:     MMU off, ldp w0, w1, [x2], x2 at a device: two word reads
    QEMU:    one 8-byte read, split only if the device's ops require it

Found on QEMU 11.1.2.

**Fix**: not yet written. The memory type is known only at the TLB
lookup, so the split belongs in the access helpers rather than the
translator. `rusty_box_aarch64` decides it there (`src/cpu/mmu.rs`,
`Cpu::pair_is_one_access`).

### 35. AT does not report the effect of SCTLR_EL1.C

**Impact**: small. With stage 1 enabled and the data cache disabled, `AT`
reports the page's Write-Back attributes, where the access would be
Non-cacheable. Software that reads `PAR_EL1` with caches off, such as a
page table check in early boot, is told of attributes that do not apply.

**Location**: `target/arm/ptw.c`, `get_phys_addr_lpae()`, reads the
attribute from `MAIR_ELx` with no reference to `SCTLR_ELx.C`, and
`target/arm/tcg/cpregs-at.c`, `do_ats_write()`, copies it to `PAR_EL1`.

**What the Arm ARM says**: D7.5.5, "The effect of SCTLR_ELx.C,
HCR_EL2.DC and HCR_EL2.CD is reflected in the result of the address
translation instructions in the PAR when these bits have an effect on the
stages of translation being reported in the PAR", and rule RLRYRY, which
returns "the resulting attributes" in `PAR_EL1`;
`AArch64_S1TreatAsNormalNC` makes the memory Normal Non-cacheable and
`EncodePARAttrs` encodes it. The comment on the IMPLEMENTATION DEFINED
`ReportedPARAttrs` words this less firmly, letting the field be the
resulting attribute "instead of the value that appears in the translation
table descriptor"; D7.5.5 is the rule for these three controls.

**Reproduce**: found by reading the source against the pseudocode; not
yet run on QEMU.

    Arm:     M = 1, C = 0, MAIR 0xFF, SH = 0b11: AT S1E1R gives ATTR Normal
             Non-cacheable, SH 0b10
    QEMU:    ATTR 0xFF, SH 0b11

Found on QEMU 11.1.2.

**Fix**: not yet written. `rusty_box_aarch64` reports the effect
(`src/cpu/mmu.rs`, `Cpu::address_translate`).

### 36. The virtio input devices count a NUL in their name's size

**Impact**: small. A driver that takes `size` bytes as the name reads a
NUL as its last character. Linux's `virtinput_probe` copies the bytes into
a zeroed buffer, so the NUL is harmless there; a driver that stores the
name with its length, as the specification allows, keeps a NUL inside it.

**Location**: `hw/input/virtio-input-hid.c`, the `virtio_keyboard_config`,
`virtio_mouse_config`, `virtio_tablet_config` and
`virtio_multitouch_config` tables, whose `VIRTIO_INPUT_CFG_ID_NAME`
entries set `.size = sizeof(VIRTIO_ID_NAME_...)`, the string literal's
length with its NUL.

**What the specification says**: virtio 1.2 (OASIS, csd01), 5.8.4:
"Strings do not include a NUL terminator", and `size` is how much
information is available. QEMU's own `virtio_input_idstr_config`, which
sets the serial, gives the length without the NUL (`snprintf`'s return).

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    virtio 1.2:  ID_NAME of "QEMU Virtio Keyboard": size 20
    QEMU:        size 21, the last byte 0

Found on QEMU 11.1.2; master (2026-10-31) has the same tables.

**Fix**: not yet written: each table's `.size` would be
`sizeof(VIRTIO_ID_NAME_...) - 1`. `rusty_box_aarch64` sizes a name to its
length (`src/devices/virtio/input.rs`, `ConfigEntry::name`).

### 37. The GICv2's virtual interface control registers keep one List register more than `GICH_VTR` reports

**Impact**: small. A hypervisor that writes the List register past the
last one `GICH_VTR.ListRegs` reports reads its value back, as though it
existed, though the GIC never signals what it holds.

**Location**: `hw/intc/arm_gic.c`, `gic_hyp_read` and `gic_hyp_write`,
whose `A_GICH_LR0 ... A_GICH_LR63` case tests `lr_idx > s->num_lrs`.
With `num_lrs` 4, `GICH_LR4` passes and reads and writes `h_lr[4]`, an
element of the `GIC_MAX_LR`-long array that `gic_get_best_virq` and
`gic_extract_lr_info`, which loop to `lr_idx < s->num_lrs`, never look at.

**What the specification says**: IHI 0048B.b §5.3.2: `GICH_VTR.ListRegs`
is "the number of implemented List registers, minus one"; Table 5-1 gives
`GICH_LRn` for the implemented ones only, and a register that is not
implemented is RAZ/WI there.

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    GICH_VTR.ListRegs 3; write 0x10000028 to GICH_LR4, read it back
    IHI 0048B: 0
    QEMU:      0x10000028

Found on QEMU 11.1.2; master (2026-11-04) has the same test.

**Fix**: not yet written: the test would be `lr_idx >= s->num_lrs`.
`rusty_box_aarch64` reads `GICH_LR4` and up as zero and ignores writes
(`src/devices/gicv2_virt.rs`, `VirtInterface::hyp_read`).

### 38. A Group 1 interrupt that cannot be signalled reads as 1022 at `GICC_IAR` and `GICV_IAR`

**Impact**: small. Software that reads 1022 takes it that a Group 1
interrupt is pending for it to acknowledge through the alias register,
where none it could be signalled is.

**Location**: `hw/intc/arm_gic.c`, `gic_acknowledge_irq`, which returns
`gic_get_current_pending_irq`'s 1022 for a Group 1 interrupt while
`AckCtl` is clear before it compares the interrupt's group priority with
the running priority, and without looking at `GICC_CTLR.EnableGrp1`
(`gic_update_internal` sets `current_pending` whatever the CPU interface
enables).

**What the specification says**: IHI 0048B.b Table 4-35 (`GICC_IAR`) and
Table 5-12 (`GICV_IAR`): the rows giving 1022 are for a highest-priority
pending interrupt "of sufficient priority to be signaled" (note a);
without one the read is 1023. §4.4.4 `ReadGICC_IAR`: a Group 1
interrupt while `GICC_CTLR.EnableGrp1` is 0 gives 1023 ("If the highest
priority isn't enabled, then no interrupt").

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    a Group 0 interrupt at priority 0x10 active, a Group 1 one at 0x20
    pending, AckCtl 0; read GICC_IAR
    IHI 0048B: 1023
    QEMU:      1022

    GICC_CTLR = EnableGrp0 only; Group 1 SPI 41 at 0x10 and Group 0 SPI
    40 at 0x20 pending; read GICC_IAR
    IHI 0048B: 1023
    QEMU:      1022

Found on QEMU 11.1.2; master (2026-11-04) has the same `gic_acknowledge_irq`.

**Fix**: not yet written: `gic_acknowledge_irq` would compare the
priority before mapping the ID. `rusty_box_aarch64` does
(`src/devices/gicv2.rs` and `src/devices/gicv2_virt.rs`, `acknowledge`).

### 39. The GICv2's virtual CPU interface has no Group 1 aliases, and its `GICV_HPPIR` omits a software interrupt's CPUID

**Impact**: small. A guest that completes Group 1 interrupts through
`GICV_AIAR` and `GICV_AEOIR`, as IHI 0048B §5.5.9 and §5.5.10 describe,
reads 0 and its EOI is lost; one that reads `GICV_HPPIR` for a virtual SGI
does not learn its source.

**Location**: `hw/intc/arm_gic.c`, `gic_cpu_read` and `gic_cpu_write`,
which serve a vCPU and have no case for offsets 0x20, 0x24 and 0x28 (a
guest error, read as 0); and `gic_get_current_pending_irq`, which returns
`current_pending`, a List register's `VirtualID` alone, for a vCPU.

**What the specification says**: IHI 0048B.b Table 5-10 has `GICV_AIAR`,
`GICV_AEOIR` and `GICV_AHPPIR` at 0x20, 0x24 and 0x28; §5.5.7 gives
`GICV_HPPIR` `GICC_HPPIR`'s bits, `CPUID` in [12:10], which §5.5.4 fills
from a software List register's bits [12:10].

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    GICH_LR0 = pending, Group 1, CPUID 5, VirtualID 3; read GICV_AHPPIR, GICV_HPPIR (AckCtl 1)
    IHI 0048B: 0x1403, 0x1403
    QEMU:      0 (guest error), 3

Found on QEMU 11.1.2; master (2026-11-04) has the same `gic_cpu_read` and
`gic_get_current_pending_irq`.

**Fix**: not yet written. `rusty_box_aarch64` implements the aliases and
reports `CPUID` (`src/devices/gicv2_virt.rs`, `VirtInterface::vcpu_read`).

### 40. The GICv2 preempts by an interrupt's whole priority, not its group priority

**Impact**: small. Only where the two groups' binary points differ
(`GICC_BPR` against `GICC_ABPR` with `CBPR` clear), and the running
priority has bits below the pending interrupt's binary point: an
interrupt of the same group priority as the active one under its own
group's point is signalled and acknowledged as though it preempted,
nesting interrupts the binary point keeps apart.

**Location**: `hw/intc/arm_gic.c`, `gic_update_internal` (`best_prio <
s->running_priority`) and `gic_acknowledge_irq` (`gic_get_priority(...) >=
s->running_priority`), for the physical and the virtual CPU interface
alike.

**What the specification says**: IHI 0048B.b §3.3.3: "the value of the
group priority field for the new interrupt must be less than the value of
the group priority field of the Running priority", both fields under the
new interrupt's group's binary point; §3.7.2 `UpdateExceptionState`:
`PriorityIsHigher(ReadGICD_IPRIORITYR(intID), GICC_RPR[cpu_id]<7:0> AND
smsk)`, with `nsmsk` for Group 1; §5.5.3 for `GICV_BPR`.

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    GICC_BPR 2, GICC_ABPR 1, CBPR 0; Group 1 interrupt A at 0x42 active
    (running priority 0x42 under ABPR - 1 = 0); Group 0 interrupt B at
    0x41 pending (mask 0xF8 under BPR 2)
    IHI 0048B: 0x41 < (0x42 AND 0xF8 = 0x40) is false: not signalled,
               GICC_IAR returns 1023
    QEMU:      0x41 < 0x42: signalled, GICC_IAR returns B

Found on QEMU 11.1.2; master (2026-11-04) has the same `gic_update_internal`
and `gic_acknowledge_irq`.

**Fix**: not yet written: both comparisons would mask the running
priority with the pending interrupt's group mask, as the pseudocode does.
`rusty_box_aarch64` does (`src/devices/gicv2.rs`, `preempts`, for both CPU
interfaces).

### 41. The GICv2's virtual CPU interface lets `GICV_ABPR` go below its minimum

**Impact**: small. A guest that writes `GICV_ABPR` with 1 or 2 reads that
value back, and its Group 1 interrupts are then grouped by priority under
a binary point the virtual interface does not have, splitting preemption
levels the hypervisor's view (`GICH_VMCR.VMABP`, `GICH_APR`) cannot hold.

**Location**: `hw/intc/arm_gic.c`, `gic_cpu_write`, case 0x1c, which
writes `s->abpr[cpu] = MAX(value & 0x7, GIC_MIN_ABPR)` for a vCPU as for
the physical interface; `gic_vmcr_write` raises the same field to
`GIC_VIRT_MIN_ABPR`.

**What the specification says**: IHI 0048B.b §5.5.3: the virtual
interface's minimum binary point "is determined by the value of
`GICH_VTR.PREbits`", 2 for five preemption bits; §3.5.3: `GICC_ABPR`'s
minimum is "one greater than the minimum value supported for" `GICC_BPR`,
so 3 for `GICV_ABPR`.

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    GICH_VTR.PREbits 4 (five bits); write 1 to GICV_ABPR, read it back
    IHI 0048B: 3
    QEMU:      1

Found on QEMU 11.1.2; master (2026-11-05) has the same `gic_cpu_write`.

**Fix**: not yet written: case 0x1c would use `gic_is_vcpu(cpu) ?
GIC_VIRT_MIN_ABPR : GIC_MIN_ABPR`, as case 0x08 does for the binary
point. `rusty_box_aarch64` does (`src/devices/gicv2_virt.rs`,
`VirtInterface::vcpu_write`).

### 42. virtio-mmio offers `VIRTIO_F_RING_RESET` and has no `QueueReset` register

**Impact**: small today, as Linux's `virtio_mmio` does not reset single
queues. A driver that does writes 1 to `QueueReset`, reads back 0 and
takes the queue as reset; the device keeps its indices and `QueueReady`
and goes on running the queue from memory the driver may now reuse.

**Location**: `hw/virtio/virtio-mmio.c`, `virtio_mmio_read` and
`virtio_mmio_write`, which have no case for offset 0x0C0 (a guest error,
read as 0); `include/hw/virtio/virtio.h`, `DEFINE_VIRTIO_COMMON_FEATURES`,
which offers `queue_reset` (`VIRTIO_F_RING_RESET`) on every transport.

**What the specification says**: virtio 1.2 Table 4.1, `QueueReset`
(0x0C0); 4.2.2.1: "The device MUST reset the queue when 1 is written to
QueueReset. The device MUST continue to present 1 in QueueReset as long
as the queue reset is ongoing. The device MUST present 0 in both
QueueReset and QueueReady when queue reset has completed"; 2.6.1.1.1:
"After the queue has been reset by the driver, the device MUST NOT
execute any requests from that virtqueue".

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    accept VIRTIO_F_RING_RESET; QueueSel 1, QueueReset 1; read QueueReady
    virtio 1.2: 0, and the queue's state reset
    QEMU:       1, the queue running

Found on QEMU 11.1.2; master (2026-11-05) has the same `virtio_mmio_write`.

**Fix**: not yet written: either a `VIRTIO_MMIO_QUEUE_RESET` case calling
`virtio_queue_reset` and clearing the queue's latched registers, as
`virtio-pci`'s `VIRTIO_PCI_COMMON_Q_RESET` does, or virtio-mmio clearing
`VIRTIO_F_RING_RESET`. `rusty_box_aarch64` implements the register
(`src/devices/virtio/mod.rs`, `Transport::reset_queue`).

### 43. A virtio configuration change sets the used-buffer bit of `InterruptStatus` too

**Impact**: small. On virtio-mmio a driver that reads `InterruptStatus`
after a configuration change finds bit 0 set too, and looks through its
queues for used buffers there are none of.

**Location**: `hw/virtio/virtio.c`, `virtio_notify_config`, which calls
`virtio_set_isr(vdev, 0x3)`.

**What the specification says**: virtio 1.2 Table 4.1, `InterruptStatus`:
bit 0 is "Used Buffer Notification", bit 1 "Configuration Change
Notification"; 4.2.2.1: "The device MUST present each event by setting
the corresponding bit in InterruptStatus from the moment it takes place,
until the driver acknowledges the interrupt by writing a corresponding
bit mask to the InterruptACK register. Bits which do not represent
events which took place MUST be zero."

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    virtio-gpu, a display resize from the host; read InterruptStatus
    virtio 1.2: 2
    QEMU:       3

Found on QEMU 11.1.2; master (2026-11-05) has the same `virtio_notify_config`.

**Fix**: not yet written: `virtio_set_isr(vdev, 0x2)`; virtio-pci's ISR
reads would need checking against legacy drivers that expect bit 0.
`rusty_box_aarch64` sets bit 1 alone (`src/devices/virtio/mod.rs`,
`Core::notify_config`).

### 44. A virtio device keeps using a queue whose `QueueReady` is 0, and uses queues before `DRIVER_OK`

**Impact**: a driver that stops a queue by writing 0 to `QueueReady` and
frees its rings has the device go on reading the old available ring and
writing the old used ring: virtio-input, for one, writes the next host
event into the freed memory.

**Location**: `hw/virtio/virtio-mmio.c`, `virtio_mmio_write`
(`VIRTIO_MMIO_QUEUE_READY` with 0 only clears `proxy->vqs[].enabled`),
and `hw/virtio/virtio.c`, `virtio_queue_notify` and `virtqueue_pop`, which
look at neither that bit nor the device status.

**What the specification says**: virtio 1.2 4.2.2.1: "The device MUST NOT
access virtual queue contents when QueueReady is zero (0x0)"; 2.1.2: "The
device MUST NOT consume buffers or send any used buffer notifications to
the driver before DRIVER_OK".

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    virtio-keyboard running; QueueSel 0, QueueReady 0; a host key press
    virtio 1.2: nothing written to guest memory
    QEMU:       the event written through the old rings

Found on QEMU 11.1.2; master (2026-11-05) has the same code.

**Fix**: not yet written: on `QueueReady` 0, a disabled flag that
`virtqueue_pop` and `virtio_queue_notify` check, and the status checked
there for `DRIVER_OK`. `rusty_box_aarch64` gates every queue access on both
(`src/devices/virtio/mod.rs`, `Vdev::usable`).

### 45. virtio-mmio's selectors and its `SHMBase` differ from Table 4.1

**Impact**: small: none bites Linux. A driver that writes
`DriverFeaturesSel` 2 and then `DriverFeatures` 0, clearing the third
word of features it accepts, clears the second instead, losing
`VIRTIO_F_VERSION_1`; one that probes with `QueueSel` past 1023 reads the
previous queue's `QueueNumMax`, and its next queue writes change that
queue; one that reads `SHMBase` for a region that does not exist reads 0.

**Location**: `hw/virtio/virtio-mmio.c`, `virtio_mmio_write`
(`VIRTIO_MMIO_DEVICE_FEATURES_SEL` and `VIRTIO_MMIO_DRIVER_FEATURES_SEL`
store `value ? 1 : 0`; `VIRTIO_MMIO_QUEUE_SEL` ignores a value of
`VIRTIO_QUEUE_MAX` or more) and `virtio_mmio_read` (no case for
`VIRTIO_MMIO_SHM_BASE_LOW` and `_HIGH`).

**What the specification says**: virtio 1.2 Table 4.1: `DeviceFeatures`
gives "bits DeviceFeaturesSel∗32 to (DeviceFeaturesSel∗32)+31", and
`DriverFeatures` likewise; `QueueNumMax` reads "zero (0x0) if the queue is
not available"; `SHMBase`: "Reading from a non-existent region (i.e.
where the ID written to SHMSel is unused) results in a base address of
0xffffffffffffffff".

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    DeviceFeaturesSel 2; read DeviceFeatures      virtio 1.2: 0   QEMU: bits 32-63
    QueueSel 0, then 0xFFFF; read QueueNumMax      virtio 1.2: 0   QEMU: queue 0's
    read SHMBaseLow                                virtio 1.2: 0xFFFFFFFF   QEMU: 0

Found on QEMU 11.1.2; master (2026-11-05) has the same code.

**Fix**: not yet written. `rusty_box_aarch64` keeps each selector whole and
reads `SHMBase` as all ones (`src/devices/virtio/mod.rs`).

### 46. virtio sets `FEATURES_OK` for a driver that accepts features the device did not offer

**Impact**: small: a driver that accepts an unoffered feature, a bug of
its own, is not told; it then runs believing it negotiated a packed ring
or another feature the device never gave.

**Location**: `hw/virtio/virtio.c`, `virtio_set_features_ex`, which masks
the driver's features with the device's and returns success, and
`virtio_mmio_write` (`VIRTIO_MMIO_STATUS`), which stores the status
whatever it returned.

**What the specification says**: virtio 1.2 2.2.2: "The device SHOULD
accept any valid subset of features the driver accepts, otherwise it MUST
fail to set the FEATURES_OK device status bit when the driver writes
it"; 3.1.1: the driver re-reads the status "to ensure the FEATURES_OK bit
is still set: otherwise, the device does not support our subset of
features".

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    accept VIRTIO_F_RING_PACKED (not offered); Status |= FEATURES_OK; read Status
    virtio 1.2: FEATURES_OK clear
    QEMU:       FEATURES_OK set

Found on QEMU 11.1.2; master (2026-11-05) has the same code.

**Fix**: not yet written: `virtio_set_features_ex` would fail for a bit
outside `host_features`, and the transports would then not store
`FEATURES_OK`, as they already do when it fails for other reasons.
`rusty_box_aarch64` refuses the bit (`src/devices/virtio/mod.rs`,
`Transport::write_status`).

### 47. The split virtqueue takes an indirect table only at the head of a chain

**Impact**: small: Linux never builds such a chain. A driver that chains
a block request's header to an indirect table holding its data and
status has the table's descriptors read as data and, when the
descriptor naming the table has `WRITE` set, the status byte written into
the table itself.

**Location**: `hw/virtio/virtio.c`, `virtqueue_split_pop`, which tests
`VRING_DESC_F_INDIRECT` on the head descriptor only and maps every later
descriptor through `virtqueue_map_desc` as a buffer.

**What the specification says**: virtio 1.2 2.7.5.3.2: "The device MUST
handle the case of zero or more normal chained descriptors followed by a
single descriptor with flags&VIRTQ_DESC_F_INDIRECT"; 2.7.5.1: "A device
MUST NOT write to a device-readable buffer", and the table is the
driver's.

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    virtio-blk: header (readable) NEXT-> descriptor INDIRECT naming
    [data (writable, 512), status (writable, 1)]; a read
    virtio 1.2: the data read into the table's buffers, the status OK
    QEMU:       "virtio-blk missing headers" or the status in the table

Found on QEMU 11.1.2; master (2026-11-05) has the same `virtqueue_split_pop`.

**Fix**: not yet written: move the indirect test into the loop over the
chain, allowed once and only outside an indirect table. `rusty_box_aarch64`
does (`src/devices/virtio/queue.rs`, `Queue::pop`).

### 48. virtio-blk and virtio-input give back a used length larger than what they wrote

**Impact**: small: Linux's drivers ignore these lengths. A driver that
trusts the used length, as 2.7.8 describes for untrusted buffers, takes a
failed read's buffer, or a 20-byte `GET_ID` buffer of which QEMU wrote
one byte, as written, and hands its stale contents on.

**Location**: `hw/block/virtio-blk.c`, `virtio_blk_req_complete`, which
pushes `req->in_len`, every device-writable byte; `hw/input/virtio-input.c`,
`virtio_input_handle_sts`, which pushes the bytes it read from a
device-readable buffer.

**What the specification says**: virtio 1.2 2.7.8.2: "The device MUST
write at least len bytes to descriptor, beginning at the first
device-writable buffer, prior to updating the used idx"; `len` is "The
number of bytes written into the device writable portion of the buffer".

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    virtio-blk read past the disk, 1024-byte data buffer: used len 1025
      virtio 1.2: at most the bytes written from the first writable one
      QEMU:       1025, of which it wrote only the status byte
    virtio-input status report of 8 bytes: used len
      virtio 1.2: 0                                          QEMU: 8

Found on QEMU 11.1.2; master (2026-11-05) has the same code.

**Fix**: not yet written. For virtio-blk the used length cannot simply
shrink to what was written: the status byte is the chain's last, and
2.7.8.3 has the driver not look past the used length, so a length short
of it hides the status. Zeroing the bytes a request does not write keeps
the length whole and true, which `rusty_box_aarch64` does
(`src/devices/virtio/block.rs`, `Block::complete`); virtio-input's status
queue would push 0 (`src/devices/virtio/input.rs`, `Input::handle_output`).

### 49. virtio-blk breaks three device requirements of section 5.2

**Impact**: small. A driver that accepts `VIRTIO_BLK_F_CONFIG_WCE` without
`VIRTIO_BLK_F_FLUSH` reads `writeback` 1 and gets a write-back cache it
cannot flush; one that reads the configuration's `unused1` padding reads
all ones; a discard with `unmap` set over sectors past the disk is told
`VIRTIO_BLK_S_IOERR`, not `VIRTIO_BLK_S_UNSUPP`.

**Location**: `hw/block/virtio-blk.c`: `virtio_blk_set_status` (the cache
mode set at `DRIVER_OK`, only without `VIRTIO_BLK_F_CONFIG_WCE`);
`virtio_blk_cfg_size_params` (the configuration ends after
`write_zeroes_may_unmap`, so the transport reads `unused1` as all ones);
`virtio_blk_handle_discard_write_zeroes` (`virtio_blk_sect_range_ok`
before the flag checks).

**What the specification says**: virtio 1.2 5.2.5.2: "If
VIRTIO_BLK_F_CONFIG_WCE is negotiated but VIRTIO_BLK_F_FLUSH is not, the
device MUST initialize writeback to 0", and "The device MUST initialize
padding bytes unused0 and unused1 to 0"; 5.2.6.2: "The device MUST set
the status byte to VIRTIO_BLK_S_UNSUPP for discard, secure erase and
write zeroes commands if any unknown flag is set. Furthermore, the device
MUST set the status byte to VIRTIO_BLK_S_UNSUPP for discard commands if
the unmap flag is set."

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    accept CONFIG_WCE, not FLUSH; FEATURES_OK; read writeback   virtio 1.2: 0     QEMU: 1
    read config byte 57 (unused1)                               virtio 1.2: 0     QEMU: 0xFF
    discard, unmap set, sector past the disk                    virtio 1.2: UNSUPP  QEMU: IOERR

Found on QEMU 11.1.2; master (2026-11-05) has the same code.

**Fix**: not yet written. `rusty_box_aarch64` sets the cache mode at
`FEATURES_OK`, ends its configuration after `unused1`, and checks the
flags first (`src/devices/virtio/block.rs`).

### 50. virtio-gpu takes a 2D transfer's offset modulo 2^32

**Impact**: small: a guest's backing is rarely 4 GiB long. A transfer
whose offset is 4 GiB or more into a resource's backing copies from the
offset's low 32 bits instead, pixels from the wrong place.

**Location**: `hw/display/virtio-gpu.c`, `virtio_gpu_transfer_to_host_2d`,
whose `uint32_t src_offset` takes `t2d.offset`, the whole of it on a
full-width transfer and `t2d.offset + stride * h` for each row of a
partial one.

**What the specification says**: virtio 1.2 5.7.6.8,
`struct virtio_gpu_transfer_to_host_2d`: `le64 offset`.

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    a 32x16 resource backed by one buffer; TRANSFER_TO_HOST_2D, offset
    (1 << 32) + 272, rectangle (4, 2, 8, 4)
    virtio 1.2: nothing copied, the offset lies past the backing
    QEMU:       the rectangle copied from offset 272

Found on QEMU 11.1.2; master (2026-11-05) has the same
`virtio_gpu_transfer_to_host_2d`.

**Fix**: not yet written: `src_offset` as `uint64_t` (and `iov_to_buf`'s
offset is a `size_t`). `rusty_box_aarch64` keeps the 64 bits
(`src/devices/virtio/gpu.rs`, `Gpu::transfer`).

### 51. virtio-gpu answers no cursor-queue command

**Impact**: small: Linux's driver gives its cursor commands no response
buffer, and sends a move with the resource it last set. A driver that
gives a response buffer, and waits for `VIRTIO_GPU_RESP_OK_NODATA` as
5.7.6.10 gives it, finds it untouched and the used length 0; a command of
another type on the cursor queue is taken as a cursor update; a move
whose `resource_id` is 0, a field the device is to ignore, hides the
cursor.

**Location**: `hw/display/virtio-gpu.c`, `virtio_gpu_handle_cursor`, which
pushes every chain with length 0, and `update_cursor`, which takes any
type but `VIRTIO_GPU_CMD_MOVE_CURSOR` as `VIRTIO_GPU_CMD_UPDATE_CURSOR`
and passes `qemu_console_set_mouse` the command's `resource_id` as the
cursor's visibility on a move too.

**What the specification says**: virtio 1.2 5.7.6.10:
`VIRTIO_GPU_CMD_UPDATE_CURSOR` and `VIRTIO_GPU_CMD_MOVE_CURSOR`:
"Response type is VIRTIO_GPU_RESP_OK_NODATA"; of the move: "Move cursor
to the place specified in pos. The other fields are not used and will be
ignored by the device"; 5.7.6.7, the response header and the error
types.

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    cursor queue: MOVE_CURSOR (56 bytes) with a 24-byte response buffer
    virtio 1.2: response OK_NODATA, used length 24
    QEMU:       response untouched, used length 0

Found on QEMU 11.1.2; master (2026-11-05) has the same
`virtio_gpu_handle_cursor`.

**Fix**: not yet written: `virtio_gpu_handle_cursor` would fill the
response header as `virtio_gpu_ctrl_response_nodata` does on the control
queue. `rusty_box_aarch64` answers each command (`src/devices/virtio/gpu.rs`,
`Gpu::handle_cursor`).

### 52. virtio-input sets the used-buffer bit for a status-queue notification that used no buffer

**Impact**: small. A driver that notifies the status queue with no report
available, or notifies a broken device, finds bit 0 of `InterruptStatus`
set and looks through its queues for used buffers there are none of; a
working device also raises its interrupt for it.

**Location**: `hw/input/virtio-input.c`, `virtio_input_handle_sts`, which
calls `virtio_notify` after its pop loop whether or not it took a
report; `hw/virtio/virtio.c`, `virtio_irq`, which calls
`virtio_set_isr(vdev, 0x1)` before `virtio_notify_vector` returns early
for a broken device (`virtio_device_disabled`).

**What the specification says**: virtio 1.2 4.2.2.1: "The device MUST
present each event by setting the corresponding bit in InterruptStatus
from the moment it takes place ... Bits which do not represent events
which took place MUST be zero"; Table 4.1, bit 0 is "Used Buffer
Notification".

**Reproduce**: found by reading the source against the specification; not
yet run on QEMU.

    virtio-keyboard, no event index, nothing on the status queue;
    QueueNotify 1, then read InterruptStatus
    virtio 1.2: 0
    QEMU:       1

Found on QEMU 11.1.2; master (2026-11-05) has the same
`virtio_input_handle_sts` and `virtio_irq`.

**Fix**: not yet written: `virtio_input_handle_sts` would notify only when
it pushed a report, and `virtio_irq` would leave the ISR alone for a
disabled device. `rusty_box_aarch64` notifies only after taking a report
(`src/devices/virtio/input.rs`, `Input::handle_output`) and a broken
device uses no buffer (`src/devices/virtio/mod.rs`, `Vdev::push`).

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
- **RMR_ELx ignore writes.** `define_arm_cp_regs` in
  `target/arm/helper.c` makes RMR_EL1 (with neither EL2 nor EL3), RMR_EL2
  and RMR_EL3 `ARM_CP_CONST` ("the bit is strictly a request, so we are in
  spec just ignoring writes"). Neither acts on the warm reset RR requests,
  which the system may ignore, but Arm's register data makes RR an
  ordinary read/write field: after `MSR RMR_EL2, X0` with X0 = 2, an
  `MRS` reads 3 on Arm and 1 on QEMU.
- **Should-be fields.** QEMU executes the exclusive loads and stores
  (`STXR`, `LDXR`, `STXP`, `LDXP`) with `Rs` or `Rt2` other than their
  should-be values, and FCMP/FCMPE with zero whatever `Rm` holds. Such a
  word is CONSTRAINED UNPREDICTABLE, and executing it as if the field held
  its should-be value is among the permitted behaviours.
- **The PL031 always counts.** In `hw/rtc/pl031.c`, `pl031_read()` returns
  1 for RTCCR ("RTC is permanently enabled") and `pl031_write()` ignores a
  write to it ("Written value is ignored"). The device has no reset.
  DDI 0224C says the RTC counts only once a 1 is written to RTCCR, and
  that a later write resets its value to zero (§2.2.4, Table 3-5, for
  every revision per Table B-2). A system reset clears RTCCR (§2.3.1).

  A patch has a Linux side. `pl031_probe()` in
  `drivers/rtc/rtc-pl031.c` sets `RTC_CR_EN` with a write on every boot.
  On a PL031 that behaves as DDI 0224C says, Linux therefore zeroes an RTC
  that firmware has already started. EDK2's `InitializePL031()` writes
  RTCCR only while it reads 0. Linux would do the same.

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
