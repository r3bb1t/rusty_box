# WHP VMM Shape Implementation Plan — Stages 0 and 1

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the WHP engine's slice model with the VMM shape — an unbounded vCPU thread, a host-time device thread driving the Bochs wheel, the hypervisor's LAPIC — so that Alpine and DLX boot on fast mode with ≥ 90 % of busy wall time inside `WHvRunVirtualProcessor`, faster than the interpreter, on a tree whose probes have answered the questions the design rests on.

**Architecture:** The spec is `docs/superpowers/specs/2026-09-03-whp-vmm-shape-design.md` — read it first; it is the authority, and its §8 fixes the staging this plan follows. This document plans **Stage 0 (probes and baseline) and Stage 1 (the fast-mode core)** in full. Stages 2–4 (long runs, transfer, surface) are outlined at the end and get their own plans after Stage 1's gate, because the last plan's ranking was refuted by the first census it ran and a plan written past the next measurement would repeat that. Every task leaves a bootable tree and a green suite: the interpreter's machine is untouched throughout, and the WHP engine keeps booting through the slice loop until Task 1.7 replaces it, because everything the new shape adds before then is gated behind a machine setting (`device_clock: HostTime`) that no existing caller sets.

**Tech Stack:** Rust workspace. `rusty_box` (CPU, machine, devices wiring), `rusty_box_core` (the no_std vocabulary: `VmInstant`, `HostClock`, `EngineFault`), `rusty_box_devices` (device API), `rusty_box_whp` (the WHP FFI leaf; the only crate that may say `unsafe`, confined to `src/sys/windows.rs`), `rusty_box_whp_engine` (the engine under change, std-only). Tests: `cargo test --release -p <crate>`; hypervisor-gated tests skip with a reason on hosts without WHP.

## Global Constraints

- **Verification cadence (CLAUDE.md):** after each edit batch `cargo check --release -p rusty_box --features std --lib`; add `cargo check --release --no-default-features -p rusty_box` whenever the edit touches `rusty_box/` (scheduler.rs, mod.rs, io.rs, engine.rs, iodev/*, pc_system.rs, cpu/arch_state.rs, cpu/api_bridge.rs are all no_std-compiled). Engine/leaf: `cargo check --release -p rusty_box_whp_engine` / `-p rusty_box_whp`.
- **Full `cargo xtask ci` before every commit that ends a task; commit only the tree the gates saw.** Never stage `ROADMAP.md`. **Never run `cargo fmt`.** Release builds only.
- Edit with the Edit/Write tools, never scripts. Bochs-source comments cite file + symbol, never line numbers. Comments state invariants, never history. No `TODO`/stub/partial work. Never `let _ = fallible()`. No `dyn` in signatures (R8). `Send` by derivation only (R6): no `unsafe impl Send`.
- Doctrine: R0 named types in public APIs — no tuples, no positional results (`Processor`, `DeviceTime`, `EngineCensus`, `VcpuCensus` below are the names); R1 **no pre-existing crate's `unsafe` count ever rises.** Task 0.2a moves the whole platform seam into a new `rusty_box_whp_sys` crate, after which `rusty_box_whp/src` is 1 — down from 38, and that one is the `pub unsafe fn map_borrowed` SIGNATURE, an obligation marker that must not be laundered away — `rusty_box_whp_sys/src` is 37, and `rusty_box_whp_engine/src` stays 1 (its discharge in `map_window`). `rusty_box_whp_sys/src` is then the one crate in the workspace whose baseline moves, and it moves only when a platform entry point is bound — which is that crate's entire purpose — by the exact count, stated in that commit. This is the doctrine's own end state for this seam (`docs/safety-doctrine.md` R1: the unavoidable `unsafe` lives in one dedicated, auditable place), and it is the user's ruling of 2026-09-04; R2 states are types; R3 `PcIo` is assembled only by a machine's own `&mut self`; R4 units are types — the tree's `VmInstant` (ticks), `VmDuration`, `HostInstant` (host nanoseconds) and `ClockHz` in `rusty_box_core::time` are the units this plan uses, and the spec's "VmTime" is realised as the `VmInstant` a Started/Stopped clock source answers; R5 one choke point per hazard with exhaustive matches; R7 provenance (Bochs symbol, or "Hyper-V TLFS" / "WHP SDK header" / "QEMU `<function>`" for what Bochs lacks); R9 tests assert guest-visible properties.
- **Threads.** `T: Instrumentation` already implies `'static` (`cpu/instrumentation/bochs.rs`: `pub trait Instrumentation: Default + 'static`); the only bound the threads add is `Send`. The workspace's release profile is `panic = "abort"` (root `Cargo.toml`), so a panic on any machine thread aborts the process by design; the poisoned-lock arms below exist for the unwind profile `cargo test` builds with, and say so. **Lock order: machine → clock.** A thread holding the clock lock never takes the machine lock.
- **Waits.** Every wait a test or a verb performs is bounded and fails by name: `wait_until(done, within, what)` panics with `what`; `wait_parked_by(Duration)` returns `None`; `FastMachine::step` refuses with `FastMachineFault::Wedged` after 10× the budget's nominal duration (floor 1 s). No wait holds the machine lock.
- **One ISO for every measurement in this plan:** the ORIGINAL Alpine ISO with its `quiet` kernel line intact (the docstring of `alpine_probe.rs` names `alpine-virt-3.24.1-x86_64.iso`). Its file name and SHA-256 are recorded in the baseline document; the 2026-09-01 A/B ran a no-quiet copy, so its 150/162/165 s are not comparable and are not the baseline.
- **Agents dispatched for this plan run on `model: opus`** (the user's Fable quota is spent; see memory `use-fable-model-only`).
- Bounded headless guest runs only (`--display headless` or `terminal`; never `--display terminal` with a Windows ISO; never `--display egui` from an agent unless asked). The user's rule "no running the emulator" is lifted only for the bounded measurement steps named below.
- Hypervisor-gated tests follow the existing pattern: in `rusty_box_whp_engine/src/lib.rs` `if !hypervisor_here() { return; }` then `let _turn = a_turn_on_the_hardware();`; in `rusty_box_whp` `if !hypervisor_present().unwrap_or(false) { return; }` then the same `a_turn_on_the_hardware()` (partition.rs already has one — never add a second copy; one partition per process is the rule the turn enforces). The two names differ by crate on purpose; do not unify them.
- Line numbers below are as of `7728086`; re-locate by symbol before editing. `emulator_api.rs` is `rusty_box/src/emulator_api.rs` (not under `emulator/`). **Paths written `rusty_box_whp/src/{sys.rs, sys/windows.rs, sys/unsupported.rs, error.rs, vcpu.rs}` are pre-0.2a**: after Task 0.2a those files are `rusty_box_whp_sys/src/{lib.rs, windows.rs, unsupported.rs, error.rs, vcpu.rs}`, and every later task means the new location. `caps.rs`, `partition.rs` and `lib.rs` stay in `rusty_box_whp`.
- **Commit messages** end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

---

## Disposition of the earlier plans' remaining items (what this plan keeps, reuses, drops)

| Item | Disposition |
|---|---|
| Injection plan T8 (head rewiring, `InterruptState` imposition) — done | **Reused**: `InjectState`, `stage_injection`, the header cache and the window arm become the legacy-8259 ExtINT path (Task 1.6). |
| Injection plan T9 (Stage 1 gate) — failed | **Superseded** by this plan's gates. Not re-run. |
| T10 `ShadowFreshness` witness | **Superseded** by the externalised mask (Task 1.4). The witness type was never built; the fields that carried the idea, `Started.shadowed` and `Started.ran_since_read_back`, are deleted in Task 1.7. |
| T11 mini-import + lazy slice end | **Moot**: no slice end. The header copy per exit is the mini-import (Task 1.4). |
| T12 fault-in triggers | **Partly kept**: snapshot/diagnostic full import (Stage 3); machine writes to VP state removed structurally — `Vcpu` is an owned handle the vCPU thread holds, so nothing else can address VP state (Task 1.1, 1.5); the census the front end reads is published by the thread when it parks (Task 1.5). |
| T13 port answers under freshness | **Absorbed** by the mask rule (Task 1.4). |
| T14 Stage 2 gate | **Superseded**. |
| Carry-forward a: stale latched PIC pin after injection | **Kept** as a detail of the ExtINT path: the INTA reconciles the pin (Task 1.6, Step 3). |
| Carry-forward b: burst path unreachable | **Replaced** by exit clustering (Stage 2). |
| Carry-forward d: freshness contract | **Kept verbatim** (Task 1.6). |
| Fix 1 (read-back skip) — done | Moot after Task 1.7 removes the slice end. |
| Fix 2 (dedup `set_tsc_deadline`) | **Dropped**: the LAPIC timer is the hypervisor's in fast mode. |
| Fix 3 (boundary due tick) | **Moot**: no scheduler boundaries in fast mode. |
| Fix 4 (icache epoch) | **Kept**, Stage 2 (needed by clustering stretches). |
| Fast-path plan T5 (`TimeBase`) | **Superseded** by `VmClockSource` over the tree's `VmInstant` (Task 1.2). |
| Fast-path T6 (exit elision) / T7 (narrow exchange) / T8 (DLX gate) | Stage 2 / Task 1.4 / Task 1.9 (G5). |
| Gate hygiene debt (`xtask ci` never ran on 374e803, 7492d80) | **Task 0.1, first thing.** |
| Peer session's zeroed segment attributes (state.rs diagnostic checks CS only) | **Task 1.4, Step 5**: segment import validates every segment, with negative controls. |
| MONITORX/MWAITX cluster; `exception()`/`interrupt()` R5 unification | **Out of scope**; named here so they are not forgotten. |
| `WHP_ALL_SHADOW` bisection mode and `WHP_MIN_SLICE_US` | **Dropped** with the slice engine (Task 1.7 deletes both environment reads, engine.rs:209 and engine.rs:2914, with the code they select); precise mode is the replacement for the bisection. |
| `alpine_probe`, `dlx_whp`, `step_bench`, `compute_bench`, `alpine_bench` examples | **Kept**; adapted to the fast machine's verbs in Task 1.8. |

---

## Stage 0 — probes and baseline (no behaviour change to any engine)

### Task 0.1: Gate hygiene, then the baseline the plan measures from

**Files:**
- Create: `docs/perf/2026-09-03-whp-vmm-baseline.md`
- Modify: `rusty_box_whp_engine/examples/alpine_probe.rs` (an `ALPINE_ENGINE=interpreter` switch, so the interpreter baseline goes through the SAME harness as the WHP one)
- Read: `rusty_box_whp_engine/examples/alpine_bench.rs:225-250` (how it selects an engine from `ALPINE_ENGINE`), `rusty_box_whp_engine/examples/dlx_whp.rs`

**Interfaces:** none produced. Consumes the existing harnesses.

- [ ] **Step 1: Run the full gate suite on head and record the outcome**

Run: `cargo xtask ci`
Expected: green. If any step is red, STOP this task and report the failing step verbatim to the user — the two commits `374e803` and `7492d80` never saw the gates and the fix is theirs to authorise, not this plan's to guess.

- [ ] **Step 2: Teach `alpine_probe` the interpreter**

`alpine_probe.rs` builds only `builder.build_on::<WhpEngine>()` (line 193) and its step loop (from line 214) reads `machine.engine().exits()/census()/inject_census()/platform_counters()`. Factor the loop into `fn drive<E: SliceEngine<()>>(machine: &mut Emulator<(), E>, engine_line: impl FnMut(&E) -> String, limit: Duration) -> Outcome` where `engine_line` renders the engine-specific part of the 10-second report; `main` reads `ALPINE_ENGINE` exactly as `alpine_bench.rs:225` does (`"interpreter"` → `builder.build()` with an empty `engine_line`; anything else → `build_on::<WhpEngine>()` with today's line). Every `RESULT` line keeps its name; the interpreter run prints `RESULT exits=none`. Build: `cargo build --release -p rusty_box_whp_engine --example alpine_probe`.

- [ ] **Step 3: Record the ISO**

Run: `Get-FileHash -Algorithm SHA256 <path to the original Alpine ISO>`. The name and hash go into the baseline document's header. If the only ISO on the machine is the no-quiet copy, STOP and ask the user for the original — nothing below is comparable without it.

- [ ] **Steps 4–6: One interleaved sweep — Alpine on both engines and DLX, three rounds**

**This host is shared and other projects build on it without warning, so the arms must alternate.** The project's standing rule (memory `perf-measurement-methodology`) is: absolute wall-clock here is not comparable across runs or days — thermal drift alone once produced a phantom 8 % win that vanished under controlled measurement — so A/B arms are interleaved round-robin inside ONE session and compared by median, with a noise floor of about 3.7 %. Blocked runs (three of one arm, then three of the other) hand every background build that happens to land during one block entirely to that block, which is how a phantom result is manufactured.

Run three rounds; each round is `alpine_probe` on WHP, then `alpine_probe` with `ALPINE_ENGINE=interpreter`, then `dlx_whp`. Strictly serial — one partition per process and one CPU-hungry alarm thread per run. Before and after every run, record the host CPU load and the number of live `rustc`/`link`/`cl` processes, so contention appears in the data rather than being assumed absent; a run whose neighbours disagree by more than the noise floor is reported with its load figures beside it, never silently averaged in.

```powershell
$env:ALPINE_ISO="<the original ISO>"; $env:ALPINE_PROBE_PATIENCE_SECS="300"; $env:DLX_WHP_PATIENCE_SECS="60"
```
Record from each Alpine log: every `milestone` line's wall, the `RESULT exits=` line (exits by class), and from `RESULT platform …` both `runtime_total_ms` and `hypervisor_ms`. The head engine's in-run share is `(runtime_total_ms − hypervisor_ms) / runtime_total_ms` — `HypervisorRuntime100ns` is the hypervisor's OVERHEAD share of the total (`rusty_box_whp/src/partition.rs:395-400`), so guest time is the difference; and neither is wall time, which is why Stage 1 adds a literal in-run clock (Task 1.5). From each DLX log record wall to `login:`. The interpreter's median wall to `login:` is what Task 1.9 compares against; the 2026-08-29 figure of 69–71 s came from a different harness and is superseded.

- [ ] **Step 7: Windows 7 at head is user-timed in the GUI**

An agent must not drive the egui GUI. Ask the user for three stopwatch timings at head (1024 MiB, W7 ISO as CD), each with TWO stamps: start = the click on "Power On VM"; stamp A = the first frame showing the Windows boot logo; stamp B = the first frame showing the edition-selection list. Both columns go into the W7 row. If the user cannot run it now, the row says "owed" and G3 stays unjudged until it is filled — the only prior figure, 6–8 minutes, was measured FROM THE LOGO on the pre-campaign tree, so without stamp A the new number cannot be compared to the old one at all.

- [ ] **Step 8: Write the baseline document**

`docs/perf/2026-09-03-whp-vmm-baseline.md` with this exact shape:

```markdown
# WHP VMM-shape campaign — baseline at 7728086

Host: <CPU model>, Windows 11 build <build>. Defender real-time scanning: <on/off>.
Other hypervisor partitions during the runs: none. No build ran concurrently.
Measured <date> with `alpine_probe` / `dlx_whp`; no GUI. Three runs unless stated.
ISO: `<file name>`, SHA-256 `<hash>` (the original, `quiet` intact).

| Guest | Engine | Milestone | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|---|---|
| Alpine | whp (slice engine) | `login:` | <s or "not in 300 s"> | … | … | … |
| Alpine | whp | in-run share (guest/total runtime, cumulative) | … | … | … | … |
| Alpine | whp | exits by class at end (`RESULT exits=`) | … | … | … | — |
| Alpine | interpreter | `login:` | … | … | … | … |
| DLX | whp | `login:` | … | … | … | … |
| Windows 7 | whp (GUI, user-timed) | power-on → boot logo | | | | |
| Windows 7 | whp (GUI, user-timed) | power-on → edition list | | | | |

## G0 — VMware Workstation on this host (user-measured)

Procedure: new VM, 1 vCPU, 256 MiB, the same Alpine ISO attached as CD, boot;
stopwatch from "power on" to the `login:` prompt, three times. Same for the
Windows 7 ISO with 1024 MiB, to the edition-selection list.

| Guest | Milestone | Run 1 | Run 2 | Run 3 | Median |
|---|---|---|---|---|---|
| Alpine | `login:` | | | | |
| Windows 7 | edition list | | | | |
```

Fill every cell this task measured. The G0 rows are the user's to fill — ask for them in the task report; **G2/G3 in Task 1.9 cannot be judged until they are filled.**

- [ ] **Step 9: Commit**

```bash
git add docs/perf/2026-09-03-whp-vmm-baseline.md rusty_box_whp_engine/examples/alpine_probe.rs
git commit -m "docs(whp): the baseline the VMM-shape plan measures from"
```

### Task 0.2a: The platform seam becomes its own crate

Doctrine R1 says a crate's `unsafe` count may never increase, and Task 0.2 binds six new
platform entry points. Both hold only if the `unsafe` lives in a crate whose job is to hold it.
This task performs that split — the conventional `-sys` shape, and the end state
`docs/safety-doctrine.md` R1 already names for this seam.

**Files:**
- Create: `rusty_box_whp_sys/Cargo.toml`, `rusty_box_whp_sys/src/lib.rs`
- Move (git mv, contents unchanged except the module paths): `rusty_box_whp/src/sys.rs` → `rusty_box_whp_sys/src/lib.rs`'s body, `rusty_box_whp/src/sys/windows.rs` → `rusty_box_whp_sys/src/windows.rs`, `rusty_box_whp/src/sys/unsupported.rs` → `rusty_box_whp_sys/src/unsupported.rs`, `rusty_box_whp/src/error.rs` → `rusty_box_whp_sys/src/error.rs`, `rusty_box_whp/src/vcpu.rs` → `rusty_box_whp_sys/src/vcpu.rs`
- Modify: `rusty_box_whp/src/lib.rs` (drop `mod sys/error/vcpu`; `use rusty_box_whp_sys as sys;` and re-export every type it re-exports today, so no downstream file changes), `rusty_box_whp/Cargo.toml` (depend on the new crate), `Cargo.toml` (workspace members), `xtask/src/ci.rs` (baselines)
- Test: the existing suites, unchanged; one new const assertion

**Interfaces:**
- Consumes: `rusty_box_core::GpaPerms` (the only external type the seam names); `crate::error::{WhpError, WhpErrorKind, WhpResult}` and `crate::vcpu::{Exit, InterruptRequest, Reg, SegmentRegister, TableRegister, ALL_REGS}` — the complete list of in-crate names `sys.rs`, `sys/windows.rs` and `sys/unsupported.rs` reference, which is why those two files move with the seam.
- Produces: `rusty_box_whp_sys` exporting exactly what `rusty_box_whp::sys` exports today plus the error and vcpu vocabulary. **`rusty_box_whp`'s public API does not change** — it re-exports the moved types under their existing paths, so `rusty_box_whp_whp_engine`, the examples and the tests compile untouched. Verify that claim with the compiler, not by reading.

**`map_borrowed` STAYS in `partition.rs`, and stays `pub unsafe fn`.** It performs no unsafe operation; its `unsafe` is a *signature marker* for an obligation only a caller can discharge — the hypervisor keeps host addresses that outlive the borrow, and `rusty_box_whp_engine`'s `map_window` (engine.rs:1084) is what discharges it. Moving it to the sys crate and presenting a "safe" `Partition::map_borrowed` would delete a live obligation from the type system while the hazard remained, which is the exact failure R1 exists to prevent. So `rusty_box_whp` ends this task at **one** `unsafe` token, not zero, and it does **not** gain `#![forbid(unsafe_code)]`; it keeps the workspace lint table's `deny` plus the one `#[expect(unsafe_code, reason = …)]` already on that function. `xtask/src/ci.rs:91` already records why that token exists — leave the comment, correct its arithmetic.

- [ ] **Step 1: Write the failing assertion**

The test is the ratchet, not a `forbid`. In `xtask/src/ci.rs`, replace `("rusty_box_whp/src", 38)` with the two rows the split must produce: `("rusty_box_whp/src", 1)` and `("rusty_box_whp_sys/src", 37)`.

- [ ] **Step 2: Run to verify it fails** — `cargo xtask ci`. Expected: the doctrine-ratchets step fails, because `rusty_box_whp/src` still holds 38 tokens and `rusty_box_whp_sys/` does not exist. Read the step's reported counts — they, not this plan, are the authority on the two numbers; if the split lands different totals, use the measured ones and say so in the commit.

- [ ] **Step 3: Perform the split**

`git mv` the five files (history matters; do not retype them). The new crate's `lib.rs` is today's `sys.rs` with `mod imp;` becoming the two `#[cfg]`-selected modules and `pub mod error; pub mod vcpu;` added. Its `Cargo.toml` mirrors `rusty_box_whp`'s: same `[target.'cfg(windows)'.dependencies.windows-sys]` block (version 0.61, feature `Win32_System_Hypervisor`), `rusty_box_core` with `std`, `tracing`, workspace inheritance for version/edition/license/authors/repository/homepage, and NO `[lints] workspace = true` (the workspace table denies `unsafe_code`, which this crate exists to allow; instead it carries its own crate-level `#![expect(unsafe_code, reason = "…")]` exactly as `rusty_box_whp/src/sys/windows.rs` does today). `map_borrowed` does NOT move (see above). Two consequences the survey measured, both expected and both to be stated in the commit message rather than left for a reviewer to discover: (1) **visibility widens.** 27 `sys` verbs, 4 seam types, and `WhpError::{contract, host_memory}` are `pub(crate)` today; once the wrapper is a different crate they must be `pub`. That is what a `-sys` crate is — its surface is public to its wrapper by construction — but widen only what the compiler demands, one item at a time, never a blanket `pub`. (2) **one rustdoc link breaks.** `vcpu.rs:243` has an intra-doc link to `crate::LocalApicMode::None`, which stays behind in `partition.rs`; demote it to plain text on the move. (`cargo doc` is not in the gate suite, so this is a correctness-of-prose fix, not a build break.)

- [ ] **Step 4: Verify nothing downstream moved**

Run: `cargo check --release -p rusty_box_whp && cargo check --release -p rusty_box_whp_engine && cargo check --release -p rusty_box_whp --examples && cargo check --release -p rusty_box_whp_engine --examples`
Expected: green with ZERO edits outside the four files named in Files. If any downstream file needs a change, the re-export list in `rusty_box_whp/src/lib.rs` is incomplete — fix the re-exports, not the caller.

- [ ] **Step 5: Move the baselines**

The two baseline rows were written in Step 1; confirm the ratchet now reports exactly them, and correct them to the measured values if not.

**And add a test step for the new crate.** `vcpu.rs` carries **12** unit tests today (`error.rs` carries none), and the "WHP leaf tests" step (`cargo test --release -p rusty_box_whp`, ci.rs:521) will no longer reach them once that file moves — a test that stops running is the "compiles clean ≠ reached" trap. Add a `Step { name: "WHP sys tests", args: &["test", "--release", "-p", "rusty_box_whp_sys"], … }` immediately before it, and confirm all 12 appear in that step's output by name.

- [ ] **Step 6: Gates and commit**

Run: `cargo xtask ci`. Then:
```bash
git add rusty_box_whp_sys/ rusty_box_whp/ xtask/src/ci.rs Cargo.toml
git commit -m "refactor(whp): the platform seam is its own crate"
```

### Task 0.2: Bind the platform verbs the design needs

The binding wraps 24 WHP entry points today (`grep -o "WHv[A-Za-z]*" rusty_box_whp_sys/src/windows.rs | sort -u`, post-0.2a). The probes and Stage 1 need these additional ones; binding them first keeps every later task free of `unsafe`.

**Files:**
(Paths below are post-0.2a: the platform seam and the ABI vocabulary live in `rusty_box_whp_sys`, the typed wrapper in `rusty_box_whp`.)
- Modify: `rusty_box_whp_sys/Cargo.toml` and `rusty_box_whp/Cargo.toml` (add `bitflags.workspace = true` to whichever crate declares `SyntheticFeatures` — the workspace pins `bitflags = "2"`, resolved 2.13.1, already used by `rusty_box`), `rusty_box_whp_sys/src/lib.rs` (`CapabilityCode`, `PropertyCode`, `RegisterValue`, the `ImpSignatures` struct), `rusty_box_whp_sys/src/windows.rs` (the FFI calls), `rusty_box_whp_sys/src/unsupported.rs` (the non-Windows stubs must stay in signature lock-step — `ImpSignatures` enforces it), `rusty_box_whp_sys/src/vcpu.rs` (`Reg`, new types), `rusty_box_whp/src/caps.rs` (`ExtendedVmExits`, `SyntheticFeatures`, `Capabilities`), `rusty_box_whp/src/partition.rs` (verbs), `rusty_box_whp/src/lib.rs` (re-exports)
- Test: `rusty_box_whp_sys/src/vcpu.rs` tests (pure encoding), `rusty_box_whp/src/caps.rs` tests, `rusty_box_whp/src/partition.rs` hypervisor-gated tests. Both crates' suites run: `cargo test --release -p rusty_box_whp_sys && cargo test --release -p rusty_box_whp`.

**Interfaces:**
- Consumes: `sys::set_property(handle, PropertyCode, u64)`, `sys::get_words/set_words`, `RegisterValue` (sys.rs:104), `Reg` (vcpu.rs:14), `ExtendedVmExits` (caps.rs:115), `LocalApicMode::{None, XApic, X2Apic}` (partition.rs:39).
- Produces (later tasks rely on these exact names):

```rust
// vcpu.rs
pub enum Reg { /* existing … */ PendingEvent, /* 128-bit; WHvRegisterPendingEvent = 0x80000002 */ ApicTpr, /* WHvX64RegisterApicTpr = 0x3008; probe-only, to record whether a named APIC register write is refused */ }
/// `WHV_X64_PENDING_EXT_INT_EVENT`: EventPending bit 0, EventType bits 1..4 (= 5 for ExtInt), Vector bits 8..16.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PendingExtIntEvent { pub vector: u8 }
impl PendingExtIntEvent {
    pub const fn as_words(self) -> [u64; 2] { [1 | (5 << 1) | ((self.vector as u64) << 8), 0] }
    pub const fn from_words(words: [u64; 2]) -> Option<Self> // None unless bit 0 set and type == 5
}
/// `WHV_X64_APIC_WRITE_TYPE`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApicWriteType { Ldr, Dfr, Svr, Lint0, Lint1 }
pub enum ExitReason { /* existing … but: */ ApicWriteTrap { register: ApicWriteType, value: u64 }, /* was payload-less */ }
/// The 4096-byte `InterruptControllerState2` page. Its layout is QEMU's
/// `whpx_lapic_state`: one 32-bit register per 16-byte slot, indexed by
/// `offset >> 4`, so a register's BYTE offset in the page EQUALS its xAPIC
/// MMIO offset (TPR at 0x80 is bytes 0x80..0x84; 0x84..0x90 are padding).
/// The first 1 KiB is meaningful; the set requires exactly 4096 bytes.
pub struct ApicStatePage(pub Box<[u8; 4096]>);   // Box: the page must not live on a test's stack
impl ApicStatePage {
    pub fn zeroed() -> Self;
    pub fn register(&self, offset: u16) -> u32;            // offset is the MMIO offset, 16-byte aligned
    pub fn set_register(&mut self, offset: u16, value: u32);
}

