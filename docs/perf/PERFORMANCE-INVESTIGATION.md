# Rusty Box — CPU Performance Investigation (handoff)

Self-contained dump of the perf investigation so another agent can continue.
Everything here is **measured**, not guessed. Hard constraint throughout:
**every optimization must preserve Bochs-observable behavior** (deviations from
`cpp_orig/bochs/` are bugs unless they're pure idiomatic-Rust improvements that
don't change behavior). The wins so far are *parity-restoring* (matching Bochs
values) or behavior-neutral.

Date: 2026-07-08. Branch context: `feature/8-multi-processor-support` (== `master`).

---

## TL;DR — what shipped (all on `origin/master` + `origin/dev`, cargo-verified)

| Commit | Change | Effect |
|--------|--------|--------|
| `1595944` | icache size 8192→65536 + page-split 8192→8; hot-loop plumbing trim | **boot 69.7s → 47.9s for 4B insns (~31% faster, 1.45×)** |
| `fc0bfb6` | softfloat `approxRecipSqrt32_1` completed | correctness (f32_sqrt, extF80_sqrt) |
| `00d707a` | harden hot-loop `unsafe` mpool access; untrack c.img | safety |
| `30901cc` | remove orphan `iodev/dma.rs`; quiet warnings | cleanup |

Release build is warning-free; `cargo test -p rusty_box` = 327 pass.

---

## Measurement methodology

### Ground-truth metric
The GUI "IPS" counter = **retired guest instructions ÷ real wall-clock second**
(`emulator.rs::status_ips_from_retired_instructions`, `total_cpu_icount()`).
- It equals host throughput **only during CPU-bound phases**.
- During HLT/idle the guest retires ~0 instructions, so it reads ~0–1K even
  though the host is busy (fast-forwarding time, servicing devices). So a lot of
  boot "non-linearity" is the metric, not a host stall.
- **Use wall-clock time for a fixed instruction budget as the authoritative
  metric** (captures busy throughput *and* idle overhead). That's what the
  before/after numbers below use.

### Harness A — CPU-core microbench (`examples/perfbench/perfbench.rs`)
Runs canonical hot loops through the full `cpu_loop` in paging-on long mode
(FlatLong64 identity map). No disk/BIOS; reproducible on any checkout.
```
PERFBENCH_MODE={mixed|alu|branch|straight} PERFBENCH_INSN=500000000 \
  cargo run --release --example perfbench --features std
```
Caveat: a hot loop stays icache-cached, so it under-represents real workloads
(no decode/icache-miss cost). Use it for dispatch/plumbing/memory microcosts.

### Harness B — real Alpine boot (the workload that matters)
```
rusty_box_gui --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom alpine-virt-3.24.1-x86_64.iso --boot cdrom \
  --display headless --no-sync-slowdown --memory-mib 256 --pci \
  --max-instructions 4000000000
```
Time it (`{ time <cmd>; } 2> t.txt`) for the authoritative number. `--display
headless` = NoGui (no GUI overhead); `--no-sync-slowdown` = full host speed.

### Profiling (Windows)
```
samply record --save-only -o prof.json.gz <cmd>          # build with CARGO_PROFILE_RELEASE_DEBUG=2
samply load --no-open --port 3587 prof.json.gz           # starts a symbol server
# then POST module-relative frame addrs to  http://127.0.0.1:<port>/<hash>/symbolicate/v5
```
Use `docs/perf/prof_analyze.py` (copied here) — it does aggregate + time-windowed
self-time, chunked/tolerant symbolication (a few addrs trip the server's parser;
skip them). Needs the samply symbol server running.

---

## Baselines (microbench, 500M insns, FlatLong64)

| Mode | MIPS | ns/insn | isolates |
|------|------|---------|----------|
| straight (32/trace) | 217 | 4.6 | amortized dispatch+execute floor |
| alu (7/trace, no mem) | 198 | 5.1 | + frequent taken branch |
| mixed (7/trace, ld+st) | 169 | 5.9 | realistic mix |
| branch (3/trace) | 157 | 6.4 | branch/trace-relookup heavy |

Solved component costs: **base per-insn ≈ 4.4 ns (~15 cyc @ 3.4GHz), per
taken-branch/trace-boundary ≈ 6 ns, per memory op ≈ 3.5 ns.**

## Real boot profile — BEFORE fixes (4B insns, ~80s wall w/ samply)

