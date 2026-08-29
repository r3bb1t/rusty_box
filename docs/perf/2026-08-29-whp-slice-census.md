# The slice census of a DLX boot on WHP — 2026-08-29

The falsification gate for
`docs/superpowers/plans/2026-08-29-whp-fast-path.md`. It was written to choose
between two outcomes. **It produced a third, and the plan's premise does not
survive it.**

## How it was run

```bash
cargo build --release -p rusty_box_whp_engine --example dlx_whp
DLX_WHP_PATIENCE_SECS=120 ./target/release/examples/dlx_whp.exe
```

Host: the development machine (Intel, WHP enabled). Engine configuration as the
example ships it — `ips = 300_000_000`, `SLICE_TICKS = 20_000_000`,
`LocalApicEmulationMode::None`, shadow processor for every exit the platform
cannot finish. Commit `4608547` plus the `platform_counters` accessor.

## What happened

**The guest did not boot.** Two of three milestones: the BIOS came up, LILO took
the disk, `login:` never appeared. The run ended by exhausting its 120 s of
patience, not by finishing.

```
got 2 of 3 milestones in 120.0s of host time and 999326 Mticks of guest time
  exits:  port 141072  mem 72897  cpuid 5  msr 0  halt 797  canceled 9942  boundary 146
  slices: 217640  [exits/slice  0=0  1=210567  2=7073  3-4=0  5-8=0  9+=0]
          ended: halted 797  canceled 14  budget 216683
                 boundary(processor 11  device 0  event 135)
  platform: io 139981  npf 73501  other 1210  cpuid 5  msr 15  halt_time_100ns 107445
  runtime:  total 105989ms  hypervisor 4150ms
```

## The numbers reconcile, so the instrument is trustworthy

- 8,009 steps in 120 s is **15 ms per step** — `step`'s wall clock, not its tick
  budget. That is why a 20 M-tick budget produced 125 M ticks of progress per
  step; the argument bounds an inner batch, the wall clock ends the call.
- 217,640 slices over 8,009 steps is **27 slices per step**, ~551 µs of wall
  time each.
- Exits tallied by class (224,713) equal exits tallied by the histogram
  (210,567 + 2×7,073), so the per-slice count and the lifetime count agree.
- Guest time 999,326 Mticks ÷ 3e8 ips = **3,331 guest-seconds**, which is 120 s
  of wall time × `HARDWARE_SPEED = 32`. The clock is doing what it says.

Cross-check against the hypervisor's own accounting, using the mapping measured
in Task 1:

| engine | platform | delta |
|---|---|---|
| port 141,072 | `io_instructions` 139,981 | 0.78 % |
| mem 72,897 | `nested_page_fault_intercepts` 73,501 | 0.82 % |
| cpuid 5 | `cpuid_instructions` 5 | exact |
| halt 797 | `other_intercepts` 1,210 | halts are a subset of this class |
| msr 0 | `msr_accesses` 15 | see below |

Both principal classes agree inside the 1 % threshold the plan set, so neither
account is measuring the wrong thing. **`halt_instructions.count` is 0 as
predicted**, with 10.7 ms of time charged to it — the count lives in
`other_intercepts`.

The one disagreement, `msr 0` against `msr_accesses 15`, is the platform
counting intercepts **it** resolved without exiting to the root partition. Same
shape as the halt finding: Hyper-V books by its own accounting, not ours. 15
events across a whole boot; noted, not chased.

## The plan's prediction, judged

> **median 1 exit per slice, >70 % of slices ending `Boundary`, and
> `total − hypervisor` a small fraction of wall time**

- **Median 1 exit per slice — CONFIRMED.** 96.75 % of slices held exactly one
  exit; not one slice in 217,640 held more than two.
- **>70 % ending `Boundary` — REFUTED, and not narrowly.** 157 slices ended on a
  boundary. That is **0.07 %**. 99.56 % ended on `Budget`. Of the three boundary
  reasons, `device` fired **zero** times.
- **Hypervisor a small fraction — CONFIRMED, and it is the finding.** 4,150 ms
  of hypervisor time against 105,989 ms of processor runtime: **3.9 %**.

## What this refutes

The plan's architecture paragraph says the engine "pays ~26.5 µs of wall time
per slice … **because after servicing any exit it drains device latches and
immediately returns `Yielded::Boundary`**". Measured, it does not. Device
latches ended zero slices. Slices end because the stretch the machine asked for
is over.

**Tasks 4, 5 and 6 remove a cost that is not being paid.** Task 4 exists to stop
leaving the partition for work already done; the engine is not leaving for that
reason. Task 5 moves device deadlines off the guest clock; deadlines are not
what ends these slices. Task 6 elides exits in MMIO-dense regions; memory exits
had **stopped entirely** by the time the run ended (frozen at 72,897 for the
last 15 s while port exits kept climbing).

Task 7 (cheaper state exchange) keeps a real premise: 19.1 µs of hypervisor time
per slice against H0's ~4 µs for a bare exit leaves ~15 µs that is entry and
state exchange. But the whole envelope is 3.9 % — halving it buys under 2 %,
nowhere near what "not less performance" requires.

## The dominant fact, which is not a performance fact

The guest was given **3,331 seconds of guest time and 102 seconds of real
hardware execution**, and never brought the kernel up. In the final 15 s it took
**zero memory exits** while port exits climbed at ~340/s, with `RIP` cycling
among about ten addresses in `0x109af3`–`0x1167c7`.

That is a kernel spinning on a port, not a machine running slowly. The engine's
overhead is 3.9 %; there is no throughput change that rescues a guest which does
not progress. This is consistent with the unresolved `#GP` on `IRET` (`do_IRQ`
returning 128 bytes low) recorded during the earlier debugging.

## Ruling

**Stop the fast-path work. Do not do Tasks 4–6.** Not for the reason the plan
anticipated — the median is 1, not 5+ — but because the mechanism those tasks
remove accounts for 0.07 % of slice endings and the hypervisor accounts for
3.9 % of runtime.

The next work is the stall: identify the port the kernel polls, and why the
interrupt that would end the poll never arrives. Re-run this census against a
guest that reaches `login:`; only then does a comparison against the interpreter
mean anything, and only then can the spec's ranking be judged on a boot rather
than on a hang.

Task 7 is not refuted and can be taken on its own merits later, with its ceiling
stated honestly: under 2 % of this workload.