// caps.rs
pub struct ExtendedVmExits { /* existing … plus */ pub apic_write_lint0_trap: bool, pub apic_write_lint1_trap: bool, pub apic_write_svr_trap: bool, pub apic_init_sipi_trap: bool, pub apic_smi_trap: bool, pub hypercall: bool }
bitflags::bitflags! {
    /// `WHV_SYNTHETIC_PROCESSOR_FEATURES` bank 0. One flag per header bitfield, in the
    /// header's order; the composite at the end is OpenVMM's VTL0 set (research report
    /// 04 §1.2), the one the design exposes. `repr(transparent)` because the word crosses
    /// the FFI as the bank's `Bank0` field (bitflags does not add it on its own).
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct SyntheticFeatures: u64 {
        /// CPUID leaves 0x40000000 and 0x40000001 are supported.
        const HYPERVISOR_PRESENT = 1 << 0;
        /// CPUID leaves 0x40000000–0x40000006 (the Hv#1 interface).
        const HV1 = 1 << 1;
        const ACCESS_VP_RUNTIME_REG = 1 << 2;
        const ACCESS_PARTITION_REFERENCE_COUNTER = 1 << 3;
        const ACCESS_SYNIC_REGS = 1 << 4;
        const ACCESS_SYNTHETIC_TIMER_REGS = 1 << 5;
        /// The VP assist page and, on x64, the APIC EOI/ICR/TPR MSRs.
        const ACCESS_INTR_CTRL_REGS = 1 << 6;
        const ACCESS_HYPERCALL_REGS = 1 << 7;
        const ACCESS_VP_INDEX = 1 << 8;
        const ACCESS_PARTITION_REFERENCE_TSC = 1 << 9;
        const ACCESS_GUEST_IDLE_REG = 1 << 10;
        const ACCESS_FREQUENCY_REGS = 1 << 11;
        const EXTENDED_GVA_RANGES_FOR_FLUSH = 1 << 15;
        const FAST_HYPERCALL_OUTPUT = 1 << 18;
        const DIRECT_SYNTHETIC_TIMERS = 1 << 22;
        const EXTENDED_PROCESSOR_MASKS = 1 << 24;
        const TB_FLUSH_HYPERCALLS = 1 << 25;
        const SYNTHETIC_CLUSTER_IPI = 1 << 26;
        const NOTIFY_LONG_SPIN_WAIT = 1 << 27;
        const QUERY_NUMA_DISTANCE = 1 << 28;
        const SIGNAL_EVENTS = 1 << 29;
        const RETARGET_DEVICE_INTERRUPT = 1 << 30;
        /// What OpenVMM grants a VTL0 guest with the offloaded APIC — every bit above.
        const OPENVMM_VTL0 = Self::HYPERVISOR_PRESENT.bits() | Self::HV1.bits()
            | Self::ACCESS_VP_RUNTIME_REG.bits() | Self::ACCESS_PARTITION_REFERENCE_COUNTER.bits()
            | Self::ACCESS_SYNIC_REGS.bits() | Self::ACCESS_SYNTHETIC_TIMER_REGS.bits()
            | Self::ACCESS_INTR_CTRL_REGS.bits() | Self::ACCESS_HYPERCALL_REGS.bits()
            | Self::ACCESS_VP_INDEX.bits() | Self::ACCESS_PARTITION_REFERENCE_TSC.bits()
            | Self::ACCESS_GUEST_IDLE_REG.bits() | Self::ACCESS_FREQUENCY_REGS.bits()
            | Self::EXTENDED_GVA_RANGES_FOR_FLUSH.bits() | Self::FAST_HYPERCALL_OUTPUT.bits()
            | Self::DIRECT_SYNTHETIC_TIMERS.bits() | Self::EXTENDED_PROCESSOR_MASKS.bits()
            | Self::TB_FLUSH_HYPERCALLS.bits() | Self::SYNTHETIC_CLUSTER_IPI.bits()
            | Self::NOTIFY_LONG_SPIN_WAIT.bits() | Self::QUERY_NUMA_DISTANCE.bits()
            | Self::SIGNAL_EVENTS.bits() | Self::RETARGET_DEVICE_INTERRUPT.bits();
    }
}
// Reading the host's bank: `SyntheticFeatures::from_bits_retain(word)` — retain, not
// truncate, so a bit this SDK's header does not name (OpenVMM's copy of the ABI already has
// one more) survives into the probe report, where `iter_names()` prints the known ones and
// `bits() & !Self::all().bits()` the unknown remainder. Asking for a bank: `bank.bits()`.
// A bank the host does not allow is a `Contract` error before the call, computed as
// `wanted.difference(allowed)` — the flags that would have been refused, by name.
pub struct Capabilities { /* existing … plus */ pub synthetic_features: SyntheticFeatures, pub processor_clock_hz: u64, pub interrupt_clock_hz: u64, pub tsc_deadline_timer: bool }

// partition.rs
impl PartitionConfig {
    pub fn synthetic_features(&mut self, bank0: SyntheticFeatures) -> WhpResult<&mut Self>; // 16-byte payload: BanksCount=1, Bank0
}
impl Partition {
    pub fn reference_time_100ns(&self) -> WhpResult<u64>;                 // WHvGetPartitionProperty(ReferenceTime)
    pub fn suspend_time(&self) -> WhpResult<()>;                          // WHvSuspendPartitionTime
    pub fn resume_time(&self) -> WhpResult<()>;                           // WHvResumePartitionTime
    pub fn read_apic_state(&self, index: u32, page: &mut ApicStatePage) -> WhpResult<()>;  // WHvGetVirtualProcessorState(InterruptControllerState2 = 0x1000)
    pub fn write_apic_state(&self, index: u32, page: &ApicStatePage) -> WhpResult<()>;
    pub fn read_words128(&self, index: u32, reg: Reg) -> WhpResult<[u64; 2]>;            // for PendingEvent
    pub fn write_words128(&self, index: u32, reg: Reg, words: [u64; 2]) -> WhpResult<()>;
}
```
(Task 1.1 moves the four per-processor verbs onto `Vcpu`; they are on `Partition` here because `Vcpu` does not exist yet.)

- [ ] **Step 1: Confirm every bit position against the header before writing a constant**

Run (read-only): `grep -n "X64ApicWriteLint0ExitTrap\|GpaAccessFaultExit\|HypervisorPresent:1\|DirectSyntheticTimers\|WHvRegisterPendingEvent \|WHvVirtualProcessorStateTypeInterruptControllerState2\|WHvPartitionPropertyCodeSyntheticProcessorFeaturesBanks\|WHvPartitionPropertyCodeReferenceTime\|WHvCapabilityCodeInterruptClockFrequency\|WHvCapabilityCodeProcessorClockFrequency\|TscDeadlineTmrSupport" "C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um\WinHvPlatformDefs.h"`
Expected: the `WHV_EXTENDED_VM_EXITS` bitfield order is `X64CpuidExit, X64MsrExit, ExceptionExit, X64RdtscExit, X64ApicSmiExitTrap, HypercallExit, X64ApicInitSipiExitTrap, X64ApicWriteLint0ExitTrap, X64ApicWriteLint1ExitTrap, X64ApicWriteSvrExitTrap, UnknownSynicConnection, RetargetUnknownVpciDevice, X64ApicWriteLdrExitTrap, X64ApicWriteDfrExitTrap, GpaAccessFaultExit` (bits 0..14; `caps.rs`'s existing test pins `gpa_access_fault` at bit 14). The synthetic-feature bit numbers above are the header's field order (report 01 §C.1). If any differs, the header wins — fix the constant and note it in the commit.

- [ ] **Step 2: Write the failing encoding tests** (pure, hypervisor-free) in `vcpu.rs` and `caps.rs`:

```rust
#[test]
fn a_pending_ext_int_event_encodes_as_the_header_lays_it_out() {
    let event = PendingExtIntEvent { vector: 0x20 };
    assert_eq!(event.as_words(), [0x0000_0000_0000_200B, 0]); // pending | type 5 << 1 | 0x20 << 8
    assert_eq!(PendingExtIntEvent::from_words([0x200B, 0]), Some(event));
    assert_eq!(PendingExtIntEvent::from_words([0, 0]), None, "an empty slot is no event");
    assert_eq!(PendingExtIntEvent::from_words([0x2001, 0]), None, "an exception event is not an ExtInt");
}

#[test]
fn the_apic_state_page_keeps_a_register_at_its_mmio_offset_in_a_sixteen_byte_slot() {
    let mut page = ApicStatePage::zeroed();
    page.set_register(0x80, 0xF0);            // TPR
    assert_eq!(&page.0[0x80..0x84], &0xF0u32.to_le_bytes(), "slot index 0x80 >> 4 = 8, times 16 bytes = byte 0x80");
    assert_eq!(page.0[0x84..0x90], [0; 12], "the 12 pad bytes of the slot stay zero");
    assert_eq!(page.register(0x80), 0xF0);
    page.set_register(0x90, 0x11);            // APR, the next slot
    assert_eq!(page.register(0x80), 0xF0, "a neighbouring slot does not disturb it");
}

#[test]
fn the_extended_exits_word_carries_the_apic_traps_where_the_header_puts_them() {
    let asked = ExtendedVmExits { apic_write_lint0_trap: true, hypercall: true, ..ExtendedVmExits::default() };
    assert_eq!(asked.as_word(), (1 << 7) | (1 << 5));
    assert_eq!(ExtendedVmExits::from_word(asked.as_word()), asked);
}

#[test]
fn the_openvmm_vtl0_synthetic_set_is_every_named_flag_and_nothing_else() {
    // A ratchet, not a header check: the composite is defined as the union of the named
    // flags, so this can only fail when a flag is added without joining the composite.
    assert_eq!(SyntheticFeatures::OPENVMM_VTL0, SyntheticFeatures::all());
    // The check that catches a DROPPED constituent. bitflags 2's `IterNames` yields a
    // defined flag only while it still covers bits no earlier flag has yielded
    // (`src/iter.rs`: "When flags fully overlap, such as in convenience flags that are a
    // shorthand for others, we won't yield both flags"), so the composite defined last is
    // NOT yielded after its 22 constituents.
    assert_eq!(SyntheticFeatures::OPENVMM_VTL0.iter_names().count(), 22);
    // The header's bit positions, spot-checked where the numbering has gaps.
    assert_eq!(SyntheticFeatures::EXTENDED_GVA_RANGES_FOR_FLUSH.bits(), 1 << 15);
    assert_eq!(SyntheticFeatures::FAST_HYPERCALL_OUTPUT.bits(), 1 << 18);
    assert_eq!(SyntheticFeatures::DIRECT_SYNTHETIC_TIMERS.bits(), 1 << 22);
    assert_eq!(SyntheticFeatures::RETARGET_DEVICE_INTERRUPT.bits(), 1 << 30);
}

#[test]
fn a_host_bank_with_a_bit_this_header_does_not_name_is_kept_not_dropped() {
    // A newer host may set a bit our SDK does not define (OpenVMM's ABI copy already has one
    // more). The capability read keeps it so the probe report can show it.
    let word = SyntheticFeatures::HV1.bits() | (1 << 40);
    let bank = SyntheticFeatures::from_bits_retain(word);
    assert!(bank.contains(SyntheticFeatures::HV1));
    assert_eq!(bank.bits() & !SyntheticFeatures::all().bits(), 1 << 40, "the unknown bit survives");
    assert_eq!(SyntheticFeatures::from_bits(word), None, "and strict parsing refuses it");
    assert_eq!(SyntheticFeatures::from_bits_truncate(word), SyntheticFeatures::HV1);
}

