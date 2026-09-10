# WHP Fast Path Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Windows Hypervisor Platform engine beat this port's own interpreter on a DLX Linux boot, by letting the guest actually run instead of leaving the partition after every exit.

**Architecture — REWRITTEN 2026-08-29 on measurement. The original premise was
wrong and Task 4 is dead; read this before anything below.**

The engine does *not* return `Yielded::Boundary` after each exit (157 of 217,640
slices did). What it does is pay for every exit twice: a full 52-register state
exchange in and out of the shadow processor, and a slice short enough to hold
exactly one exit. Two measurements set the whole direction:

**The hypervisor is 81.9× the interpreter when the guest touches no device**
(`cargo run --release -p rusty_box_whp_engine --example compute_bench`):
100,000,003 instructions, 1.32 s interpreted against 0.02 s on hardware —
75.6 M/s against 6,190 M/s. Native execution is not the problem and never was.

**A DLX boot is 27.8 s against the interpreter's 10.6 s, and 93 % of its memory
exits are one thing**: 65,536 of 70,715 land in the VGA planar aperture
`0xa8000`–`0xaf000`, exactly 8,192 per 4 KiB page — four plane passes over 2,048
words, the kernel clearing video memory one `mov` at a time. At ~136 µs an exit
that is ~8.9 s of the 27.8 s spent clearing the screen.

So the deficit is not spread across the boot; it is concentrated in tight MMIO
loops where the guest executes one trapping instruction after another in the
same region. For a 65,536-instruction VGA clear the *interpreter* is the right
engine — it does that work with zero exits, which is precisely why it wins.

This plan therefore stops trying to make exits cheap enough to win, and instead
stops taking them: run on hardware where hardware is good, and hand the shadow a
**burst** when the guest enters an MMIO-dense phase. Supporting work: narrow the
per-exit exchange (wtf reads 3 registers on an exit; we move 52 each way), and
decouple slice length from the guest deadline (`HARDWARE_SPEED = 32` divides a
guest-time deadline into a host-time budget 32× shorter, which is a self-
inflicted 32× multiplier on slice count).

**Tech Stack:** Rust 2021, `windows-sys` (WHP FFI, confined to `rusty_box_whp/src/sys/windows.rs`), the in-tree Bochs-derived interpreter as the shadow processor.

**Spec:** `docs/superpowers/specs/2026-08-29-whp-fast-path-and-api-redesign-design.md` §1, §4, §7. The two-API redesign (§5) is a **separate plan** — do not start it here.

## Global Constraints

- Branch is `wip/atom-execctx`. Do NOT create a new branch. Do NOT push.
- **Never stage `ROADMAP.md`** — it is the owner's file. Stage with `git add -A -- . ':!ROADMAP.md'`.
- **Never run `cargo fmt`** — this crate is not rustfmt-clean; it rewrites 169 files.
- Release builds only: every `cargo` invocation carries `--release`.
- After each edit batch: `cargo check --release -p rusty_box --features std --lib` **and** `cargo check --release --no-default-features -p rusty_box`. The no_std build is a different compile, not a subset.
- Before every commit: `cargo xtask ci` must pass (22 steps). Commit only the tree the gates saw.
- Use the Edit/Write tools for edits. A scripted edit is justified only for a genuinely repetitive rewrite across many files — say so, show the script, and re-read a sample.
- `unsafe` lives only in `rusty_box_whp/src/sys/windows.rs`; every block names its invariant owner. The xtask ratchet fails on an increase.
- Bochs-source comments cite file + symbol, never line numbers.
- Comments state the invariant that holds today. No history, no "this used to", no before/after.
- No `let _ = <Result>`; no bare tuples or meaningless bools in public returns (doctrine R0).
- Commit messages end with: `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`

## Baseline to beat

Re-measured 2026-08-29 after the `prev_rip` fix (`8ac9c94`), which is what made
WHP reach login at all.

| | Time to `dlx login:` |
|---|---|
| Interpreter, headless (`RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=450000000`) | **10.6 s** |
| WHP | **27.8 s** — 2.6× slower, the gap this plan closes |
| WHP, `WHP_ALL_SHADOW=1` | ~92 s (bisection mode, not a target) |

Where the 27.8 s goes: ~2.4 s of guest execution on hardware, 6.8 s of
hypervisor time (74 % of the 9.2 s of processor runtime), and ~18.6 s of
host-side servicing — 185,724 exits at ~136 µs each.

And the case the whole engine exists for, which the boot number says nothing
about:

| `compute_bench`, 100,000,003 instructions, no device access | |
|---|---|
| Interpreter | 1.32 s — 75.6 M instructions/s |
| WHP | **0.02 s — 6,190 M instructions/s, 81.9×** |

The DLX WHP example is `rusty_box_whp_engine/examples/dlx_whp.rs`
(`DLX_WHP_PATIENCE_SECS` bounds it); the compute benchmark is
`rusty_box_whp_engine/examples/compute_bench.rs`.

---

### Task 1: Wrap the hypervisor's own performance counters — **DONE**

> **Implemented and measured. Four things written below turned out to be wrong;
> the shipped code follows the measurement, not this sketch. Read these before
> trusting any number this task's sketch quotes.**
>
> 1. **`WHV_PROCESSOR_INTERCEPT_COUNTERS` has FOURTEEN classes, not the eleven
>    sketched.** The three missing ones are `NestedPageFaultIntercepts`,
>    `Hypercalls` and `RdpmcInstructions` — and the first of those is where an
>    unmapped or permission-refused guest-physical access lands, i.e. *exactly*
>    the MMIO exits Tasks 4–6 exist to remove. Following the sketch literally
>    would have made this plan's own subject invisible to its own instrument.
> 2. **The buffer is 224 bytes (28 `u64`), not the 176 the sketch sized.** Step 7's
>    `[0u64; 22]` was short by six words. The shipped call takes `&mut [u64]`
>    rather than `&mut [u8]`, so the platform's 8-byte writes land on an aligned
>    buffer by construction rather than by luck.
> 3. **The platform does NOT refuse counters for a processor that has never
>    run** — it returns zeros. Step 7's `# Errors` clause claiming otherwise is
>    wrong, and a test asserting a refusal there would fail.
> 4. **Hyper-V does not increment `HaltInstructions.Count`.** See Task 3's
>    mapping table — this is the one that changes a later task.
>
> Ratchet note, flagged by the implementer and accepted: `rusty_box_whp/src`'s
> unsafe baseline went 35 → 36. A new platform call is the one reason that count
> rises; the block is confined to `sys/windows.rs` like every other, and the
> re-baseline carries that justification inline in `xtask/src/ci.rs`.