Self-time (symbolicated):
```
18.2% cpu_loop_n_impl      (per-instruction loop plumbing)
12.6% execute_instruction  (opcode→handler dispatch, jump table)
12.1% get_icache_entry     (trace lookup)
 9.0% fetch_decode64       (DECODING cold code — invisible in microbench)
 4.2% mov_cr3_rq           (CR3/TLB churn)
 2.5% read_linear_qword
 1.8% lookup_opcode_64
 ...  read_linear_byte, smc_write_check, prefetch, now(clock), ...
```
Time-windowed (the "non-linear" boot): BIOS-POST window was **17% clock-reads +
23% prefetch** (idle/IO), then kernel/userspace windows were **decode+icache
16–29%**. ~45% of runtime was loop-plumbing + dispatch before any instruction
semantics ran.

---

## Root causes + status

### RC1 — icache 8× smaller than Bochs  ✅ FIXED (`1595944`)
`rusty_box/src/cpu/icache.rs`: `BX_ICACHE_ENTRIES` was **8192** vs Bochs
`icache.h` **64*1024 = 65536**. Also the page-split array was wrongly sized to
`BX_ICACHE_ENTRIES` instead of Bochs's `BX_ICACHE_PAGE_SPLIT_ENTRIES = 8`. The
undersized trace cache thrashed on real code working sets → constant re-decode.
Fix restored both to Bochs values. mpool (576*1024) already matched Bochs.
**Result:** `fetch_decode64` 8.95% → **3.50%** (decode work more than halved).

### RC2 — hot-loop plumbing does work Bochs's inner loop doesn't  ✅ FIXED (`1595944`, hardened `00d707a`)
`rusty_box/src/cpu/cpu.rs::cpu_loop_n_impl`:
- dead `trace_iter` counter (incremented every insn, never read) — removed.
- `mpool[instr_idx]` bounds-check panic branch — now `get_unchecked` + a
  `debug_assert!` bounds check + documented SAFETY invariant.
- `ilen==0||ilen>15` validation `assert!` (ran in release; Bochs doesn't) —
  gated behind `#[cfg(debug_assertions)]`.
- `perf_instructions` counter — gated behind `#[cfg(feature="profiling")]`.
Removing the per-instruction panic paths let the compiler optimize the hot loop
far better than the raw op-count implied (this over-delivered).

### RC3 — memory fast-path call depth  (NOT done)
`mov_* → *_virtual_qword_64 → *_linear_qword`. The wrappers are already
`#[inline]`; `read_linear_qword`'s ~2.8% is the real TLB-hit work (matches
Bochs). Low remaining upside. Preserve MMIO/cross-page/permission/A20 semantics.

### RC4 — dispatch (12–14%)  (inherent)
`execute_instruction` is a `match` over ~3677 dense `#[repr(u16)]` opcodes → a
jump table (single indirect jump). Hot loops repeat ~7 targets, so it's
well-predicted — the cost is inherent to interpretation. Beating it needs
threaded/tail-call dispatch or a template-JIT = large project, high Bochs-parity
risk. NOT recommended without a dedicated effort.

---

## Fixes: measured before/after (clean timed, 4B-insn Alpine boot)

| Stage | wall | host IPS | vs original |
|-------|------|----------|-------------|
| Original (8192 icache) | 69.67s | 57.4M | — |
| + icache→65536 | 59.14s | 67.6M | −15% |
| + plumbing trim | **47.93s** | **83.5M** | **−31% (1.45×)** |

## Real boot profile — AFTER fixes (icache+plumbing)

```
17.4% cpu_loop_n_impl
16.3% get_icache_entry     (now #2; hash + entry-array load + SMC first_bytes)
13.7% execute_instruction  (dispatch, inherent)
 3.6% fetch_decode64       (down from 9.0%)
 2.8% read_linear_qword
 2.0% mov_cr3_rq
 1.7% smc_write_check
```
Top 3 ≈ 47% of time. Line-level: `get_icache_entry:2460` (~2%) is the per-hit SMC
`first_bytes` compare; `:2501` (~2%) is the `serve_icache_miss` call (residual
cold-code decode, inherent).

---

## Remaining levers (ranked; all carry the noted parity risk)

1. **SMC `first_bytes` per-hit compare in `get_icache_entry` (~2%)** —
   `cpu/cpu.rs:~2460`. A Rusty-Box addition on top of Bochs's page-write-stamp
   SMC (`smc_write_check`, wired into all write paths in `access.rs`). Comment
   says it guards page-**remap** (mmap/library-load) cases the stamp misses.
   Removing it needs proof the write-stamp + `break_links` fully cover SMC AND
   remap → **SMC/stale-code correctness risk**; needs dedicated SMC/remap tests.
   Bonus if removed: shrinks `BxICacheEntry` by 8 bytes × 65536 = better
   lookup locality.
2. **`sync_lapic_intr_event()` per-instruction poll (~1%)** — `cpu/cpu.rs:~2236`.
   Reads `self.lapic.intr_pending` every instruction to bridge it into
   `async_event`. Bochs signals `async_event` at the source (apic.cc) instead of
   polling. Relocating requires auditing every `intr_pending` setter/reader
   (`cpu/apic.rs`, `emulator.rs`) and proving interrupt delivery lands on the
   same instruction boundary → **interrupt-timing/parity risk**.
