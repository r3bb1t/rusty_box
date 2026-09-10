# Alpine Reaches `login:` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unblock Alpine's benchmark from a harness bug, and produce a measured verdict naming why DLX loses IRQ 14 — without guessing at a fix.

**Architecture:** Two independent changes to example programs. `alpine_bench` gains the `host_shared` CPUID clamp that `alpine_probe` already applies. `dlx_whp` prints two numbers it already has access to — the fabric's acknowledge count and the census's per-vector injection tally — which together distinguish three candidate causes in one run. No engine code changes.

**Tech Stack:** Rust, `rusty_box_whp_engine` examples, Windows Hypervisor Platform.

**Spec:** `docs/superpowers/specs/2026-09-08-alpine-reaches-login-design.md`

## Global Constraints

- Read `CLAUDE.md` first and follow it. **Edit with the Edit/Write tools, never shell heredocs, `sed -i`, or Python**; release builds only; **never run `cargo fmt`**; never `let _ = <Result>`; comments state today's invariant, never history; **Bochs citations name file + symbol, never line numbers**.
- **Never use the LSP tools** (`mcp__lsp__references`/`definition`/`diagnostics`) — they hang indefinitely in this workspace. The compiler is the oracle. rust-analyzer here also emits stale false-positive errors; reproduce every diagnostic with a real cargo command.
- `cargo xtask ci` before every commit. **Never pipe it and never append to its line** — a pipeline's exit code masks the gate's. Redirect to a log, read it for `ci: N steps passed`, grep it for `FAILED`.
- **A gate run here can be KILLED, not failed**, by another project's session running `Stop-Process` on `xtask.exe` by name. Signature: the log stops mid-step, `grep -c FAILED` is 0, exit code `0xffffffff`. Retry once before concluding anything. Do not wait on foreign `cargo`/`xtask` processes — check their paths first (`Get-Process xtask | Select Id,Path`); the `work_backend` ones are dev servers that never exit and share no build lock. **Never kill a process you did not start.**
- **This host corrupts timing measurements.** It has entered Modern Standby mid-run (245 s of a 360 s run) and dropped to battery at half clock (BogoMIPS 2254 vs ~4600–5560). Neither shows in benchmark output. Before quoting any timing number, confirm AC power and no standby episode in the run's window via the Windows System event log, and state that alongside the number.
- **Never stage** `ROADMAP.md`, `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md`, or the three untracked `docs/superpowers/specs/2026-08-22-*.md` files. Explicit `git add <path>` only; never `git add -A`. **Leave `stash@{0}` alone.**
- Commit messages end with: `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
- Branch: `wip/atom-execctx`. Do not create branches.
- Four WHP tests flake under host load. Reproduce with the FULL `-p rusty_box_whp_engine --lib` suite; a single-test run filters out the others and is not a fair reproduction.

---

### Task 1: `alpine_bench` clamps CPUID to what the host can carry

Alpine's kernel comes up and then dies writing back `xcr0 = 0xe7` — AVX-512 state — to a partition on a CPU without AVX-512. `alpine_probe` and the GUI narrow the guest's features to what the host's XSAVE can carry; this benchmark never did.

**Files:**
- Modify: `rusty_box_whp_engine/examples/alpine_bench.rs`

**Interfaces:**
- Consumes: `BxParams`, `X86Feature` (already imported by `alpine_probe.rs`; add the same imports here), `EmulatorConfig` (`alpine_bench.rs`'s `config`).
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Read the working version first**

Read `host_shared` in `rusty_box_whp_engine/examples/alpine_probe.rs` and its call site (`cpu_params: host_shared(topology)`). **Copy it verbatim** rather than writing your own — the two examples should narrow identically, and a second, subtly different clamp is worse than none. Note `core::arch::x86_64::__cpuid_count` is safe to call on x86_64 and needs no `unsafe` block.

- [ ] **Step 2: Where the clamp goes (verified — no investigation needed)**

`EmulatorConfig` has a `pub cpu_params: BxParams` field (`rusty_box/src/emulator/mod.rs`), defaulting to `BxParams::default()`. `alpine_probe` sets `cpu_params: host_shared(topology)` in its own config literal.

`alpine_bench`'s `config(clock: DeviceClock) -> EmulatorConfig` currently sets `memory`, `memory_block_size`, `ips`, `pci_enabled` and `device_clock`, then `..EmulatorConfig::default()` — so it inherits `BxParams::default()` unclamped. Add `cpu_params` to that same literal.

`config` takes no topology today. Read how `alpine_probe` obtains the `BxParams` it passes to `host_shared` and follow it: if it derives one from a topology helper, use the same helper; if `BxParams::default()` is the base, then `cpu_params: host_shared(BxParams::default())` is the whole change. **Both examples must end up narrowing identically** — that is the point.

- [ ] **Step 3: Apply the clamp**

Add `host_shared` to `alpine_bench.rs`, copied from `alpine_probe.rs`, with a doc comment recording why the benchmark needs it:

```rust
/// `CpuCapabilities::HostShared::narrow` from `rusty_box_gui/src/config.rs`,
/// in effect: drop AVX-512 / AVX when the host cannot carry their XSAVE
/// state, and MONITOR/MWAIT unconditionally.
///
/// Without this the guest enables state the partition cannot hold, and the
/// first write-back of a 64-bit kernel context fails
/// `WHvSetVirtualProcessorRegisters` with `0xC0350005` — measured on an
/// i5-12450H, which has no AVX-512, at `xcr0 = 0xe7`.
```

Apply it at the same point in the machine's construction that `alpine_probe` does.

- [ ] **Step 4: Verify it builds and the clamp is reached**

```bash
cargo build --release -p rusty_box_whp_engine --example alpine_bench
```
Expected: clean build.

The clamp is only meaningful if it actually narrows on this host. Confirm by running the bench (Step 5) and checking the fault is gone — a clamp that compiles but does not change the guest's features would leave the identical `0xC0350005` at `xcr0 = 0xe7`.

- [ ] **Step 5: Run the benchmark and record BOTH arms**

```bash
cargo run --release -p rusty_box_whp_engine --example alpine_bench
```

Record, verbatim: host time to `BIOS`, `ISOLINUX` and `login:` for the interpreter arm and the hypervisor arm, and the bench's own summary lines.

**Then check the Windows System event log for standby or power-source events inside the run's window and state what you found.** A run that half-slept is not a measurement. Both arms of the previous run were clean; if yours were not, say so and re-run, reporting both.

**Report the hypervisor's outcome truthfully:**
- If it still faults, give the new HRESULT, RIP and `xcr0`.
- If it now gets further and stops elsewhere, name where.
- If it reaches `login:`, give the time and the interpreter's time beside it. Note the interpreter's own figures swung 110.8 s–149.6 s across two clean consecutive runs, so **one interpreter number is not a baseline** — say what you measured, not what it "should" be.

- [ ] **Step 6: Gate and commit**

```bash
cargo xtask ci > /tmp/gate-clamp.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-clamp.log
grep -c FAILED /tmp/gate-clamp.log
```

```bash
git add rusty_box_whp_engine/examples/alpine_bench.rs
git commit -m "fix(whp): alpine_bench clamps CPUID to what the host's XSAVE can carry