`WHvGetVirtualProcessorCounters` reports per-intercept-class **count and time**, and total-vs-hypervisor runtime, with no instrumentation of ours. It is the independent check on Task 3's census: if our numbers and the platform's disagree, one of them is measuring the wrong thing and we must know before redesigning.

**Files:**
- Modify: `rusty_box_whp/src/sys.rs` (add `CounterSet` to the crate-internal enums)
- Modify: `rusty_box_whp/src/sys/windows.rs` (the FFI call)
- Modify: `rusty_box_whp/src/sys/unsupported.rs` (the non-Windows shim)
- Modify: `rusty_box_whp/src/partition.rs` (the safe method)
- Modify: `rusty_box_whp/src/lib.rs` (re-export the two counter structs)

**Interfaces:**
- Consumes: `RawPartition`, `WhpResult`, `WhpError::unsupported`, the existing `check(...)` helper in `sys/windows.rs`.
- Produces:
  ```rust
  pub struct InterceptCounter { pub count: u64, pub time_100ns: u64 }
  pub struct InterceptCounters {
      pub page_invalidations: InterceptCounter,
      pub control_register_accesses: InterceptCounter,
      pub io_instructions: InterceptCounter,
      pub halt_instructions: InterceptCounter,
      pub cpuid_instructions: InterceptCounter,
      pub msr_accesses: InterceptCounter,
      pub other_intercepts: InterceptCounter,
      pub pending_interrupts: InterceptCounter,
      pub emulated_instructions: InterceptCounter,
      pub debug_register_accesses: InterceptCounter,
      pub page_fault_intercepts: InterceptCounter,
  }
  pub struct RuntimeCounters { pub total_100ns: u64, pub hypervisor_100ns: u64 }
  impl Partition {
      pub fn intercept_counters(&self, index: u32) -> WhpResult<InterceptCounters>;
      pub fn runtime_counters(&self, index: u32) -> WhpResult<RuntimeCounters>;
  }
  ```

- [ ] **Step 1: Verify the struct layout against the local binding before writing any of it**

Do not trust the field order above. Read the actual declaration:

```bash
grep -rn "WHV_PROCESSOR_INTERCEPT_COUNTERS\|WHV_PROCESSOR_RUNTIME_COUNTERS\|WHvGetVirtualProcessorCounters\|WHV_PROCESSOR_COUNTER_SET" ~/.cargo/registry/src/*/windows-sys-*/src/Windows/Win32/System/Hypervisor/mod.rs | head -20
```

Then print the two struct bodies and the counter-set constants. **Write down the exact field order you found and use that.** A struct whose fields are one position out reports `HaltInstructions` as `IoInstructions` and every conclusion downstream is wrong — this tree has already shipped one bug of exactly that shape (a VMX capability MSR block shifted by one encoding, whose first test asserted the shift was correct).

If `windows-sys` does not declare these, stop and report — do not hand-declare the FFI.

- [ ] **Step 2: Write the failing test**

In `rusty_box_whp/src/partition.rs`, in the existing `#[cfg(test)] mod tests`:

```rust
/// The platform counts a guest's port writes, and says so without being asked.
///
/// This is the engine's independent witness: it is the hypervisor's own
/// accounting, not ours, so a disagreement between it and the engine's census
/// means one of the two is measuring something other than what it claims.
#[test]
fn the_platform_counts_the_guests_port_instructions() {
    let Some(mut partition) = halting_partition() else {
        return; // no hypervisor on this host; the helper has said so
    };

    let before = partition
        .intercept_counters(0)
        .expect("a running partition reports its counters")
        .io_instructions
        .count;

    // out 0xE9, al ; hlt
    partition.run_until_halt_with(&[0xE6, 0xE9, 0xF4]);

    let after = partition
        .intercept_counters(0)
        .expect("a running partition reports its counters")
        .io_instructions
        .count;

    assert_eq!(
        after - before,
        1,
        "one OUT executed, so the platform must report exactly one I/O intercept"
    );
}
```

If `halting_partition()` has no `run_until_halt_with` helper, add one beside it that writes `code` at the reset vector, runs until `ExitReason::Halt`, and returns. Keep it in the test module.

- [ ] **Step 3: Run it and watch it fail**

```bash
cargo test --release -p rusty_box_whp --lib the_platform_counts -- --nocapture
```

Expected: FAIL to compile — `no method named intercept_counters`.

- [ ] **Step 4: Add the counter-set enum**

In `rusty_box_whp/src/sys.rs`, beside `PropertyCode`:

```rust
/// Which counter set `WHvGetVirtualProcessorCounters` should report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CounterSet {
    /// Per-intercept-class count and time.
    Intercepts,
    /// Total and hypervisor-attributed runtime.
    Runtime,
}
```

- [ ] **Step 5: Implement the Windows call**

In `rusty_box_whp/src/sys/windows.rs`, using the field order you recorded in Step 1:

```rust
pub(crate) fn get_counters(
    partition: RawPartition,
    index: u32,
    set: CounterSet,
    out: &mut [u8],
) -> WhpResult<usize> {
    const CALL: &str = "WHvGetVirtualProcessorCounters";
    let raw = match set {
        CounterSet::Intercepts => WHvProcessorCounterSetIntercepts,
        CounterSet::Runtime => WHvProcessorCounterSetRuntime,
    };
    let Ok(len) = u32::try_from(out.len()) else {
        return Err(WhpError::contract(CALL));
    };
    let mut written: u32 = 0;
    // SAFETY: the platform writes at most `len` bytes into `out`, and `len` is
    // `out.len()`; `written` is a plain out-parameter this call owns for the
    // duration. The buffer outlives the call because it is borrowed.
    check(
        unsafe {
            WHvGetVirtualProcessorCounters(
                partition.get(),
                index,
                raw,
                out.as_mut_ptr().cast(),
                len,
                &mut written,
            )
        },
        CALL,
    )?;
    Ok(written as usize)
}
```

- [ ] **Step 6: Implement the non-Windows shim**

In `rusty_box_whp/src/sys/unsupported.rs`:

```rust
pub(crate) fn get_counters(
    _partition: RawPartition,
    _index: u32,
    _set: CounterSet,
    _out: &mut [u8],
) -> WhpResult<usize> {
    Err(WhpError::unsupported(CALL))
}
```

- [ ] **Step 7: Add the safe surface**

In `rusty_box_whp/src/partition.rs`, define the two public structs exactly as in **Interfaces** above (each field documented with what the platform counts), then:

```rust
    /// What the guest has been leaving the hardware for, counted by the
    /// hypervisor rather than by this port.
    ///
    /// The count AND the time per class, so a class that is rare but slow is
    /// distinguishable from one that is frequent and cheap — which no tally
    /// this engine keeps can tell apart.
    ///
    /// # Errors
    /// [`crate::WhpErrorKind::Platform`] if the platform refuses, which it
    /// does for a processor that has never run.
    pub fn intercept_counters(&self, index: u32) -> WhpResult<InterceptCounters> {
        let mut raw = [0u8; core::mem::size_of::<[u64; 22]>()];
        let written = sys::get_counters(self.handle.0, index, sys::CounterSet::Intercepts, &mut raw)?;
        InterceptCounters::from_bytes(&raw[..written])
            .ok_or_else(|| WhpError::contract("WHvGetVirtualProcessorCounters(Intercepts)"))
    }
```

Write `from_bytes` as a private associated function that reads `u64`s in the order recorded in Step 1 and returns `None` if `written` is shorter than the struct. Do the same for `runtime_counters`. **Do not `unwrap`**; a short read is a contract error, not a panic.

- [ ] **Step 8: Run the test**

```bash
cargo test --release -p rusty_box_whp --lib the_platform_counts -- --nocapture
```

Expected: PASS on this host. On a host without WHP it returns early and still passes — that is the established convention here.

- [ ] **Step 9: Verify both build configurations**

```bash
cargo check --release -p rusty_box --features std --lib
cargo check --release --no-default-features -p rusty_box
cargo test --release -p rusty_box_whp --lib
```

- [ ] **Step 10: Commit**

```bash
git add -A -- . ':!ROADMAP.md'
git commit -F - <<'EOF'
feat(whp): ask the hypervisor what the guest has been leaving it for

`WHvGetVirtualProcessorCounters` reports a count AND a time for each
intercept class, and total against hypervisor-attributed runtime, without
this port instrumenting anything. It is the independent witness for the
engine's own census: a tally we keep can only say what we thought we saw,
and the two disagreeing is the useful signal.

Field order was read out of the local `windows-sys` declaration rather than
assumed. A counter struct one position out reports halts as port I/O, and
this tree has shipped that shape of bug before.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
```

---

### Task 2: Census the slices — how many exits, and why each slice ended — **DONE**

> **Implemented and verified. Six things below are wrong; the shipped code
> follows the tree, not this sketch.**
>
> 1. **Step 4's recording site cannot see the number it needs.** At that point
>    `exits` is the `&mut ExitCounts` destructured from `self` — the machine's
>    *lifetime* tally, not the slice's. Bucketing it files every slice under a
>    bucket that ratchets upward forever. The per-slice count is a local of
>    `run_the_exit_loop` and is carried out on `SliceOutcome`.
> 2. **The ending is a STATE, not a tally at each `return`.**
>    `Yielded::Boundary` now carries `BoundaryReason { ProcessorAsked,
>    DeviceLatched, EventToDeliver }` and the whole slice is recorded at one
>    choke point, `SliceCensus::record` (R2, R5) — because a tally written at a
>    `return` cannot also see how many exits the slice held. **Task 4's Step 4
>    is transcribed for this**; do not add `census.* += 1` back.
> 3. **Both loops are instrumented**, the hardware one and the `WHP_ALL_SHADOW`
>    one. Leaving the second blind would report *zero slices* for a run in which
>    slices demonstrably ran — a silent lie inside the diagnostic itself, and
>    comparing the two paths is the whole point of the bisection.
> 4. **`SliceCensus` and `ExitCounts` were unnameable** by any consumer: `mod
>    engine` is private and `lib.rs` re-exported only `WhpEngine`. Both are now
>    re-exported. Check this for every new public type in this crate.
> 5. **Step 6's "watchdog's per-second line" does not exist** — the watchdog is a
>    detached thread owning an `Arc<AtomicU64>`; it holds no machine and cannot
>    reach `engine()`. The per-second line is in the main boot loop, and `report`
>    printed no exit counts at all before this.
> 6. **The Files list was short by two** (`lib.rs`, `examples/dlx_whp.rs`).
>
> Measured on the two-exit test: `slices: 1`, `exits_per_slice[2] == 1`,
> `ended_halted: 1` — the slice was **not** ended by the port write, because the
> 0xE9 debug port latches nothing. That neither confirms nor refutes Task 4's
> premise for a real device; Task 3's DLX census is what decides it.



**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (the census type, the loop, the report)

**Interfaces:**
- Consumes: `ExitCounts` (existing, `engine.rs`), `Yielded` (existing), `WhpEngine::exits()`.
- Produces:
  ```rust
  pub struct SliceCensus {
      pub slices: u64,
      /// Exits per slice, bucketed: [0, 1, 2, 3-4, 5-8, 9+].
      pub exits_per_slice: [u64; 6],
      pub ended_halted: u64,
      pub ended_canceled: u64,
      pub ended_budget: u64,
      /// `Boundary`, split by which question was answered yes.
      pub ended_wants_machine_boundary: u64,
      pub ended_needs_boundary: u64,
      pub ended_event_to_deliver: u64,
  }
  impl WhpEngine { pub const fn census(&self) -> SliceCensus; }
  ```

- [ ] **Step 1: Write the failing test**

In `rusty_box_whp_engine/src/lib.rs`, in the existing `#[cfg(test)] mod tests`:

```rust
/// A slice that services a port write and then halts is ONE slice with two
/// exits, not two slices with one each.
///
/// The distinction is the whole subject of this work: an engine that leaves
/// the partition after every exit buys one VM entry per exit and runs the
/// guest for approximately no time.
#[test]
fn a_port_write_and_a_halt_are_two_exits_in_one_slice() {
    if !hypervisor_here() {
        return;
    }

    let _turn = a_turn_on_the_hardware();
    // out 0xE9, al ; hlt
    let mut machine = machine_running(&[0xE6, DEBUG_PORT, 0xF4]);
    machine.step(RunBudget::Ticks(1_000_000)).expect("the hypervisor runs the guest");

    let census = machine.engine().census();
    assert_eq!(census.slices, 1, "one step is one slice");
    assert_eq!(
        census.exits_per_slice[2], 1,
        "that slice held two exits — the OUT and the HLT — so the 2-bucket has one entry; \
         census was {census:?}"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test --release -p rusty_box_whp_engine a_port_write_and_a_halt -- --nocapture
```

Expected: FAIL to compile — `no method named census`.