#[test]
fn asking_for_more_than_the_host_allows_names_the_refused_flags() {
    let allowed = SyntheticFeatures::HYPERVISOR_PRESENT | SyntheticFeatures::HV1;
    let wanted = SyntheticFeatures::OPENVMM_VTL0;
    let refused = wanted.difference(allowed);
    assert!(refused.contains(SyntheticFeatures::ACCESS_SYNTHETIC_TIMER_REGS));
    assert!(!refused.contains(SyntheticFeatures::HV1));
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test --release -p rusty_box_whp`
Expected: compile errors naming `PendingExtIntEvent`, `ApicStatePage`, `apic_write_lint0_trap`, `SyntheticFeatures`.

- [ ] **Step 4: Implement the types and the FFI**

In `sys.rs`: add `CapabilityCode::{SyntheticProcessorFeaturesBanks, ProcessorClockFrequency, InterruptClockFrequency, ProcessorFeaturesBanks}`, `PropertyCode::{SyntheticProcessorFeaturesBanks, ReferenceTime}`, a `RegisterValue::Words128([u64; 2])` shape and `shape_of(Reg::PendingEvent)` returning it, plus `ImpSignatures` fields for `get_property_word`, `set_property_bytes`, `suspend_time`, `resume_time`, `get_vp_state`, `set_vp_state`. In `sys/windows.rs` call `WHvGetPartitionProperty`, `WHvSetPartitionProperty` with a 16-byte buffer (`WHV_SYNTHETIC_PROCESSOR_FEATURES_BANKS { BanksCount: 1, Reserved0: 0, Bank0 }`), `WHvSuspendPartitionTime`, `WHvResumePartitionTime`, `WHvGetVirtualProcessorState`/`WHvSetVirtualProcessorState` with `WHvVirtualProcessorStateTypeInterruptControllerState2` and a 4096-byte buffer (the set requires exactly 4096 — report 01 §A.3). Each new `unsafe` block names its invariant owner as the existing ones do; the token count rises in `rusty_box_whp_sys/src` and nowhere else — **raise that crate's baseline in `xtask/src/ci.rs` by exactly the new count in the same commit and state the count in the message**. `rusty_box_whp/src` stays at 1 — `map_borrowed`'s signature, and nothing else — so if any edit here would add an `unsafe` there, the edit is in the wrong crate. Mirror every signature in `unsupported.rs`. In `caps.rs` read the two frequencies and the synthetic bank into `Capabilities`; `tsc_deadline_timer` comes from `WHV_PROCESSOR_FEATURES1.TscDeadlineTmrSupport` via `WHvCapabilityCodeProcessorFeaturesBanks` bank 1 — locate the bit with `grep -n "TscDeadlineTmrSupport" WinHvPlatformDefs.h` and count its position in `WHV_PROCESSOR_FEATURES1`.

- [ ] **Step 5: Run the encoding tests; then the leaf's hypervisor-gated tests**

Run: `cargo test --release -p rusty_box_whp`
Expected: PASS, including a new gated test in `partition.rs`:

```rust
/// The state page round-trips through the platform unchanged, at its exact size.
#[test]
fn the_apic_state_page_round_trips_through_the_platform() {
    if !crate::hypervisor_present().unwrap_or(false) { return; }
    let _turn = a_turn_on_the_hardware();
    let mut config = PartitionConfig::new().unwrap();
    config.processor_count(1).unwrap().local_apic(LocalApicMode::X2Apic).unwrap();
    let mut partition = config.setup().unwrap();
    partition.create_processor(0).unwrap();
    let mut page = ApicStatePage::zeroed();
    partition.read_apic_state(0, &mut page).expect("a fresh processor's APIC page is readable");
    assert_ne!(page.register(0x30), 0, "a fresh VP's version register is populated; the page is not a zero buffer");
    println!("P2: hypervisor LAPIC version register = {:#x}", page.register(0x30)); // recorded in the probe document, not asserted
    partition.write_apic_state(0, &page).expect("the same page is accepted back at its exact size");
    let mut back = ApicStatePage::zeroed();
    partition.read_apic_state(0, &mut back).expect("readable again");
    assert_eq!(page.0[..1024], back.0[..1024], "the first KiB — every register the model owns — survives the round trip");
}
```

- [ ] **Step 6: Gates and commit**

Run: `cargo xtask ci`. Then:
```bash
git add rusty_box_whp/ xtask/src/ci.rs
git commit -m "feat(whp): bind the APIC state page, partition time, the synthetic feature bank and the pending event"
```

### Task 0.3: Probes P1–P8, measured on this host, recorded in the repo

**Files:**
- Modify: `rusty_box_whp/examples/whp_probe.rs` (add `q11`–`q18` beside `q1`–`q10`; reuse `Finding` (line 50), `Guest` (139: `new(configure)`, `load(code)`, `run()`, `peek(offset)`), `run_with_rescue` (885), `install_handler` (996), `wake_attempt` (919); the `layout` module's `CODE`, `HANDLER`, `RAM`)
- Create: `docs/whp-platform-probe-2026-09-03.md` (same shape as `docs/whp-platform-probe-2026-08-27.md`: host table, one `### N.` section per question with a result table, "What this changes" table)

**Interfaces:** consumes Task 0.2's verbs. Produces the recorded answers the spec's §7 risks depend on; Task 1.6 reads P1's and P5's answers, Task 1.5 reads P3's.

- [ ] **Step 1: P1 — halt-suspend is clearable in x2APIC mode and wakes a halted VP for a pending ExtINT**

Add `q11_halt_suspend_clear_under_x2apic()`. Guest (real mode, hand-assembled as the file's others): `install_handler(vector 0x20)` (writes `MARK` to the debug port then `iret`); code `sti; hlt; hlt`. Partition: `Guest::new(|c| c.local_apic(LocalApicMode::X2Apic))`. Run 1: `run_with_rescue(200 ms)` — expected to park (rescued). Then from the main thread: `write_words128(0, Reg::PendingEvent, PendingExtIntEvent{vector:0x20}.as_words())` and `set_internal_activity(0, InternalActivity{halt_suspend:false, ..})`, in that order. Run 2: `run_with_rescue(200 ms)`. **The finding is a table with one row per run**: `exit reason`, `elapsed`, `rescued`, `handler ran` (the port write exited / `MARK` observed) — so "never woke" is distinguishable from "woke on the wrong run". Control A: the same without the activity write (expect run 2 parks until rescue — QEMU's `whpx_vcpu_kick_out_of_hlt` exists for exactly this). Control B: `LocalApicMode::None` — the earlier probe's `ACCESS_DENIED` on the activity write must reproduce, proving the register refusal was mode-specific. Each control gets the same per-run rows.

- [ ] **Step 2: P2 — the state page round-trips through the platform, and whether named APIC writes are refused**

The model half of P2 is Stage 3's conversion; here the platform half: `read_apic_state` → `write_apic_state` → `read_apic_state` bit-identical for the first 1 KiB (Task 0.2 Step 5's test, recorded under P2 together with the version register value it printed). In the same experiment attempt `write_reg(0, Reg::ApicTpr, 0x20)` on the stopped VP and record the outcome: Hyperlight reports `ACCESS_DENIED` for named APIC register writes on its hosts while emulation is on, and the SDK header advertises the names — which of the two this host does decides whether the page is the ONLY write path here.

- [ ] **Step 3: P3 — whether `X64Halt` exits occur in APIC mode**

`q12_halt_exit_under_x2apic()`: guest `cli; hlt`. Run with a 200 ms rescue. Finding: the exit reason the run returned with — `Halt` (an exit occurred) or `Canceled` (it parked until the rescue). Both reference VMMs keep a handler; Task 1.5's `Halt` arm is written from this answer.

- [ ] **Step 4: P4 — cancel stickiness under x2APIC** (`q13`): cancel a VP that is NOT running, then run a guest that would take 100 ms; finding: immediate `Canceled` return, or the full run. (H0's Q8 measured sticky under `None`.)

- [ ] **Step 5: P5 — the LINT0 write trap** (`q14`): partition with `extended_vm_exits{ apic_write_lint0_trap: true }`, guest writes `0x10700` (masked, ExtINT) to `0xFEE00350` via `mov [ds:0x0350], eax` with DS base 0xFEE0_0000 set through the platform's segment write before the run (`Guest::load` sets flat segments; write `Reg::Ds` with `base: 0xFEE0_0000` afterwards). Finding: `ExitReason::ApicWriteTrap { register: Lint0, value: 0x10700 }`. Then, with LVT0 left masked by the guest, inject a pending ExtINT event from the host exactly as P1 does and record whether the handler runs anyway: QEMU and OpenVMM inject without consulting LVT0 and report 05 could not establish whether the platform gates the event — the answer decides whether the fabric's LVT0 tracking (Task 1.6) is a fidelity nicety or a correctness requirement, and whether Task 1.6's gated LVT0 test is written.

- [ ] **Step 6: P6 — the synthetic bank is accepted and the guest sees Hv#1** (`q15`): `config.synthetic_features(SyntheticFeatures::OPENVMM_VTL0)` before setup with `X2Apic`; guest executes `cpuid` leaf 0x40000000 and 0x40000001 and 0x40000003, reporting EBX/ECX/EDX of the first and EAX of the others byte-by-byte on the debug port. Finding: "Microsoft Hv", "Hv#1", and the 0x40000003 EAX bits (must include HYPERCALL bit 5 and VP_INDEX bit 6 — Linux ignores the leaves without them, report 05 §1(e)). Also record `Capabilities::synthetic_features` as the host allows it.

- [ ] **Step 7: P7 — TSC-deadline and the APIC bus clock** (`q16`): record `Capabilities::{tsc_deadline_timer, interrupt_clock_hz, processor_clock_hz}`; under the bank, the guest reads MSR 0x40000023 (`HV_X64_MSR_APIC_FREQUENCY`) and 0x40000022 and reports them. Finding: equality of the two frequency pairs.

- [ ] **Step 8: P8 — partition time suspend freezes the TSC** (`q17`): create, run a guest that halts (`None` mode so it exits), `read_reg(Tsc)` → `suspend_time()` → host sleep 50 ms → `read_reg(Tsc)` again: equal; then run once more and read: advanced.

- [ ] **Step 9: Run the probe and write the document**

Run: `cargo run --release -p rusty_box_whp --example whp_probe *> probe-2026-09-03.log`
Write `docs/whp-platform-probe-2026-09-03.md` from the log. Every section states the measured result, not the expectation. **If P1's answer is "does not wake", stop after this task and report: Task 1.6 has a different shape (QEMU's Windows-10 workaround, spec §7).**

- [ ] **Step 10: Gates and commit**

Run: `cargo xtask ci` (the "WHP probe builds" step compiles the example). Then:
```bash
git add rusty_box_whp/examples/whp_probe.rs docs/whp-platform-probe-2026-09-03.md
git commit -m "probe(whp): the eight questions the VMM-shape design rests on, measured"
```

### Task 0.4: Register the divergences before the code that introduces them

**Files:**
- Modify: `docs/bochs-parity-divergences.md` (after H4)

**Interfaces:** none. Doctrine R7: an entry, never a code comment.

- [ ] **Step 1: Write entry H5 — on the fast engine, device time is host time and a tick is a unit**

Follow D5's shape exactly (`**Bochs:**`, `**rusty_box:**`, `### What the guest observes`, `### Why the divergence is the correct side`, `### Price of closing it`, `**Status:**`). Bochs: `pc_system.cc` advances `ticksTotal` once per retired instruction (`BX_TICK1`) and every device deadline is an instruction count. rusty_box fast mode: `ticksTotal` advances by host time × `ips` on the device thread; the interpreter is unchanged. Guest observes: time per instruction is the host's, not one tick; `RDTSC` and every device counter advance together at wall rate; a snapshot's absolute-tick deadlines mean the same instants. Correct side: the guest's TSC is the host's already (H2), and a device clock on any other base fails Linux's calibration and watchdog (research report 05 §4). Price of closing it: none — closing it is the interpreter.

- [ ] **Step 2: Write entry H6 — the 8042 serial delay is a one-shot on the fast engine**

Bochs `keyboard.cc bx_keyb_c::init` registers `timer_handler` continuous at `serial_delay` (150 µs) and `timer_handler` calls `periodic(1)` every fire. rusty_box fast mode: the same 150 µs delay armed as a one-shot whenever the controller has work for it — `timer_pending != 0 || irq1_requested || irq12_requested` — and re-armed after each fire while that still holds; the interpreter keeps the continuous timer. Guest observes: identical IRQ1/IRQ12 timing to within one period; nothing else. Correct side: on host time the continuous form is 6,700 device-thread wakes a second that call `periodic(1)` on a controller with nothing pending. Price of closing it: those wakes.

- [ ] **Step 3: Write entry H7 — the TSC's rate changes across an engine transfer**

Bochs `cpu/proc_ctrl.cc get_TSC` returns `bx_pc_system.time_ticks()`: one tick per retired instruction, always. rusty_box: on the fast engine the TSC is the host's (H2); a transfer to the precise mode continues the counter from the same value at one tick per instruction, and a transfer back resumes it at the host's rate. Guest observes: a rate discontinuity at the instant of transfer, with no backward step. Correct side: inherent to having a precise mode at all. Price of closing it: none — closing it would mean no precise mode. Status: open and deliberate; the memory notes already call this "the fingerprint of switching".

- [ ] **Step 4: Write entry H8 — a level-triggered IOAPIC entry is re-serviced on the guest's EOI**

Bochs `iodev/ioapic.cc receive_eoi` is a one-line `BX_DEBUG`; remote-IRR is never set; a level line still asserted after the handler's EOI is re-serviced only when the next `service_ioapic` scan runs (its `irr` bit is kept for level entries — `ioapic.rs service`). rusty_box precise mode: identical (`ioapic.rs receive_eoi` logs). Fast mode: the hypervisor LAPIC reports the EOI of a level vector as an `X64ApicEoi` exit, and `IrqFabric::resample_on_eoi(vector)` runs the scan at once when a level entry with that vector still has its line asserted, re-issuing the request — QEMU `ioapic_eoi_broadcast`. Guest observes on the fast engine: a level interrupt whose line stays high is re-delivered right after EOI instead of at the next unrelated scan, which is what the hardware does. Provenance: QEMU `hw/intc/ioapic.c ioapic_eoi_broadcast` (R7). Status: fast-engine only; proven hypervisor-free in Task 1.6, on hardware in Stage 2.

- [ ] **Step 5: Commit**

```bash
git add docs/bochs-parity-divergences.md
git commit -m "docs(parity): H5-H8 registered ahead of the code — host-time clock, 8042 one-shot, transfer TSC rate, EOI resample"
```

### Task 0.5: Stage 0 gate

- [ ] `docs/perf/2026-09-03-whp-vmm-baseline.md` has every cell this plan measures filled, the ISO's name and SHA-256, the host conditions; the G0 cells are filled by the user or the task report says they are still owed.
- [ ] `docs/whp-platform-probe-2026-09-03.md` records P1–P8 with results, per-run rows for P1. P1 = wakes (else the plan stops here, see Task 0.3 Step 9).
- [ ] `cargo xtask ci` green on the committed tree.
- [ ] Report to the user before starting Stage 1.

---

## Stage 1 — the fast-mode core

Stage 1 replaces the slice model. **Sequencing rule:** the WHP engine keeps booting through the slice loop after every task up to and including 1.6, because the hypervisor-LAPIC mode, the fabric backend and the thread wiring are all selected by `EmulatorConfig.device_clock == DeviceClock::HostTime`, a setting introduced in Task 1.3 that no production caller sets before Task 1.8 (the gated tests of 1.6 and 1.7 set it on their own machines). Task 1.7 deletes the slice loop and moves the engine's tests onto `FastMachine`; Task 1.8 flips the harnesses; from then on the WHP engine boots only through `FastMachine`. The interpreter's machine (`device_clock: Ticks`, the default) is untouched by every task.

### Task 1.1: A per-processor `Vcpu` handle, owned by the thread that runs it