Alpine's kernel comes up and then dies writing back xcr0 = 0xe7 — AVX-512 state
— to a partition on a CPU that has no AVX-512, failing
WHvSetVirtualProcessorRegisters with 0xC0350005 at 8-10s. The benchmark built
from EmulatorConfig::default() and never applied the host_shared narrowing that
alpine_probe and the GUI both do.

A benchmark that cannot run the guest measures nothing.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: `dlx_whp` prints the two numbers that name the IRQ-14 defect

DLX loses IRQ 14 (IDE, slave 8259 via the cascade) after a few commands while IRQ 0 (PIT, master) keeps firing at ~100/s. A slave vector that is acknowledged but never delivered leaves the master's cascade line, IRQ2, in service forever — which blocks every slave IRQ and leaves higher-priority IRQ 0 untouched. That is the shape of the symptom, but it has not been measured. These two numbers decide it.

**Files:**
- Modify: `rusty_box_whp_engine/examples/dlx_whp.rs`

**Interfaces:**
- Consumes: `census.injections.injected` and `census.injections.windows_armed` (already printed by this example); `census.injections.injected_per_vector: [u32; 256]` (`engine.rs`, collected at every placement, never printed); the fabric's `acknowledge_count() -> u64` (`rusty_box/src/iodev/irq.rs`, `pub`).
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Establish how to reach the acknowledge count from this example**

`acknowledge_count` is `pub` on `IrqFabric`. The engine crate's tests reach it as `guard.processor(0).io.device_manager().irq().acknowledge_count()`. Establish the equivalent path from `dlx_whp`'s machine handle — it holds a `FastMachine`, so `with_machine(|m| …)` is the likely route.

If the count cannot be reached from an example without adding a public accessor, STOP and report that. **Do not add engine API for a diagnostic** without saying so first — this task is meant to print numbers that already exist.

- [ ] **Step 2: Print the per-vector tally and the acknowledge count**

`dlx_whp` already prints, near the end of a run:

```rust
    println!(
        "  injected {} windows_armed {}",
        census.injections.injected, census.injections.windows_armed
    );
```

Extend that report with the two numbers the verdict needs. Print only non-zero vectors — a 256-entry array is unreadable and the interesting fact is which vectors appear at all:

