# WHP Injection + Lazy Exchange Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** External interrupts inject through the partition instead of ending the slice for shadow delivery, and slice boundaries stop exchanging processor state they don't need — VMware-league OS boot/install throughput on `--engine whp`.

**Architecture:** Three gated stages per the spec (`docs/superpowers/specs/2026-09-01-whp-injection-lazy-exchange-design.md` — read it first, it is the authority): Stage 0 extracts the INTA chain into one shared body plus probes and counters (no behavior change); Stage 1 adds `stage_injection` (one choke point writing `WHvRegisterPendingInterruption`, window-armed via `DeliverabilityNotifications`); Stage 2 makes the state exchange lazy behind a `ShadowFreshness` witness with a free mini-import from exit headers. QEMU `whpx-all.c` is the design precedent throughout.

**Tech Stack:** Rust workspace; crates `rusty_box` (CPU/machine), `rusty_box_whp` (platform leaf), `rusty_box_whp_engine` (the engine under change). Tests: `cargo test --release -p <crate> --lib`; hypervisor-gated tests skip on hosts without WHP.

## Global Constraints

- **Verification cadence (CLAUDE.md):** after each edit batch run `cargo check --release -p rusty_box --features std --lib`; add `cargo check --release --no-default-features -p rusty_box` whenever the edit touches `rusty_box/` (arch_state.rs, event.rs, io.rs are all no_std-compiled). Engine/leaf crates: `cargo check --release -p rusty_box_whp_engine` / `-p rusty_box_whp`.
- **Full `cargo xtask ci` before every commit that ends a task; commit only the tree the gates saw. Never stage `ROADMAP.md`. Never run `cargo fmt`.**
- Release builds only. Edit with Edit/Write tools, never scripts. Bochs-source comments cite file + symbol, never line numbers. Comments state invariants, never history. No `TODO`/stub/partial work. Never `let _ = fallible()`.
- Doctrine: R0 named types in public APIs; R5 one choke point per hazard, exhaustive matches; R7 provenance; R9 tests assert guest-visible properties.
- Bounded headless guest runs only (`--display headless` or `terminal`; **never `--display terminal` with a Windows ISO** — graphics-mode frames make multi-GB logs). Never `--display egui` from an agent unless the user asked to watch.
- Hypervisor-gated tests follow the existing pattern: `if !hypervisor_here() { return; }` + `a_turn_on_the_hardware()` (see `rusty_box_whp_engine/src/lib.rs` tests).
- The boot oracle for gates: `cargo run --release -q -p rusty_box_whp_engine --example dlx_whp` (3/3 milestones, ~2.0s baseline) and the bounded Alpine run from `docs/whp-alpine-handoff.md` (300–600s window, milestones ISOLINUX→OpenRC→login).

---

## Stage 0 — extraction, probes, counters (no behavior change)

### Task 1: Deliverability probes and the CR8→TPR verb

**Files:**
- Modify: `rusty_box/src/cpu/arch_state.rs` (beside `has_an_event_to_deliver`, ~line 549, and `DeliverableInterrupt` at ~line 46)
- Test: same file's `#[cfg(all(test, feature = "std"))] mod tests`