- [ ] **Step 3: Add the census type and field**

In `engine.rs`, beside `ExitCounts`, define `SliceCensus` exactly as in **Interfaces**, deriving `Clone, Copy, Debug, Default, PartialEq, Eq`, each field documented. Add `census: SliceCensus` to `WhpEngine` (which already derives `Default`), and:

```rust
    /// How the guest's time has been divided into slices, and what ended each.
    ///
    /// A slice holding one exit means the engine bought a VM entry and a VM
    /// exit and ran the guest for nothing in between; the shape of this
    /// histogram is therefore the shape of the problem.
    #[must_use]
    pub const fn census(&self) -> SliceCensus {
        self.census
    }
```

- [ ] **Step 4: Count exits within a slice and attribute the ending**

In `run_the_exit_loop`, add a local `exits_this_slice: u32` incremented beside the existing `counts` match. Replace the two boundary returns so each records *which* question fired:

```rust
        if cpu.wants_a_machine_boundary() {
            counts.boundary += 1;
            census.ended_wants_machine_boundary += 1;
            return Ok(Yielded::Boundary);
        }
        if io.needs_boundary() {
            counts.boundary += 1;
            census.ended_needs_boundary += 1;
            return Ok(Yielded::Boundary);
        }

        io.sync_io_events(cpu);

        if cpu.has_an_event_to_deliver() {
            counts.boundary += 1;
            census.ended_event_to_deliver += 1;
            return Ok(Yielded::Boundary);
        }
```

Thread `census: &mut SliceCensus` through `run_the_exit_loop` and `run_until_the_machine_is_needed` the same way `counts` is threaded. Record the bucket and the ending in `run_slice` after the outcome is known:

```rust
        let bucket = match exits {
            0 => 0,
            1 => 1,
            2 => 2,
            3..=4 => 3,
            5..=8 => 4,
            _ => 5,
        };
        self.census.slices += 1;
        self.census.exits_per_slice[bucket] += 1;
```

with `Halted` / `Canceled` / `Budget` incrementing their own counters in the existing `match yielded`.

- [ ] **Step 5: Run the test**

```bash
cargo test --release -p rusty_box_whp_engine a_port_write_and_a_halt -- --nocapture
```

Expected: PASS. If the 2-bucket is empty and the 1-bucket has two entries, the engine is already ending the slice after the port write — **that is the defect this plan exists to fix, and the test has just caught it.** In that case change the assertion to document today's behaviour with a `// TODO(Task 4)`-free comment naming the expected post-Task-4 value, and note it in the commit message. Do not weaken the test after Task 4.

- [ ] **Step 6: Report the census from the example**

In `rusty_box_whp_engine/examples/dlx_whp.rs`, in the watchdog's per-second line and in the final `report`, print the census beside the exit counts.

- [ ] **Step 7: Gates and commit**

```bash
cargo check --release --no-default-features -p rusty_box
cargo xtask ci
git add -A -- . ':!ROADMAP.md'
git commit -F - <<'EOF'
feat(whp): count the slices, and record what ended each one

`ExitCounts` says what the guest asked for; it cannot say how that was
divided into slices, and the division is the thing in question. The census
adds an exits-per-slice histogram and splits `Yielded::Boundary` by which of
the three questions was answered yes, because "a device latched work" and
"an interrupt is deliverable" call for different fixes.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
```

---

### Task 3: Measure a DLX boot, and decide whether this plan is right

It is the falsification gate the spec demands, and it may invalidate Tasks 4–7.

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (Step 0 only — the accessor below)
- Modify: `rusty_box_whp_engine/src/lib.rs` (re-export the counter types)
- Create: `docs/perf/2026-08-29-whp-slice-census.md`

- [x] **Step 0: Make the platform's counters reachable at all** — **DONE**

> Shipped as `WhpEngine::platform_counters(&self) -> Result<PlatformCounters>`,
> using the crate's own `rusty_box::cpu::Result`/`CpuError` — **not** the
> `WhpResult` sketched below. `WhpError::contract` is `pub(crate)` in
> `rusty_box_whp`, deliberately: an outside crate must not fabricate an error
> claiming the platform said something. Not-started reuses the existing wording
> `UnsupportedCpuOperation { operation: "the partition did not start" }`, and a
> platform refusal goes through the existing `platform_failed` mapper, so this
> accessor reports the way every other verb in this engine already does.
> `PlatformCounters`, `InterceptCounter`, `InterceptCounters` and
> `RuntimeCounters` are re-exported.


Task 2 established that they are not. `Partition::intercept_counters` and
`runtime_counters` exist (Task 1), but the `Partition` lives in the private
`Started` inside `WhpEngine`, and `lib.rs` re-exports only `WhpEngine`,
`ExitCounts` and `SliceCensus`. **Step 2 below cannot read what it asks for
until this exists.**

Add to `WhpEngine` one accessor returning **both** counter sets in a single
named struct — never a tuple (R0), and one call rather than two so the two
halves describe the same instant:

```rust
/// What the hypervisor itself charged this guest, beside what this engine
/// believes it did.
///
/// The independent check on [`Self::census`]: a disagreement means one of
/// the two is measuring something other than what it claims. See the
/// mapping in this task — it is not field-for-field obvious.
///
/// # Errors
/// [`WhpErrorKind::Contract`] if the engine has not started a partition, and
/// whatever the platform returns otherwise.
pub fn platform_counters(&self) -> WhpResult<PlatformCounters>;
```

Not-started is a contract error rather than `None`: asking a stopped engine
what the hardware charged is a caller mistake, and collapsing a platform
refusal into the same `None` would discard a `Result` (forbidden here).
Re-export `InterceptCounters`, `RuntimeCounters` and `InterceptCounter` from
`rusty_box_whp_engine`, or the returned struct is unnameable — the same gap
Task 2 found for `ExitCounts`.

- [ ] **Step 1: Build and run a bounded DLX boot with the census**

```bash
cargo build --release -p rusty_box_whp_engine --example dlx_whp
```

Then, in PowerShell:

```powershell
$env:RUST_LOG="info"; $env:DLX_WHP_PATIENCE_SECS="120"
& ".\target\release\examples\dlx_whp.exe" *> census.log
```

- [ ] **Step 2: Extract the four numbers that decide the plan**

From the final report: `slices`, the `exits_per_slice` histogram, the four ending tallies, and the platform's `intercept_counters` / `runtime_counters` at exit.

- [ ] **Step 3: Check the prediction, and act on the answer**

The spec predicts: **median 1 exit per slice, >70% of slices ending `Boundary`, and `total_100ns − hypervisor_100ns` a small fraction of wall time.**