3. **Clock-read overhead in idle/HLT (`now`/`clock`, up to 17% of the BIOS-POST
   window)** — `emulator.rs` HLT wall-clock throttle calls `Instant::now()` per
   spin iteration. Invisible to the IPS counter (idle phase), but it inflates
   **wall-clock boot time**. Reduce clock-read frequency in the HLT loop. Lower
   parity risk (host-side pacing), but verify no timer/interrupt-latency change.
4. **Dispatch / threaded interpreter / JIT** — see RC4. Big, risky. Only with an
   explicit decision.

Honest ceiling for further *parity-safe* interpreter work: maybe another ~5–10%.
The 31% already captured came from the one real structural divergence (icache).

---

## Key files
- Hot loop: `rusty_box/src/cpu/cpu.rs` — `cpu_loop_n_impl`, `get_icache_entry`.
- icache: `rusty_box/src/cpu/icache.rs` (sizing consts at top).
- dispatch: `rusty_box/src/cpu/dispatcher.rs` (`execute_instruction`).
- memory: `rusty_box/src/cpu/access.rs` (`read/write_linear_*`, `*_virtual_qword_64`).
- Bochs reference: `cpp_orig/bochs/bochs/cpu/{icache.h,cpu.cc}`.
- Harness: `rusty_box/examples/perfbench/perfbench.rs` (also copied to `docs/perf/`).
- Analysis: `docs/perf/prof_analyze.py`.
- Raw samply profiles (gitignored, repo root): `alpine_boot.json.gz` (before),
  `alpine_boot_v2.json.gz` (after icache+plumbing), plus `perfbench` profiles.
  Re-analyze with `samply load` + `prof_analyze.py`.

## How to reproduce a fresh before/after
1. Build with symbols: `CARGO_PROFILE_RELEASE_DEBUG=2 cargo build --release -p rusty_box_gui`.
2. Time the Harness-B command above (4B insns) → wall-clock.
3. To attribute: `samply record --save-only ...`, then `samply load` + run
   `prof_analyze.py` (set HASH/PORT/PROF env). Compare `fetch_decode64` /
   `get_icache_entry` / `cpu_loop_n_impl` self-time.
4. Parity gate for any change: `cargo test -p rusty_box` (327) must stay green;
   any change touching SMC/interrupts/memory needs targeted reasoning + ideally a
   real boot-to-login diff.

---

## Windows installer follow-up (2026-07-14)

The later Windows investigation used the Windows 10 22H2 ISO, 2 GiB guest and
host RAM, PCI enabled, `--ips 120000000`, `--no-sync-slowdown`, and the release
GUI binary with its headless display backend. The comparison baseline is the
clean `58cc7ef` worktree; both sides used the same warmed ISO and BIOS images.

| Instruction milestone | `58cc7ef` | optimized worktree | wall change |
|---:|---:|---:|---:|
| 1,000,000,000 | 12.81s | 11.90s | −7.1% |
| 4,000,000,000 | 31.83s | 31.78s | −0.2% |
| 34,608,645,920 | 306.52s | 299.05s | −2.4% |
| 88,198,927,292 | 1056.72s | 1052.74s | −0.4% |

The 1B, 4B, and 88B optimized measurements predate the final decoded-handler
pool; the final architecture was rerun at the unattended 34.6B milestone. It
selects a monomorphized handler once during trace decode and keeps it in a
parallel mpool, matching Bochs' per-instruction `execute1` design without
putting a generic function pointer in the decoder crate. The same change
improved `perfbench[mixed]` from the investigation's 170.7 MIPS reference to
173.4–175.2 MIPS.

The larger improvement seen by the user—from roughly 13–17 MIPS to 58.172 MIPS
at the visible "Starting installation" screen—primarily comes from running the
optimized release binary instead of `cargo run`'s debug profile.

Correctness/performance work retained from the late-boot investigation:

- stale PIC `PENDING_INTR` assertions are reconciled without rescanning an
  empty PIC on every instruction;
- decoded instructions cache their monomorphized handler;
- page-write stamps replace the redundant per-hit instruction-byte guard;
- real-mode `REP INSW` can write directly into mapped RAM and bulk-read IDE
  data while preserving segment/page fault ordering, instrumentation hooks,
  SMC invalidation, retired counts, and ATA/ATAPI IRQ boundaries.

The real GUI reached the Windows installer start screen at 88,198,927,292
retired guest instructions. Treat instruction milestones as phase markers, not
linear predictors: the early 1B and unattended 34.6B windows improved, while
the earlier pre-handler 4B and 88B comparisons were nearly unchanged.