**Files:**
- Modify: `rusty_box_whp/src/partition.rs` (new `Vcpu`, `VpCounters`; `Partition::run/cancel_run/read_reg/write_reg/read_regs/write_regs/read_registers/write_registers/read_segments/write_segments/read_tables/write_tables/read_xsave/write_xsave/inject/internal_activity/set_internal_activity/read_apic_state/write_apic_state/read_words128/write_words128/intercept_counters/runtime_counters/translate_gva` move onto `Vcpu`/`VpCounters` (the six per-shape verbs at partition.rs:659/735/760/743/773/786 included — they have live callers); `partition.rs`'s own gated tests — `the_platform_counts_the_guests_port_writes_apart_from_its_halts` (1204), the Task 0.2 page test and the rest — are callers too), `rusty_box_whp/src/lib.rs` (export, Send assertions)
- Modify every other caller: `rusty_box_whp_engine/src/engine.rs` (`Started` gains `vcpu: Vcpu`, declared BEFORE `partition` — it names the partition's handle, so the field-order comment at engine.rs:470-474 now covers it too), `state.rs`, `xsave.rs` (`refresh_from`/`write_to` take `&impl VpRegisters` instead of `(&Partition, index)`), `lib.rs` (DELETE the `Vp<'_>` adapter at lib.rs:80-107 — `Vcpu` implements `state::VpRegisters` directly, in `state.rs`), `lib.rs` tests; `rusty_box_whp_engine/examples/step_bench.rs` (`partition.read_reg`/`write_reg` at 189-204 → the `Vcpu`); `rusty_box_whp/examples/whp_probe.rs` (`Guest` gains `vcpu: Vcpu`; its `write_segments` at 167 moves with it)
- Test: `rusty_box_whp/src/lib.rs` const assertions and one `compile_fail` doctest; a gated test in `partition.rs`

**Interfaces:**
- Consumes: `RawPartition` (Copy, `Send + Sync`, asserted at pre-0.2a sys.rs:234-235, afterwards in the sys crate's `lib.rs`), `Canceller` (partition.rs:906) and `InterruptRequester` (936) and their lifetime rule (doc comment at partition.rs:556-569).
- Produces:

```rust
/// One virtual processor of a partition, owned by the thread that runs it.
///
/// Not `Copy` and not `Sync`, on purpose: every verb that reads or writes VP
/// state is here and nowhere else, so "only the vCPU thread touches VP state"
/// (`WHV_E_INVALID_VP_STATE` exists for the alternative) is a property of the
/// type, not a convention. What other threads may do to a running processor is
/// exactly the three copyable tokens: [`Canceller`], [`InterruptRequester`],
/// [`VpCounters`]. Obligation (the canceller's): a `Vcpu` must not outlive its
/// partition; the owner that moved it into a thread joins that thread before
/// the partition drops (Task 1.7's `FastMachine` does this in its `Drop` body).
#[derive(Debug)]
pub struct Vcpu { handle: RawPartition, index: u32, one_thread: PhantomData<Cell<()>> }  // PhantomData<Cell<()>>: Send, !Sync
impl Partition {
    /// Hand out the processor once. `Contract` error if it was never created or is already taken.
    pub fn take_vcpu(&mut self, index: u32) -> WhpResult<Vcpu>;
    /// A copyable token for the two counter reads, usable from any thread.
    pub fn vp_counters(&self, index: u32) -> WhpResult<VpCounters>;
}
#[derive(Clone, Copy, Debug)]
pub struct VpCounters { handle: RawPartition, index: u32 }
impl VpCounters {
    pub fn intercept_counters(&self) -> WhpResult<InterceptCounters>;
    pub fn runtime_counters(&self) -> WhpResult<RuntimeCounters>;
}
impl Vcpu {
    pub fn index(&self) -> u32;
    pub fn canceller(&self) -> Canceller;
    pub fn counters(&self) -> VpCounters;
    pub fn run(&self) -> WhpResult<Exit>;
    pub fn read_regs(&self, regs: &[Reg], out: &mut [u64]) -> WhpResult<()>;
    pub fn write_regs(&self, regs: &[Reg], words: &[u64]) -> WhpResult<()>;
    pub fn read_registers(&self, regs: &[Reg], out: &mut [RegisterValue]) -> WhpResult<()>;
    pub fn write_registers(&self, regs: &[Reg], values: &[RegisterValue]) -> WhpResult<()>;
    // The per-shape verbs, same shapes as today's `Partition` ones minus `index` (partition.rs:659-786):
    pub fn read_reg(&self, reg: Reg) -> WhpResult<u64>;
    pub fn write_reg(&self, reg: Reg, word: u64) -> WhpResult<()>;
    pub fn read_segments(&self, regs: &[Reg], out: &mut [SegmentRegister]) -> WhpResult<()>;
    pub fn write_segments(&self, regs: &[Reg], segments: &[SegmentRegister]) -> WhpResult<()>;
    pub fn read_tables(&self, regs: &[Reg], out: &mut [TableRegister]) -> WhpResult<()>;
    pub fn write_tables(&self, regs: &[Reg], tables: &[TableRegister]) -> WhpResult<()>;
    pub fn read_xsave(&self, out: &mut [u8]) -> WhpResult<usize>;
    pub fn write_xsave(&self, area: &[u8]) -> WhpResult<()>;
    pub fn read_apic_state(&self, page: &mut ApicStatePage) -> WhpResult<()>;
    pub fn write_apic_state(&self, page: &ApicStatePage) -> WhpResult<()>;
    pub fn inject(&self, event: PendingInterruption) -> WhpResult<()>;
    pub fn read_words128(&self, reg: Reg) -> WhpResult<[u64; 2]>;
    pub fn write_words128(&self, reg: Reg, words: [u64; 2]) -> WhpResult<()>;
    pub fn internal_activity(&self) -> WhpResult<InternalActivity>;
    pub fn set_internal_activity(&self, activity: InternalActivity) -> WhpResult<()>;
    pub fn translate_gva(&self, gva: u64) -> WhpResult<GvaTranslation>;
}
```
`Partition` keeps: `map/remap/unmap/unmap_subrange/mapped_regions/perms_at/bytes_at/bytes_at_mut/dirty_pages/create_processor/take_vcpu/vp_counters/canceller/interrupt_requester/request_interrupt/reference_time_100ns/suspend_time/resume_time/set_property_late`, plus a `taken: Vec<u32>` beside its `created` list. `run(&mut self, index)` and the other per-VP verbs on `Partition` are deleted (breaking; there are no external users). In the engine crate: `impl state::VpRegisters for Vcpu` (state.rs), and the trait gains `read_xsave`/`write_xsave` so the vector file goes through the same seam (`xsave.rs` `refresh_from`/`write_to` call them).

- [ ] **Step 1: Write the failing tests**

In `rusty_box_whp/src/lib.rs` beside the existing `assert_send::<Partition>()`: `assert_send::<Vcpu>(); assert_send_sync::<VpCounters>();`. On `Vcpu`'s doc comment, the negative half as a doctest:

````rust
/// ```compile_fail
/// # use rusty_box_whp::Vcpu;
/// fn share(vcpu: &Vcpu) { std::thread::scope(|s| { s.spawn(|| vcpu.index()); }); }
/// ```
````
(`cargo test -p rusty_box_whp` — the "WHP leaf tests" ci step runs doctests because it passes no `--lib`.) In `partition.rs` tests (gated):

```rust
/// A processor runs from a thread that is not the one holding the partition, and is handed out once.
#[test]
fn a_vcpu_runs_on_another_thread_while_the_partition_is_held_here() {
    if !crate::hypervisor_present().unwrap_or(false) { return; }
    let _turn = a_turn_on_the_hardware();
    let mut partition = halting_partition().expect("a partition");        // partition.rs:1020 — RAM mapped, VP 0 created
    let vcpu = partition.take_vcpu(0).expect("the processor is handed out once");
    load_real_mode_code(&mut partition, &vcpu, &[0xF4]).expect("hlt loaded at the entry point"); // the load half of run_until_halt_with (1062), factored out
    let ran = std::thread::spawn(move || vcpu.run()).join().unwrap().unwrap();
    assert!(matches!(ran.reason, ExitReason::Halt), "the guest halted on the other thread: {ran:?}");
    assert_eq!(partition.mapped_regions(), 1, "the partition was usable here throughout");
    assert!(partition.take_vcpu(0).is_err(), "a processor is taken once");
}
```

- [ ] **Step 2: Run to verify failure** — Run: `cargo test --release -p rusty_box_whp`. Expected: `Vcpu` not found.
- [ ] **Step 3: Implement `Vcpu` and `VpCounters`; move the verbs; delete the `Partition` copies; split `run_until_halt_with` (1062) into `load_real_mode_code(&mut Partition, &Vcpu, code)` and the run; update every caller** (`cargo check --release -p rusty_box_whp_engine` and `cargo check --release -p rusty_box_whp --examples` enumerate them — the compiler is the oracle, not grep).
- [ ] **Step 4: Run both crates' suites** — `cargo test --release -p rusty_box_whp && cargo test --release -p rusty_box_whp_engine`. Expected: all green (engine tests still run through the old slice loop, now via `Started.vcpu`).
- [ ] **Step 5: Gates and commit** — `cargo xtask ci`; `git commit -m "refactor(whp): a processor is an owned handle its thread holds, the partition is its map"`.

### Task 1.2: `VmClockSource` — the pausable host clock that earns the machine's ticks

**Files:**
- Create: `rusty_box_whp_engine/src/vm_clock.rs`
- Modify: `rusty_box_whp_engine/src/lib.rs` (`mod vm_clock;`; create `#[cfg(test)] pub(crate) mod fixtures` holding `SharedClock` — Task 1.4 adds the machine helpers to the same module, Task 1.7's device-thread test reuses the clock)
- Test: `vm_clock.rs` unit tests (hypervisor-free)

**Interfaces:**
- Consumes: `rusty_box_core::time::{HostClock, HostInstant, ClockHz, VmInstant, VmDuration}` (`HostClock::now(&self) -> HostInstant`; `HostInstant::{from_nanos, as_nanos, nanos_since}`; `ClockHz::{new(hz) -> Option<Self>, ticks_from_nanos_floor(nanos) -> VmDuration, nanos_at(VmInstant) -> u64}`; `VmInstant::{from_ticks, ticks, since, add}`). `ManualClock` in core is `Copy` and advances through `&mut self`, so the tests below use their own shared handle.
- Produces:

```rust
/// A host clock over `std::time::Instant` — the `HostClock` the engine runs on.
/// Nanoseconds since the clock was created; `instant_of` turns a `HostInstant`
/// back into something a thread can sleep until.
pub struct StdClock { epoch: std::time::Instant }
impl StdClock { pub fn new() -> Self; pub fn instant_of(&self, at: HostInstant) -> std::time::Instant; }
impl HostClock for StdClock { fn now(&self) -> HostInstant; }

/// Whether the clock is running (R2: a state is a type).
enum State { Running { vm_at: VmInstant, host_at: HostInstant }, Stopped { vm_at: VmInstant } }

/// The spec's "VmTime": guest time that advances only while the guest may run,
/// answered in the tree's own unit. `now()` is `vm_at` plus the ticks the host
/// clock has earned since the anchor at `rate` (floor; the anchor moves on each
/// `stop`, so at most one tick is lost per pause, never per read).
pub struct VmClockSource<H: HostClock> { state: State, rate: ClockHz, host: H }
impl<H: HostClock> VmClockSource<H> {
    pub fn stopped_at(vm_at: VmInstant, rate: ClockHz, host: H) -> Self;
    pub fn now(&self) -> VmInstant;                     // Running: vm_at + rate.ticks_from_nanos_floor(host.now().nanos_since(host_at)); Stopped: vm_at
    pub fn start(&mut self);                            // Stopped → Running anchored at host.now(); idempotent
    pub fn stop(&mut self) -> VmInstant;                // Running → Stopped at now(); idempotent; returns the frozen value
    pub fn is_running(&self) -> bool;
    pub fn rate(&self) -> ClockHz;
    /// The host instant at which `at` will be `now()`. `None` while Stopped — the device thread has nothing to wait for.
    pub fn host_instant_of(&self, at: VmInstant) -> Option<HostInstant>;   // host_at + rate.nanos_at(VmInstant::from_ticks(at.since(vm_at).ticks()))
}
```

- [ ] **Step 1: Write the failing tests**

```rust
// In lib.rs's new `#[cfg(test)] pub(crate) mod fixtures` (a `vm_clock.rs` test-module item would be unreachable from Task 1.7's tests):
/// A `HostClock` two owners can advance — the test and the source under test.
#[derive(Clone, Default)]
pub(crate) struct SharedClock(std::rc::Rc<std::cell::Cell<u64>>);
impl SharedClock { pub(crate) fn advance_nanos(&self, n: u64) { self.0.set(self.0.get() + n); } }
impl HostClock for SharedClock { fn now(&self) -> HostInstant { HostInstant::from_nanos(self.0.get()) } }

fn mhz(hz: u64) -> ClockHz { ClockHz::new(hz).expect("a rate") }

#[test]
fn time_advances_only_while_running() {
    let host = SharedClock::default();
    host.advance_nanos(1_000);
    let mut clock = VmClockSource::stopped_at(VmInstant::from_ticks(0), mhz(1_000_000_000), host.clone()); // 1 tick = 1 ns
    host.advance_nanos(500);
    assert_eq!(clock.now().ticks(), 0, "stopped: the host moved, the guest did not");
    clock.start();
    host.advance_nanos(700);
    assert_eq!(clock.now().ticks(), 700);
    let frozen = clock.stop();
    host.advance_nanos(10_000);
    assert_eq!(clock.now(), frozen);
    clock.start();
    host.advance_nanos(300);
    assert_eq!(clock.now().ticks(), 1_000, "a pause is invisible to the guest");
    clock.start();
    assert_eq!(clock.now().ticks(), 1_000, "start is idempotent");
}

#[test]
fn ticks_are_earned_at_the_machines_rate() {
    let host = SharedClock::default();
    let mut clock = VmClockSource::stopped_at(VmInstant::from_ticks(0), mhz(50_000_000), host.clone()); // Ips::BOCHS_DEFAULT
    clock.start();
    host.advance_nanos(1_000_000_000);
    assert_eq!(clock.now().ticks(), 50_000_000, "one second is ips ticks");
    host.advance_nanos(10);
    assert_eq!(clock.now().ticks(), 50_000_000, "10 ns is less than a 20 ns tick — floor, not round");
    host.advance_nanos(10);
    assert_eq!(clock.now().ticks(), 50_000_001);
}

#[test]
fn a_host_instant_exists_for_a_future_vm_time_only_while_running() {
    let host = SharedClock::default();
    let mut clock = VmClockSource::stopped_at(VmInstant::from_ticks(0), mhz(50_000_000), host.clone());
    assert!(clock.host_instant_of(VmInstant::from_ticks(5)).is_none());
    clock.start();
    assert_eq!(clock.host_instant_of(VmInstant::from_ticks(5)), Some(HostInstant::from_nanos(100)), "5 ticks at 50 MHz is 100 ns after the anchor");
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test --release -p rusty_box_whp_engine vm_clock`. Expected: module missing.
- [ ] **Step 3: Implement** as the Interfaces block says; `StdClock::instant_of(at) = self.epoch + Duration::from_nanos(at.as_nanos())`.
- [ ] **Step 4: Run to verify pass.**
- [ ] **Step 5: Commit** — `git commit -m "feat(whp): a pausable host clock that earns the machine's ticks while its guest is on hardware"`.

### Task 1.3: The machine's resident-driver seam (no behaviour change for the interpreter)

The machine must let an engine (a) run device time from a thread, (b) hand a processor and its parts to an exit servicer, (c) route an IOAPIC delivery and a raised PIC pin to a backend, (d) say which clock its devices run on. All four are additions; the interpreter's path is unchanged and the suite must stay green bit-for-bit.

**Files:**
- Modify: `rusty_box/src/emulator/engine.rs` (trait additions, defaulted; `DeliveryRoute`), `rusty_box/src/emulator/mod.rs` (`Processor`, `processor`, `engine_mut`, `EmulatorConfig.device_clock`, the `pic_pin_published` field), `rusty_box/src/emulator/scheduler.rs` (`deliver_ioapic_to_lapics` (714-773) → route through the engine; `sync_final_event_levels` (1220) → publish the PIC pin as an edge), `rusty_box/src/emulator/timers.rs` (`service_device_time`), `rusty_box/src/iodev/irq.rs` (`IoApicDelivery` pub type from `PendingIoApicDelivery`), `rusty_box/src/iodev/keyboard.rs` (`#[cfg(test)] pub(crate) serial_fires_seen: u64`, incremented by `fires` in `timer_fired` at keyboard.rs:2724 — the counter the catch-up test reads)
- Test: `rusty_box/src/emulator/tests.rs`

**Interfaces:**
- Consumes: `PendingIoApicDelivery { pin, vector, delivery_mode, trigger_mode, dest, dest_mode, needs_pic_iac }` (ioapic.rs:307), `sync_final_event_levels` (scheduler.rs:1220 — publishes `int_pin_asserted()` as a LEVEL on every commit), `service_scheduler_boundary(elapsed_ticks) -> CpuResult<bool>` (scheduler.rs:972 — `Ok(true)` means a reset was applied and `elapsed_ticks` was NOT ticked, 1029; `Ok(false)` is the normal path, 1175; guest power-off is `stop_cause = StopCause::GuestPowerOff` + `stop_flag`, 985/997), `run_slice`'s destructuring (mod.rs:634-652), `rusty_box_core::EngineFault`.
- Produces:

```rust
// rusty_box/src/iodev/irq.rs — the record an engine may route
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IoApicDelivery { pub vector: u8, pub delivery_mode: u8, pub trigger_mode: u8, pub dest: u32, pub dest_mode: u8 }

// rusty_box/src/emulator/engine.rs — defaulted, so SoftwareEngine and the test engine are untouched
/// Where an IOAPIC delivery went (R5: the machine matches all three).
pub enum DeliveryRoute {
    /// The machine writes the model's IRR as it does today.
    Model,
    /// The engine delivered it elsewhere; the machine does nothing.
    Backend,
    /// The engine's backend refused it. The machine leaves the message pending (the IOAPIC's
    /// stuck path, `complete_deferred_delivery(.., false)`) and returns the fault from the boundary.
    Refused(EngineFault),
}
pub trait SliceEngine<T: Instrumentation> {
    /* existing … */
    /// Where an IOAPIC delivery goes. Default: the machine's own LAPIC models.
    fn route_ioapic_delivery(&mut self, delivery: IoApicDelivery) -> DeliveryRoute { let _ = delivery; DeliveryRoute::Model }
    /// The 8259's INT pin to the boot processor CHANGED level (an edge — the machine calls this
    /// only on a transition). Default: nothing — the interpreter reads the event word. An engine
    /// whose backend refuses returns the fault; the machine surfaces it from the boundary.
    fn pic_pin_changed(&mut self, asserted: bool) -> Result<(), EngineFault> { let _ = asserted; Ok(()) }
}

// rusty_box/src/emulator/mod.rs
/// Which clock the machine's devices run on (R2). `Ticks`: the wheel advances as the scheduler
/// retires guest instructions — the interpreter, and the slice engine. `HostTime`: the wheel is
/// driven by a thread from a host clock — the fast machine. Selects the hypervisor-LAPIC mode
/// and the fabric backend in `WhpEngine` (Task 1.6) and the 8042 one-shot (Task 1.7).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DeviceClock { #[default] Ticks, HostTime }
pub struct EmulatorConfig { /* existing … plus */ pub device_clock: DeviceClock }

/// One processor and the parts it executes against, for an engine that services an exit —
/// `run_slice`'s own destructuring, reachable by a caller (R3: assembled here, by the machine).
pub struct Processor<'a, T: Instrumentation, E> { pub cpu: &'a mut BxCpuC<T>, pub io: PcIo<'a>, pub engine: &'a mut E }
impl<T: Instrumentation, E: SliceEngine<T>> Emulator<T, E> {
    pub fn processor(&mut self, index: usize) -> Processor<'_, T, E>;
    /// The engine value, mutably — for a resident engine's driver to reach its own state under the machine lock.
    pub fn engine_mut(&mut self) -> &mut E;
    pub fn device_clock(&self) -> DeviceClock;
    /// The configuration this machine was built from (today a private field; the harnesses and the fast machine read `ips` from it).
    pub fn config(&self) -> &EmulatorConfig;
}

// rusty_box/src/emulator/timers.rs
/// What one advance of device time did (R0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceTime {
    /// The wheel's next deadline as an absolute tick, `None` with nothing armed (`pc_system.next_timer_deadline_at()`).
    pub next_deadline: Option<u64>,
    /// A hardware reset was applied and `elapsed_ticks` was not ticked (the boundary's `Ok(true)`).
    pub reset_applied: bool,
    /// The boundary asked the machine to stop, and why (`stop_cause` when the stop flag was set by it).
    pub stop: Option<StopCause>,
}
impl<T: Instrumentation, E: SliceEngine<T>> Emulator<T, E> {
    /// Advance device time by `elapsed_ticks` and do everything a scheduler boundary does with it —
    /// `service_scheduler_boundary` for a machine whose guest is not on this thread.
    pub fn service_device_time(&mut self, elapsed_ticks: u64) -> CpuResult<DeviceTime>;
}
```
(`elapsed_ticks: u64` matches `service_scheduler_boundary`'s own parameter; the tick↔`VmInstant` conversion is the device thread's, Task 1.7.)

- [ ] **Step 1: Write the failing tests** (in `rusty_box/src/emulator/tests.rs`, hypervisor-free, on the interpreter's machine; `furnished_machine()` is a new helper factored from the setup lines of `pit_irq0_delivers_through_ioapic_pin2_to_cpu0` (tests.rs:872-895): `Emulator::new_with_mode(cfg, FlatProtected32)`, `devices.init`, `device_manager.init`, `pc_system.initialize(ips)`, `devices.set_timer_ips(ips)`, `register_timer_owners()`, then `rearm_device_timers_after_hardware_reset()` so the 8042's continuous timer is armed; `program_ioapic_entry(machine, pin, vector)` is the four `irq.mmio_write` calls from the same test, factored):

```rust
/// The default route is the model: an IOAPIC delivery lands in the boot processor's IRR exactly as before.
#[test]
fn the_default_ioapic_route_writes_the_model_lapic() {
    let mut machine = furnished_machine();
    let now = machine.pc_system.time_ticks();
    machine.cpu_mut().lapic.write_aligned(0xF0, 0x1FF, now);      // software-enable the LAPIC
    program_ioapic_entry(&mut machine, 1, 0x31);                    // ISA line 1 is IOAPIC pin 1 (only line 0 is remapped, to pin 2 — ioapic.rs set_pin_level)
    machine.device_manager.irq.raise(rusty_box_devices::api::IrqLine(1));
    let outcome = machine.service_device_time(0).unwrap();          // the boundary publishes it
    assert!(!outcome.reset_applied && outcome.stop.is_none());
    assert_ne!(machine.cpu_ref(0).pending_event & BxCpuC::<()>::BX_EVENT_PENDING_LAPIC_INTR, 0, "the model route raised LAPIC INTR");
    assert_eq!(machine.cpu_mut().lapic.acknowledge_int(), 0x31, "and it carried the entry's vector");
}

/// service_device_time replays every missed 8042 period (no coalescing) and names the next deadline.
#[test]
fn service_device_time_fires_every_due_period_and_names_the_next_deadline() {
    let mut machine = furnished_machine();
    let ips = machine.config.ips.per_second_u64();
    let period = ips * u64::from(crate::iodev::keyboard::KBD_SERIAL_DELAY_USEC) / 1_000_000;  // 150 ticks: furnished_machine keeps the pin-2 test's `Ips::new(1_000_000)` (tests.rs:881); ips-derived, so any rate holds
    let before = machine.pc_system.time_ticks();
    let elapsed = 1_000_000;
    let outcome = machine.service_device_time(elapsed).unwrap();
    assert_eq!(machine.pc_system.time_ticks(), before + elapsed);
    assert_eq!(machine.device_manager.keyboard.serial_fires_seen, elapsed / period,
        "one serial-delay fire per period crossed — tickn replays them; a single countdown_event after the jump would coalesce them into one");
    assert_eq!(outcome.next_deadline, Some(before + (elapsed / period + 1) * period), "the next deadline is the 8042's next period");
}

/// The engine hears the PIC pin as an edge: once per transition, not once per boundary.
#[test]
fn the_pic_pin_reaches_the_engine_once_per_transition() {
    let mut machine = furnished_machine_on::<MapWatchingEngine>();   // the file's test engine (tests.rs:36), given a `pic_edges: Vec<bool>` recorder behind `pic_pin_changed`
    machine.device_manager.irq.pic_mut().master.imr = 0xFE;                  // unmask IRQ0
    machine.device_manager.irq.raise(rusty_box_devices::api::IrqLine(0));
    machine.service_device_time(0).unwrap();
    machine.service_device_time(0).unwrap();
    machine.service_device_time(0).unwrap();
    assert_eq!(machine.engine().pic_edges, vec![true], "three boundaries with the pin high published one rising edge");
    let _vector = machine.device_manager.irq.acknowledge();                   // the INTA lowers the pin
    machine.service_device_time(0).unwrap();
    assert_eq!(machine.engine().pic_edges, vec![true, false]);
}
```
(`furnished_machine_on::<E>()` is `furnished_machine` over `Emulator::<(), E>::with_engine(config, mode)` (emulator_api.rs:960), which default-constructs the engine — so it takes no engine value; `MapWatchingEngine` gains a `Default` and the recording override.)

- [ ] **Step 2: Run to verify failure** — `cargo test --release -p rusty_box --lib --features std emulator::tests`. Expected: method missing.
- [ ] **Step 3: Implement**: `service_device_time(elapsed)` = `let reset_applied = self.service_scheduler_boundary(elapsed)?;` then `DeviceTime { next_deadline: self.pc_system.next_timer_deadline_at(), reset_applied, stop: if self.stop_flag.load(Relaxed) { Some(self.stop_cause) } else { None } }` — the boundary already feeds the wheel through `tickn` in deadline-sized `u32` steps (`scheduler.rs:1142-1166`), and it must stay that way: `tickn` runs one `countdown_event` per deadline crossed, whereas a single `countdown_event` after a large jump COALESCES a periodic timer's missed fires into one (`pc_system.rs:744-746`); in `deliver_ioapic_to_lapics` build `IoApicDelivery` from the record and `match self.engine.route_ioapic_delivery(d) { DeliveryRoute::Model => /* existing body */, DeliveryRoute::Backend => true, DeliveryRoute::Refused(fault) => { self.engine_fault = Some(fault); false } }` (R5: exhaustive; `engine_fault: Option<EngineFault>` is a new machine field the boundary turns into `Err(CpuError::UnsupportedCpuOperation { operation: fault.at() })` after the drain, so a refused delivery is never silent) — leave `deliver_lapic_bus_interrupt` (`scheduler.rs:574-597`) untouched, the ICR-IPI path shares it; in `sync_final_event_levels`, keep the level publication into cpu0's `BX_EVENT_PENDING_INTR` exactly as it is and ADD an edge: `if asserted != self.pic_pin_published { self.pic_pin_published = asserted; if let Err(fault) = self.engine.pic_pin_changed(asserted) { self.engine_fault = Some(fault); } }` (`pic_pin_published: bool` is a machine field, written `false` by the two placement constructors at mod.rs:956/1029 and cleared again in `Emulator::reset` (mod.rs:1370) — a guest reset must not leave it `true` and swallow the next rising edge; `engine_fault` is the same field the `Refused` route fills — one surfacing point, R5); add `processor(index)` as `run_slice`'s destructuring returning `Processor { cpu, io, engine }`; add `engine_mut`, `device_clock`, `config`, `EmulatorConfig.device_clock` (default `Ticks`; the snapshot does NOT carry it — it is a property of the driver, not of the guest). The test engine's `MapWatchingEngine::pic_pin_changed` pushes `asserted` and returns `Ok(())`.
- [ ] **Step 4: Run the whole `rusty_box` suite, std and no_std checks** — `cargo test --release -p rusty_box --lib --features std && cargo check --release --no-default-features -p rusty_box`. Expected: green; no behaviour change.
- [ ] **Step 5: Gates and commit** — `cargo xtask ci`; `git commit -m "feat(machine): the seams a resident engine drives — device time, a processor with its parts, an edge on the PIC pin, and where a delivery goes"`.

### Task 1.4: The externalised mask — import what an exit needs, export what was imported

**Files:**
- Create: `rusty_box_whp_engine/src/exchange.rs` (the group-wise exchange)
- Modify: `rusty_box/src/cpu/arch_state.rs` (`ArchGroups` bitflags beside `VcpuArchState` at line 324; `import_arch_groups`/`export_arch_groups` beside the whole-state pair they generalise — `import_arch_state` (751), `export_arch_state` (665); `ArchStateError::SegmentPresentWithoutAttributes`), `rusty_box/src/cpu/api_bridge.rs` (`next_instruction_touches_vector_state`; `load_segment` (206) validates), `rusty_box_whp_engine/Cargo.toml` (no new dependency — the mask type is `rusty_box`'s), `rusty_box_whp_engine/src/state.rs` (split `export`/`import` into per-group functions the mask calls; move `Recorder` out of the private `mod tests` (418) into `#[cfg(test)] pub(crate) mod test_vp` and extend it), `rusty_box_whp_engine/src/xsave.rs` (`patch`/`fill` unchanged; called by the mask's `VECTOR` group only), `rusty_box_whp_engine/src/lib.rs` (extend the `#[cfg(test)] pub(crate) mod fixtures` Task 1.2 created with the machine helpers named below)
- Test: `exchange.rs` unit tests against `Recorder` (hypervisor-free); `arch_state.rs`/`api_bridge.rs` tests for the segment rule
- Cadence: this task edits `rusty_box/cpu/*` — run `cargo check --release --no-default-features -p rusty_box` too.

**Interfaces:**
- Consumes: `VcpuArchState` (arch_state.rs:324: `gprs, rip, rflags, segments[6], ldtr, tr, gdtr, idtr, cr0..cr4, cr8, dr[4], dr6, dr7, msrs, fpu, vector, opmask, mxcsr, xcr0`), `import_arch_state(&mut self, &VcpuArchState) -> Result<(), ArchStateError>` (751), `pub(crate) trait VpRegisters` (state.rs:138: `read_words/write_words/read_registers/write_registers` + Task 1.1's `read_xsave/write_xsave`), `Recorder` (state.rs:427), `XsaveArea::of_this_host()` (xsave.rs:100/116), `InjectState::at_reset()` (engine.rs:375, made `pub(crate)`), `VpContext { rip, rflags, cs: SegmentRegister, instruction_length, cr8, execution_state }` (vcpu.rs:414-425, all fields `pub`).
- Produces:

```rust
// rusty_box/src/cpu/arch_state.rs — the machine crate owns the group set, because the CPU's
// import takes it; the engine's mask is this same type (no second type, no conversion).
bitflags::bitflags! {
    /// The register groups a processor's architectural state moves in (R2). In an engine's
    /// externalised mask a set bit means "the live value is in the partition; the shadow's copy
    /// is stale"; `all()` is a processor the shadow has never read, `empty()` one the shadow
    /// fully describes. Never the TSC, which no exchange carries.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ArchGroups: u16 {
        const GPRS = 1 << 0;
        const RIP_RFLAGS = 1 << 1;
        const CONTROL_REGS = 1 << 2;      // CR0, CR2, CR3, CR4, CR8
        const DEBUG_REGS = 1 << 3;
        const SEGMENTS = 1 << 4;          // ES..GS, LDTR, TR
        const TABLES = 1 << 5;            // GDTR, IDTR
        const MSRS = 1 << 6;              // MsrState: EFER .. XCR0
        const VECTOR = 1 << 7;            // fpu, vector, opmask, mxcsr — the XSAVE area
        const INTERRUPT_STATE = 1 << 8;
    }
}
pub enum ArchStateError { /* existing … */ SegmentPresentWithoutAttributes { index: usize } }

// rusty_box/src/cpu/arch_state.rs (the two group functions, beside import_arch_state/export_arch_state) and
// rusty_box/src/cpu/api_bridge.rs (the decoder question)
impl<T: Instrumentation> BxCpuC<T> {
    /// `import_arch_state` restricted to `groups` (CS loads keep `handle_cpu_mode_change`).
    pub fn import_arch_groups(&mut self, state: &VcpuArchState, groups: ArchGroups) -> Result<(), ArchStateError>;
    pub fn export_arch_groups(&self, out: &mut VcpuArchState, groups: ArchGroups);
    /// Whether `bytes` decode to an instruction that reads or writes vector or x87 state. New work in
    /// `api_bridge.rs` (nothing named `is_sse_or_avx` exists): decode with the CPU's own decoder and
    /// answer from the decoded opcode's ISA-extension requirement — the per-opcode requirement the
    /// icache ISA gate already computes (`cpu/decoder/mod.rs`; the `BX_ISA_EXTENSIONS_ARRAY_SIZE`
    /// bitmap from `rusty_box_decoder/src/lib.rs:42`). Any x87/SSE*/AVX*/AVX-512/F16C/FMA/AES-NI/
    /// PCLMULQDQ/SHA requirement → true; an opcode the decoder refuses → true (import the vector file
    /// rather than trust a stale one).
    pub fn next_instruction_touches_vector_state(&self, bytes: &[u8]) -> bool;
}

// rusty_box_whp_engine/src/exchange.rs
/// What an exit class needs before the interpreter may finish it (spec §3.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExitClass { PlainPort, StringPort, Mmio, Cpuid, Msr, Exception, Full /* transfer, snapshot, diagnostics */ }
pub const fn needs(class: ExitClass) -> ArchGroups;

/// The seam's bookkeeping for one processor.
pub struct Exchange { externalised: ArchGroups, imported_this_exit: ArchGroups }
impl Exchange {
    pub fn at_reset() -> Self;                                   // everything externalised (the partition owns a fresh VP)
    /// Copy the header's free fields into the shadow: CS, RIP, RFLAGS, CR8→TPR, interrupt shadow. No platform call.
    pub fn take_header<T: Instrumentation>(&mut self, cpu: &mut BxCpuC<T>, vp: &VpContext, inject: &mut InjectState);
    /// Read the groups `class` needs that are still externalised (plus VECTOR when `bytes` says so, or when `bytes` is empty); mark them imported.
    pub fn import_for<V: VpRegisters, T: Instrumentation>(&mut self, vp: &V, cpu: &mut BxCpuC<T>, class: ExitClass, bytes: &[u8], xsave: &mut XsaveArea) -> Result<()>;
    /// Write back exactly the groups imported this exit; mark them externalised again. Returns how many platform calls it made.
    pub fn export_imported<V: VpRegisters, T: Instrumentation>(&mut self, vp: &V, cpu: &mut BxCpuC<T>, xsave: &mut XsaveArea) -> Result<usize>;
    pub fn import_everything<V: VpRegisters, T: Instrumentation>(&mut self, vp: &V, cpu: &mut BxCpuC<T>, xsave: &mut XsaveArea) -> Result<()>;  // ExitClass::Full
}

// rusty_box_whp_engine/src/state.rs
pub(crate) fn read_groups<V: VpRegisters>(vp: &V, groups: ArchGroups, into: &mut VcpuArchState) -> Result<usize>;   // one read_registers call over the union of the groups' names (TSC never named)
pub(crate) fn write_groups<V: VpRegisters>(vp: &V, groups: ArchGroups, from: &VcpuArchState) -> Result<usize>;
#[cfg(test)] pub(crate) mod test_vp {
    pub(crate) struct Recorder { /* existing stores … */ }
    impl Recorder { pub(crate) fn seed(&self, reg: Reg, word: u64); pub(crate) fn value_of(&self, reg: Reg) -> u64; pub(crate) fn reads_of(&self, reg: Reg) -> usize; pub(crate) fn writes_of(&self, reg: Reg) -> usize; pub(crate) fn total_reads(&self) -> usize; pub(crate) fn xsave_writes(&self) -> usize; }
    impl VpRegisters for Recorder { /* existing four + read_xsave/write_xsave over a Vec<u8> */ }
}

// rusty_box_whp_engine/src/lib.rs — the module Task 1.2 created (it already holds `SharedClock`)
#[cfg(test)] pub(crate) mod fixtures {
    pub(crate) const CODE: u64 = 0x1000; pub(crate) const DEBUG_PORT: u8 = 0xE9; pub(crate) const MARK: u8 = 0x5A;
    pub(crate) fn machine_running(code: &[u8]) -> Box<Emulator<(), WhpEngine>>;        // today's lib.rs:129 (with_engine, no devices)
    pub(crate) fn machine_with_devices(code: &[u8]) -> Box<Emulator<(), WhpEngine>>;   // today's lib.rs:150 (MachineBuilder, full device set)
    pub(crate) fn hypervisor_here() -> bool;                                            // today's lib.rs:195
    pub(crate) fn a_turn_on_the_hardware() -> MutexGuard<'static, ()>;                  // today's lib.rs:215
    /// Poll `done` until it holds or `within` passes; then panic with `what`. Never holds a lock across the sleep.
    pub(crate) fn wait_until(done: impl FnMut() -> bool, within: Duration, what: &str);
}
```
`needs`: `PlainPort → ArchGroups::empty()` (RIP+RAX written directly, as `service_port_access` does today); `StringPort → GPRS | RIP_RFLAGS | SEGMENTS | CONTROL_REGS`; `Mmio → GPRS | RIP_RFLAGS | SEGMENTS | CONTROL_REGS | MSRS` (EFER for mode); `Cpuid → GPRS | RIP_RFLAGS`; `Msr → GPRS | RIP_RFLAGS | CONTROL_REGS | MSRS`; `Exception → GPRS | RIP_RFLAGS | SEGMENTS | CONTROL_REGS | DEBUG_REGS | MSRS | INTERRUPT_STATE`; `Full → ArchGroups::all()`. `VECTOR` is imported lazily: `import_for` inserts it when `cpu.next_instruction_touches_vector_state(bytes)`; when the exit carries no bytes (a read-only window write, probe finding 2), `VECTOR` is imported unconditionally. The import set is `needs(class).union(vector).intersection(self.externalised)`; the export set is `self.imported_this_exit` — plain flag algebra.

- [ ] **Step 1: Write the failing tests** (`exchange.rs`, against `Recorder`; `test_cpu()` is a local helper: `let mut machine = fixtures::machine_running(&[]); let Processor { cpu, .. } = machine.processor(0);` — hypervisor-free because no partition is started):

```rust
#[test]
fn a_plain_port_exit_moves_nothing_and_a_cpuid_exit_moves_only_what_cpuid_can_change() {
    let vp = Recorder::default(); let mut machine = machine_running(&[]); let cpu = machine.processor(0).cpu; let mut xs = XsaveArea::of_this_host();
    let mut ex = Exchange::at_reset();
    ex.import_for(&vp, cpu, ExitClass::PlainPort, &[0xE6, 0xE9], &mut xs).unwrap();
    assert_eq!(vp.total_reads(), 0);
    ex.import_for(&vp, cpu, ExitClass::Cpuid, &[0x0F, 0xA2], &mut xs).unwrap();
    assert!(vp.reads_of(Reg::Rax) == 1 && vp.reads_of(Reg::Es) == 0 && vp.reads_of(Reg::Efer) == 0);
    let calls = ex.export_imported(&vp, cpu, &mut xs).unwrap();
    assert!(vp.writes_of(Reg::Rax) == 1 && vp.writes_of(Reg::Es) == 0, "export writes exactly what was imported");
    assert_eq!(calls, 1, "one batched write");
}

#[test]
fn an_imported_group_is_not_read_twice_until_it_is_exported() {
    let vp = Recorder::default(); let mut machine = machine_running(&[]); let cpu = machine.processor(0).cpu; let mut xs = XsaveArea::of_this_host();
    let mut ex = Exchange::at_reset();
    ex.import_for(&vp, cpu, ExitClass::Msr, &[0x0F, 0x30], &mut xs).unwrap();
    ex.import_for(&vp, cpu, ExitClass::Cpuid, &[0x0F, 0xA2], &mut xs).unwrap();
    assert_eq!(vp.reads_of(Reg::Rax), 1, "GPRS were already local");
}

/// The whole point of the mask: a group this exit never imported keeps the PARTITION's live
/// value — the shadow's stale copy is never written over it.
#[test]
fn a_group_that_was_not_imported_is_not_written_back() {
    let vp = Recorder::default(); let mut machine = machine_running(&[]); let cpu = machine.processor(0).cpu; let mut xs = XsaveArea::of_this_host();
    vp.seed(Reg::Cr3, 0x0000_1000); vp.seed(Reg::Dr7, 0x0000_0400);
    let mut ex = Exchange::at_reset();
    ex.import_for(&vp, cpu, ExitClass::Cpuid, &[0x0F, 0xA2], &mut xs).unwrap();
    let mut drifted = VcpuArchState::default();                                          // the shadow drifts locally, through the task's own API
    cpu.export_arch_groups(&mut drifted, ArchGroups::all());
    drifted.cr3 = 0xDEAD_0000; drifted.dr7 = 0xDEAD_0400; drifted.segments[0].selector = 0x0020;
    cpu.import_arch_groups(&drifted, ArchGroups::CONTROL_REGS | ArchGroups::DEBUG_REGS | ArchGroups::SEGMENTS).unwrap();
    ex.export_imported(&vp, cpu, &mut xs).unwrap();
    assert_eq!(vp.value_of(Reg::Cr3), 0x0000_1000, "CONTROL_REGS were not imported, so they are not exported");
    assert_eq!(vp.value_of(Reg::Dr7), 0x0000_0400);
    assert_eq!(vp.writes_of(Reg::Es), 0);
}

#[test]
fn the_tsc_is_never_written_and_the_xsave_area_crosses_only_for_vector_state() {
    let vp = Recorder::default(); let mut machine = machine_running(&[]); let cpu = machine.processor(0).cpu; let mut xs = XsaveArea::of_this_host();
    let mut ex = Exchange::at_reset();
    ex.import_everything(&vp, cpu, &mut xs).unwrap();
    ex.export_imported(&vp, cpu, &mut xs).unwrap();
    assert_eq!(vp.writes_of(Reg::Tsc), 0);
    assert_eq!(vp.xsave_writes(), 1, "Full imports the vector file, so Full exports it");
    let mut ex = Exchange::at_reset();
    ex.import_for(&vp, cpu, ExitClass::Cpuid, &[0x0F, 0xA2], &mut xs).unwrap();
    ex.export_imported(&vp, cpu, &mut xs).unwrap();
    assert_eq!(vp.xsave_writes(), 1, "unchanged: CPUID never touches the vector file");
    let mut ex = Exchange::at_reset();
    ex.import_for(&vp, cpu, ExitClass::Mmio, &[0x66, 0x0F, 0x6F, 0x05, 0, 0, 0xE0, 0xFE], &mut xs).unwrap(); // movdqa xmm0, [0xFEE00000]
    ex.export_imported(&vp, cpu, &mut xs).unwrap();
    assert_eq!(vp.xsave_writes(), 2, "an MMIO exit whose instruction touches xmm0 moves the vector file");
}

#[test]
fn the_header_copy_costs_no_platform_call_and_lands_in_the_shadow() {
    let vp = Recorder::default(); let mut machine = machine_running(&[]); let cpu = machine.processor(0).cpu; let mut inject = InjectState::at_reset();
    let mut ex = Exchange::at_reset();
    let header = VpContext { rip: 0x1234, rflags: 0x202, cs: SegmentRegister { base: 0, limit: 0xFFFF, selector: 0, attributes: 0x93 }, instruction_length: 2, cr8: 3, execution_state: 0 };
    ex.take_header(cpu, &header, &mut inject);
    let mut shadow = VcpuArchState::default();
    cpu.export_arch_groups(&mut shadow, ArchGroups::RIP_RFLAGS | ArchGroups::CONTROL_REGS);
    assert_eq!((shadow.rip, shadow.rflags, shadow.cr8), (0x1234, 0x202, 3), "RIP, RFLAGS and CR8→TPR landed");
    assert_eq!(vp.total_reads(), 0);
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test --release -p rusty_box_whp_engine exchange`.
- [ ] **Step 3: Implement**: `ArchGroups` in `arch_state.rs`; `import_arch_groups`/`export_arch_groups` as the existing whole-state functions with a group filter (the whole-state pair becomes the `all()` case); `read_groups/write_groups` over the existing `WORD_REGS`/`MSR_REGS`/`SEGMENT_REGS`/`TABLE_REGS` tables (the TSC skip stays by `TSC_SLOT`); `import_for` unions the class's groups (plus `VECTOR` per the rule) minus already-local ones, issues ONE `read_registers` over the union's names, then `import_arch_groups` for those groups only; `export_imported` mirrors it with one `write_registers`; `INTERRUPT_STATE` uses the existing `held_interrupt_state` rule (impose bit 0 as 1 only when the shadow retired the shadowing instruction — carry `Started::shadowed`'s semantics into `Exchange`).
- [ ] **Step 4: Run the tests to verify pass; run the engine suite** (the old loop still uses the whole-state paths — unchanged until Task 1.5 switches it).
- [ ] **Step 5: Segment import validates every segment, with controls** (the peer session's defect): in `api_bridge.rs` `load_segment` (206), the validity rule that today guards CS (`state.rs` diagnostic "read-back CS cannot be executing") is applied to every segment: a segment whose attributes say present with every other attribute bit zero and a non-zero limit is refused with `ArchStateError::SegmentPresentWithoutAttributes { index }`, surfaced through the engine's refused-state dump. Tests in `arch_state.rs` (`SegmentState.attributes` is the `SegmentAttributes` newtype, arch_state.rs:92 — build the words with `SegmentAttributes::from_bits`): (a) `attributes: SegmentAttributes::from_bits(0x0080)` (present, nothing else) with `limit: 0xFFFFF` is refused by `import_arch_groups(.., SEGMENTS)`; (b) a real-mode data segment (`from_bits(0x0093)`, `limit 0xFFFF`) is accepted; (c) a not-present segment (`from_bits(0)`, limit 0) is accepted — the rule must refuse the corruption and nothing legitimate.
- [ ] **Step 6: Gates and commit** — `cargo check --release --no-default-features -p rusty_box && cargo xtask ci`; `git commit -m "feat(whp): an exit imports the register groups it needs and exports the ones it imported"`.

### Task 1.5: The vCPU thread — an unbounded run loop that services exits under the machine lock

**Files:**
- Create: `rusty_box_whp_engine/src/vcpu_thread.rs`
- Modify: `rusty_box_whp_engine/src/engine.rs` (move `service_port_access` (~2780), `finish_on_the_shadow`, the `ExitReason` service match arms (the `match exit.reason {` at engine.rs:2005, closing at 2143 — NOT the `ExitHistory` classification match at 1986-2003), `report_the_fault`, the refused-state dump into `vcpu_thread.rs` as methods of the servicer; `Started.vcpu` becomes `Option<Vcpu>`; new `bring_up`, `install_control`, `controls`), `rusty_box_whp_engine/src/lib.rs` (`mod vcpu_thread;`, exports, Send assertions), `rusty_box_whp_engine/Cargo.toml` (`[lints] workspace = true`), `rusty_box_core/src/engine.rs` (`EngineFaultKind::Unserviced`), `rusty_box_whp_sys/src/vcpu.rs` (`ExitReason::name() -> &'static str`)
- Test: `vcpu_thread.rs` unit tests for the exit-request protocol (hypervisor-free) and a gated test in `lib.rs`

**Interfaces:**
- Consumes: `Vcpu`, `Exchange`, `Emulator::processor`, `PcIo::emulate_one`/`finish_the_instruction`, `InjectState`, `EngineFault::new(kind, at)` / `with_code` (`rusty_box_core::engine`, re-exported at the crate root), `ExitCounts` (engine.rs:578).
- Produces:

```rust
// rusty_box_core/src/engine.rs — one more kind (the enum is #[non_exhaustive]; Display gains its line).
// A wedged thread is `FastMachineFault::Wedged` (Task 1.7), a fast-machine refusal, not an engine fault.
pub enum EngineFaultKind { /* existing … */
    /// The engine received an exit it has no service for.
    Unserviced,
}

// vcpu_thread.rs
/// Why a vCPU thread left its run loop (R0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Parked { Paused, GuestPowerOff, Fault(EngineFault) }

/// One thread's account of itself, readable from any thread: plain copies of shared atomics,
/// plus the platform's own counters as of the thread's last park (read by the thread — the
/// only holder of the `Vcpu` — so no counter call ever happens off-thread).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct VcpuCensus { pub runs: u64, pub in_run_nanos: u64, pub exits: ExitCounts, pub platform_at_last_park: Option<PlatformCounters> }

/// The cross-thread controls for one vCPU thread. Clone to hand to the front end and the device thread.
#[derive(Clone)]
pub struct VcpuControl {
    exit_requested: Arc<AtomicBool>, park_reason: Arc<Mutex<Option<Parked>>>, ext_int_pending: Arc<AtomicBool>,
    canceller: Canceller, parked: Arc<(Mutex<Option<Parked>>, Condvar)>, census: Arc<SharedCensus>,
}
impl VcpuControl {
    /// Ask the thread to leave the run and park with `why`. Records the reason, sets the flag, THEN cancels —
    /// a cancel that lands between runs is consumed by the flag check before the next entry (QEMU's exit_request protocol).
    pub fn request_park(&self, why: Parked) -> WhpResult<()>;
    /// A PIC INT pin rose. Cancels the VP only on the false→true transition of `ext_int_pending`
    /// (`compare_exchange`), so a pin that stays high across many boundaries costs one exit, not one per boundary.
    pub fn raise_ext_int(&self) -> WhpResult<()>;
    /// Wait for the thread to park. `None` if it has not within `within` — the test or verb decides what that means.
    pub fn wait_parked_by(&self, within: Duration) -> Option<Parked>;
    pub fn resume(&self);                 // clears exit_requested and the park slot, notifies the thread
    /// Park the thread and let it return: a parked thread that sees `stop` leaves `run_loop` instead of waiting for `resume`.
    pub fn stop(&self) -> WhpResult<()>;
    pub fn census(&self) -> VcpuCensus;   // Acquire loads; never touches the VP
}

/// The park protocol as a pure function, so its ordering is testable without a partition:
/// flag first, cancel second. `cancel` observes the flag already set.
pub(crate) trait CancelRun { fn cancel(&self) -> WhpResult<()>; }
impl CancelRun for Canceller { fn cancel(&self) -> WhpResult<()> { Canceller::cancel(self) } }
pub(crate) fn park_request(exit_requested: &AtomicBool, cancel: &impl CancelRun) -> WhpResult<()>;

pub struct VcpuThread<T: Instrumentation + Send> {
    vcpu: Vcpu, index: usize, machine: Arc<Mutex<Box<Emulator<T, WhpEngine>>>>, exchange: Exchange, xsave: XsaveArea, inject: InjectState, control: VcpuControl,
}
impl<T: Instrumentation + Send> VcpuThread<T> {
    pub fn spawn(vcpu: Vcpu, index: usize, machine: Arc<Mutex<Box<Emulator<T, WhpEngine>>>>) -> (JoinHandle<()>, VcpuControl);
    fn run_loop(mut self);   // loop { if exit_requested → park(reason); pre-run (Task 1.6's ExtINT staging); let t = Instant::now(); let exit = vcpu.run(); census.in_run_nanos += t.elapsed(); census.runs += 1; service(exit) }
    fn service(&mut self, exit: Exit) -> Continue;   // lock machine (a poisoned lock → park Fault(Host, "machine lock poisoned by a panicking peer thread") — reachable only under the unwind profile cargo test builds with; release aborts); take_header; match reason { … }; drop lock
    fn park(&mut self, why: Parked);   // refreshes census.platform_at_last_park from vcpu.counters(); publishes `why`; blocks on the condvar until resume()
}
enum Continue { Run, Park(Parked) }

// engine.rs
impl WhpEngine {
    /// The thread controls, installed by whoever spawned the threads; empty until then. `pic_pin_changed` is a no-op while empty.
    pub(crate) fn install_control(&mut self, index: usize, control: VcpuControl);
    pub(crate) fn partition(&self) -> Option<&Partition>;
}
/// Build the partition, map the machine, create VP 0 and hand its `Vcpu` out — `start()` reachable
/// without a slice. `Contract` error if the processor was already taken.
pub(crate) fn bring_up<T: Instrumentation>(machine: &mut Emulator<T, WhpEngine>) -> Result<Vcpu>;   // let Processor { mut io, engine, .. } = machine.processor(0); let started = start(&mut engine.started, &mut io)?; started.vcpu.take()…

// ExitCounts (engine.rs:578) gains one bucket per arm the service match has, so "no exit of class X" is assertable:
pub struct ExitCounts { pub port: u64, pub memory: u64, pub cpuid: u64, pub msr: u64, pub exception: u64, pub halt: u64, pub canceled: u64, pub window: u64, pub apic_eoi: u64, pub apic_write: u64, pub boundary: u64 /* deleted in 1.7 */, pub other: u64 }
impl ExitCounts { pub fn total(&self) -> u64; }
```
The `service` match (R5, exhaustive on `ExitReason`): `IoPortAccess` plain → `service_port_access` (writes RIP+RAX, no import); string/REP or TF → `import_for(StringPort)`, `finish_the_instruction`, `export_imported`; `MemoryAccess` → `import_for(Mmio)`, `finish_the_instruction`, `export_imported` (the burst rule is gone; clustering is Stage 2); `Cpuid` → `import_for(Cpuid)`, the existing CPUID answer on the shadow, export; `MsrAccess` → `import_for(Msr)`, `finish_the_instruction`, export; `Exception` → `import_for(Exception)`, `report_the_fault` if asked, `finish_the_instruction`, export; `ApicEoi{vector}` → under the lock `io.device_manager().irq.receive_eoi(vector as u8)` (today's log-only verb; Task 1.6 replaces the call with `resample_on_eoi`), no import; `InterruptWindow` → `Continue::Run` (Task 1.6 gives it a body); `Canceled` → if `exit_requested` → `Park(park_reason)`, else `Continue::Run` (a cancel for `raise_ext_int` — the pre-run stages it); `Halt` → `Continue::Run` in this task (the partition is still `LocalApicMode::None` here, so a `hlt` guest exits and re-enters; no test in this task halts — the guests end in `jmp $`; Task 1.6 writes the final rule from P3); `ApicSmiTrap` → `import_everything`, `cpu.deliver_smi()`, `io.emulate_one` until out of SMM (the existing `run_the_shadow_out_of_smm`), `export_imported`; `ApicWriteTrap{..}` → `Continue::Run` (Task 1.6 gives it a body); `ApicInitSipiTrap | SynicSintDeliverable | Hypercall | Rdtsc | UnrecoverableException | InvalidVpRegisterValue | UnsupportedFeature | None | Unrecognized(_)` → `Park(Fault(EngineFault::new(EngineFaultKind::Unserviced, reason.name())))` with the refused-state dump (the existing `report_the_state_the_platform_refused`; `ExitReason::name(&self) -> &'static str` is a new `const fn`). In Stage 1 these are faults; Stage 2's clustering supplies the bounded-stretch retry the spec's §3.4 names. Guest power-off: the device thread requests `Parked::GuestPowerOff` (Task 1.7).

- [ ] **Step 1: Write the failing tests**

Hypervisor-free (the protocol, order-sensitive):
```rust
/// A canceller that records what the flag said at the moment it was asked to cancel.
struct RecordingCancel<'a> { flag: &'a AtomicBool, saw: Cell<Option<bool>> }
impl CancelRun for RecordingCancel<'_> { fn cancel(&self) -> WhpResult<()> { self.saw.set(Some(self.flag.load(Ordering::Acquire))); Ok(()) } }

#[test]
fn a_park_request_sets_the_flag_before_it_cancels() {
    let flag = AtomicBool::new(false);
    let cancel = RecordingCancel { flag: &flag, saw: Cell::new(None) };
    park_request(&flag, &cancel).unwrap();
    assert_eq!(cancel.saw.get(), Some(true), "the cancel saw the flag already set — a cancel landing between runs is consumed by the flag check, never lost");
    assert!(flag.load(Ordering::Acquire));
}
```
Gated (in `lib.rs` tests; `fixtures::shared(machine) -> (Arc<Mutex<Box<Emulator<(), WhpEngine>>>>, Vcpu)` is a new fixture: `bring_up(&mut machine)` then wrap — the constructor Task 1.7's `adopt` replaces for production):
```rust
/// A guest on the vCPU thread writes a byte to the debug port and spins; the byte reaches the machine's device and the thread parks on request.
#[test]
fn a_vcpu_thread_services_a_port_exit_under_the_machine_lock() {
    if !hypervisor_here() { return; }
    let _turn = a_turn_on_the_hardware();
    let (machine, vcpu) = shared(machine_running(&[0xB0, MARK, 0xE6, DEBUG_PORT, 0xEB, 0xFE])); // mov al, MARK; out DEBUG_PORT, al; jmp $
    let (join, control) = VcpuThread::spawn(vcpu, 0, machine.clone());
    let mut seen = Vec::new();
    wait_until(|| { seen.extend(machine.lock().unwrap().debug_port().take_output()); seen.contains(&MARK) }, Duration::from_secs(2), "the guest's MARK reached the debug port");
    control.request_park(Parked::Paused).unwrap();
    assert_eq!(control.wait_parked_by(Duration::from_secs(5)), Some(Parked::Paused), "the thread parked within 5 s — a cancel that never stuck");
    let census = control.census();
    assert!(census.runs >= 1 && census.exits.port == 1 && census.in_run_nanos > 0, "{census:?}");
    control.stop().unwrap();                                   // a parked thread told to stop returns from run_loop
    join.join().unwrap();
}
```
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement** `vcpu_thread.rs` by MOVING the exit arms out of the service match (engine.rs:2005–2143) and `service_port_access` (~2780) — same code, new owner; the whole-state read-back/impose pair around each errand becomes `import_for`/`export_imported`. The machine lock is `std::sync::Mutex`; the run itself (`vcpu.run()`) is called with the lock RELEASED; `in_run_nanos` is the `Instant` delta around it. `bring_up` and `install_control` in engine.rs. Add `[lints]\nworkspace = true` to `rusty_box_whp_engine/Cargo.toml`: the crate does not opt into the workspace lint table today; its existing crate-level `#![expect(unsafe_code, …)]` becomes the one lift and the ratchet baseline of 1 is unchanged — run `cargo check --release -p rusty_box_whp_engine` right after adding it and fix (or `expect` by name, with a reason) every OTHER workspace lint that surfaces, in this task. Add to `lib.rs`: `const _: () = { const fn s<M: Send>() {} const fn ss<M: Send + Sync>() {} s::<Emulator<(), WhpEngine>>(); ss::<VcpuControl>(); s::<VcpuThread<()>>(); };` — `Emulator<(), WhpEngine>` is the type that goes into the `Arc<Mutex<_>>`, and `rusty_box`'s own assertion (mod.rs:2188-2191) covers `Emulator<()>` = `SoftwareEngine` only.
- [ ] **Step 4: Run the engine suite.** Expected: the new tests pass; the old slice-loop tests still pass (the old loop is untouched until Task 1.7; it reads `Started.vcpu.as_ref()` and refuses with `CpuError::UnsupportedCpuOperation { operation: "the processor was taken by a thread" }` when `None`).
- [ ] **Step 5: Gates and commit** — `git commit -m "feat(whp): a processor runs on its own thread and services its exits under the machine lock"`.

### Task 1.6: The hypervisor's LAPIC, the IOAPIC backend, and the legacy ExtINT path

Everything here is selected by `device_clock == HostTime` (Task 1.3). Under `Ticks` — every existing caller — `start()` keeps `LocalApicMode::None`, `route_ioapic_delivery` answers `Model`, and the slice engine boots exactly as before this task. The gated tests below build their machines with `HostTime` and drive them on the vCPU thread.

**Files:**
- Modify: `rusty_box/src/pc_system.rs` (`device_clock: DeviceClock` field, default `Ticks`, with `pub(crate) fn set_device_clock(&mut self, clock: DeviceClock)` and `pub fn device_clock(&self) -> DeviceClock`; `initialize(ips)` (635) keeps its signature — no caller changes), `rusty_box/src/emulator/mod.rs` (`init_memory_and_pc_system` (1067) calls the setter with the config's value, so `PcIo::pc_system().device_clock()` answers inside `start()`), `rusty_box_whp_engine/src/engine.rs` (`start()`: mode by `device_clock`; `msr_exits` gains `apic_base_write: true`, `extended_vm_exits` gains `apic_write_lint0_trap: true, apic_smi_trap: true` under `HostTime`; `Started.apic_mode: LocalApicMode`; the `SliceEngine` overrides `route_ioapic_delivery`/`pic_pin_changed`), `rusty_box_whp_engine/src/vcpu_thread.rs` (pre-run ExtINT staging; `InterruptWindow`, `ApicEoi`, `ApicWriteTrap`, `Halt` arms), `rusty_box/src/memory/plan.rs` (delete the dead `LOCAL_APIC_REGION` carve-out — see Step 3), `rusty_box/src/iodev/irq.rs` (`set_lint0(value: u64)`, `lint0_admits_ext_int() -> bool`, `resample_on_eoi(vector: u8) -> bool`), `rusty_box/src/iodev/ioapic.rs` (`has_asserted_level_entry(vector: u8) -> bool`)
- Test: `lib.rs` gated tests; `irq.rs` unit tests for LVT0 tracking and the EOI resample; `vcpu_thread.rs` pure test for the readiness predicate; `plan.rs` unit test for the carve-out deletion

**Interfaces:**
- Consumes: `IoApicDelivery`, `DeliveryRoute`, `InterruptRequester::request(InterruptRequest) -> WhpResult<()>` (partition.rs:946), `InterruptRequest { kind, destination_mode, trigger_mode, destination, vector }`, `PendingExtIntEvent`, `InjectState` (engine.rs:344; `refresh_from(&VpContext)` reads `InterruptionPending` = execution_state bit 6, `InterruptShadow` = bit 12, `IF` = rflags bit 9), `acknowledge_external_interrupt` (event.rs:661), `BxIoApic::service` (ioapic.rs:786 — the scan that queues `PendingIoApicDelivery`s; level entries keep their `irr` bit), `IrqFabric::service_ioapic` (irq.rs:179), P1's and P5's recorded answers, P3's answer.
- Produces (engine):

```rust
impl<T: Instrumentation> SliceEngine<T> for WhpEngine {
    fn route_ioapic_delivery(&mut self, d: IoApicDelivery) -> DeliveryRoute {
        match &self.started {
            None => DeliveryRoute::Model,                                             // reset, restore: no partition yet — the model is the truth
            Some(s) if s.apic_mode == LocalApicMode::None => DeliveryRoute::Model,   // the slice engine's mode
            Some(s) => match s.partition.interrupt_requester().request(InterruptRequest {
                kind: from_delivery_mode(d.delivery_mode), /* 0 Fixed, 1 LowestPriority, 4 Nmi, 5 Init, 6 Sipi; 7 (ExtINT) → Refused: an IOAPIC entry programmed ExtINT is the PIC path */
                destination_mode: from(d.dest_mode), trigger_mode: from(d.trigger_mode), destination: d.dest, vector: d.vector.into() }) {
                Ok(()) => DeliveryRoute::Backend,
                Err(e) => DeliveryRoute::Refused(EngineFault::with_code(EngineFaultKind::Vcpu, "WHvRequestInterrupt", e.hresult())),   // WhpError::hresult() -> i32, error.rs:70
            },
        }
    }
    fn pic_pin_changed(&mut self, asserted: bool) -> Result<(), EngineFault> {
        // Only the boot processor has the 8259's INT pin. No control yet (slice engine, or before adopt) → nothing to tell.
        match (asserted, self.controls.first()) {
            (true, Some(c)) => c.raise_ext_int().map_err(|e| EngineFault::with_code(EngineFaultKind::Vcpu, "WHvCancelRunVirtualProcessor", e.hresult())),
            _ => Ok(()),
        }
    }
}
```
- Produces (vCPU thread, pre-run — the redirected `stage_injection`): `pub(crate) const fn ext_int_permitted(vp: &VpContext) -> bool` = IF set ∧ no interrupt shadow ∧ no interruption pending (the same three bits `InjectState::refresh_from` reads). If `ext_int_pending` is set and the fabric says `lint0_admits_ext_int()`: decide from the header cache `inject` (or a `PendingEvent` read only if the cache is stale) — permitted → lock; `pop_deliverable_vector` (the counted INTA, which also reconciles the stale pin — carry-forward a); `write_words128(PendingEvent, PendingExtIntEvent{vector}.as_words())` and `set_internal_activity(halt_suspend: false)` (P1's answer decides whether the activity write is needed); clear `ext_int_pending`; not permitted → write `DeliverabilityNotifications { interrupt_notification: 1, priority: 0 }` once (`inject.window` records it) and run; on `InterruptWindow` exit → `inject.window = None`, loop to pre-run. The freshness contract stands: the INTA happens only after positive evidence from the header or the window exit. `Halt` arm, final: if P3 recorded "no Halt exits in APIC mode" → `Park(Fault(EngineFault::new(Unserviced, "X64Halt under the hypervisor APIC")))`; if P3 recorded "Halt exits occur" → `Continue::Run` (the hypervisor parks the VP on re-entry until an interrupt). `ApicWriteTrap { register: Lint0, value }` → under the lock `io.device_manager().irq.set_lint0(value)`; other registers → `Continue::Run` (traps not asked for). `ApicEoi { vector }` → under the lock: `if io.device_manager().irq.resample_on_eoi(vector as u8) { machine.service_device_time(0)? }` — the boundary routes what the resample queued through `route_ioapic_delivery` (Task 1.7's per-exit catch-up subsumes the explicit call).

- [ ] **Step 1: Write the failing tests**

`irq.rs` (hypervisor-free; `program_ioapic_pin(fabric, pin, vector, level: bool)` is a small local helper over `mmio_write` — a different name from Task 1.3's three-argument `program_ioapic_entry` in emulator/tests.rs — IOREGSEL 0x10+2·pin then IOWIN low dword `vector | (level as u32) << 15`, IOREGSEL 0x11+2·pin then IOWIN high dword 0):
```rust
/// The hardware behaviour the hypervisor LAPIC expects of us (H8): a level entry whose line is
/// still high is re-serviced the moment the guest EOIs its vector. Neither this port nor Bochs
/// did this before; `receive_eoi` was a log line.
#[test]
fn an_eoi_re_services_a_level_entry_whose_line_is_still_asserted() {
    let mut fabric = IrqFabric::new();
    program_ioapic_pin(&mut fabric, 5, 0x45, true);
    fabric.set_ioapic_pin(5, true);
    let (first, n) = fabric.ioapic_mut().take_pending_deliveries();
    assert_eq!(n, 1); assert_eq!(first[0].vector, 0x45);
    assert!(fabric.resample_on_eoi(0x45), "line still high → re-serviced");
    let (again, n) = fabric.ioapic_mut().take_pending_deliveries();
    assert_eq!(n, 1); assert_eq!(again[0].vector, 0x45);
    fabric.set_ioapic_pin(5, false);
    assert!(!fabric.resample_on_eoi(0x45), "line low → nothing");
    assert_eq!(fabric.ioapic_mut().take_pending_deliveries().1, 0);
    program_ioapic_pin(&mut fabric, 6, 0x46, false);
    fabric.set_ioapic_pin(6, true);
    fabric.ioapic_mut().take_pending_deliveries();
    assert!(!fabric.resample_on_eoi(0x46), "edge entries are never resampled");
    assert_eq!(fabric.ioapic_mut().take_pending_deliveries().1, 0);
}

#[test]
fn lvt0_gates_the_legacy_path_the_way_the_lapic_would() {
    let mut fabric = IrqFabric::new();
    fabric.set_lint0(0x0001_0700);       // masked, ExtINT
    assert!(!fabric.lint0_admits_ext_int());
    fabric.set_lint0(0x0000_0700);       // unmasked, ExtINT
    assert!(fabric.lint0_admits_ext_int());
    fabric.set_lint0(0x0000_0030);       // unmasked, Fixed vector 0x30 — not ExtINT: the guest wants a fixed vector on LINT0
    assert!(!fabric.lint0_admits_ext_int());
}
```
`vcpu_thread.rs` (hypervisor-free — the readiness predicate the freshness contract rests on):
```rust
#[test]
fn the_ext_int_readiness_predicate_reads_the_three_header_bits() {
    let base = VpContext { rip: 0, rflags: 0x202, cs: SegmentRegister { base: 0, limit: 0xFFFF, selector: 0, attributes: 0x9B }, instruction_length: 0, cr8: 0, execution_state: 0 };
    assert!(ext_int_permitted(&base), "IF set, no shadow, nothing pending");
    assert!(!ext_int_permitted(&VpContext { rflags: 0x002, ..base }), "IF clear");
    assert!(!ext_int_permitted(&VpContext { execution_state: 1 << 12, ..base }), "interrupt shadow");
    assert!(!ext_int_permitted(&VpContext { execution_state: 1 << 6, ..base }), "an interruption is already pending");
}
```
`lib.rs` (gated; `fixtures::machine_with_devices_on(clock: DeviceClock, code: &[u8])` is `machine_with_devices` with `device_clock` set on the config — added in this task) — adapt the existing `a_device_interrupt_reaches_a_hardware_guest_by_injection` (lib.rs:631): its PIT guest programs the 8259 and expects vector 0x08 through the PIC; on a `HostTime` machine on the vCPU thread it must pass with the same assertions (the test itself becomes the device thread: it advances `service_device_time` in a `wait_until` loop until the MARKs arrive). Add:
```rust
/// An IOAPIC-routed edge reaches a guest whose LAPIC is the hypervisor's, and the LAPIC's own
/// registers are not trapped: the only memory exits are the four IOAPIC programming writes.
#[test]
fn an_ioapic_edge_is_delivered_by_the_hypervisor_apic_without_an_exit() {
    if !hypervisor_here() { return; }
    let _turn = a_turn_on_the_hardware();
    let (machine, vcpu) = shared(machine_with_devices_on(DeviceClock::HostTime, &IOAPIC_EDGE_GUEST));
    let (join, control) = VcpuThread::spawn(vcpu, 0, machine.clone());
    let ips = machine.lock().unwrap().config().ips.per_second_u64();
    let mut seen = Vec::new();
    wait_until(|| {
        let mut m = machine.lock().unwrap();
        m.service_device_time(ips / 1_000).unwrap();            // the test is the device thread: one millisecond of wheel per poll
        seen.extend(m.debug_port().take_output());
        seen.len() >= 3
    }, Duration::from_secs(5), "three PIT ticks reached the guest's handler through the hypervisor LAPIC");
    let census = control.census();                             // read BEFORE the park — the park's cancel is an exit
    assert!(seen.iter().all(|b| *b == MARK), "{seen:#04x?}");
    assert_eq!(census.exits.memory, 4, "IOREGSEL/IOWIN ×2 — and NOT the SVR and EOI writes to the LAPIC page, which the hypervisor owns");
    assert_eq!(census.exits.port, 3 + seen.len() as u64, "three PIT programming writes plus one MARK per tick; the deliveries themselves cost no exit");
    assert_eq!((census.exits.canceled, census.exits.window, census.exits.halt), (0, 0, 0), "no cancel, no window, no halt: {census:?}");
    control.request_park(Parked::Paused).unwrap();
    control.wait_parked_by(Duration::from_secs(5)).expect("parked");
    drop(control); join.join().unwrap();
}
```
`IOAPIC_EDGE_GUEST`, hand-assembled in the fixtures module the way `whp_probe.rs` annotates its byte strings — every byte carries its mnemonic. Layout in the 8 MiB machine: code at `CODE` = 0x1000 (about 150 bytes), handler at 0x1100, GDT at 0x2000 (three descriptors: null; code `base 0, limit 0xFFFFF, 0x9A, 0xCF`; data `0x92, 0xCF`), GDTR image at 0x1F00 (`limit 0x17, base 0x2000`), IDT at 0x3000 (0x41 gates; only gate 0x40 at 0x3200 is filled: `offset 0x1100, selector 0x08, type 0x8E`), IDTR image at 0x1F08 (`limit 0x207, base 0x3000`); the test writes the two tables and the two register images with `mem_write` before handing the machine to `shared`. Listing:
```
; real mode at 0x1000
cli
lgdt [0x1F00]                       ; 0F 01 16 00 1F
lidt [0x1F08]                       ; 0F 01 1E 08 1F
mov eax, cr0 ; or al, 1 ; mov cr0, eax
jmp 0x08:pm32                       ; 66 EA <pm32 as dword> 08 00
pm32:                               ; 32-bit code, flat
mov ax, 0x10 ; mov ds, ax ; mov es, ax ; mov ss, ax ; mov esp, 0x7000
mov dword [0xFEE000F0], 0x1FF       ; SVR: enable, spurious 0xFF — NOT an exit under the hypervisor LAPIC
mov dword [0xFEC00000], 0x14        ; IOREGSEL → entry 2 low   (pin 0 is remapped to pin 2 by ioapic.rs set_pin_level, Bochs "timer connected to pin #2")
mov dword [0xFEC00010], 0x40        ;   vector 0x40, fixed, physical, edge, unmasked
mov dword [0xFEC00000], 0x15        ; IOREGSEL → entry 2 high
mov dword [0xFEC00010], 0           ;   destination APIC 0
mov al, 0x34 ; out 0x43, al         ; PIT ch0, lo/hi, mode 2
mov al, 0x00 ; out 0x40, al         ; count 0x1000 low
mov al, 0x10 ; out 0x40, al         ; count 0x1000 high  (3.4 ms per tick)
sti
spin: jmp spin                      ; EB FE — no HLT: delivery must interrupt a RUNNING processor
; handler at 0x1100
mov al, MARK ; out 0xE9, al
mov dword [0xFEE000B0], 0           ; EOI — not an exit either
iretd
```
The 8259 is left at its reset mask (IMR 0xFF), so IRQ0 reaches the guest only through the IOAPIC. Conditional on P5: if P5 recorded that the platform does NOT gate an injected ExtINT on LVT0, add `a_masked_lvt0_keeps_the_legacy_path_closed` — the PIT guest above with the 8259 unmasked and no IOAPIC entry, `mov dword [0xFEE00350], 0x10700` (LVT0 masked) before `sti`: no MARK arrives within 500 ms of wheel, and `census.exits.window == 0` (the fabric never staged what the LAPIC would have masked). If P5 recorded that the platform gates it, the LVT0 tracking is a mirror for the model and the test is not written — say which in the commit.

- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement** exactly as the Interfaces block says. In `start()`, mode selection by `io.pc_system().device_clock()`: `Ticks` → `LocalApicMode::None` (unchanged); `HostTime` → `config.local_apic(LocalApicMode::X2Apic)`, and if the platform refuses that property, `XApic`, and if it refuses that too, `Err(EngineFault::new(Unsupported, "no hypervisor local APIC on this host"))` — record the mode taken in `Started.apic_mode`. `LOCAL_APIC_REGION` and its `carve_outs.add` in `plan.rs` (45, 115) are deleted as dead code — no candidate window can overlap 0xFEE0_0000..0xFEF0_0000 (RAM stops at `BX_PCI_HOLE_START` = 0xC000_0000, resumes at 4 GiB, the BIOS candidate starts at or above 0xFFC0_0000), so the deletion maps nothing new; add a `plan.rs` unit test that derives a plan for a machine with RAM above 4 GiB and asserts no window intersects that range. What retires the LAPIC memory exits is the emulation mode, not this deletion. Keep `set_lapic_tpr_from_cr8` on the header copy (the mirror stays roughly current between pauses; exactness comes from the page at pause). `resample_on_eoi(vector)`: `if self.ioapic.has_asserted_level_entry(vector) { self.service_ioapic(); true } else { false }` — `has_asserted_level_entry` = some entry with that vector, `trigger_mode() != 0`, and its `irr` bit set (a level entry keeps `irr` until its line drops — `set_pin_level`).
- [ ] **Step 4: Run the engine suite** — the injection tests from the campaign (`a_vector_pending_at_a_slice_head_is_injected_not_shadow_delivered`, `a_vector_raised_inside_an_errand_is_injected_at_its_tail`, `a_vector_pending_behind_an_errands_mov_ss_lands_after_the_stack_switch`, `a_delivery_never_interrupts_a_context_whose_if_a_shadow_errand_cleared`) are rewritten onto the vCPU-thread harness (`HostTime`, `shared`, the test as device thread) with the same guests and the same assertions: they are the ExtINT path's tests now. Delete none; a test that no longer has a subject (slice heads) is rewritten to the equivalent property on the thread, or its removal is justified in the commit. The `Ticks` tests keep passing through the slice loop.
- [ ] **Step 5: Gates and commit** — `cargo check --release --no-default-features -p rusty_box && cargo xtask ci`; `git commit -m "feat(whp): the hypervisor owns the local APIC; the fabric delivers through it and the 8259 through the window"`.

### Task 1.7: The device thread, the fast machine, and the removal of the slice engine

**Files:**
- Create: `rusty_box_whp_engine/src/device_thread.rs`, `rusty_box_whp_engine/src/fast_machine.rs`
- Modify: `rusty_box_whp_engine/src/engine.rs` (DELETE: `run_slice`'s body, `run_the_exit_loop`, `run_slice_on_the_shadow`, `run_until_the_machine_is_needed`, `install_the_shadow`, `impose_the_shadow`, `read_back_into_the_shadow`, `impose_after_errand`, `burst_on_the_shadow`, `everything_on_the_shadow` and the `WHP_ALL_SHADOW` read (engine.rs:209), the `WHP_MIN_SLICE_US` read (2914) and `shortest_hardware_slice`, `SLICE_RESOLUTION`, `HARDWARE_SPEED`/`hardware_speed`/`default_hardware_speed`, `host_time_exactly`/`host_time_for`/`deadline`/`ticks_elapsed`, the `Yielded` enum, `Started.alarm/shadowed/held/held_interrupt_state/ran_since_read_back/consecutive_mmio`, `BURST_AFTER`, `ExitCounts.boundary`, `SliceCensus`; KEEP: `start()`, `bring_up`, `install_the_machines_map`, `memory_map_changed`, `TRAPPED_MSRS`/`TRAPPED_CPUID_LEAVES`, `ExitCounts`, `InjectCensus`, `PlatformCounters`, `report_the_state_the_platform_refused`, `ExitHistory`), DELETE `rusty_box_whp_engine/src/alarm.rs` (its two-phase wait moves into `device_thread.rs`); `rusty_box_whp_engine/src/lib.rs` (exports; tests moved to the fast machine's verbs); `rusty_box_whp_engine/src/vcpu_thread.rs` (the per-exit wheel catch-up, Step 3); `rusty_box/src/emulator/timers.rs` (8042 arming per H6 under `device_clock`), `rusty_box/src/iodev/keyboard.rs` (`needs_serial_tick()`), `rusty_box/src/iodev/mod.rs` (the port-dispatch tail enqueues the keyboard one-shot). The wheel's position is already public: `Emulator::ticks(&self) -> u64` (mod.rs:1560) is `pc_system.time_ticks()`
- Test: `device_thread.rs` unit tests (hypervisor-free, `SharedClock` from the fixtures), `fast_machine.rs` gated tests

**Interfaces:**
- Consumes: `VmClockSource<StdClock>`, `Emulator::service_device_time -> CpuResult<DeviceTime>`, `VcpuControl`, `Partition::{suspend_time, resume_time}`, `WhpEngine::partition()`, `bring_up`, `install_control`, `StopCause::GuestPowerOff`.
- Produces:

```rust
// device_thread.rs
/// What the device thread waits on. `served` is bumped and notified after EVERY service, so a
/// waiter (`FastMachine::step`) has a sentinel that actually occurs.
pub struct DeviceThreadControl { wake: Arc<(Mutex<DeviceWake>, Condvar)>, served: Arc<(Mutex<u64>, Condvar)> }   // DeviceWake { earlier_deadline: Option<VmInstant>, stop: bool, run: bool }
impl DeviceThreadControl {
    pub fn deadline_moved_earlier(&self, at: VmInstant);
    pub fn pause(&self); pub fn resume(&self); pub fn stop(&self);
    /// Block until the thread has serviced once more than `since`, or `within` passes. Returns the new generation.
    pub fn wait_served_by(&self, since: u64, within: Duration) -> Option<u64>;
    pub fn served(&self) -> u64;
}
pub fn spawn<T: Instrumentation + Send>(machine: Arc<Mutex<Box<Emulator<T, WhpEngine>>>>, clock: Arc<Mutex<VmClockSource<StdClock>>>, vcpus: Vec<VcpuControl>) -> (JoinHandle<()>, DeviceThreadControl);
/// One service: catch the wheel up to the clock and run the boundary. Testable without a thread.
pub(crate) fn service_once<T: Instrumentation, E: SliceEngine<T>, H: HostClock>(machine: &mut Emulator<T, E>, clock: &VmClockSource<H>) -> CpuResult<DeviceTime>;   // generic in the engine: the unit test drives an interpreter machine (`build()` returns Box<Emulator<T, SoftwareEngine>>, builder.rs:437), the device thread a WhpEngine one
// elapsed = clock.now().ticks().saturating_sub(machine.ticks()); machine.service_device_time(elapsed)
// The loop: (1) lock `wake`; wait while !(run || stop); stop → exit. (2) lock `clock` ALONE (never while holding the
// machine): host_deadline = host_instant_of(next_deadline); unlock. (3) wait on `wake` until host_deadline or a
// notification (deadline_moved_earlier/pause/stop) — the two-phase wait from alarm.rs (sleep beyond 2 ms, spin the
// remainder), moved here, not rewritten. (4) lock machine; service_once (which locks clock inside — order machine → clock);
// unlock machine. (5) Ok(dt): next_deadline = dt.next_deadline; dt.stop == Some(GuestPowerOff) → every vcpu.request_park
// (Parked::GuestPowerOff), run = false. Err(e): every vcpu.request_park(Parked::Fault(EngineFault::new(Host, "device time"))), run = false.
// (6) served += 1; notify_all. Loop.

// fast_machine.rs — what a WHP machine IS from now on
/// Why a fast-machine verb refused (R0).
#[derive(Debug)]
pub enum FastMachineFault { Engine(EngineFault), Wedged { waited: Duration }, NoInstructionCount, DeviceClockIsTicks, Platform(WhpError) }
impl Display for FastMachineFault …; impl std::error::Error …

/// How far a step got and why it stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StepOutcome { pub ticks: u64, pub stop: StepStop }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepStop { BudgetSpent, GuestPowerOff, Faulted(EngineFault) }

/// The census of a fast machine — one struct, named fields (R0).
#[derive(Clone, Debug)]
pub struct EngineCensus { pub exits: ExitCounts, pub vcpus: Vec<VcpuCensus>, pub injections: InjectCensus }

pub struct FastMachine<T: Instrumentation + Send = ()> {
    shared: Arc<Mutex<Box<Emulator<T, WhpEngine>>>>, clock: Arc<Mutex<VmClockSource<StdClock>>>,
    vcpus: Vec<(JoinHandle<()>, VcpuControl)>, devices: Option<(JoinHandle<()>, DeviceThreadControl)>, state: RunState /* Paused | Running */,
}
impl<T: Instrumentation + Send> FastMachine<T> {
    /// Take a built machine (`device_clock` must be `HostTime`, else `DeviceClockIsTicks`), start its partition
    /// (`bring_up`), spawn the vCPU thread and install its control, spawn the device thread. Returns it PAUSED.
    pub fn adopt(machine: Box<Emulator<T, WhpEngine>>) -> Result<Self, FastMachineFault>;
    pub fn resume(&mut self) -> Result<(), FastMachineFault>;   // { lock machine; lock clock; start } ; devices.resume(); each vcpu.resume() — partition time resumes on the next run
    pub fn pause(&mut self) -> Result<(), FastMachineFault>;    // each vcpu.request_park(Paused) (NO lock held); each wait_parked_by(5 s) → None ⇒ Wedged; devices.pause(); { lock machine; lock clock; stop; partition.suspend_time() }
    pub fn step(&mut self, budget: RunBudget) -> Result<StepOutcome, FastMachineFault>;
    pub fn with_machine<R>(&mut self, f: impl FnOnce(&mut Emulator<T, WhpEngine>) -> R) -> R;   // paused-only (debug_assert + a documented contract): display(), debug_port(), mem_read, reg_read for tests and the probes
    pub fn engine_census(&self) -> EngineCensus;                   // controls' census() + the machine's exits/inject_census under a brief lock; touches no VP
}
impl<T: Instrumentation + Send> Drop for FastMachine<T> {
    // THE BODY, NOT THE FIELD ORDER, DISCHARGES THE ORDERING OBLIGATION: `shared` is declared first, so
    // field-order dropping would release the machine (and its partition) before the threads that hold a
    // `Vcpu` and a `Canceller` naming it. The body: pause; devices.stop() and join the device thread;
    // each vcpu control.stop() and join its thread (a vCPU thread still wedged after the 5 s bound is
    // DETACHED, never joined — its next platform call fails with an invalid handle and it returns);
    // THEN the fields drop.
}
```
`step(RunBudget::Ticks(n))`: `resume()`; `let start = clock.lock().now(); let target = start.add(VmDuration::from_ticks(n))`; `devices.deadline_moved_earlier(target)` — the step's end is a deadline for the device thread, so it wakes and bumps `served` at the target instead of at the next device deadline; `let wall_deadline = Instant::now() + max(1 s, 10 × n / ips)`; loop { `served = devices.wait_served_by(served, 10 ms)`; if `clock.lock().now() >= target` → break BudgetSpent; if any vcpu's `wait_parked_by(0)` is `Some(GuestPowerOff | Fault(f))` → break with it; if `Instant::now() > wall_deadline` → `pause()` best-effort, `Err(Wedged { waited })` }; `pause()`; `Ok(StepOutcome { ticks: now.since(start).ticks(), stop })`. No machine lock is held across any wait. `RunBudget::Instructions(_)` → `Err(NoInstructionCount)`.

`MachineBuilder::build_on::<WhpEngine>()` keeps returning `Box<Emulator<T, WhpEngine>>`; `FastMachine::adopt` is the one step after it. Callers (the examples, the GUI in Stage 4) call `adopt`. This keeps the machine crate free of any thread type; Stage 4 may revisit once the GUI needs one verb set over both machines.

- [ ] **Step 1: Write the failing tests**

`device_thread.rs` (hypervisor-free — `service_once` on a `Ticks` interpreter machine, whose 8042 continuous timer is armed; the arithmetic under test is engine-independent):
```rust
#[test]
fn service_once_catches_the_wheel_up_to_the_clock_and_names_the_next_deadline() {
    let host = SharedClock::default();
    let mut machine = MachineBuilder::new(EmulatorConfig::default()).build().expect("an interpreter machine");   // Ips::BOCHS_DEFAULT = 50 MHz, 8042 period 150 µs = 7_500 ticks
    let rate = ClockHz::new(machine.config().ips.per_second_u64()).unwrap();
    let start = machine.ticks();
    let mut clock = VmClockSource::stopped_at(VmInstant::from_ticks(start), rate, host.clone());
    clock.start();
    host.advance_nanos(400_000);                                   // 400 µs = 20_000 ticks: two 8042 periods due
    let outcome = service_once(&mut machine, &clock).unwrap();
    assert_eq!(machine.ticks(), start + 20_000, "the wheel is exactly where the clock is");
    let next = outcome.next_deadline.expect("the 8042's continuous timer is armed");
    assert!(next > start + 20_000 && next <= start + 27_500, "the next deadline is within one 8042 period (7_500 ticks) of now: {next}");
    assert!(outcome.stop.is_none() && !outcome.reset_applied);
    let again = service_once(&mut machine, &clock).unwrap();
    assert_eq!(machine.ticks(), start + 20_000, "no host time passed, no ticks earned");
    assert_eq!(again.next_deadline, Some(next));
}
```
`fast_machine.rs` (gated; `EmulatorConfig::default()` is 50 MHz, so 10 ms is 500_000 ticks):
```rust
#[test]
fn step_in_ticks_runs_the_guest_for_that_much_vm_time_and_pauses() {
    if !hypervisor_here() { return; }
    let _turn = a_turn_on_the_hardware();
    let mut machine = FastMachine::adopt(machine_with_devices_on(DeviceClock::HostTime, &[0xEB, 0xFE])).unwrap(); // jmp $
    let ips = machine.with_machine(|m| m.config().ips.per_second_u64());
    let ten_ms = ips / 100;
    let outcome = machine.step(RunBudget::Ticks(ten_ms)).unwrap();
    assert!(outcome.ticks >= ten_ms && outcome.ticks < ten_ms + ips / 500, "honest time within one device-thread wake (2 ms): {outcome:?}");
    assert_eq!(outcome.stop, StepStop::BudgetSpent);
    assert!(machine.with_machine(|m| m.ticks()) >= ten_ms, "the wheel followed the clock");
    let census = machine.engine_census();
    let vcpu = census.vcpus[0];
    assert!(vcpu.in_run_nanos >= 9_000_000, "the vCPU thread was inside WHvRunVirtualProcessor for ≥ 9 of the 10 ms: {vcpu:?}");
    let platform = vcpu.platform_at_last_park.expect("counters refreshed at the park");
    let guest_100ns = platform.runtime.total_100ns - platform.runtime.hypervisor_100ns;
    assert!(guest_100ns > platform.runtime.hypervisor_100ns, "cross-check: guest time exceeds hypervisor overhead: {platform:?}");
    assert!(census.exits.canceled <= 1 && census.exits.total() <= 2, "a spinning guest exits only for the pause's cancel: {:?}", census.exits);
}

/// A halted guest stays parked inside the hypervisor and is woken by the device thread's interrupt.
/// `PIT_HALT_GUEST` (fixtures, hand-assembled and annotated): the SETUP of the guest at lib.rs:640-656
/// (IVT entry 8 → isr; OCW1 unmask IRQ0 alone; PIT ch0 mode 2, count 0x0400) followed by
/// `sti` / `park: hlt` / `jmp park`, with the same isr (`mov al, MARK; out 0xE9, al; mov al, 0x20;
/// out 0x20, al; iret`). No trial loop, no IRR polling — the guest is halted whenever it is not in
/// its handler, which is the state under test. (The original guest at lib.rs:631 reaches its `hlt`
/// only after 0x4000 trials, ~14 s of VM time, so it cannot serve here.)
#[test]
fn a_guest_that_halts_stays_parked_in_the_hypervisor_and_wakes_for_a_device_interrupt() {
    if !hypervisor_here() { return; }
    let _turn = a_turn_on_the_hardware();
    let mut machine = FastMachine::adopt(machine_with_devices_on(DeviceClock::HostTime, &PIT_HALT_GUEST)).unwrap();
    let ips = machine.with_machine(|m| m.config().ips.per_second_u64());
    let mut marks = Vec::new();
    for _ in 0..40 {                                                   // up to 400 ms of VM time
        machine.step(RunBudget::Ticks(ips / 100)).unwrap();
        machine.with_machine(|m| marks.extend(m.debug_port().take_output()));
        if marks.len() >= 5 { break; }
    }
    // PIT mode 2, count 0x400 at 1.193182 MHz = 858 µs per tick: 400 ms of VM time is ~466 ticks; five is the floor.
    assert!(marks.len() >= 5 && marks.iter().all(|b| *b == MARK), "the PIT's ticks woke the halted guest through the ExtINT path: {marks:#04x?}");
    let census = machine.engine_census();
    let n = marks.len() as u64;
    assert_eq!(census.exits.memory, 0);
    assert_eq!(census.exits.port, 4 + 2 * n, "four programming writes (OCW1, PIT control, two counts), then exactly a MARK and an EOI per tick — nothing else leaves the run for a port: {:?}", census.exits);
    assert!(census.exits.canceled <= n + 40, "at most one cancel per delivery (the legacy path's price) plus one per step's pause: {:?}", census.exits);
    assert!(census.injections.injected >= n, "every tick crossed as a pending-event write: {:?}", census.injections);
    // The halt line is P3's: write the ONE of these two that Task 0.3 recorded, citing the probe document.
    //   P3 = "no X64Halt exits under the hypervisor APIC":  assert_eq!(census.exits.halt, 0, "a halted VP stays inside the run: {:?}", census.exits);
    //   P3 = "X64Halt exits occur":                          assert!(census.exits.halt <= n + 1, "one halt exit per park at most: {:?}", census.exits);
}

/// §3.2: a guest reading a timer device sees time pass between two reads with no wheel deadline between them.
#[test]
fn a_pit_read_sees_time_pass_between_two_reads() {
    if !hypervisor_here() { return; }
    let _turn = a_turn_on_the_hardware();
    // PIT_LATCH_GUEST, real mode, hand-assembled and annotated in fixtures:
    //   mov al,0x34; out 0x43,al; xor al,al; out 0x40,al; out 0x40,al          — ch0 mode 2, count 65536
    //   xor al,al; out 0x43,al                                                  — latch ch0
    //   in al,0x40; mov bl,al; in al,0x40; mov bh,al                            — low, high
    //   mov al,bl; out 0xE9,al; mov al,bh; out 0xE9,al                          — report the reading
    //   mov cx,0x8000; spin: loop spin                                          — ~30 µs of real time at hardware speed
    //   (latch, read, report again exactly as above)
    //   jmp $
    let mut machine = FastMachine::adopt(machine_with_devices_on(DeviceClock::HostTime, &PIT_LATCH_GUEST)).unwrap();
    let ips = machine.with_machine(|m| m.config().ips.per_second_u64());
    machine.step(RunBudget::Ticks(ips / 100)).unwrap();
    let out = machine.with_machine(|m| m.debug_port().take_output().collect::<Vec<u8>>());
    assert_eq!(out.len(), 4, "two 16-bit readings: {out:#04x?}");
    let first = u16::from_le_bytes([out[0], out[1]]); let second = u16::from_le_bytes([out[2], out[3]]);
    assert_ne!(first, second, "the PIT counted between the reads although no device deadline lay between them — the exit path caught the wheel up from the clock");
}
```
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement** `device_thread.rs` and `fast_machine.rs`. **The per-exit wheel catch-up (spec §3.2):** in `VcpuThread::service`, after taking the machine lock and before dispatching any `IoPortAccess`/`MemoryAccess` arm, `service_once(&mut machine, &clock)?` (the thread holds a clone of the `Arc<Mutex<VmClockSource>>`; `service_once` locks the clock INSIDE the machine lock — order machine → clock) so a PIT/PM-timer/HPET read answers from the wheel at this instant; the no-work fast path in `service_scheduler_boundary` keeps this cheap when nothing is due. Then the H6 arming: `rearm_device_timers_after_hardware_reset` (timers.rs:177) registers the keyboard continuous under `Ticks` and does NOT arm it under `HostTime`; `BxKeyboardC::needs_serial_tick(&self) -> bool` = `timer_pending != 0 || irq1_requested || irq12_requested`; the port-dispatch tail in `BxDevicesC` (iodev/mod.rs, where `scheduler_boundary_requested` is latched at 620/823) enqueues `TimerRequest::Activate { deadline_ticks: now + KBD_SERIAL_DELAY_USEC × ips / 1e6, period_ticks: 0, continuous: false }` for `DeviceTimerOwner::Keyboard` when `device_clock == HostTime && needs_serial_tick()` and the timer is not already armed; `fire_keyboard_timer` (482) re-arms the same way while `needs_serial_tick()` still holds after the fires. Then DELETE the slice engine as listed; `WhpEngine::run_slice` becomes `Err(CpuError::UnsupportedCpuOperation { operation: "a machine on the hypervisor is driven by FastMachine, not by slices" })` — it is never called once the machine is adopted (the scheduler runs only under `Emulator::step`, which `FastMachine` never calls), and the refusal is the R5 choke point for a caller that forgot to adopt. Send assertions in `lib.rs`: `ss::<DeviceThreadControl>(); s::<FastMachine<()>>();`.
- [ ] **Step 4: Move the engine crate's tests** onto `FastMachine` (`machine_running`/`machine_with_devices` gain a `HostTime` twin `machine_with_devices_on`; the slice-loop tests either become `step`-driven with the same guests and assertions or are named in the commit with the property that replaced them — `a_port_write_and_a_halt_are_two_exits_in_one_slice` and `a_slice_ending_on_an_errand_skips_the_read_back_and_the_machine_reads_the_shadow` have no subject without slices and are removed with that justification; `trace_of` uses `step(Ticks)` on a `FastMachine`). Every retained test passes.
- [ ] **Step 5: `cargo test --release -p rusty_box --lib --features std` and `cargo check --release --no-default-features -p rusty_box`** (keyboard.rs, iodev/mod.rs, timers.rs, pc_system.rs are no_std-compiled).
- [ ] **Step 6: Gates and commit** — `cargo xtask ci`; `git commit -m "feat(whp): the machine runs its devices on host time and its guest on an unbounded processor"`.

### Task 1.8: The harnesses on the fast machine

**Files:**
- Modify: `rusty_box_whp_engine/examples/alpine_probe.rs`, `dlx_whp.rs`, `alpine_bench.rs`, `step_bench.rs`, `compute_bench.rs` (config `device_clock: HostTime`; build → `FastMachine::adopt`; `step(Ticks)` → `StepOutcome`; the screen scrape and census reads go through `with_machine`/`engine_census`; a `FastMachineFault::Wedged` is printed as `RESULT wedged waited=<s>` and the run exits non-zero)
- Test: the examples build (`cargo build --release -p rusty_box_whp_engine --examples`); `dlx_whp` boots 3/3.

- [ ] **Step 1:** Adapt each example; keep every printed `RESULT` line's name. In `alpine_probe`: the `census.slices` field of the 10-second line becomes `runs=<VcpuCensus.runs>`; sample `(wall, in_run_nanos, runtime.total_100ns, runtime.hypervisor_100ns, exits)` at EVERY milestone hit (the `seen[index] = Some(..)` site, alpine_probe.rs:225-232, already runs once per milestone) and print, at the end, one line per adjacent pair: `RESULT in_run_share phase=<A>..<B> share=<(in_run_nanos_B − in_run_nanos_A) / (wall_B − wall_A)> guest_share=<((total−hyp)_B − (total−hyp)_A) / (total_B − total_A)> exits=<exits_B − exits_A>`; the kernel phase is `"Linux version".."login:"` (the ISOLINUX milestone records the loader's first APPEARANCE, never its exit). For the fatal-signature scan, accumulate the set of distinct screen lines seen across all samples (the 80×25 scrape is a snapshot; a panic that scrolls off between two samples must still count) and print `RESULT fatal_signatures=<n>` over that set.
- [ ] **Step 2:** `cargo build --release -p rusty_box_whp_engine --examples`.
- [ ] **Step 3:** `dlx_whp` ×3: 3/3 milestones each. Record wall.
- [ ] **Step 4:** Commit — `git commit -m "chore(whp): the measurement harnesses drive the fast machine"`.

### Task 1.9: Stage 1 gate

- [ ] DLX on fast mode: 3/3 milestones, three runs, wall recorded (G5 half).
- [ ] Alpine on fast mode via `alpine_probe`, 300 s patience, the baseline ISO: reaches `login:`; wall < the interpreter's median. **Measure the comparison in ONE interleaved sweep, not against this document's numbers.** The baseline document's figures were taken on a shared host on a different day, and the standing rule (memory `perf-measurement-methodology`) is that absolute wall-clock here is not comparable across days. So the gate sweep is three rounds of {fast mode, interpreter}, alternating, in one session, load recorded per run, compared by median with a 3.7 % noise floor. The baseline document is the record of where head stood, not the number the gate subtracts from; `RESULT in_run_share` over the kernel phase ≥ 0.90 in each run (G1, measured as `Instant` deltas around `vcpu.run()`); `guest_share` over the same window reported beside it as the platform's cross-check. **The window is `"ISOLINUX"` → `login:`, not `"Linux version"` → `login:`.** Task 0.1 measured that `"Linux version"` is never recorded on either engine — the 80×25 scrape samples after that line has scrolled away, while later milestones survive — so a window opening there is uncomputable. `"ISOLINUX"` is caught reliably at 0.3–0.8 s, and the boot-loader time it adds is under a second against a phase of minutes. If Task 1.8's accumulate-distinct-lines scrape (added there for the fatal-signature scan) makes `"Linux version"` reliable, prefer it and say so in the gate report.
- [ ] The same three Alpine runs: `RESULT fatal_signatures=0` (no `Oops`, no `general protection`, no `unexpected_intr` in any line seen).
- [ ] `cargo xtask ci` green; the `unsafe` token baselines exactly as Task 0.2 left them (`rusty_box_whp/src` 1 — `map_borrowed`'s signature — `rusty_box_whp_sys/src` at its bound-entry-point count, `rusty_box_whp_engine/src` 1); `Send` assertions present for `Vcpu`, `VpCounters`, `Emulator<(), WhpEngine>`, `VcpuControl`, `VcpuThread<()>`, `DeviceThreadControl`, `FastMachine<()>`.
- [ ] Baseline document gains a "Stage 1" row block with the numbers, including `in_run_share` and exits by class over the kernel phase; if G0 is still unfilled, say so.
- [ ] Recorded as unproven on hardware until Stage 2: the level-triggered EOI resample (hypervisor-free test only in 1.6 — no Stage 1 guest programs a level entry); the idle-at-`login:` exits/s figure G1's second half names (Stage 2 measures it with the enlightenments).
- [ ] **Stop for user review.** Stages 2–4 are planned from these measurements.

---

## Stages 2–4 — outline (each gets its own plan after Stage 1's gate)

**Stage 2 — long runs.** (a) Exit clustering: `rusty_box_whp_engine/src/clustering.rs`, a `(rip, ExitClass)` history table (256 entries, VirtualBox's thresholds: probe after 8 hits, run ≤ 8192 instructions, stop after 32 without an exit, demote after 512 useless probes) called from `VcpuThread::service` before the per-class import; the stretch uses `import_everything`/`emulate_batch`/`export_imported`, and a PIC/IOAPIC interrupt raised during a stretch is delivered by the interpreter's own path on the shadow (the fabric backend is bypassed for the stretch's duration — the machine lock is held throughout); the unserviced-exit faults of Task 1.5 gain the bounded-stretch retry here. (b) Icache epoch: `BxICache` entries carry a `u32` epoch; `discard_decoded_traces` bumps it; lookup compares. (c) Enlightenments: `start()` sets `synthetic_features(OPENVMM_VTL0)` unless the machine profile hides them; `Hypercall` exits answered with `HV_STATUS_INVALID_HYPERCALL_CODE`; stray synthetic MSR writes ignored, reads 0; the profile knob on `EmulatorConfig`. (d) The level EOI resample proven on hardware (a guest with a level IOAPIC entry). Gate: G1–G3 against G0, idle exits/s.

**Stage 3 — transfer.** (a) LAPIC page ↔ `BxLocalApic` conversion (`cpu/apic.rs`: `load_from_page`/`store_to_page`, register offsets 0x20…0x3E0, byte offset = MMIO offset) plus a NEW re-arm entry — the model has none today: it takes the page's current count and divide, sets `ticks_initial = now − (timer_initial − ccr) × divide` so `get_current_timer_count` (apic.rs:1972-1981) and the snapshot epoch check (apic.rs:2620-2637) stay coherent, and arms `now + ccr × divide`; in TSC-deadline mode the count is 0 and the entry converts the deadline MSR (a guest-TSC value) to a machine tick instead; `store_to_page` derives the current count from the wheel deadline; (b) `FastMachine::pause` does the full import + page read; `resume` the full export + page write + TSC write under `suspend_time`; (c) snapshot = pause + `save_snapshot`; restore = `restore` + resume; (d) `FastMachine::into_precise(self) -> Box<Emulator<T, SoftwareEngine>>` and `FastMachine::from_precise(Box<Emulator<T, SoftwareEngine>>) -> FastMachine<T>` — the engine type parameter changes at the transfer, so the machine's `engine: E` field is swapped through `Emulator::<T, E>::with_engine_swapped::<E2>(self, e2: E2) -> Box<Emulator<T, E2>>` (a move of every field but the engine), and `device_clock` flips with it; (e) the precise-mode Hv#1 component (`rusty_box/src/cpu/hyperv.rs`, spec §3.6) with its snapshot section. Gate: G4, G5.

**Stage 4 — surface.** `run_interactive` on the front-end thread for a `FastMachine` (input pump under the lock, 40 ms display refresh from the LFB dirty bitmap), `rusty_box_gui` `drive<E>` split into two arms by engine, `StopHandle` = `pause` + terminal, docs (`docs/getting-started.md`, `CLAUDE.md` architecture block), memory updates. Gate: G6 and the whole suite.

---

## Self-review (run at write time, re-run after the four-lens review)

- **Spec coverage:** §3.1 threads → Tasks 1.5, 1.7 (machine lock, `Send` assertions incl. `Emulator<(), WhpEngine>`, panic = abort stated, lock order machine → clock); §3.2 time → 1.2 (`VmClockSource` over `VmInstant`), 1.3 (`service_device_time`, `tickn` catch-up, `DeviceClock`), 1.7 (device thread, pause/resume, the per-exit wheel catch-up for PIT/PM/HPET reads, H5–H8 in 0.4); §3.3 interrupts/LAPIC → 1.6 (mode by `device_clock`, LVT0 tracking, ExtINT with the pure readiness predicate, request backend with `Model` fallback and `Refused`, the EOI resample, the dead carve-out); §3.4 exits → 1.4, 1.5 (clustering and icache epoch → Stage 2 outline); §3.5 transfer → Stage 3 outline; §3.6 Hv#1 → Stage 3 outline; §3.7 probes → 0.3 (per-run rows for P1; P2 named-register write and P5 LVT0-masked delivery included); gates → 0.5, 1.9; failure handling → 1.5 (`Parked::Fault`, poison under unwind only), 1.7 (drop body, detach on wedge); tests → each task. Enlightenments (§3.3) are Stage 2 by the spec's own staging.
- **Reconciled with the four-lens plan review (2026-09-03; findings archived in `docs/research/whp-2026-09-03/08-plan-review-*.md`):** every blocking and major finding is applied above, with one refuted on re-reading the tree — "this port has no IRQ0→pin2 override": it does, inside `BxIoApic::set_pin_level` (ioapic.rs:698-702, Bochs "timer connected to pin #2"), not in `IrqFabric`; the IOAPIC edge test therefore programs entry 2 for the PIT as first written, and drives the PIT from inside the guest because the fabric's raise verbs are `pub(crate)`. The measurement corrections (in-run share as `Instant` deltas, guest time = total − hypervisor as the cross-check, the kernel window `"Linux version".."login:"`, three interpreter runs through `alpine_probe`, the original ISO with its hash, host conditions, two W7 stamps) are in 0.1, 1.7, 1.8, 1.9. A final read-only accuracy pass over the rewritten Stage 1 (2026-09-04, ~245 tree claims checked) found six wrong and eleven minor statements — the six per-shape `Vcpu` verbs with live callers, `WhpError::hresult()` not `code()`, no `is_sse_or_avx` in the tree, `service_once` generic in the engine, a PIT guest that actually halts, `Emulator::reset` for the pin-edge field — all applied above.
- **Placeholder scan:** no TBD/TODO; every test has a body; every helper a test names is either an existing symbol with its line or declared in a Produces/fixtures block (`SharedClock`, `wait_until`, `shared`, `machine_with_devices_on`, `program_ioapic_entry` (emulator/tests.rs) and `program_ioapic_pin` (irq.rs), `furnished_machine`, `furnished_machine_on`, `MapWatchingEngine`, `RecordingCancel`, `PIT_HALT_GUEST`, `PIT_LATCH_GUEST`, `IOAPIC_EDGE_GUEST`, `load_real_mode_code`).
- **Type consistency:** `Vcpu`/`VpCounters` (1.1) are what 1.4–1.7 consume; `VmClockSource<StdClock>`/`VmInstant` (1.2) are what 1.7 drives; `IoApicDelivery`/`DeliveryRoute::{Model, Backend, Refused}`/`Processor`/`DeviceTime`/`DeviceClock` (1.3) are what 1.5–1.7 use; `ArchGroups`/`Exchange`/`ExitClass` (1.4) are what 1.5 uses; `VcpuControl::{request_park(Parked), raise_ext_int, wait_parked_by, census}`/`Parked`/`VcpuCensus`/`bring_up`/`install_control` (1.5) are what 1.6/1.7 use; `FastMachine::{adopt, step → StepOutcome, engine_census → EngineCensus}`/`FastMachineFault` (1.7) are what 1.8 calls. `hypervisor_present()` is the leaf's name, `hypervisor_here()` the engine's.
- **Known open point carried, not hidden:** Task 1.7's `build_on` return type — the plan chooses `FastMachine::adopt` over an associated type on the unsealed trait; Stage 4 may revisit once the GUI needs one verb set over both machines.
