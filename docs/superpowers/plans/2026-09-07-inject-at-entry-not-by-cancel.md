# Inject At Entry, Not By Cancel — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A legacy 8259 interrupt reaches a guest running on the hypervisor, by restoring the pending-event injection that measurement shows works and replacing the one thing that was broken — a cancel issued without knowing whether the processor was inside a run.

**Architecture:** Revert the two commits that removed the working mechanism, then add an `in_run: AtomicBool` to `VcpuControl` that the raiser consults before cancelling and the run loop maintains around `WHvRunVirtualProcessor`, with a Dekker re-check that closes the lost-wakeup race. `stage_the_legacy_interrupt`'s body is restored unchanged — it was correct.

**Tech Stack:** Rust, `rusty_box_whp_engine`, Windows Hypervisor Platform.

**Spec:** `docs/superpowers/specs/2026-09-07-inject-at-entry-not-by-cancel-design.md`

## Global Constraints

- Read `CLAUDE.md` first and follow it. In particular: **edit with the Edit/Write tools, never shell heredocs, `sed -i`, or Python**; release builds only; **never run `cargo fmt`**; never `let _ = <Result>`; comments state today's invariant, never history; Bochs citations name file + symbol, never line numbers.
- **Never use the LSP tools** (`mcp__lsp__references`/`definition`/`diagnostics`). They hang indefinitely in this workspace — one attempt burned 31 minutes and wrote nothing. The compiler is the oracle; `grep` is for the survey before it.
- Verification: `cargo check --release -p rusty_box --features std --lib`, plus `cargo check --release -p rusty_box --no-default-features` for anything under `emulator/`.
- `cargo xtask ci` before every commit. **Never pipe it and never append to its line** — a pipeline's exit code masks the gate's. Redirect to a log, then read the log for `ci: N steps passed` AND grep it for `FAILED`.
- **Never stage** `ROADMAP.md`, `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md`, or the three untracked `docs/superpowers/specs/2026-08-22-*.md` files — someone else's work. Explicit `git add <path>` only; never `git add -A`.
- Commit messages end with: `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
- Branch: `wip/atom-execctx`. Do not create branches.
- **Known-flaky, host-load sensitive:** `fast_machine::tests::step_in_ticks_runs_the_guest_for_that_much_vm_time_and_pauses` and the halt-errand test. Reproduce with the FULL `-p rusty_box_whp_engine --lib` suite — a single-test run filters out the others and is not a fair reproduction. Any other failure is yours.
- Other sessions share this repo; `cargo` may print `Blocking waiting for file lock`. Check `powershell -NoProfile -Command "Get-Process cargo | Select Id,StartTime,CPU"` — near-zero CPU means blocked. **Never kill a process you did not start.**

---

### Task 1: Restore the working mechanism

Three commits removed a delivery path that measurement shows works. Revert all three. Nothing is designed in this task; it exists so the next task starts from a known-good baseline that the hardware test can verify.

**Revert all three, not two.** `bbccbce` removed `SliceEngine::pic_pin_changed`, which was the ONLY caller of `VcpuControl::raise_ext_int` — verified: at `720192b` the name survives solely in a doc comment. Reverting only `c9d2fbd` and `720192b` would restore the whole staging mechanism with nothing to trigger it, and the hardware delivery test would fail. The tree must land exactly on `0329d76`'s code, which is the state where that test provably passes.

This drops `owns_the_guests_local_apic`. It was a nicer seam than the `pic_pin_changed` it replaced, but keeping it would mean inventing a new caller for `raise_ext_int` inside the one task whose entire purpose is to reach a *known-good* baseline. Re-introducing it later is a separate, verifiable change; doing it here would put unproven code under the test that is supposed to be proving the baseline.

**Files:**
- Revert: commits `c9d2fbd`, `720192b` and `bbccbce`

**Interfaces:**
- Produces (all restored): `VcpuThread::stage_the_legacy_interrupt(&mut self) -> Continue`; `ext_int_request(&AtomicBool, &AtomicBool, &impl CancelRun) -> WhpResult<()>`; `VcpuControl::{ext_int_pending, ext_int_blocked, raise_ext_int}`; `InjectState` with `permits_ext_int`/`note_placed_event`; `InjectCensus` with `injected`/`injected_per_vector`; `WhpEngine::{controls, install_control}`; `SliceEngine::pic_pin_changed` (the trait method that calls `raise_ext_int`). Task 2 changes `ext_int_request` and the run loop; Task 3 uses `InjectCensus`.

- [ ] **Step 1: Revert, newest first**

```bash
git revert --no-edit c9d2fbd
git revert --no-edit 720192b
git revert --no-edit bbccbce
```

Expected: all three apply cleanly — nothing has touched these files since.

If any revert conflicts, STOP and report rather than resolving by hand: a
hand-merged baseline is no longer the state the delivery test was proven against,
which is this task's whole purpose.

- [ ] **Step 2: Confirm the tree builds and the suite is whole again**

```bash
cargo check --release -p rusty_box --features std --lib
cargo check --release -p rusty_box --no-default-features
cargo test --release -p rusty_box_whp_engine --lib
```

Expected: both checks exit 0. The suite returns to **49 tests** (44 + the 5 the deletion took), and the `emulator::` module returns to its pre-`720192b` count — the two `a_backend_apic_*` tests go with the revert, since the routing they cover is gone.

Confirm the tree really is `0329d76`'s code:

```bash
git diff --stat 0329d76 -- rusty_box/src rusty_box_whp_engine/src
```
Expected: **empty**. Any output means the reverts did not land cleanly and the baseline is not the proven one.

- [ ] **Step 3: Prove the restored delivery actually delivers — and is not a vacuous pass**

The test is gated on `hypervisor_here()` with a bare `return`, so a skip looks exactly like a pass and takes the same 0.03 s. Run it, then mutate it and watch it fail:

```bash
cargo test --release -p rusty_box_whp_engine --lib a_legacy_8259_vector_reaches -- --test-threads=1
```
Expected: PASS.

Now edit `rusty_box_whp_engine/src/lib.rs`, changing the assertion

```rust
            census.injected >= 3 && census.injected_per_vector[8] >= 3,
