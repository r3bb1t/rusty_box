# Upstream issue — FILED 2026-07-13 as https://github.com/bochs-emu/Bochs/issues/791
# Fix PR — FILED 2026-07-13 as https://github.com/bochs-emu/Bochs/pull/792
# (branch fix/cpuid-freq-leaves on the r3bb1t/Bochs fork, commit 55d154e0f)

The issue was created from the body below (everything after the `---`), with
the full captured kernel log inlined as a collapsed section. This file stays
as the local source of record.

**Fix prototype (2026-07-13, not yet offered upstream):** implemented and
verified as `docs/bochs-cpuid-freq-fix.patch` (98 insertions, 11 files) on
branch `fix/cpuid-freq-leaves` of the cpp_orig/bochs checkout. Adds
`cpu: cpuid_freq=hardware|none|ips` (default `hardware` = today's behavior)
via `bx_cpuid_t::get_freq_leaf_15/16`, routed through all six affected
models, documented in user.dbk. Verified with Ubuntu 26.04 / kernel 7.0
boots at ips=4000000 (logs in docs/bochs-cpuid-freq-{none,ips}-dmesg.log):

| mode               | observed dmesg                                                          |
|--------------------|-------------------------------------------------------------------------|
| default (hardware) | `Detected 3500.000 MHz processor` + `3499.912 MHz TSC`, lpj=3499912 — identical to stock (no regression) |
| none               | `Fast TSC calibration using PIT` + `Detected 3.999 MHz processor`, lpj=3999 — true rate measured |
| ips                | `Detected 4.000 MHz processor`, lpj=4000, no PIT needed — true rate via CPUID |

Notes from filing (NOT part of the issue text):

- **Verification status (2026-07-13):** every claim below was re-verified against
  primary sources. Bochs side: byte-checked against master commit `70da922c`
  (2026-07-12) — both `get_TSC()` and the skylake-x leaf values are unchanged
  from the values quoted. Kernel side: all mechanisms and version thresholds
  verified by grepping *tagged* mainline sources (v4.7–v5.3 and current
  master `arch/x86/kernel/tsc.c`), not from memory.
- **No duplicate exists.** Searched bochs-emu/Bochs issues+PRs (open and
  closed, 17 term sets), the legacy SourceForge tracker, and the web. Closest
  cousin is #690 (Windows ACPI clock drift — different mechanism).
- **The bug is now reproduced live on a stock build (2026-07-13).** Upstream
  master (`70da922c`) was built unmodified with MinGW gcc 15.1 (recipe of
  `.conf.win64-cross-mingw32`, native) and booted Ubuntu 26.04 live-server
  (kernel 7.0.0-14-generic) with the exact repro config below plus
  `console=ttyS0,115200` added in the GRUB editor. The full captured kernel
  serial log is at `docs/bochs-tsc-cpuid-repro-dmesg.log` (438 lines) —
  attach it to the issue. Every quoted "observed" line below is copied
  verbatim from that log. (The only media tweak: the ISO's grub.cfg
  `set timeout=30` was byte-patched to `timeout=-1` so the menu could be
  edited — irrelevant to the bug.)
- Tracker conventions: no issue templates; CPU issues are triaged tersely and
  technically (see #567, #668). Keep it code-anchored, no fluff.

---

**Title:** Post-Skylake CPU models declare a TSC frequency via CPUID leaves
0x15/0x16 that the emulated TSC does not run at — Linux ≥ 4.8 guests trust it
and TSC-based guest time runs ~875× slow

## Summary

The six CPU models added since 2017 (`corei7_skylake_x`, `corei3_cnl`,
`corei7_icelake_u`, `tigerlake`, `sapphire_rapids`, `arrow_lake`) copy CPUID
leaves 0x15 (TSC / core crystal clock) and 0x16 (processor frequency)
byte-for-byte from real-hardware dumps. Leaf 0x15 is not informational: the
SDM (Vol. 2A, CPUID, Table 3-8, leaf 15H) architecturally *defines*
TSC frequency = core crystal clock × EBX/EAX. For `corei7_skylake_x` the
leaves pin the TSC at ~3.5 GHz, but Bochs's TSC is the raw instruction-tick
counter and advances at the configured `ips` rate — 4 MIPS by default
(config.cc), 12–95 MIPS in the user guide's own example table.

Since Linux v4.8 the kernel trusts these leaves and **never calibrates against
the PIT** (commit `aa297292d708` "x86/tsc: Enumerate SKL cpu_khz and tsc_khz
via CPUID": `cpu_khz_from_cpuid()` consumes leaf 0x16 before
`quick_pit_calibrate()` is ever reached). The problem is not that guest time
diverges from wall time — that is normal under `ips`. It is that the guest's
time sources become **internally inconsistent within the same emulated
timebase**: PIT, HPET, RTC and ACPI PM timer all advance at correct emulated
rates, while everything TSC-derived (sched_clock/printk timestamps,
udelay/mdelay, loops_per_jiffy, TSC-deadline arithmetic) runs
`declared_freq / ips` — exactly 875× slow at the default `ips=4000000`.
Kernels ≤ 4.7 are unaffected: they PIT-calibrate and measure the true tick
rate, which is why this has shipped since Bochs 2.6.10 (2019) without a
report.

## Environment

- Bochs: current master (verified at `70da922c`, 2026-07-12; version string
  3.0.devel). Affected releases: 2.6.10 onward for
  skylake_x/cnl/icelake_u, 2.7 onward for tigerlake, 2.8 onward for
  sapphire_rapids, 3.0 for arrow_lake — the leaf values have been
  byte-identical since each model was added.
- Guest: any Linux x86-64 kernel ≥ v4.8 (symptom detail varies by kernel
  version, table below). Reproduced with Ubuntu 26.04 live-server,
  kernel 7.0.0-14-generic.
- Host build used for the repro: master `70da922c`, MinGW-w64 gcc 15.1,
  `--enable-x86-64 --enable-avx --enable-evex --enable-vmx=2 --enable-svm
  --enable-pci --enable-all-optimizations --enable-static-link --with-win32`.

## Reproduction

```
cpu: model=corei7_skylake_x, count=1, ips=4000000
```

Boot any recent Linux kernel (`console=ttyS0,115200` appended via the GRUB
editor to capture the log over `com1: mode=file`).

**Observed** (stock master `70da922c`; Ubuntu 26.04 live-server, kernel
7.0.0-14-generic; full log attached):

```
[    0.000000] DMI:  , BIOS Bochs 3.0.devel 16/02/2025
[    0.000000] tsc: Detected 3500.000 MHz processor
[    0.000000] tsc: Detected 3499.912 MHz TSC
...
[    0.056790] clocksource: tsc-early: mask: 0xffffffffffffffff max_cycles: 0x3272fd97217, max_idle_ns: 440795241220 ns
[    0.059869] Calibrating delay loop (skipped), value calculated using timer frequency.. 6999.82 BogoMIPS (lpj=3499912)
...
[   79.515870] smpboot: CPU0: Intel(R) Core(TM) i7-7800X CPU @ 3.50GHz (family: 0x6, model: 0x55, stepping: 0x4)
...
[   86.378459] clocksource: Switched to clocksource tsc
```

- The two `Detected` values are exactly the leaf-0x16-derived numbers
  (3,500,000 kHz and 23,972 kHz × 292/2 = 3,499,912 kHz) — the emulated TSC
  actually ticks at 4 MHz, 875× slower.
- **No** `Fast TSC calibration using PIT` line anywhere in the log — the PIT
  path is unreachable because `cpu_khz_from_cpuid()` (leaf 0x16) succeeds
  first.
- **No** `Refined TSC clocksource calibration` line:
  `tsc_refine_calibration_work()` did measure the true (≈4 MHz) rate against
  HPET/ACPI-PM, but `if (abs(tsc_khz - freq) > tsc_khz/100) goto out;`
  silently discarded it and registered the TSC clocksource at ~3.5 GHz.
- **No** clocksource-watchdog demotion (the only "watchdog" line is the
  unrelated perf NMI watchdog): on ≥ v5.16 the watchdog is disabled for
  exactly this CPU model (`b50db7095fe0` — CONSTANT_TSC + NONSTOP_TSC +
  TSC_ADJUST, all enumerated by the model), so the ~875×-slow TSC remains
  the timekeeping clocksource for good.
- Delay-loop calibration is **skipped** and `loops_per_jiffy` is pinned to
  the fictitious frequency: `lpj=3499912` is numerically the bogus `tsc_khz`
  (HZ=1000). Every `udelay(n)` therefore spins ~3500 ticks per requested µs —
  but ticks retire at `ips`=4 MHz, so each requested µs takes 875 µs of the
  guest's own PIT/HPET/RTC time. Driver delays and timeouts inflate ~875×,
  which is what makes these boots crawl.

On kernels 4.8–5.15 the ending differs: the clocksource watchdog demotes the
TSC within ~1 s ("Marking clocksource 'tsc' as unstable") and wall-clock
timekeeping recovers via hpet/acpi_pm — but sched_clock/printk timestamps,
udelay/mdelay and loops_per_jiffy stay TSC-scaled ~875× slow in both regimes
(nothing reverts them on demotion): boots appear hung, timeouts inflate by
three orders of magnitude.

Compare `cpu: model=corei7_haswell_4770` (max std leaf 0xD, no frequency
leaves): the kernel PIT-calibrates, measures the true tick rate, and guest
time is consistent.

## Root cause

The TSC is the raw virtual tick counter (cpu/proc_ctrl.cc):

```c
Bit64u BX_CPU_C::get_TSC(void)
{
  Bit64u tsc = bx_pc_system.time_ticks() + BX_CPU_THIS_PTR tsc_adjust;
  return tsc;
}
```

`bx_pc_system.time_ticks()` advances one tick per emulated instruction
(cpu/cpu.cc: `BX_TICK1()` per instruction, `BX_TICKN(delta)` when batched),
and pc_system.cc maps exactly `ips` ticks to one emulated second
(`m_ips = double(ips) / 1000000.0`). `RDTSC`/`RDTSCP` return this value
unscaled (through `get_Virtual_TSC()`, which is the identity outside VMX/SVM
guests). Meanwhile cpu/cpudb/intel/corei7_skylake-x.cc declares:

```c
max_std_leaf = 0x16;
...
case 0x00000015: // Time Stamp Counter and Nominal Core Crystal Clock Information
  get_leaf(leaf, 0x00000002, 0x00000124, 0x00000000, 0x00000000);
  // TSC/crystal ratio EBX/EAX = 292/2 = 146; ECX=0: crystal not enumerated
case 0x00000016: // Processor Frequency Information (doubles as switch `default:`)
  get_leaf(leaf, 0x00000dac, 0x00000fa0, 0x00000064, 0x00000000);
  // 3500 MHz base / 4000 MHz max / 100 MHz bus
```

What Linux (`arch/x86/kernel/tsc.c`) computes from these values, by kernel
version — the PIT is never consulted in any era ≥ 4.8:

| Kernel      | Mechanism                                                                          | Resulting tsc_khz |
|-------------|------------------------------------------------------------------------------------|-------------------|
| v4.8        | leaf 0x15 unusable (no crystal) → `tsc_khz = cpu_khz` from leaf 0x16                | 3,500,000         |
| v4.9–v4.14  | hardcoded 25 MHz SKX crystal (`6baf3d61821f`) × 292/2; KNOWN_FREQ set from v4.10    | 3,650,000         |
| v4.15–v5.2  | SKX crystal quirk removed (`b51120309348`, Cc: stable) → `tsc_khz = cpu_khz` (0x16) | 3,500,000         |
| v5.3+       | crystal derived from leaf 0x16 (`604dc9170f24`): 3500·1000·2/292 = 23,972 kHz       | 3,499,912         |

Nothing on the Bochs side reconciles the declaration with the tick rate:
generic CPUID code (cpu/cpuid.cc) never touches leaves 0x15/0x16 — they exist
only as hardcoded per-model dumps — and no `clock: sync=` mode changes the
ticks-per-emulated-second relation or is even consulted by `get_TSC()`
(virt_timer's realtime mode rescales only its own virtual timers;
slowdown merely host-sleeps). The kernel's own correction machinery is
defeated as shown above (the 1% refine guard discards the correct
measurement; on ≥5.16 the watchdog is bypassed). Since v5.3 the kernel
additionally pre-sets `lapic_timer_period` from the CPUID-derived crystal, so
the bogus leaves poison LAPIC-timer calibration as well.

(Related detail found while analyzing this: `bx_local_apic_c::set_tsc_deadline`
in cpu/apic.cc arms the deadline against raw `bx_pc_system.time_ticks()`
rather than `get_TSC()`, so guest writes to IA32_TSC / IA32_TSC_ADJUST are
also ignored by TSC-deadline arming.)

## Affected models

Exactly the six models that implement leaves 0x15/0x16 (registered `cpu:
model=` names per cpudb.h). "Linux tsc_khz" is what a modern (v5.3+) kernel
derives; the real TSC ticks at `ips` regardless:

| Model              | Max std leaf | Leaf 0x15 EAX/EBX/ECX  | Linux tsc_khz       | KNOWN_FREQ |
|--------------------|--------------|------------------------|---------------------|------------|
| `corei7_skylake_x` | 0x16         | 2 / 292 / 0            | 3,499,912 (via 0x16)| no         |
| `corei3_cnl`       | 0x16         | 2 / 184 / 24.0 MHz     | 2,208,000           | yes        |
| `corei7_icelake_u` | 0x1B         | 2 / 78  / 38.4 MHz     | 1,497,600           | yes        |
| `tigerlake`        | 0x1B         | 2 / 126 / 38.4 MHz     | 2,419,200           | yes        |
| `sapphire_rapids`  | 0x20         | 2 / 176 / 25.0 MHz     | 2,200,000           | yes        |
| `arrow_lake`       | 0x20         | 2 / 218 / 38.4 MHz     | 4,185,600           | yes        |

The five models with a nonzero leaf-0x15 ECX declare the crystal directly, so
the kernel sets `X86_FEATURE_TSC_KNOWN_FREQ` and skips even the refined
calibration outright (since v4.10). Unaffected: `broadwell_ult` (max leaf
0x14) and everything older (≤ 0xD) — `cpuid_level < 0x15` makes both kernel
CPUID paths return 0 and the PIT calibration runs. No AMD model enumerates
these leaves (and the kernel paths are Intel-vendor-gated anyway).

## Suggested fixes

Either the declaration or the implementation has to follow the other, in
increasing order of surface:

1. **Stop enumerating what isn't delivered (smallest conforming change).**
   Report leaf 0x15 with EBX=0/ECX=0 and zero leaf 0x16. Per SDM Table 3-8,
   EBX=0 means "TSC/core crystal clock ratio is not enumerated" — the
   architecturally sanctioned opt-out. This is exactly QEMU's choice:
   `target/i386/cpu.c cpu_x86_cpuid()` has no `case 0x15`/`0x16` at all, both
   leaves fall to the reserved-zeros default, and guests fall back to PIT
   calibration and measure the true rate. Implementation wrinkle: leaf 0x16
   doubles as the switch `default:` arm in corei7_skylake-x.cc and
   corei3_cnl.cc.

2. **Derive leaves 0x15/0x16 from `ips`.** Compute leaf 0x16 EAX from
   `ips / 1e6` and fill leaf 0x15 so that `crystal × EBX / EAX == ips`.
   Kernels that trust CPUID then compute the *true* tick rate. Caveats: only
   4 of the 6 affected models carry a GHz suffix in their brand string
   (sapphire_rapids and arrow_lake have none), and backsolving ECX at low
   `ips` leaves a ~1.5% residual from the kernel's `crystal_khz = ecx_hz /
   1000` integer division unless EBX/EAX are rescaled too. Could be gated
   behind a config option (e.g. `cpuid_freq=ips|dump`) to preserve
   byte-faithful dumps on request.

3. **Scale the TSC to the declared frequency.** Make `get_TSC()` return
   `time_ticks() × (declared_freq / ips)` in fixed point. The 128-bit
   multiply/shift machinery already exists in proc_ctrl.cc for VMX TSC
   scaling (`get_Virtual_TSC()`: `long_mul` + shift-48;
   `compute_physical_TSC_delay()`: `long_div`) and can be reused. `set_TSC`,
   the IA32_TSC / TSC_ADJUST / TSC_DEADLINE MSR paths, and
   `bx_local_apic_c::set_tsc_deadline` need the same conversion. Keeps CPUID
   a faithful dump; largest surface, and the TSC advances in coarse steps.

4. **Minimum:** document in bochsrc's `cpu:` section that the post-Skylake
   models declare a hardware TSC frequency via CPUID that modern kernels
   (≥ 4.8) trust without calibrating, so they require `ips` near the declared
   frequency for consistent guest timekeeping — and recommend pre-Skylake
   models otherwise. Today no Bochs documentation mentions this interaction,
   and (coincidentally) the sample bochsrc snippets in the user guide all use
   pre-Skylake models.

For reference: when QEMU/KVM *does* advertise a TSC frequency via CPUID (the
default-on `vmware-cpuid-freq` leaf 0x40000010), it populates it from the
actual virtual TSC rate and suppresses it when that rate isn't stable and
known — mainstream VMMs either decline to enumerate a TSC frequency or
enumerate the true one.

## Workarounds for users

- Use a model without the frequency leaves (`cpu: model=corei7_haswell_4770`
  or `model=broadwell_ult`) — at the cost of AVX-512 and the newer ISA
  extensions the six affected models exist to provide.
- Setting `ips` to the declared frequency (3500000000 fits the Bit32u
  parameter) restores internal consistency, but on a host executing
  ~100–200 MIPS it makes *all* emulated time run 20–35× slower than wall
  clock, and it breaks again on the next model switch. `clock: sync=` does
  not help: no sync mode alters the TSC/tick relation.

## References

- Intel SDM Vol. 2A, CPUID, Table 3-8 — leaf 15H ("TSC frequency = core
  crystal clock frequency × EBX/EAX"; EBX=0/ECX=0 = not enumerated) and leaf
  16H (spec/nameplate frequencies). Leaf 16H alone is informational, but
  Linux uses it precisely to reconstruct the leaf-15H crystal when ECX=0 —
  and a 875× error is beyond any spec-vs-actual tolerance that caveat
  contemplates.
- Linux commits (mainline versions verified against tagged sources):
  - `aa297292d708` — "x86/tsc: Enumerate SKL cpu_khz and tsc_khz via CPUID" (v4.8)
  - `6baf3d61821f` — SKX added to 25 MHz crystal quirk list (v4.9)
  - `47c95a46d0fa` + `4ca4df0b7eb0` — X86_FEATURE_TSC_KNOWN_FREQ skips refined calibration (v4.10)
  - `b51120309348` — "x86/tsc: Fix erroneous TSC rate on Skylake Xeon": quirk removed because real SKX crystals are 24 MHz (workstation, −4% drift) / EMI-reduced ~−0.25% (server) (v4.15, Cc: stable)
  - `604dc9170f24` — "x86/tsc: Use CPUID.0x16 to calculate missing crystal frequency" (v5.3)
  - `b50db7095fe0` — clocksource watchdog disabled for qualifying TSCs (v5.16)
- Thematic cousin in this tracker: #690 (Windows ACPI clock drift under
  `clock: sync=realtime`) — different mechanism, same "guest timekeeping"
  area.