```rust
    // Every vector this engine placed, and every acknowledge the machine's own
    // controllers performed. They must match: an acknowledge without a placement
    // is a vector taken from the 8259 and never delivered, and on the SLAVE that
    // is unrecoverable — the master's cascade line stays in service until the
    // guest EOIs it, so every later slave IRQ is blocked while IRQ 0, being
    // higher priority, keeps arriving. The retired hardware test asserted this
    // same equality.
    let placed: std::vec::Vec<(usize, u32)> = census
        .injections
        .injected_per_vector
        .iter()
        .enumerate()
        .filter(|(_, count)| **count != 0)
        .map(|(vector, count)| (vector, *count))
        .collect();
    println!("  placed per vector: {placed:?}");
    println!("  acknowledges {acknowledges}  injected {}", census.injections.injected);
```

where `acknowledges` is read via the path established in Step 1.

- [ ] **Step 3: Build**

```bash
cargo build --release -p rusty_box_whp_engine --example dlx_whp
```
Expected: clean build.

- [ ] **Step 4: Run once and read the verdict**

```bash
cargo run --release -p rusty_box_whp_engine --example dlx_whp
```

Check the System event log for standby/power events in the run's window first, and say what you found.

Then state which of these three the numbers select. **Do not fix anything** — naming the cause is this task's entire deliverable:

- **`acknowledges > injected`** → vectors are acknowledged and never delivered. The INTA/entry window is real, and the cascade wedge follows. The next spec is applepie's model: deliver on the shadow so acknowledge and delivery are one step.
- **`acknowledges == injected`, and `injected_per_vector[14]` non-zero but static after early boot** → delivery is sound; the IDE stopped raising. That is a device investigation, not an interrupt one.
- **`injected_per_vector[14]` zero throughout** → vector 14 is never acknowledged at all. The pin never rises, or a gate refuses it — start at `stage_the_legacy_interrupt`'s `lint0_admits_ext_int` and `permits_ext_int` gates.

Report the full per-vector list either way: which vectors appear, and their counts. Vector 8 is the PIT, vector 14 is the IDE on the slave's default base.

- [ ] **Step 5: Gate and commit**

```bash
cargo xtask ci > /tmp/gate-census.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-census.log
grep -c FAILED /tmp/gate-census.log
```

```bash
git add rusty_box_whp_engine/examples/dlx_whp.rs
git commit -m "diag(whp): dlx_whp reports placements per vector and acknowledges against them

DLX loses IRQ 14 after a few IDE commands while IRQ 0 keeps arriving at ~100/s.
A slave vector acknowledged and never delivered leaves the master's cascade line
in service forever, which blocks every slave IRQ and leaves higher-priority IRQ 0
untouched — the exact shape of the symptom.

The census already counts placements per vector and the fabric already counts
acknowledges; neither was printed. Their equality is the invariant the retired
hardware test asserted, and their difference names the defect.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage.** The spec's clamp is Task 1; its invariant-and-verdict is Task 2; its "deliberately not in this design" (no fix for the window) is honoured — neither task changes engine code, and Task 2 Step 4 ends at naming a cause. The spec's measurement discipline appears in Global Constraints and again in both tasks' run steps. The spec's success criteria map one-to-one onto Task 1 Step 5 and Task 2 Step 4.

**2. Placeholder scan.** No TBD/TODO. Both code steps carry their code. The two steps that direct investigation rather than transcription — finding `alpine_bench`'s CPU-params path, and reaching `acknowledge_count` from an example — each say what to match, and each say to STOP and report rather than improvise if the assumption fails. That is deliberate: both are places where the spec could be wrong about the codebase, and an implementer inventing a path would hide that.

**3. Type consistency.** `injected_per_vector` is `[u32; 256]` and is iterated as such; `acknowledge_count()` returns `u64` and is only printed; `census.injections` is the field name `dlx_whp` already uses. No new signatures are introduced by either task.

**4. Risks this plan does not remove.**

- **Task 2's reach is half-verified.** `acknowledge_count` IS `pub` (`iodev/irq.rs`), and Task 1's clamp site is confirmed (`EmulatorConfig::cpu_params`, `emulator/mod.rs`). What remains unverified is only the path from `dlx_whp`'s `FastMachine` handle down to the fabric; Step 1 says to stop rather than add engine API silently.
- **The verdict may be none of the three.** The three causes are exhaustive over *these two numbers*, not over reality — a fourth possibility (for example, the count matching while the delivered vector is wrong) would show as equal counts with an unexpected per-vector list. The instruction to report the full list regardless is what catches that.
- **This plan does not make Alpine faster, and does not claim to.** It removes one harness bug and produces one verdict. The user's bar — Alpine to `login:` faster than the interpreter — belongs to the spec written against that verdict.