```

to `census.injected >= 300_000 && census.injected_per_vector[8] >= 300_000,` and re-run the same command.

Expected: **FAIL** with `injected 3, of vector 8 3`. That message is the proof the guest's own ISR ran three times. **Restore the assertion to `>= 3` before continuing** and re-run to confirm PASS.

- [ ] **Step 4: Gate**

```bash
cargo xtask ci > /tmp/gate-revert.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-revert.log
grep -c FAILED /tmp/gate-revert.log
```
Expected: a `ci: N steps passed` line and a FAILED count of 0.

- [ ] **Step 5: Commit**

The two reverts are already commits. Nothing further to commit unless Step 3's mutation was left in the tree — verify with `git status --porcelain` that `rusty_box_whp_engine/src/lib.rs` is clean.

---

### Task 2: The raiser learns whether the processor is running

The whole defect. `WHvCancelRunVirtualProcessor` is latched when the processor is not inside `WHvRunVirtualProcessor`, and consumes the next entry retiring nothing. Microsoft does not document this; it is measured. The raiser must therefore ask before cancelling, and the run loop must answer.

**Files:**
- Modify: `rusty_box_whp_engine/src/vcpu_thread.rs` (`VcpuControl` gains `in_run`; `ext_int_request` gains a parameter; the run loop maintains the flag)

**Interfaces:**
- Consumes: `VcpuControl::{ext_int_pending, ext_int_blocked}`, `CancelRun` (Task 1).
- Produces: `VcpuControl::in_run: AtomicBool`; `ext_int_request(pending: &AtomicBool, blocked: &AtomicBool, in_run: &AtomicBool, cancel: &impl CancelRun) -> WhpResult<()>`.

- [ ] **Step 1: Write the failing tests**

In `rusty_box_whp_engine/src/vcpu_thread.rs`'s test module, beside the existing `ext_int_request` tests.

Use the counting canceller that is already there — `struct CountingCancel(Cell<u32>)`, built as `CountingCancel(Cell::new(0))` and read as `cancel.0.get()`. Do **not** use `RecordingCancel`: it is `RecordingCancel<'a> { flag: &'a AtomicBool, saw: Cell<Option<bool>> }`, has no `Default`, and records a flag rather than a count.

```rust
    /// A request raised while the processor is between runs cancels nothing.
    ///
    /// `WHvCancelRunVirtualProcessor` is latched when the processor is not
    /// inside `WHvRunVirtualProcessor`: the next entry returns having retired
    /// no instruction. A guest that exits often is therefore never able to
    /// execute its way to the `STI` that would let it accept the vector, which
    /// is the livelock this flag exists to prevent. The request stays owed, and
    /// the thread's own next entry stages it.
    #[test]
    fn a_request_raised_between_runs_cancels_nothing() {
        let owed = AtomicBool::new(false);
        let blocked = AtomicBool::new(false);
        let in_run = AtomicBool::new(false);
        let cancel = CountingCancel(Cell::new(0));

        ext_int_request(&owed, &blocked, &in_run, &cancel).expect("a request is recordable");

        assert_eq!(cancel.0.get(), 0, "a processor that is not running is not cancelled");
        assert!(owed.load(Ordering::SeqCst), "and the vector is still owed");
    }

    /// A request raised while the processor is inside its run cancels once.
    ///
    /// This is the case the cancel exists for: a guest in a long run that takes
    /// no exits has no other moment at which the thread could stage a vector.
    #[test]
    fn a_request_raised_inside_a_run_cancels_once() {
        let owed = AtomicBool::new(false);
        let blocked = AtomicBool::new(false);
        let in_run = AtomicBool::new(true);
        let cancel = CountingCancel(Cell::new(0));

        ext_int_request(&owed, &blocked, &in_run, &cancel).expect("a request is recordable");

        assert_eq!(cancel.0.get(), 1, "a running processor is fetched out exactly once");
        assert!(owed.load(Ordering::SeqCst), "and the vector is owed until it is staged");
    }