Cross-check the engine's tally against the platform's using **this mapping**,
which was measured during Task 1 and is not the obvious one:

| engine `ExitCounts` | platform counter | why not the obvious field |
|---|---|---|
| `port` | `io_instructions.count` | — |
| `memory` | `nested_page_fault_intercepts.count` | an unmapped or permission-refused guest-physical access is a *second-level* fault; `page_fault_intercepts` is the guest's own paging |
| `halt` | `other_intercepts.count` | **`halt_instructions.count` stays ZERO.** Hyper-V books a root-serviced `HLT`'s count under `other_intercepts` and charges only its *time* to `halt_instructions`. Reading the obvious field finds nothing and looks like a disagreement |
| `cpuid` | `cpuid_instructions.count` | — |
| `msr` | `msr_accesses.count` | — |

Then act on what the two together say:

- **If confirmed** — proceed to Task 4.
- **If the median is 5+ and slices end on `Budget`** — §1's ranking is wrong. **Stop. Do not do Tasks 4–6.** Task 7 (cheaper exchange) becomes the whole plan, and the spec needs revising first.
- **If `port` and `io_instructions.count` disagree by more than 1%** — stop and
  find out why before trusting either.

- [ ] **Step 4: Write the measurement down**

Create `docs/perf/2026-08-29-whp-slice-census.md` recording: the command, the host, the raw numbers, which branch of Step 3 was taken, and the wall time to each boot milestone. This is the before-figure every later task is judged against.

- [ ] **Step 5: Commit**