**Interfaces:**
- Consumes: `DeliverableInterrupt::MASK` (private const, same file), `self.pending_event` (crate-visible field), `self.is_unmasked_event_pending(u32)`, `self.lapic.intr`, `self.lapic.set_tpr(u8)`, `self.interrupts_enabled()`.
- Produces (Stage 1 relies on these exact signatures):
  - `pub fn has_deliverable_ext_int(&self) -> bool` — **IF-blind, raw pending**: an external interrupt exists whether or not it may be taken now (the engine's window-arming decision needs exactly this).
  - `pub fn has_non_ext_int_event(&self) -> bool` — the deliverable set minus the ExtInt bits, unmasked semantics preserved (SMI/NMI keep their own masking).
  - `pub fn set_lapic_tpr_from_cr8(&mut self, cr8: u8)` — `MOV CR8` never exits on WHP; the LAPIC's TPR refreshes from the exit header before any delivery decision.

- [ ] **Step 1: Write the failing tests** (append to arch_state.rs tests, using the existing `BxCpuBuilder` pattern from `an_attribute_word_round_trips_through_its_parts`'s module):

```rust
/// The window-arming probe is IF-blind: a line asserted under CLI is still
/// a reason to arm an interrupt window, and only delivery is gated on IF.
#[test]
fn a_pending_external_interrupt_is_reported_even_with_interrupts_masked() {
    let mut cpu = test_cpu(); // the module's existing constructor helper; if the
                              // module lacks one, build via BxCpuBuilder as the
                              // neighbouring tests do
    cpu.signal_event(BxCpuC::<()>::BX_EVENT_PENDING_INTR);
    cpu.set_rflags_for_api(0x2); // IF = 0
    assert!(cpu.has_deliverable_ext_int(), "raw pending must not consult IF");
    assert!(!cpu.has_an_event_to_deliver(), "the deliverable question still says no");
}

/// SMI is not an external interrupt: it must show on the non-ext probe and
/// never on the ext probe.
#[test]
fn an_smi_is_not_an_external_interrupt() {
    let mut cpu = test_cpu();
    cpu.signal_event(BxCpuC::<()>::BX_EVENT_SMI);
    assert!(!cpu.has_deliverable_ext_int());
    assert!(cpu.has_non_ext_int_event());
}

/// CR8 is the top four TPR bits, exactly as `export_arch_state` reads them
/// back (`lapic.get_tpr() >> 4`).
#[test]
fn cr8_lands_in_the_task_priority_register() {
    let mut cpu = test_cpu();
    cpu.set_lapic_tpr_from_cr8(0x9);
    let mut state = VcpuArchState::default();
    cpu.export_arch_state(&mut state);
    assert_eq!(state.cr8, 0x9);
}
```

(If `BX_EVENT_SMI` is spelled differently, read the `BX_EVENT_*` consts in `cpu/cpu.rs` and use the SMI one verbatim — do not guess.)

- [ ] **Step 2: Run to verify failure.** `cargo test --release -p rusty_box --lib --features std arch_state::tests` → the three tests FAIL to compile (methods missing).

- [ ] **Step 3: Implement**, next to `has_an_event_to_deliver`:

```rust
/// Whether an external interrupt EXISTS — asserted line or pending LAPIC
/// vector — regardless of whether it may be taken now. IF-blind on
/// purpose: an engine that injects has to arm an interrupt window for a
/// vector the guest is masking, and a probe that consulted IF would never
/// arm one. Only [`DeliverableInterrupt::MASK`]'s bits count; an SMI, an
/// NMI or an INIT is not an external interrupt and never reaches an
/// injection path.
#[must_use]
pub fn has_deliverable_ext_int(&self) -> bool {
    self.pending_event & DeliverableInterrupt::MASK != 0 || self.lapic.intr
}

/// Whether something OTHER than an external interrupt is deliverable —
/// the events an engine must still hand to the shadow (SMI, NMI, INIT,
/// shutdown), with each event's own masking preserved.
#[must_use]
pub fn has_non_ext_int_event(&self) -> bool {
    self.is_unmasked_event_pending(!DeliverableInterrupt::MASK)
}

/// The task-priority register as `MOV CR8` writes it: the top four bits
/// of the local APIC's TPR. A guest's `MOV CR8` retires on the hardware
/// without an exit, so the engine refreshes this from the exit header
/// before any delivery decision — the same route `import_arch_state`
/// takes for the whole-state exchange.
pub fn set_lapic_tpr_from_cr8(&mut self, cr8: u8) {
    self.lapic.set_tpr((cr8 & 0xF) << 4);
}
```

- [ ] **Step 4: Verify** `cargo test --release -p rusty_box --lib --features std arch_state::tests` PASS, then `cargo check --release --no-default-features -p rusty_box`.
- [ ] **Step 5: `cargo xtask ci`, then commit** `fix(cpu): the engine can ask about external interrupts without taking them`.

### Task 2: Extract the INTA chain into one shared body

**Files:**
- Modify: `rusty_box/src/cpu/event.rs` (the priority-5 arm, lines ~421-564)
- Modify: `rusty_box/src/emulator/io.rs` (new `PcIo` verb beside `emulate_one`, ~line 143)
- Test: `rusty_box/src/cpu/event.rs` tests (or the module the existing HAE tests live in — find with `grep -rn "diag_hae_intr" rusty_box/src --include=*.rs`)

**Interfaces:**
- Consumes: the chain exactly as it stands (quoted below), `DeliverableInterrupt::MASK`, `PcIo`'s ExecCtx construction (copy `emulate_one`'s plumbing shape).
- Produces:
  - On ExecCtx (event.rs): `pub(crate) fn acknowledge_external_interrupt(&mut self) -> AcknowledgedInterrupt` where

    ```rust
    /// What one INTA moment produced.
    pub(crate) enum AcknowledgedInterrupt {
        /// The LAPIC answered; delivery (or a VMX fold) is the caller's.
        Lapic(u8),
        /// The 8259 answered through the fabric's counted INTA; spurious
        /// vectors arrive here exactly as a real INTA would produce them.
        Pic(u8),
        /// Nothing deliverable; the deasserted pin was reconciled.
        None,
    }
    ```
  - On PcIo (io.rs): `pub fn pop_deliverable_vector<T: Instrumentation>(&mut self, cpu: &mut BxCpuC<T>) -> Option<u8>` — builds the ExecCtx the way `emulate_one` does, calls the same shared body, returns the vector from either arm. This is the engine's INTA moment in Stage 1.

**The extraction, precisely.** The shared body takes these pieces of the current arm VERBATIM (this is a refactor, not a rewrite — R5, one body):

- LAPIC arm: `if self.lapic.intr { self.clear_event(PENDING_LAPIC_INTR); let vector = self.lapic.acknowledge_int(); self.sync_lapic_events(); if vector > 0 { self.activity_state = Active; return Lapic(vector) } }` — the `#[cfg(debug_assertions)]` diag counters move with it.
- PIC arm: `if self.device_manager.irq.int_pin_asserted() { let vector = self.device_manager.irq.acknowledge(); self.activity_state = Active; return Pic(vector) }` — the existing `tracing::trace!` line moves with it.
- None arm: the reconcile block verbatim (`irq.pic_mut().reconcile_deasserted_intr(); self.clear_event(PENDING_INTR); if pending_event & PENDING_LAPIC_INTR == 0 { self.async_event = BX_ASYNC_EVENT_STOP_TRACE; }` + its diag counter).

What stays in `handle_async_event`, in its current order: the VMX pre-checks (virtual-interrupt fast path, `vmexit_check_ext_intr_no_ack`), then the call, then per-arm: the posted-interrupt fold (`Lapic(v)` + `in_vmx_guest` + `vmx_posted_interrupt_processing(v)` → consumed), `vmexit_check_event_intr(vector)`, `self.ext = true`, `self.interrupt(vector, ExternalInterrupt, false, false, 0)`, `self.ext = false`, the `prev_rip` updates and `CpuLoopRestart` handling — all exactly as today. The `vector > 0` LAPIC gate lives in the shared body (a zero vector falls through to the PIC arm, as today's `delivered` flag arranges).

- [ ] **Step 1: Write the failing test** — the parity test that pins the extraction (in the tests module where TestMachine/exec harness lives; `grep -rn "TestMachine" rusty_box/src/cpu/exec_ctx.rs` shows the harness):

```rust
/// The extracted INTA body and the interpreter's delivery agree: raising a
/// PIC line and letting handle_async_event run delivers the same vector,
/// through the same counted acknowledge, as popping it directly.
#[test]
fn the_popper_and_the_interpreter_acknowledge_identically() {
    // Machine A: raise IRQ1 (vector 0x21 after BIOS-less default init: read
    // the PIC's default vector base in iodev/pic.rs and use the real value),
    // deliver via the interpreter loop, record irq.acknowledge_count().
    // Machine B: identical setup, call io.pop_deliverable_vector(cpu),
    // assert Some(same vector) and the same acknowledge_count delta (1).
    // Build both machines with the harness this module already uses.
}
```

Write it as real code against the harness found in step 0 of this task (the grep); assert vector equality and `acknowledge_count` delta equality. Also port a spurious-vector case: assert a masked-then-lowered line pops the 8259's spurious vector (the PIC's `iac()` produces it; the test raises then masks IRQ before popping).

- [ ] **Step 2: Run to verify failure** (compile failure: `pop_deliverable_vector` missing).
- [ ] **Step 3: Implement the extraction** as specified above. `pop_deliverable_vector` maps `Lapic(v) | Pic(v)` → `Some(v)`, `None` → `None`, with `debug_assert!(v > 0 || matches!(..., Pic(_)))`.
- [ ] **Step 4:** `cargo test --release -p rusty_box --lib --features std` (whole lib — the delivery path is load-bearing everywhere), plus the no_std check.
- [ ] **Step 5:** `cargo xtask ci`; commit `refactor(cpu): one INTA body answers the interpreter and any engine`.

### Task 3: Census counters and the provenance fix

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (`ExitCounts` ~line 360, `SliceCensus`, the wtf comment at ~line 1501)

**Interfaces:**
- Produces on `ExitCounts` (pub, `#[non_exhaustive]`): `pub window: u64` (InterruptWindow exits), and on the engine a new `pub struct InjectCensus { pub injected: u64, pub windows_armed: u64, pub injected_per_vector: [u32; 256] }` with `pub fn inject_census(&self) -> &InjectCensus` — zero until Stage 1 wires them.

- [ ] **Step 1:** Add the fields + accessor + doc comments (each field's doc names the invariant: `injected` must equal the fabric's `acknowledge_count` delta attributable to injection — the Risk-1 tripwire).
- [ ] **Step 2:** Replace the wtf attribution in the cancel-rule comment with QEMU: the re-enter-on-`InterruptionPending` precedent is `whpx-all.c` (post_run caches the bit; pre_run refuses to stage over it); wtf's Canceled arm stops-and-restores and holds no such rule (R7).
- [ ] **Step 3:** `cargo check --release -p rusty_box_whp_engine`; `cargo test --release -p rusty_box_whp_engine --lib`.
- [ ] **Step 4:** `cargo xtask ci`; commit `feat(whp): the census learns to count injections before any exist`.

### Task 4: Partition-setup audit — ProcessorFeatures and restore hygiene

**Files:**
- Modify: `rusty_box_whp/src/sys.rs` (`PropertyCode`), `rusty_box_whp/src/sys/windows.rs` (`property_code` match), `rusty_box_whp/src/sys/unsupported.rs` (nothing — properties go through the existing `set_property`), `rusty_box_whp/src/partition.rs` (`PartitionConfig` builder verb), `rusty_box_whp_engine/src/engine.rs` (`start()`)
- Test: `rusty_box_whp/src/partition.rs` hypervisor-gated test

**Interfaces:**
- Produces: `PropertyCode::ProcessorFeatures` (payload 8 bytes, `WHvPartitionPropertyCodeProcessorFeatures = 0x1001`) and `PartitionConfig::processor_features(&mut self, features: u64) -> WhpResult<&mut Self>`.

- [ ] **Step 1:** Read what the platform offers: in `start()` the engine already queries `CapabilityCode::Features`; add a query of the banked processor features via the existing `capability()` path (`WHvCapabilityCodeProcessorFeatures = 0x1003` — add to `CapabilityCode` + `capability_code` with payload 8). Set the partition property to exactly what the capability reported (the wtf-issue-252 class: leaving it unset makes `wrmsr IA32_SPEC_CTRL` #GP on hardware where the shadow build never shows it).
- [ ] **Step 2:** Hypervisor-gated test in partition.rs: build a partition with `processor_features(reported)`, assert setup succeeds; and assert `capability(ProcessorFeatures)` returns nonzero on this host.
- [ ] **Step 3:** Verify snapshot-restore zeroes in-flight events: `grep -n "PendingInterruption\|PendingEvent" rusty_box/src/emulator/emulator_api.rs rusty_box/src/emulator/snapshot.rs rusty_box_whp_engine/src/engine.rs`. If restore does not already write `PendingInterruption = 0` into the partition (wtf `LoadState` precedent: "Ensure that there's no pending event"), add it to the engine's restore path (the `held`-invalidating branch) via `vp.write_words(&[Reg::PendingInterruption], &[0])`.
- [ ] **Step 4:** ci; commit `fix(whp): the partition is told which processor features the host banks`.

### Task 5: Stage 0 gate

- [ ] Run the DLX oracle 3×: 3/3 milestones, wall time within noise of 2.0s.
- [ ] Run bounded Alpine (600s, `--display terminal`, per the handoff repro): reaches `login:`, zero fatal signatures.
- [ ] Interpreter delivery unchanged: the Task-2 parity test passes; `acknowledge_count` histogram on a fixed-budget DLX run matches a pre-stage run (capture before starting Stage 0; compare with the dlx example's census output).
- [ ] `cargo xtask ci` green. **Stop for user review before Stage 1.**

---

## Stage 1 — injection

### Task 6: `InjectState` and header refresh

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (`Started` struct; the exit loop where `exit.vp` is read)
- Test: engine.rs unit tests (shadow-only, no hypervisor needed)

**Interfaces:**
- Produces:

```rust
/// What the last exit header said about deliverability — refreshed from
/// headers, never from register reads (QEMU whpx-all.c post_run).
struct InjectState {
    /// `ExecutionState` bit 6 — a delivery the platform has not landed.
    in_flight: bool,
    /// `Rflags` bit 9 from the header.
    if_flag: bool,
    /// The header's `Cr8` nibble.
    cr8: u8,
    /// A `DeliverabilityNotifications` armed at this priority, not yet
    /// answered by a window exit.
    window: Option<u8>,
}
```

with `fn refresh_from(&mut self, vp: &VpContext)` setting the first three (window is owned by `stage_injection`/the window arm alone). `Started` gains `inject: InjectState`.

- [ ] **Step 1: Failing test** — `refresh_from` decodes a fabricated `VpContext` (`execution_state` 0x0040 → `in_flight`; rflags 0x202 → `if_flag`; cr8 nibble). Pure struct test.
- [ ] **Step 2–4:** Implement; wire `inject.refresh_from(&exit.vp)` as the FIRST statement after each `partition.run` returns (before the history record); tests + checks pass.
- [ ] **Step 5:** ci; commit `feat(whp): the engine remembers what the last exit said about deliverability`.

### Task 7: `stage_injection` and the interrupt-window arm

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs`
- Test: engine.rs hypervisor-gated guest test

**Interfaces:**
- Consumes: Task 1 probes, Task 2's `io.pop_deliverable_vector`, `Partition::inject(index, PendingInterruption)` (partition.rs:783; `PendingInterruption { kind: InterruptionType::Interrupt, vector, error_code: None }`), `vp.write_words(&[Reg::DeliverabilityNotifications], &[word])` where the word sets `InterruptNotification` (bit 0) and `InterruptPriority` (bits 2..6, value `vector >> 4`) per `WHV_X64_DELIVERABILITY_NOTIFICATIONS_REGISTER`.
- Produces:

```rust
/// Outcome of one staging decision, for the census and the caller's log.
enum Staged { Nothing, Windowed, Injected(u8) }

fn stage_injection<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
) -> Result<Staged>
```

implementing spec §Stage-1 A verbatim: (1) in-flight → Nothing; (2) `cpu.set_lapic_tpr_from_cr8(inject.cr8)`; (3) `!cpu.has_deliverable_ext_int()` → Nothing; (4) `started.shadowed || !inject.if_flag` → arm window if not armed at ≥ priority, Windowed; (5) `io.pop_deliverable_vector(cpu)` — THE INTA moment; (6) None → Nothing; (7) `partition.inject(...)`, `in_flight = true`, census `injected += 1`, `injected_per_vector[v] += 1`, Injected(v).

- [ ] **Step 1: Failing guest test** (hypervisor-gated, modeled on `a_machine_on_the_hypervisor_runs_a_guest...`):

```rust
/// A device interrupt reaches a hardware guest by injection: the guest
/// STIs and HLTs; the machine's PIT fires; the ISR writes a byte to the
/// debug port and HLTs again. The byte proves the vector crossed through
/// the partition, and the census proves it went by injection, not by a
/// shadow emulate_one.
#[test]
fn an_external_interrupt_reaches_a_hardware_guest_by_injection() {
    // guest real-mode program: set an IVT entry for vector 8 pointing at an
    // ISR that does `mov al, MARK; out 0xE9, al; mov al, 0x20; out 0x20, al;
    // iret`, then `sti; hlt; hlt`. Program the PIT for a short period via
    // ports 0x43/0x40 before the sti (all real instruction bytes, as the
    // existing tests write them).
    // Assert: debug port received MARK; engine.inject_census().injected >= 1.
}
```

- [ ] **Step 2:** Run: FAILS (no `stage_injection`, censi zero — delivery still shadow-side; the byte may arrive but `injected` is 0, which is the assertion that bites).
- [ ] **Step 3:** Implement `stage_injection` + the window arm: replace `ExitReason::InterruptWindow => Err(unserviced(...))` with `{ counts.window += 1; started.inject.window = None; }` falling through to the post-exit tail (no injection logic in the arm). Wire `stage_injection` at the two call sites Task 8 builds — for THIS task, call it only from the post-exit tail position after the existing `sync_io_events` (minimal wiring so the test can pass; Task 8 finishes the seams).
- [ ] **Step 4:** Test passes; all engine tests pass; checks.
- [ ] **Step 5:** ci; commit `feat(whp): an external interrupt is a register write, not a shadow errand`.

### Task 8: Rewire the slice head and post-exit tail; retire the delivery boundary

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (slice-head block ~888-926; post-exit tail ~1589-1619; the serviced-exit warn ~1426-1437; `EventDelivery::Engine` doc ~788-790; the design-assertion comment ~878-880)

**Interfaces:** consumes everything above; produces the final Stage-1 control flow (spec §B, §C, §E):

- Slice head: `sync_io_events` → non-ExtInt events only via `emulate_one` (probe: `has_non_ext_int_event`, still guarded by `!shadowed`) → SMM run-out → `install_the_shadow` → **if a shadow stretch consumed `shadowed` since the last install: `vp.write_words(&[Reg::InterruptState], &[0])`** (Risk 7; track with a `bool` set where `mem::replace(&mut started.shadowed, false)` runs) → `stage_injection` → run.
- Post-exit tail: after refresh + `sync_io_events`: machine boundaries unchanged; `has_non_ext_int_event()` → `Boundary(EventToDeliver)` (NMI/SMI/INIT still end the slice); otherwise `stage_injection` and **continue the loop** — ExtInt no longer ends a slice.
- The serviced-exit warning block: bit 6 on a *serviced* exit now means "our injection has not landed" — replace the warn-only block with the re-enter treatment (route straight back into the partition exactly as the Canceled arm does, sharing its `delivery_in_flight` mechanism) and rewrite the comment to say so.
- Delete the retired design-assertion comment ("this engine never writes WHvRegisterPendingInterruption") and reword the `EventDelivery::Engine` doc: delivery is still the machine's — the vector, the ack and the priority all come from this machine's controllers — only the final push crosses as a register.

- [ ] **Step 1: Failing assertion first:** extend Task 7's test with `assert_eq!(census.ended_event_to_deliver_ext, 0)` — add that counter split if `BoundaryReason::EventToDeliver` census doesn't distinguish; simplest: assert `engine.census().slices` did NOT grow per interrupt (the PIT test with N fires must show ≪ N slices). Run: FAILS under Task 7's minimal wiring (head-of-slice delivery still ends slices).
- [ ] **Step 2:** Implement the rewiring above.
- [ ] **Step 3:** Full engine test suite + rusty_box lib suite + no_std check (engine only — no rusty_box change in this task, skip no_std if untouched).
- [ ] **Step 4:** Risk-2 watchdog: add the boundary debug assert `debug_assert!(!cpu.has_deliverable_ext_int() || started.inject.in_flight || started.inject.window.is_some())` at slice end, with a comment citing QEMU's equivalent abort.
- [ ] **Step 5:** ci; commit `feat(whp): a slice survives its interrupts`.

### Task 9: Stage 1 gate

- [ ] DLX oracle 3×: 3/3 milestones; wall time — record it (expect ≤ 2.0s; any regression is a stop).
- [ ] Alpine bounded 600s ×2: reaches `login:`, zero fatal signatures, and the irq-tracing median vector latency (the engine logs it; compare against the 158µs baseline) collapses to ~exit-round-trip scale.
- [ ] W7 headless 300s: no fault, reaches the same phase as the 2026-09-01 baseline runs.
- [ ] Census: `inject_census().injected` ≈ fabric `acknowledge_count` delta; `window` exits nonzero only when masked delivery occurred; no `hda: unexpected_intr` / unclaimed-`int3` in any log.
- [ ] Interleaved A/B (this branch tip vs the pre-Stage-1 commit, per `docs` perf methodology): boot wall-clock not worse; expect better.
- [ ] ci green. **Stop for user review before Stage 2.**

---

## Stage 2 — lazy exchange

### Task 10: The `ShadowFreshness` witness and the choke point

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs`
- Test: engine.rs unit + hypervisor-gated tests

**Interfaces:**
- Produces (spec §Stage-2 verbatim):

```rust
enum ShadowFreshness {
    ShadowCurrent,
    PlatformCurrent { mirror: MirrorValidity },
}
enum MirrorValidity { ExitHeader, Invalidated }
```

`Started` gains `freshness: ShadowFreshness`. The choke point: `fn current_shadow<'a, T: Instrumentation>(started: &'a mut Started, cpu: &mut BxCpuC<T>, machine_ticks: u64) -> Result<()>` — wraps today's `read_back_into_the_shadow` whole (export + XSAVE + InterruptState + `discard_decoded_traces` + TSC rebase), no-ops on `ShadowCurrent`, sets `ShadowCurrent` after. Every current caller of `read_back_into_the_shadow` converts to it; `read_back_into_the_shadow` becomes private to the choke point. Transitions: any `partition.run` return sets `PlatformCurrent{ExitHeader}`; `service_port_access`'s Rip+Rax write sets `Invalidated`; shadow execution and installs leave `ShadowCurrent`.

- [ ] **Step 1: Failing test:** unit test on the transition table (fabricate `Started` via the existing test paths or factor the enum's transition into free functions tested directly): run→`PlatformCurrent`, choke→`ShadowCurrent`, choke again→no second read (count reads via a test counter on the fake `VpRegisters` recorder in state.rs tests — extend `Recorder` with a read counter and assert one batch, not two).
- [ ] **Step 2–4:** Implement; this task changes NO caller behavior (every site still faults in eagerly by calling the choke point exactly where readback ran before) — the suite must stay green bit-for-bit.
- [ ] **Step 5:** ci; commit `refactor(whp): one witness says whose registers are current`.

### Task 11: The mini-import and the lazy slice end

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (the unconditional slice-end readback, currently "Whatever ended the run, the shadow must describe the processor"; the slice-death diagnostic)

**Interfaces:**
- Produces: `fn mini_import<T: Instrumentation>(cpu: &mut BxCpuC<T>, vp: &VpContext, inject: &InjectState)` — writes `set_rip`, `set_rflags_for_api`, `set_lapic_tpr_from_cr8` from the header. The slice end calls `mini_import` always and `current_shadow` **only when** the slice ended `Halted` needing `record_halt` bookkeeping? No — `record_halt` is engine-written and needs no readback; the full fault-in at slice end happens only for: a `Boundary(EventToDeliver)` (non-ExtInt shadow delivery follows), an error path (the slice-death dump forces `current_shadow` first — Risk 9), or nothing at all otherwise. The between-slice consumers (scheduler IF predicate, HLT fast-forward, interactive gates) are served by the mini-import (spec's verified list).

- [ ] **Step 1: Failing measurement, not a unit test:** add census counters `full_exports`/`full_imports` inside the choke point and `state::import` wrapper; assert in the PIT guest test from Task 7 that `full_exports` grows ≪ slices (e.g. `< slices / 4` for that interrupt-driven guest). Run: FAILS (today it's ≥ 1 per slice).
- [ ] **Step 2:** Implement: slice end does mini-import + freshness bookkeeping; `install_the_shadow` early-outs entirely when `held` matches AND freshness is `PlatformCurrent` (nothing shadow-side changed: compare via the existing held-comparison after a `current_shadow`… no — the point is NOT reading back. When freshness is `PlatformCurrent` and no machine architectural write occurred (tracked by a `machine_wrote: bool` on `Started`, set by Task 12's hooks), the install skips both compare and write).
- [ ] **Step 3:** Run engine suite + the Task 7/8 guest tests: green, counter assertion passes.
- [ ] **Step 4:** ci; commit `feat(whp): a timer boundary borrows the header instead of the processor`.

### Task 12: Fault-in triggers — snapshots, machine writes, diagnostics, TSC

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs`; the engine-facing snapshot/API entry points it implements (find with `grep -rn "fn snapshot\|fn restore\|SliceEngine" rusty_box_whp_engine/src/engine.rs` and the trait's definition in `rusty_box/src/emulator/`)
- Test: hypervisor-gated snapshot round-trip test

**Interfaces:** consumes `current_shadow`. Produces: every trigger from spec §Stage-2:
1. Shadow execution paths already call the choke point (Task 10).
2. Snapshot save → `current_shadow` first. Restore → invalidate `held`, zero `PendingInterruption` in the partition (Task 4 wiring), freshness = `ShadowCurrent`.
3. Machine architectural writes: the engine's surface that lets the machine write the CPU mid-ownership — `memory_map_changed` is memory-only; the writes arrive between slices via the machine owning `cpu` directly, so the guard is at the NEXT install: `machine_wrote` detection stays the held-comparison — a machine write between slices lands in the shadow, the next `install_the_shadow` sees `held != state`… **but under lazy freshness the shadow is stale, so the comparison would merge divergent copies (Risk 4).** The rule: `install_the_shadow`, when freshness is `PlatformCurrent` and the held-comparison detects a shadow-side diff, must `current_shadow` FIRST (fault in), re-apply the machine's write on top? — No: it cannot re-apply. Therefore the machine write itself must fault in first. The engine cannot intercept `&mut cpu` access by the machine, so the install-side rule is: on `PlatformCurrent` + diff detected → `tracing::error!` the multi-field diff (the Risk-4 tripwire) and do a full impose (shadow wins, matching today's semantics: the machine wrote the shadow it could see). Additionally expose `WhpEngine::describe(&mut self, cpu)` (calls `current_shadow`) and call it from the engine-trait hook the machine invokes before SIPI/`inject_interrupt` (`grep -rn "deliver_sipi\|inject_interrupt" rusty_box/src/emulator/scheduler.rs` — add the pre-write engine hook to the `SliceEngine` trait with a default no-op so the interpreter engine is untouched).
4. Slice-death dump and `report_the_fault`: `current_shadow` before dumping.
5. TSC: reads leave the runtime path — `cpu.set_tsc` rebase happens only inside the choke point (it already lives in the readback body; verify nothing else calls it per slice).

- [ ] **Step 1: Failing test:** hypervisor-gated — boot the Task-7 PIT guest for N slices, snapshot-save, restore into a fresh machine, save again, assert byte-equality of the two snapshots (the spec's gate (c) as a unit test — save→restore→save).
- [ ] **Step 2:** Implement the five triggers.
- [ ] **Step 3:** Suite + checks green.
- [ ] **Step 4:** ci; commit `fix(whp): everything that looks at the shadow first makes it true`.

### Task 13: `service_port_access` under freshness

**Files:**
- Modify: `rusty_box_whp_engine/src/engine.rs` (`service_port_access`, ~line 2001)

**Interfaces:** the aliasing rule (QEMU's vmport hazard): the targeted `write_words(&[Rip, Rax], ...)` is legal only while freshness is `PlatformCurrent` — it then also sets `MirrorValidity::Invalidated` (the mirror no longer matches the partition). If freshness is `ShadowCurrent` (a shadow stretch serviced something mid-slice — possible via burst paths), the results must be written into the SHADOW (`cpu.set_rip`, gpr write) instead, or the next flush clobbers them.

- [ ] **Step 1:** Write the guard with an exhaustive `match` on freshness (R5) — both arms real, no fallthrough.
- [ ] **Step 2:** Engine suite green (the DLX example is the live consumer: run it, 3/3 milestones).
- [ ] **Step 3:** ci; commit `fix(whp): a port answer lands wherever the registers currently live`.

### Task 14: Stage 2 gate

- [ ] Full-exchange census: `full_exports`+`full_imports` per second during a steady stretch of Alpine boot ≈ shadow-execution events (report both directions; the spec's gate (a)).
- [ ] Interleaved A/B vs the Stage-1 tip: boot wall-clock improved (DLX + Alpine; report numbers).
- [ ] Snapshot save→restore→save byte-equality (Task 12's test, plus once against Alpine mid-boot).
- [ ] Risk-2 watchdog silent (no debug-assert trips) over a full Alpine boot in a debug-assertions build of the engine crate (`cargo test` profile covers it; for the boot use `--profile release-with-debug-asserts` only if one exists — otherwise run the bounded boot on the dev profile once, slowness accepted).
- [ ] Same boot milestones: DLX 3/3, Alpine `login:`, W7 to installer phase.
- [ ] ci green. **Campaign complete — hand back for the throughput re-measure (575s Alpine baseline, 158µs latency baseline) and user review.**

---

## Self-review checklist (ran at write time)

- Spec coverage: Stage 0 items 1-6 → Tasks 1-4; §A-F → Tasks 6-8; §Stage-2 state machine/choke/mini-import/triggers/aliasing → Tasks 10-13; all three gates → Tasks 5, 9, 14; the ten risks each land in a task (1→T7/T9, 2→T8, 3→T2's debug_assert, 4→T12.3, 5→T8's re-enter reuse, 6→T7 window dedup + T9 census, 7→T8's InterruptState clear, 8→T2 spurious test, 9→T11/T12.4, 10→T4).
- No placeholders: every step names real files, real symbols verified against the tree this session (`pending_event`, `is_unmasked_event_pending`, `lapic.acknowledge_int`, `int_pin_asserted`/`acknowledge`, `Partition::inject`, `PendingInterruption` layout, `Reg::DeliverabilityNotifications`, `Reg::InterruptState`).
- Known intentional deviation: none. `CpuidResultList` is out of scope per the spec.