```

Do **not** add a third test for "a held pin costs one cancel". That property is already covered by the existing `a_pin_reported_at_every_boundary_costs_one_cancel_per_vector`, which you update in Step 1b rather than duplicate.

- [ ] **Step 1b: Carry the existing `ext_int_request` tests onto the new signature**

The two tests already in that module — `a_pin_reported_at_every_boundary_costs_one_cancel_per_vector` and its neighbour that drives `blocked` — call `ext_int_request` with three arguments and will not compile after Step 4.

Give each a `let in_run = AtomicBool::new(true);` beside its existing `owed`/`blocked` bindings and pass `&in_run` as the third argument. `true` is the value that preserves what each test currently asserts: both were written when every raise cancelled, which is now the inside-a-run case. Change nothing else about them — their subjects are unaffected by this task.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --release -p rusty_box_whp_engine --lib a_request_raised`
Expected: FAIL to compile — `ext_int_request` takes 3 arguments, not 4.

- [ ] **Step 3: Add the flag to `VcpuControl`**

In `rusty_box_whp_engine/src/vcpu_thread.rs`, add to the `VcpuControl` struct beside `ext_int_pending`:

```rust
    /// Whether this processor's thread is inside `WHvRunVirtualProcessor`
    /// right now.
    ///
    /// The one question a canceller must ask. `WHvCancelRunVirtualProcessor`
    /// issued to a processor that is NOT inside a run is latched by the
    /// platform and spent on the next entry, which returns having retired no
    /// instruction — undocumented behaviour, measured here. A guest that exits
    /// often is then never able to execute its way to the `STI` that would let
    /// it accept the vector.
    ///
    /// `SeqCst` on every access, on both sides: this and `ext_int_pending` are
    /// a Dekker pair, and a weaker ordering admits the interleaving in which
    /// the raiser reads this as false while the thread reads the request as
    /// unset, so neither acts and the vector is owed forever.
    pub(crate) in_run: AtomicBool,
```

Initialise it `AtomicBool::new(false)` wherever `ext_int_pending` is initialised in `spawn`.

- [ ] **Step 4: Make the raiser ask**

Replace `ext_int_request`'s signature and body:

```rust
pub(crate) fn ext_int_request(
    pending: &AtomicBool,
    blocked: &AtomicBool,
    in_run: &AtomicBool,
    cancel: &impl CancelRun,
) -> WhpResult<()> {
    let first = pending
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok();
    if !(first || blocked.load(Ordering::SeqCst)) {
        return Ok(());
    }
    // Only a processor inside its run can be fetched out of one. A cancel to
    // one that is between runs is latched and spent on the next entry; the
    // thread stages this request before that entry anyway, so there is nothing
    // a cancel here could win.
    if in_run.load(Ordering::SeqCst) {
        return cancel.cancel();
    }
    Ok(())
}
```