```bash
git add -A -- . ':!ROADMAP.md'
git commit -m "docs(perf): the slice census of a DLX boot before the fast path

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: Stop leaving the partition for work that has already been done

**Do not start this task until Task 3 confirmed the prediction.**

`io.needs_boundary()` is asked *before* `io.sync_io_events(cpu)` drains the latches, so a device that latched anything during the exit ends the slice — even though draining is all that was owed. The machine is only genuinely needed when a device *timer is due*, when the processor itself asks, or when an interrupt must be delivered.

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (`run_the_exit_loop`)
- Modify: `rusty_box/src/emulator/io.rs` (`PcIo::needs_boundary` — narrow it)

**Interfaces:**
- Consumes: `PcIo::sync_io_events`, `PcIo::needs_boundary`, `BxPcSystemC::get_num_cpu_ticks_left_next_event`.
- Produces: `PcIo::has_a_deadline_due(&self) -> bool` — true only when a device timer has actually come due.

- [ ] **Step 1: Write the failing test**

In `rusty_box_whp_engine/src/lib.rs` tests:

```rust
/// A guest that writes a port many times does so in ONE slice.
///
/// Servicing a port write drains what the device latched; draining it is the
/// whole of what the machine was owed. Leaving the partition afterwards buys
/// a VM entry and a VM exit and runs the guest for nothing.
#[test]
fn many_port_writes_do_not_each_end_the_slice() {
    if !hypervisor_here() {
        return;
    }

    let _turn = a_turn_on_the_hardware();
    // out 0xE9,al ; out 0xE9,al ; out 0xE9,al ; out 0xE9,al ; hlt
    let mut machine = machine_running(&[
        0xE6, DEBUG_PORT, 0xE6, DEBUG_PORT, 0xE6, DEBUG_PORT, 0xE6, DEBUG_PORT, 0xF4,
    ]);
    machine.step(RunBudget::Ticks(10_000_000)).expect("the hypervisor runs the guest");

    let census = machine.engine().census();
    assert_eq!(
        census.slices, 1,
        "four port writes and a halt are one slice, not five; census was {census:?}"
    );
    assert_eq!(
        census.ended_needs_boundary, 0,
        "draining a latch is not a reason to hand the machine back"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test --release -p rusty_box_whp_engine many_port_writes -- --nocapture
```

Expected: FAIL — `census.slices` is 5 (or `ended_needs_boundary` is 4).

- [ ] **Step 3: Give `PcIo` a question about deadlines rather than latches**

In `rusty_box/src/emulator/io.rs`, beside `needs_boundary`:

```rust
    /// Whether a device deadline has actually come due.
    ///
    /// Distinct from [`Self::needs_boundary`], which reports that a device
    /// LATCHED something — an interrupt line, a hold request, a timer armed
    /// while it answered. Draining a latch is what `sync_io_events` does, and
    /// an engine that handed the machine back for it would leave the partition
    /// after every device access and buy a VM entry per access.
    ///
    /// A deadline is different: only the machine can fire a timer, so a
    /// deadline that has come due is work the guest cannot be allowed to run
    /// past.
    #[must_use]
    pub fn has_a_deadline_due(&self) -> bool {
        self.pc_system.get_num_cpu_ticks_left_next_event() == 0
    }
```

- [ ] **Step 4: Drain first, then ask only the questions that remain**

In `engine.rs`, replace the block from Task 2 Step 4 with:

```rust
        // Drain what the device latched BEFORE asking whether the machine is
        // needed: draining is what the latch was for, and a question asked
        // first would be answered yes by work this line has already done.
        io.sync_io_events(cpu);

        if cpu.wants_a_machine_boundary() {
            counts.boundary += 1;
            return Ok(Yielded::Boundary(BoundaryReason::ProcessorAsked));
        }
        if io.has_a_deadline_due() {
            counts.boundary += 1;
            return Ok(Yielded::Boundary(BoundaryReason::DeviceLatched));
        }
        if cpu.has_an_event_to_deliver() {
            counts.boundary += 1;
            return Ok(Yielded::Boundary(BoundaryReason::EventToDeliver));
        }
```

**The census is NOT incremented here.** Task 2 made the ending a state carried
on `Yielded::Boundary(BoundaryReason)` and records the whole slice at one choke
point (`SliceCensus::record`, R5), because a tally written at each `return`
cannot also see how many exits the slice held. Adding `census.* += 1` back at
these sites would double-count every slice.

Delete the `io.sync_io_events(cpu)` call inside the `ExitReason::IoPortAccess` arm — it is now unconditional for every exit, which is what a device reached through the shadow needs too.

- [ ] **Step 5: Run the test**

```bash
cargo test --release -p rusty_box_whp_engine many_port_writes -- --nocapture
cargo test --release -p rusty_box_whp_engine
```

Expected: PASS, and all other engine tests still pass.

- [ ] **Step 6: Re-run the DLX census and compare**

Repeat Task 3 Step 1. Record the new histogram in `docs/perf/2026-08-29-whp-slice-census.md` under an "after Task 4" heading. **The 1-bucket should have collapsed.** If it has not, stop — the boundary was not the cause and Task 5 will not help either.

- [ ] **Step 7: Gates and commit**

```bash
cargo check --release --no-default-features -p rusty_box
cargo xtask ci
git add -A -- . ':!ROADMAP.md'
git commit -F - <<'EOF'
feat(whp): draining a latch is not a reason to hand the machine back

`needs_boundary` was asked before `sync_io_events` drained the latches, so
any device that latched anything while answering an exit ended the slice.
Since a device latches something on almost every access, the engine left the
partition after almost every exit — buying one VM entry and one VM exit, and
running the guest for approximately no time between them.

The drain now happens first, unconditionally, for every exit rather than only
for a port write: a device reached through the shadow latches the same way.
What remains a reason to leave is what the machine alone can do — a processor
asking for a boundary, a device deadline actually come due, an interrupt
ready to deliver.

`PcIo::has_a_deadline_due` names that last distinction, which
`needs_boundary` could not: it reported that a timer was ARMED, not that one
was DUE.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
```

---

### Task 5: Anchor device deadlines to the host clock

Even with Task 4, slice length is still set by `deadline(request, ips)` converting the machine's next device deadline into host time. That is the closed loop the spec's decision 1 breaks: guest clock → ticks → run time → next deadline → slice length.

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (`deadline`, `run_slice`)
- Modify: `rusty_box/src/emulator/engine.rs` (the timing policy on the trait)

**Interfaces:**
- Consumes: `SliceRequest::instructions`, `ticks_elapsed`, `HARDWARE_SPEED`.
- Produces:
  ```rust
  /// How an engine's guest time relates to the host's.
  pub enum TimeBase {
      /// Guest time is retired instructions. Deterministic; the interpreter's.
      Retired,
      /// Guest time is host time at the machine's rate. Real-time; a
      /// hypervisor's, and not reproducible.
      HostAnchored,
  }
  pub trait SliceEngine<T> { const TIME_BASE: TimeBase; /* ...existing... */ }
  ```

- [ ] **Step 1: Write the failing test**

In `rusty_box_whp_engine/src/lib.rs` tests:

```rust
/// A slice runs for a host stretch this engine chose, not for the guest time
/// the machine asked for.
///
/// The machine sizes a request by the ticks to its next device deadline. An
/// engine whose guest time IS host time cannot honour that as a bound without
/// re-creating the loop it exists to break, so it honours a floor of its own
/// and reports what actually elapsed.
#[test]
fn a_host_anchored_slice_is_not_bounded_by_the_machines_tick_request() {
    assert!(matches!(
        <WhpEngine as SliceEngine<()>>::TIME_BASE,
        TimeBase::HostAnchored
    ));

    // A request of 300 ticks is 1 microsecond of guest time at 300 M ips.
    // The engine must not try to run for that: it cannot stop the hardware
    // that precisely, and trying is what pinned it to one exit per slice.
    let asked = core::time::Duration::from_nanos(1_000);
    assert!(
        slice_span(SliceRequest::ticks(300), 300_000_000) > asked,
        "a host-anchored engine runs for its own resolution, not the request"
    );
}
```

If `SliceRequest::ticks` does not exist as a constructor, use whatever the existing type offers and adjust; do not invent an API for the test's convenience.

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test --release -p rusty_box_whp_engine a_host_anchored_slice -- --nocapture
```

Expected: FAIL to compile — no `TIME_BASE`, no `slice_span`.

- [ ] **Step 3: Declare the timing policy on the seam**

In `rusty_box/src/emulator/engine.rs`, add `TimeBase` (documented as in **Interfaces**) and `const TIME_BASE: TimeBase;` to `SliceEngine`. Set `Retired` on `SoftwareEngine`, `HostAnchored` on `WhpEngine`.

Give it no default. An engine that does not say which clock it keeps is one whose snapshots silently will not cross to the other.

- [ ] **Step 4: Rename `deadline` to `slice_span` and stop deriving it from the request**

In `engine.rs`:

```rust
/// How long this engine runs the guest before handing the machine back.
///
/// NOT the machine's tick request converted to host time. That conversion is
/// the loop this engine exists outside of: the machine sizes a request by the
/// ticks to its next device deadline, and honouring it as a bound makes slice
/// length a function of device timing — which on this platform means
/// microseconds, and one exit per slice.
///
/// A host-anchored engine runs for a stretch it can actually measure and stop,
/// then reports the guest time that elapsed. The request survives as a FLOOR,
/// so a machine asking for a long stretch still gets one.
fn slice_span(request: SliceRequest, ips: u64) -> Duration {
    host_time_for(request.instructions(), ips).max(SLICE_RESOLUTION)
}
```

- [ ] **Step 5: Run the test and the suite**

```bash
cargo test --release -p rusty_box_whp_engine
cargo test --release -p rusty_box --features std --lib emulator::
```

Expected: PASS.

- [ ] **Step 6: Re-run the DLX census, record, and check the milestone times**

Repeat Task 3 Step 1; append an "after Task 5" section. Record wall time to `LILO`, `Linux version`, `VFS: Mounted`.

- [ ] **Step 7: Gates and commit**

```bash
cargo check --release --no-default-features -p rusty_box
cargo xtask ci
git add -A -- . ':!ROADMAP.md'
git commit -F - <<'EOF'
feat(engine): an engine says which clock its guest time keeps

`SliceEngine::TIME_BASE` distinguishes an engine whose guest time is retired
instructions from one whose guest time is host time. The distinction was
implicit and load-bearing: a snapshot crosses between engines only when both
denominate time the same way, and nothing said so.

The WHP engine stops converting the machine's tick request into a host-time
bound. That conversion closed a loop — guest clock to ticks to run time to
the next device deadline and back to slice length — which on this platform
resolves to microseconds, and therefore to one exit per slice. The request
survives as a floor, so a machine asking for a long stretch still gets one.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
```

---

### Task 6: Elide exits in regions the guest is trapping through

VirtualBox took a Windows 2000 boot on this same API from 32 min 12 s to 58.66 s — parity with native AMD-V — by detecting regions with excessive MMIO/PIO exits and emulating several instructions per exit instead of returning to hardware (`NEMR3Native-win.cpp:3036-3078`). The machinery here already exists for `REP`.

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (the elision table and the decision)
- Modify: `rusty_box/src/emulator/io.rs` (`PcIo::emulate_batch` already exists — reuse it)

**Interfaces:**
- Consumes: `PcIo::emulate_batch(cpu, n) -> Result<u64>`, `Exit::vp.rip`.
- Produces: no public surface; `ExitCounts` gains `elided: u64`.

- [ ] **Step 1: Write the failing test**

```rust
/// A guest looping on a trapped access stops paying a VM exit per iteration.
///
/// The first few exits are serviced on the hardware's terms. Once a region
/// has proven it traps repeatedly, the engine runs it on the shadow instead:
/// an exit costs about four microseconds and an interpreted instruction about
/// six nanoseconds, so a region trapping once per few hundred instructions is
/// cheaper interpreted.
#[test]
fn a_region_that_traps_repeatedly_is_run_on_the_shadow() {
    if !hypervisor_here() {
        return;
    }

    let _turn = a_turn_on_the_hardware();
    // mov ecx, 64 ; L: out 0xE9, al ; loop L ; hlt
    let mut machine = machine_running(&[
        0x66, 0xB9, 0x40, 0x00, // mov cx, 64
        0xE6, DEBUG_PORT,       // L: out 0xE9, al
        0xE2, 0xFC,             // loop L
        0xF4,                   // hlt
    ]);
    machine.step(RunBudget::Ticks(50_000_000)).expect("the hypervisor runs the guest");

    let exits = machine.engine().exits();
    assert!(
        exits.elided > 0,
        "a port write in a tight loop must eventually be serviced without a VM exit; \
         exits were {exits:?}"
    );
    assert!(
        exits.port < 64,
        "not every iteration should have cost an exit; exits were {exits:?}"
    );

    let written: std::vec::Vec<u8> = machine.debug_port().take_output().collect();
    assert_eq!(written.len(), 64, "every write must still reach the device");
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test --release -p rusty_box_whp_engine a_region_that_traps -- --nocapture
```

Expected: FAIL — no `elided` field.

- [ ] **Step 3: Add the elision table**

In `engine.rs`:

```rust
/// How many times a guest-physical page may trap before the engine stops
/// returning to hardware for it.
///
/// An exit costs about four microseconds on this host and an interpreted
/// instruction about six nanoseconds, so a page trapping more often than once
/// per few hundred instructions is cheaper on the shadow. Eight is well below
/// that break-even and low enough that a text-mode console reaches it in a
/// single line of output.
const TRAPS_BEFORE_ELISION: u8 = 8;

/// How many instructions the shadow runs when it takes over a region.
///
/// Long enough to leave the trapping loop, short enough that a device deadline
/// is not missed by much — the machine still gets the processor back at the
/// end of the batch.
const ELIDED_BATCH: u64 = 4096;

/// Pages the guest keeps trapping through, and how often.
///
/// Direct-mapped and tiny on purpose: this is consulted on every exit, and a
/// page evicted by a collision merely pays hardware's price again.
struct ElisionTable {
    pages: [(u64, u8); Self::SLOTS],
}

impl ElisionTable {
    const SLOTS: usize = 64;

    const fn new() -> Self {
        Self { pages: [(u64::MAX, 0); Self::SLOTS] }
    }

    /// Record a trap at `rip` and answer whether this region has earned the
    /// shadow.
    fn trapped(&mut self, rip: u64) -> bool {
        let page = rip >> 12;
        let slot = (page as usize) % Self::SLOTS;
        let (held, count) = self.pages[slot];
        if held == page {
            let count = count.saturating_add(1);
            self.pages[slot] = (page, count);
            count >= TRAPS_BEFORE_ELISION
        } else {
            self.pages[slot] = (page, 1);
            false
        }
    }
}
```

- [ ] **Step 4: Consult it when servicing a port exit**

In the `ExitReason::IoPortAccess` arm, before `service_port_access`:

```rust
                if started.elision.trapped(exit.vp.rip) {
                    // This region has proven it traps; interpreting it is
                    // cheaper than the exits it would otherwise cost. The
                    // shadow runs the machine's own dispatch, so the device
                    // sees exactly what it would have seen.
                    finish_on_the_shadow(started, cpu, io, Trapped::Elided)?;
                    counts.elided += 1;
                } else if access.string_op || access.rep_prefix {
```

Add `elision: ElisionTable` to `Started` (constructed `ElisionTable::new()`), `elided: u64` to `ExitCounts`, and an `Elided` variant to `Trapped` whose servicing runs `io.emulate_batch(cpu, ELIDED_BATCH)` instead of `finish_the_instruction`. Every `Started` destructure names the new field — the compiler will list them.

- [ ] **Step 5: Run the test and the suite**

```bash
cargo test --release -p rusty_box_whp_engine
```

Expected: PASS, including `a_machine_on_the_hypervisor_runs_a_guest_and_its_output_reaches_the_devices` — the device must still see every byte.

- [ ] **Step 6: Re-run the DLX census**

Append "after Task 6". **This is where the milestone times should move most.**

- [ ] **Step 7: Gates and commit**

```bash
cargo check --release --no-default-features -p rusty_box
cargo xtask ci
git add -A -- . ':!ROADMAP.md'
git commit -F - <<'EOF'
feat(whp): a region that keeps trapping is cheaper interpreted than trapped

An exit costs about four microseconds on this host; an interpreted
instruction costs about six nanoseconds. A guest region trapping more often
than once per few hundred instructions is therefore cheaper run on the shadow
than handed back to hardware, and a text-mode console or a PIO disk probe is
far denser than that.

The engine now counts traps per guest page and, past a threshold, runs the
region on the shadow in batches instead of returning to the partition. The
device sees exactly what it would have seen: the shadow uses the machine's own
dispatch, which is the same path the interpreter takes.

VirtualBox took a Windows 2000 boot on this same API from 32 minutes to 59
seconds with this technique, reaching parity with native virtualisation.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
```

---

### Task 7: Stop exchanging the whole processor twice per slice

QEMU's WHPX reads **16** registers on a vmexit against its full set of 42, and takes `eip`/`eflags` out of the exit context with no call at all (`whpx-all.c:139`, `:710`); `whpx_vcpu_post_run` makes zero API calls (`:2116`).

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (`read_back_into_the_shadow`)
- Modify: `rusty_box_whp_engine/src/state.rs` (a narrow read)

**Interfaces:**
- Consumes: `Exit::vp.execution_state`, `state::export`, `state::import`.
- Produces: `state::export_gprs(&vp, &mut VcpuArchState)` — the 16 words a serviced exit can change.

- [ ] **Step 1: Write the failing test**

In `state.rs` tests, using the existing `Recorder`:

```rust
/// The narrow read touches the general-purpose registers and nothing else.
///
/// A serviced exit changes a handful of words. Reading the segment registers,
/// the descriptor tables and thirteen MSRs back for it is three platform calls
/// spent on state the exit could not have altered.
#[test]
fn the_narrow_read_asks_only_for_the_words_an_exit_can_change() {
    let vp = Recorder::default();
    let original = distinctive();
    import(&vp, &original).expect("a recorder refuses nothing");

    let mut read_back = VcpuArchState::default();
    export_gprs(&vp, &mut read_back).expect("a recorder refuses nothing");

    assert_eq!(read_back.gprs, original.gprs, "the words it exists to fetch");
    assert_eq!(read_back.rip, original.rip);
    assert_eq!(
        read_back.segments[1], SegmentState::default(),
        "a segment the narrow read did not ask for must be left alone, not \
         silently filled with a stale value"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
cargo test --release -p rusty_box_whp_engine the_narrow_read -- --nocapture
```

Expected: FAIL to compile — no `export_gprs`.

- [ ] **Step 3: Add the narrow read**

In `state.rs`:

```rust
/// The words alone: the general-purpose registers, `RIP` and `RFLAGS`.
///
/// What a serviced exit can have changed. Segments, descriptor tables and MSRs
/// are not among them, and asking for them costs three platform calls each way
/// for state that cannot have moved.
///
/// # Errors
/// Whatever the platform refused.
pub(crate) fn export_gprs<V: VpRegisters>(vp: &V, state: &mut VcpuArchState) -> WhpResult<()> {
    let mut words = [0u64; WORD_VALUES];
    vp.read_words(WORD_REGS, &mut words)?;
    unpack_words(state, &words);
    Ok(())
}
```

Factor the existing word-unpacking in `export` into `unpack_words(state, &words)` and call it from both, so the two cannot drift.

- [ ] **Step 4: Delete the `InterruptState` read**

`ExitContext.ExecutionState` already carries `InterruptShadow` (bit 12) and `InterruptionPending` (bit 6), and the engine already reads that word. Remove the `vp.read_words(&[Reg::InterruptState], …)` from `read_back_into_the_shadow` and set `started.shadowed` from `(exit.vp.execution_state >> 12) & 1 == 1` at the point the exit is taken.

- [ ] **Step 5: Run the tests**

```bash
cargo test --release -p rusty_box_whp_engine
```

Expected: PASS, including `a_processor_survives_being_installed_and_read_back` and `the_time_stamp_counter_is_never_written_to_the_processor`.

- [ ] **Step 6: Re-run the DLX census and the E2 buckets**

Append "after Task 7", and record the per-bucket timings (`partition.run`, `export_arch_state`, `state::import`, `state::export`) so the remaining overhead is attributed rather than guessed.

- [ ] **Step 7: Gates and commit**

```bash
cargo check --release --no-default-features -p rusty_box
cargo xtask ci
git add -A -- . ':!ROADMAP.md'
git commit -F - <<'EOF'
feat(whp): read back only what a serviced exit can have changed

A slice exchanged the whole processor twice: fifty-two registers each way,
nine platform calls, for an exit that in the common case moved `RIP` and one
general-purpose register. Segments, descriptor tables and thirteen
model-specific registers cannot be changed by servicing a port write, and
asking for them cost three calls in each direction.

The `InterruptState` read is deleted outright: `ExitContext.ExecutionState`
carries the interrupt shadow in a word this engine already reads, so it was a
platform call spent re-fetching something it had.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
```

---

### Task 8: The gate — beat the interpreter on a DLX boot

**Files:**
- Modify: `xtask/src/ci.rs` (a WHP boot-speed step, `requires` a hypervisor)
- Modify: `docs/perf/2026-08-29-whp-slice-census.md` (the final row)

- [ ] **Step 1: Record the final numbers**

Run both engines to `login:` three times each, alternating, and record the median:

```powershell
$env:RUSTY_BOX_HEADLESS="1"; $env:MAX_INSTRUCTIONS="450000000"
& ".\target\release\examples\dlxlinux.exe" *> interp.log

$env:DLX_WHP_PATIENCE_SECS="120"
& ".\target\release\examples\dlx_whp.exe" *> whp.log
```

- [ ] **Step 2: Decide honestly whether the goal was met**

The goal is **WHP reaching `login:` faster than the interpreter's 15.0 s**. If it is not met, say so plainly in the doc with the number reached, and stop — do not adjust the goal to fit the result. A partial improvement is still worth having and worth recording as such.

- [ ] **Step 3: Add the gate only if it passes**

If WHP is faster, add a `xtask ci` step running `dlx_whp` with a wall-clock ceiling set 50% above the measured median, marked as requiring a hypervisor (skip-with-reason on hosts without one, as the existing WHP steps do).

- [ ] **Step 4: Final gates and commit**

```bash
cargo xtask ci
git add -A -- . ':!ROADMAP.md'
git commit -F - <<'EOF'
feat(whp): gate the hypervisor engine on beating the interpreter

The engine existed to be faster and was not; a gate that only checks it boots
would let it regress back. The ceiling is set from a measured median rather
than a hoped-for figure, and skips with a reason on a host without a
hypervisor.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
```

---

## Self-review

**Spec coverage.** §1 (the measured problem) → Tasks 1–3. §4.1 (let the guest run) → Task 4. §4.2 (host-anchored time) → Task 5. §4.3 (exit elision) → Task 6. §4.4 (cheaper exchange) → Task 7. §7 (E1, E2, M1 gate) → Tasks 3, 7 Step 6, 8.

**Not covered here, deliberately:** §5 (the API design), §9 (REPLAN v4 relationships, including H5/egui). Those are the second plan.

**Known gap, stated rather than hidden:** §4.4's dirty-flag and vector-file items are named in the spec but only the narrow read and the `InterruptState` deletion are planned. The dirty flag changes when the shadow is authoritative — a typed state under doctrine R2, not a bool — and belongs with the API plan where that type is designed. Task 7's commit should not claim §4.4 is complete.

**Ordering risk.** Task 4 depends on Task 3 confirming the prediction, and Task 3 has an explicit stop branch. Tasks 5–7 are independently valuable and can be reordered; Task 6 is expected to move the milestone times most.

**Type consistency.** `SliceCensus` fields are used identically in Tasks 2 and 4. `ElisionTable::trapped` is defined and called once each. `export_gprs` is defined in Task 7 Step 3 and used in Step 4. `PcIo::has_a_deadline_due` is defined in Task 4 Step 3 and used in Step 4. `PcIo::emulate_batch` already exists in the tree.
