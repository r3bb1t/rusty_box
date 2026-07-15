# CPU Hot-Loop Performance Optimization — Design

**Date:** 2026-07-07
**Status:** Design (awaiting review)
**Scope:** Behavior-preserving throughput optimization of the CPU interpreter hot path.

## Hard constraint

**Any deviation from original Bochs observable behavior is a bug, not a win.** Every
change here is behavior-preserving. The search space is limited to places where Rusty
Box does *more* work than Bochs, or implements a Bochs optimization less efficiently —
i.e. closing the gap *to* Bochs, never diverging *from* it.

## Goals

- Increase interpreter throughput (MIPS) with zero change to guest-observable behavior.
- Each change proven both (a) parity-preserving and (b) measurably faster, independently.

## Non-goals

- No JIT / block compilation (would diverge from Bochs's interpreter model in structure
  and is out of scope).
- No changes to instruction semantics, flag computation, memory model, or SMP/timer
  scheduling behavior.
- Not chasing a 2×. Realistic parity-safe upside from Tracks A+B is ~15–25%.

## Baseline (measured 2026-07-07)

Measured with a temporary CPU-core microbenchmark (`rusty_box/examples/perfbench`, safe
to delete) that runs canonical hot loops through the full `cpu_loop` path in paging-on
long mode (FlatLong64 identity map — no disk/BIOS needed, reproducible on any checkout).
Profiled with `samply` (ETW) symbolicated against the PDB.

Throughput (500M instructions per shape):

| Loop shape             | MIPS | ns/insn | Isolates                         |
|------------------------|------|---------|----------------------------------|
| `straight` (32/trace)  | 217  | 4.6     | amortized dispatch+execute floor |
| `alu` (7/trace, no mem)| 198  | 5.1     | + frequent taken branch          |
| `mixed` (7/trace, ld+st)| 169 | 5.9     | realistic mix                    |
| `branch` (3/trace)     | 157  | 6.4     | branch / trace-relookup heavy    |

Derived per-component cost: base per-instruction ≈ 4.4 ns (~15 cyc), per taken-branch /
trace-boundary ≈ 6 ns, per memory op ≈ 3.5 ns.

Symbolicated self-time (`mixed`, 2B instructions):

| %     | Function                              | Nature                        |
|-------|---------------------------------------|-------------------------------|
| 30.8% | `cpu_loop_n_impl`                     | per-instruction loop plumbing |
| 13.8% | `execute_instruction`                 | opcode→handler dispatch       |
| ~21%  | `read/write_linear_qword` + `*_virtual_qword_64` | memory path        |
| 5.3%  | `smc_write_check`                     | SMC check on every store      |
| rest  | `add_eq_gq`, `dec_eq`, …              | actual instruction semantics  |

~45% of runtime is loop plumbing + dispatch before any instruction meaning runs.

**Context:** 169 MIPS is already competitive with real Bochs. This is squeezing an
already-good interpreter, not fixing a pathology.

## Root causes

- **RC1 — Per-instruction plumbing does work Bochs's inner loop doesn't (30.8%).**
  - `trace_iter` counter incremented every instruction, never read (dead).
  - `perf_instructions` — second counter beside `icount`, only read for a once-per-batch
    stderr line (~2.7% self-time, line-confirmed).
  - `ilen == 0 || ilen > 15` validation branch every instruction; Bochs doesn't validate
    in the hot loop.
  - `sync_lapic_intr_event()` polls `lapic.intr_pending` every instruction. Bochs never
    polls the APIC per-instruction — it signals `async_event` at the source when an
    interrupt becomes deliverable.
  - `mpool[instr_idx]` carries a bounds-check panic branch each instruction (the index is
    always icache-valid).
- **RC2 — Memory fast path is a 3–4-frame call chain (~21%).** A store is
  `mov_eq_gq → write_virtual_qword_64 → write_linear_qword → smc_write_check`; Bochs
  collapses the TLB-hit path into one inlined function. Loads mirror this.
- **RC3 — `smc_write_check` is a non-inlined call on every store (5.3%)**, even when the
  target page has no cached code (the common case is a fast-return that still pays call +
  hash overhead).