Update `VcpuControl::raise_ext_int` to pass `&self.in_run`.

- [ ] **Step 5: Maintain the flag around the run, and close the race**

In the run loop in `run_loop`, the staging call and `self.vcpu.run()` are already adjacent. Wrap the entry:

```rust
            self.control.in_run.store(true, Ordering::SeqCst);
            // The raiser may have set the request in the window between the
            // staging above and this store, and read `in_run` as false, so it
            // issued no cancel and nothing else will report it. Enter only with
            // nothing owed; otherwise go round and stage it.
            if self.control.ext_int_pending.load(Ordering::SeqCst) {
                self.control.in_run.store(false, Ordering::SeqCst);
                continue;
            }
            self.control.census.runs.fetch_add(1, Ordering::Release);
            let exit = self.vcpu.run();
            self.control.in_run.store(false, Ordering::SeqCst);
```

Keep the existing `runs.fetch_add` exactly where it was relative to `run()`.

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test --release -p rusty_box_whp_engine --lib
cargo check --release -p rusty_box --no-default-features
```
Expected: the two new tests pass, the two carried-over ones still pass, and the suite is **51** and green (49 + 2).

- [ ] **Step 7: Gate and commit**

```bash
cargo xtask ci > /tmp/gate-inrun.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-inrun.log
grep -c FAILED /tmp/gate-inrun.log
```

```bash
git add rusty_box_whp_engine/src/vcpu_thread.rs
git commit -m "fix(whp): only a processor inside its run is fetched out of one

WHvCancelRunVirtualProcessor issued to a processor that is not inside
WHvRunVirtualProcessor is latched by the platform and spent on the next entry,
which returns having retired no instruction. Microsoft documents nothing about
this case; it is measured here. The raiser cancelled on the pin's rise without
asking, so a guest that exits often could never execute its way to the STI that
would let it accept the vector: 104,563,825 cancels against 17,909 port exits,
frozen at one RIP, while an exit-free test guest delivered perfectly.

The run loop now answers the question, and the entry re-checks the request under
the flag so the two form a Dekker pair rather than a lost wakeup.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: Prove it on hardware, and boot DLX

The acceptance criterion.

**Files:**
- Modify: `rusty_box_whp_engine/src/lib.rs` (one new hardware test)

**Interfaces:**
- Consumes: `machine_with_devices_on`, `drive_on_a_thread`, `ThreadedRun`, `hypervisor_here`, `a_turn_on_the_hardware`, `InjectCensus` (all restored by Task 1).
- Produces: nothing.

- [ ] **Step 1: Write the halted-guest test**

A guest that halts is the case a placed event alone does not wake — the platform leaves `halt_suspend` set and never runs the handler. `stage_the_legacy_interrupt`'s `set_internal_activity(RUNNING)` is what clears it, and nothing currently proves that line is load-bearing, which makes it exactly the kind of "redundant write" a later reader deletes. Add to `rusty_box_whp_engine/src/lib.rs`, beside the other hardware tests:

```rust
    /// A guest that halts still takes its tick.
    ///
    /// A placed pending event does not clear `halt_suspend`: measured, a
    /// processor parked in `HLT` produces no halt exit and does not run the
    /// handler until the suspend is cleared, which is why the staging writes
    /// the whole internal-activity register before it enters. QEMU needs the
    /// same thing and calls it `whpx_vcpu_kick_out_of_hlt`
    /// (`target/i386/whpx/whpx-all.c`): "we also manually do inject some
    /// interrupts via WHvRegisterPendingEvent instead of WHVRequestInterrupt,
    /// which does not reset the HLT state".
    ///
    /// Without that write this test hangs until its deadline with an empty
    /// debug port, which is the whole point of having it.
    #[test]
    fn a_halted_guest_still_takes_its_legacy_tick() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        // isr at CODE+0x29, exactly as the delivery test's guest.
        let machine = machine_with_devices_on(
            DeviceClock::HostTime,
            &[
                0x31, 0xC0, //             xor ax, ax
                0x8E, 0xD8, //             mov ds, ax
                0x8E, 0xD0, //             mov ss, ax
                0xBC, 0x00, 0x70, //       mov sp, 0x7000
                0xC7, 0x06, 0x20, 0x00, 0x29, 0x10, // mov word [0x20], isr (IVT[8])
                0xC7, 0x06, 0x22, 0x00, 0x00, 0x00, //  mov word [0x22], 0
                0xB0, 0xFE, //             mov al, 0xFE — unmask IRQ0 alone
                0xE6, 0x21, //             out 0x21, al
                0xB0, 0x34, //             mov al, 0x34 — ch0, lo/hi, mode 2
                0xE6, 0x43, //             out 0x43, al
                0xB0, 0x00, //             mov al, 0x00 — count 0x0400 low
                0xE6, 0x40, //             out 0x40, al
                0xB0, 0x04, //             mov al, 0x04 — count 0x0400 high
                0xE6, 0x40, //             out 0x40, al
                0xFB, //                   sti (CODE+0x25)
                // halt (CODE+0x26): the guest asks for nothing and takes no
                // exits; only a placed event that also clears the suspend can
                // reach the handler.
                0xF4, //                   hlt
                0xEB, 0xFD, //             jmp halt
                // isr (CODE+0x29)
                0xFB, //                   sti
                0x90, //                   nop
                0xB0, MARK, //             mov al, MARK
                0xE6, DEBUG_PORT, //       out 0xE9, al
                0xB0, 0x20, //             mov al, 0x20
                0xE6, 0x20, //             out 0x20, al — non-specific EOI
                0xCF, //                   iret
            ],
        );
        let ThreadedRun { machine, written, .. } = drive_on_a_thread(
            machine,
            std::time::Duration::from_secs(10),
            "a halted guest's own ISR ran",
            |_, seen| seen.len() >= 3,
        );
        assert!(
            !written.is_empty() && written.iter().all(|byte| *byte == MARK),
            "the PIT's ticks must reach the ISR of a guest that halts: {written:#04x?}"
        );
        let guard = machine.lock().expect("the machine's lock");
        assert!(
            guard.engine().inject_census().injected >= 3,
            "and each must be a placed event: {}",
            guard.engine().inject_census().injected
        );
    }
```

- [ ] **Step 2: Run it, and prove the HLT kick is what carries it**

```bash
cargo test --release -p rusty_box_whp_engine --lib a_halted_guest_still_takes -- --test-threads=1
```
Expected: PASS.

Then comment out the `vcpu.set_internal_activity(RUNNING)` call in `stage_the_legacy_interrupt` and re-run.
Expected: **FAIL** — the deadline elapses with an empty debug port. **Restore the line** and re-run to confirm PASS. This is the only evidence that write is load-bearing.

- [ ] **Step 3: Boot DLX, three runs**

```bash
cargo run --release -p rusty_box_whp_engine --example dlx_whp
```
Run it three times. Record, verbatim for each run: the milestone count, the cancel count, and the in-run share.

Expected: **3/3 milestones**, three times, with the cancel count a small constant rather than millions.

**If it is not 3/3, STOP and report the real number.** Do not adjust the milestones and do not patch a second defect into this task. A truthful 2/3 with the failing milestone named is the required output; the design doc anticipates that the interrupt arriving may reveal a separate progress defect, and that gets its own investigation.

- [ ] **Step 4: Check Alpine has not regressed**

The campaign's purpose is throughput, and Alpine is the guest that exercises the device model hardest. It reaches userspace over the I/O APIC, which this change does not touch — so this step is a guard, not a hypothesis.

```bash
cargo run --release -p rusty_box_whp_engine --example alpine_bench
```
Compare against the same command at `c9d2fbd` (`git stash` is not needed — the examples read no plan state). Record both numbers.

Expected: within run-to-run noise. **A regression is a finding to report, not something to tune away here.**

- [ ] **Step 5: Gate and commit**

```bash
cargo xtask ci > /tmp/gate-accept.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-accept.log
grep -c FAILED /tmp/gate-accept.log
```