- **RC4 — Dispatch (13.8%) is a well-predicted jump table** (3677 dense `#[repr(u16)]`
  opcodes; the hot loop repeats ~7 targets → not misprediction-bound). Improving it means
  threaded / tail-call dispatch — an interpreter restructure that is high-risk for parity
  and uncertain payoff.

## Validation methodology (per commit, one optimization per commit)

1. **Parity proof:** full existing test suite (187 tests) passes unchanged. Changes
   touching the LAPIC poll, memory semantics, or dispatch get additional targeted
   reasoning/tests.
   - **Known gap:** DLX/Alpine disk images are not in the repo, so full-boot parity can't
     be run here. Mitigation: rely on the test suite + targeted reasoning. If a disk image
     is provided, add a boot-diff (register/icount trace before vs after) as the strongest
     gate.
2. **Performance proof:** `perfbench` across all 4 shapes + criterion decode bench, before
   vs after. A change with no measurable improvement is reverted — no speculative
   complexity.

## Track A — Hot-loop plumbing trim (RC1)

Low risk, surgical. Target ~8–12%.

1. Delete dead `trace_iter` (`cpu/cpu.rs` trace loop).
2. Gate `perf_instructions` increment — and any other `perf_*` counters incremented on
   hot paths (audit: `perf_tlb_hit/miss`, `perf_page_walk`, `perf_icache_miss`,
   `perf_prefetch`) — behind `#[cfg(feature = "profiling")]`; gate their readers to match.
3. Gate the `ilen == 0 || ilen > 15` validation behind `#[cfg(debug_assertions)]`.
4. **Relocate the per-instruction LAPIC poll (parity-sensitive, own commit):** signal
   `async_event`/`BX_EVENT_PENDING_LAPIC_INTR` at the point `lapic.intr_pending` is set
   (or at trace-break/batch boundary), removing the per-instruction `sync_lapic_intr_event`
   call. Prerequisite: audit every setter and reader of `lapic.intr_pending`
   (`cpu/apic.rs`, `emulator.rs`) and prove observable equivalence — no interrupt delivered
   earlier or later relative to instruction boundaries than today.
5. `mpool[instr_idx]` → `get_unchecked(instr_idx)` with a SAFETY comment (index always in
   `[0, BX_ICACHE_MEM_POOL)` from the icache).

## Track B — Memory fast-path flatten + SMC inline (RC2 + RC3)

Medium risk (unsafe pointer paths). Target ~8–12%.

1. Inline the TLB-hit fast path of `read_linear_*` / `write_linear_*` and their
   `*_virtual_qword_64` wrappers so the common path is one flat sequence; keep the slow
   path (translate / MMIO / cross-page split) as a `#[cold] #[inline(never)]` call.
2. Fold `smc_write_check`'s `page_write_stamps[index] == 0` fast-return inline into the
   store path so the common store makes **zero** calls; only a genuine SMC hit calls
   `handle_smc_scan`.
3. Preserve exactly: MMIO detection, cross-page splitting, permission checks + `on_lin_access`
   hooks (instrumentation feature), A20 masking, alignment / #AC behavior.

## Track C — Dispatch restructuring (RC4) — gated experiment

High risk / uncertain. Not committed delivery.

1. Prototype tail-call or stored-fn-pointer dispatch behind a feature flag.
2. Measure on `perfbench`; run the full test suite.
3. **Go/no-go:** keep only if it shows a measurable win AND full parity holds. Honest
   prior: no-go (the jump table is already well-predicted). Document the result either way.

## Sequencing

A → B → C. Each track independently measured, one optimization per commit. Reassess
cumulative gain after A and B before deciding how far to push C.

## Risks / open questions

- LAPIC-poll relocation (A4) is the highest-parity-risk item; if the audit can't prove
  equivalence, keep the poll but hoist it (e.g. once per trace-break) as a fallback.
- No full-boot validation available in-repo (disk-image gap).
- Track C may yield nothing; that is an acceptable documented outcome.
- `perfbench` and the temporary Cargo.toml example entry are investigation scaffolding —
  delete at the end unless kept intentionally as a perf regression harness.