```bash
git add rusty_box_whp_engine/src/lib.rs
git commit -m "test(whp): a guest that halts still takes its legacy tick

A placed pending event does not clear halt_suspend, so a halted processor takes
the event and stays halted. The staging writes the whole internal-activity
register before entering to prevent that, and nothing proved the write was
load-bearing — exactly the shape of line a later reader deletes as redundant.
Commenting it out now fails this test at its deadline with an empty debug port.

QEMU carries the same workaround as whpx_vcpu_kick_out_of_hlt.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: Register the divergence and retire the refuted design

**Files:**
- Modify: `docs/bochs-parity-divergences.md`
- Modify: `docs/superpowers/specs/2026-09-07-the-8259-through-the-hypervisors-apic-design.md` (status line only)

- [ ] **Step 1: Mark the refuted design refuted**

Change that spec's `**Status:** design, awaiting review` line to:

```markdown
**Status:** REFUTED BY MEASUREMENT — superseded by
`2026-09-07-inject-at-entry-not-by-cancel-design.md`. `WHvRequestInterrupt`
refuses vector 0x08 (an APIC takes no vector below 16) and, with the 8259
remapped above the floor, drops it into an APIC a legacy guest never enables.
Kept for the reasoning and the measurements; do not implement it.
```

- [ ] **Step 2: Register the divergence**

Append to `docs/bochs-parity-divergences.md`, following the file's existing entry format:

```markdown
### D-WHP-EXTINT — the legacy line is placed as a pending event, not taken by an INTA at the processor

**Applies to:** the WHP engine in fast mode only. The interpreter is unaffected.

Bochs raises a flag and reads the vector only when the processor is ready
(`pc_system.cc raise_INTR`, then `cpu/event.cc`'s `DEV_pic_iac()`). This engine
cannot: the processor is inside the hypervisor and there is no INTA cycle to
join. It performs the acknowledge itself, on the machine's side, once the guest's
own readiness is known from the last exit header — `IF` set, no interrupt shadow,
no delivery already in flight — and places the resolved vector as a
`WHvX64PendingEventExtInt`.

**Evidence it must be this way:** `WHvRequestInterrupt`, the alternative, refuses
vector `0x08` with `0xC0350005` because a local APIC takes no vector below 16
(this port's own `BX_LAPIC_FIRST_VECTOR`, and Bochs `cpu/apic.cc trigger_irq`).
`WHV_INTERRUPT_TYPE` offers no ExtINT and no LocalInt0, so the wire cannot be
asserted at all. QEMU places the same event for the same reason
(`target/i386/whpx/whpx-all.c whpx_vcpu_pre_run`).

**Guest-visible difference:** the vector leaves the 8259 a few instructions
earlier than Bochs would take it. A guest that masks the IRQ in that window still
receives it. No guest is known to depend on the difference, and the same is
already true of an I/O APIC entry in ExtINT mode, which is the other way a PC
wires this controller.
```

- [ ] **Step 3: Gate and commit**

```bash
cargo xtask ci > /tmp/gate-docs.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-docs.log
```

```bash
git add docs/bochs-parity-divergences.md docs/superpowers/specs/2026-09-07-the-8259-through-the-hypervisors-apic-design.md
git commit -m "docs(whp): register the ExtINT placement divergence, retire the refuted design

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage.** The spec's root cause is Task 2. Its "restore what worked" is Task 1. Its five testing requirements map to: the trigger's two branches (Task 2 Step 1, plus a third test for the held pin), delivery-with-mutation-check (Task 1 Step 3), the halted guest (Task 3 Steps 1–2), DLX end to end (Task 3 Step 3), and Alpine throughput (Task 3 Step 4). The divergence is Task 4. The spec's "what this does not address" is honoured by Task 3 Step 3, which stops rather than patching.

**Placeholder scan.** No TBD/TODO. Every code step carries the code. Both mutation checks name the exact edit, the exact expected failure text, and the restore.

**Type consistency.** `ext_int_request` takes four parameters in Task 2 Steps 1, 4 and its call site; `in_run` is `AtomicBool` throughout and `SeqCst` at every access on both sides; `InjectCensus::injected` is used in Task 3 exactly as Task 1 restores it. `stage_the_legacy_interrupt` returns `Continue`, matching its restored body.

**One risk this plan does not remove.** Task 2's Dekker pair is the only thing standing between a held pin and a lost wakeup, and its two branches are unit-tested but its *interleaving* is not — no test here forces the raiser and the thread to race. The hardware tests exercise it incidentally and DLX exercises it hard, but a targeted concurrency test would need to drive both sides deterministically, which the current `VcpuControl` shape does not allow. Recorded rather than hidden.
