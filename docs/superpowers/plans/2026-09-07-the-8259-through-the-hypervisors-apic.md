# The 8259 Through the Hypervisor's APIC — Implementation Plan

> **SUPERSEDED — DO NOT IMPLEMENT.** Its design was refuted by measurement: a
> hypervisor local APIC cannot carry the 8259's vectors. `WHvRequestInterrupt`
> refuses vector `0x08` with `0xC0350005` because an APIC takes no vector below
> 16, and with the controller remapped above that floor the call is accepted
> while the vector vanishes into an APIC a legacy guest never enables. Tasks 1–3
> of this plan were implemented, then reverted by
> `docs/superpowers/plans/2026-09-07-inject-at-entry-not-by-cancel.md`, which
> restores the mechanism this one deleted. See
> `docs/superpowers/specs/2026-09-07-the-8259-through-the-hypervisors-apic-design.md`
> for the full refutation, and divergence `H9` for what shipped instead.
>
> Kept for its reasoning and its measurements, which the replacement builds on.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A legacy 8259 interrupt reaches a guest running on the hypervisor, by acknowledging it on the machine's side and handing the resolved vector to the hypervisor's own local APIC — deleting the hand-written readiness gate that livelocks.

**Architecture:** The machine already routes I/O APIC messages through `SliceEngine::route_ioapic_delivery`, which maps an ExtINT delivery to `InterruptKind::Fixed` with an already-resolved vector and hands it to `WHvRequestInterrupt`. This plan routes the 8259's INT pin down that same path. The engine gains one question — does it own the guest's local APIC — and loses `pic_pin_changed`, the pending/blocked flags, the cancel and the staging step.

**Tech Stack:** Rust, `rusty_box` (machine), `rusty_box_whp_engine` (WHP engine), Windows Hypervisor Platform.

**Spec:** `docs/superpowers/specs/2026-09-07-the-8259-through-the-hypervisors-apic-design.md`

## Global Constraints

- Read `CLAUDE.md` first and follow it. In particular: edit with Edit/Write tools, **never** shell heredocs or Python; release builds only; never run `cargo fmt`; never `let _ = <Result>`; comments state today's invariant, never history; Bochs citations name file + symbol, never line numbers.
- Verification cadence: `cargo check --release -p rusty_box --features std --lib` after each edit batch, **plus** `cargo check --release -p rusty_box --no-default-features` for any change under `emulator/` or `memory/`.
- `cargo xtask ci` before every commit. **Never append anything after the gate command on the same shell line** — a pipeline's exit code masks the gate's. Redirect to a log, echo `$?` separately, and read the gate's own `ci: N steps passed` line.
- **Never stage** `ROADMAP.md`, `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md`, or the three untracked `docs/superpowers/specs/2026-08-22-*.md` files — someone else's work. Use explicit `git add <path>`; never `git add -A`.
- Commit messages end with: `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
- The WHP halt-errand test is a **pre-existing** flake at roughly 1 run in 3. Re-run before blaming a change for it.
- Branch: `wip/atom-execctx`. Do not create new branches.

---

### Task 1: The engine says who owns the guest's local APIC

Replaces the action verb `pic_pin_changed` with a question. Nothing behavioural changes yet — this task only moves the seam, so it can be reviewed on its own.

**Files:**
- Modify: `rusty_box/src/emulator/engine.rs` (the `SliceEngine` trait: remove `pic_pin_changed`, add `owns_the_guests_local_apic`)
- Modify: `rusty_box/src/emulator/scheduler.rs` (delete the `pic_pin_changed` call site in `sync_final_event_levels`)
- Modify: `rusty_box/src/emulator/mod.rs` (delete the `pic_pin_published` field, its two placement-construction writes, the destructure arm, and the call site at the reset path)
- Modify: `rusty_box/src/emulator/tests.rs` (both test engines lose their `pic_pin_changed` override)
- Modify: `rusty_box_whp_engine/src/engine.rs` (`WhpEngine` loses `pic_pin_changed`, gains `owns_the_guests_local_apic`)

**Interfaces:**
- Consumes: `WhpEngine::started`, `LocalApicMode` (both already in `engine.rs`).
- Produces: `SliceEngine::owns_the_guests_local_apic(&self) -> bool`, default `false`. Task 2 calls it.

- [ ] **Step 1: Write the failing test**

In `rusty_box_whp_engine/src/engine.rs`'s test module:

```rust
    /// A machine whose partition has no local APIC of its own keeps this
    /// machine's model APIC, and says so.
    ///
    /// The answer decides who acknowledges the 8259: an engine that owns the
    /// guest's APIC takes the legacy line as a resolved vector, and one that
    /// does not leaves it to `set_legacy_intr_level` and the deferred
    /// acknowledge. An engine that has not started a partition at all owns
    /// nothing.
    #[test]
    fn an_engine_with_no_partition_owns_no_local_apic() {
        let engine = WhpEngine::default();
        assert!(
            !<WhpEngine as SliceEngine<()>>::owns_the_guests_local_apic(&engine),
            "an engine that has started nothing cannot be the guest's APIC"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release -p rusty_box_whp_engine --lib an_engine_with_no_partition -- --nocapture`
Expected: FAIL to compile — `no method named owns_the_guests_local_apic`.

- [ ] **Step 3: Add the trait method and remove `pic_pin_changed`**

In `rusty_box/src/emulator/engine.rs`, delete the whole `pic_pin_changed` method from the `SliceEngine` trait and add in its place:

```rust
    /// Whether this ENGINE, rather than this machine's own `cpu/apic.rs`, is
    /// the local APIC the guest reads.
    ///
    /// The one question the legacy 8259 path turns on. An engine that answers
    /// `true` is handed that line as an already-resolved vector through
    /// [`Self::route_ioapic_delivery`], because a local APIC is given numbers
    /// and not wires — there is no verb anywhere on this seam for asserting
    /// LINT0. An engine that answers `false` leaves the line to the machine's
    /// own model APIC, which is asked for its vector only when the processor
    /// can take it.
    ///
    /// Defaults to `false`, which is the answer for every engine that runs the
    /// guest on this machine's own processor.
    #[must_use]
    fn owns_the_guests_local_apic(&self) -> bool {
        false
    }
```

- [ ] **Step 4: Delete the call site and its bookkeeping**

In `rusty_box/src/emulator/scheduler.rs`, in `sync_final_event_levels`, delete the entire `if asserted || self.pic_pin_published { ... }` block and the long comment above it that explains the level-versus-edge publication — the comment describes a mechanism that no longer exists.

In `rusty_box/src/emulator/mod.rs`, delete the `pic_pin_published` field and its doc, the two `core::ptr::addr_of_mut!((*ptr).pic_pin_published).write(false);` lines in the placement constructors, the `pic_pin_published: _,` destructure arm, and the `if self.pic_pin_published { ... }` block that calls `pic_pin_changed(false)`.

In `rusty_box/src/emulator/tests.rs`, delete both `fn pic_pin_changed` overrides.

- [ ] **Step 5: Implement the WHP engine's answer**

In `rusty_box_whp_engine/src/engine.rs`, delete `fn pic_pin_changed` entirely and add to the same `impl SliceEngine<T> for WhpEngine` block:

```rust
    fn owns_the_guests_local_apic(&self) -> bool {
        self.started
            .as_ref()
            .is_some_and(|started| started.apic_mode != LocalApicMode::None)
    }
```

- [ ] **Step 6: Run the checks**

```bash
cargo check --release -p rusty_box --features std --lib
cargo check --release -p rusty_box --no-default-features
cargo test --release -p rusty_box_whp_engine --lib an_engine_with_no_partition
```
Expected: both checks clean; the test PASSES.

The `ext_int_pending`/`ext_int_blocked`/`raise_ext_int` machinery is now unreachable from the machine but still compiles. Dead-code warnings on it are expected and are cleared in Task 3.

- [ ] **Step 7: Commit**

```bash
git add rusty_box/src/emulator/engine.rs rusty_box/src/emulator/scheduler.rs rusty_box/src/emulator/mod.rs rusty_box/src/emulator/tests.rs rusty_box_whp_engine/src/engine.rs
git commit -m "refactor(engine): the seam asks who owns the guest's local APIC instead of announcing a pin

A pin level was the wrong thing to tell an engine. What the machine actually
needs to know is who the guest's local APIC is, because that decides who
acknowledges the 8259 and when: an engine that owns the APIC is handed a
resolved vector, and one that does not leaves the line to the model APIC's
deferred acknowledge.

No behaviour changes here. The legacy path still runs through
\`set_legacy_intr_level\`; the routing that uses this answer arrives next.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: The machine acknowledges the 8259 and routes its vector

The behavioural change. The machine's boundary, when the engine owns the APIC and LINT0 admits the line, performs the counted acknowledge and routes the vector as a Fixed physical edge delivery to the boot processor.

**Files:**
- Modify: `rusty_box/src/emulator/scheduler.rs` (`sync_final_event_levels`: branch the legacy line, add `route_the_legacy_line`)
- Test: `rusty_box/src/emulator/tests.rs`

**Interfaces:**
- Consumes: `SliceEngine::owns_the_guests_local_apic` (Task 1); `IrqFabric::{lint0_admits_ext_int, int_pin_asserted, acknowledge, acknowledge_count}` (`iodev/irq.rs` — the first three are `pub(crate)`, which is enough for `scheduler.rs` and `tests.rs`); `IoApicDelivery { vector: u8, delivery_mode: IoApicDeliveryMode, trigger_mode: IoApicTrigger, dest: u32, dest_mode: IoApicDestinationMode }` (`iodev/irq.rs`, `Copy + Debug`, exactly five fields — `needs_pic_iac` belongs to `PendingIoApicDelivery`, not this); `SliceEngine::route_ioapic_delivery(&mut E, IoApicDelivery) -> DeliveryRoute`.
- Produces: `Emulator::route_the_legacy_line(&mut self)`, private to `scheduler.rs`, called from `sync_final_event_levels`.

`Emulator::engine(&self) -> &E` already exists at `rusty_box/src/emulator/run.rs:977`
(not in `mod.rs`, which holds only `engine_mut()`) and `tests.rs` uses it throughout.
Do not add one — that is a duplicate definition.

- [ ] **Step 1: Write the failing tests**

In `rusty_box/src/emulator/tests.rs`. `MapWatchingEngine` is the existing test engine; it needs to answer `true` and record what it was routed. Add to it:

```rust
        fn owns_the_guests_local_apic(&self) -> bool {
            self.owns_apic
        }
```

add the two fields to its struct (`owns_apic: bool`, `routed: std::vec::Vec<crate::iodev::irq::IoApicDelivery>`), default both, and make its `route_ioapic_delivery` push onto `routed` and answer `DeliveryRoute::Backend`.

Then the tests:

```rust
    /// An engine that owns the guest's local APIC is handed the 8259's line as
    /// a resolved vector, once per boundary that finds the pin high.
    ///
    /// A local APIC is given numbers, not wires: there is no verb for asserting
    /// LINT0 on a hypervisor's APIC, so the acknowledge happens here and the
    /// number travels. Bochs takes the same branch wherever the line reaches an
    /// APIC rather than a processor — `iodev/ioapic.cc service_ioapic` reads
    /// `DEV_pic_iac()` for a mode-7 entry before delivering it.
    #[test]
    fn a_backend_apic_is_handed_the_8259s_vector_not_its_pin() {
        let mut machine = furnished_machine_on::<MapWatchingEngine>();
        machine.engine_mut().owns_apic = true;
        machine.device_manager.irq.pic_mut().master.imr = 0xFE; // unmask IRQ0
        machine
            .device_manager
            .irq
            .raise(rusty_box_devices::api::IrqLine(0));

        let before = machine.device_manager.irq.acknowledge_count();
        machine.sync_event_flags();

        assert_eq!(
            machine.device_manager.irq.acknowledge_count(),
            before + 1,
            "one boundary with the pin high is one INTA"
        );
        let routed = &machine.engine().routed;
        assert_eq!(routed.len(), 1, "and one delivery: {routed:?}");
        assert_eq!(
            routed[0].delivery_mode,
            crate::iodev::ioapic::IoApicDeliveryMode::ExtInt,
            "the message says where the vector came from"
        );
        assert_eq!(routed[0].dest, 0, "the legacy wire goes to the boot processor");
        assert_eq!(
            machine.cpu().pending_event & BxCpuC::<()>::BX_EVENT_PENDING_INTR,
            0,
            "and the model APIC is NOT also told — that would deliver twice"
        );
    }

    /// A masked LINT0 refuses the line without spending it, on the backend path
    /// exactly as on the model one (divergence D6).
    #[test]
    fn a_backend_apic_is_handed_nothing_when_lint0_refuses() {
        let mut machine = furnished_machine_on::<MapWatchingEngine>();
        machine.engine_mut().owns_apic = true;
        machine.device_manager.irq.set_bsp_lint0(0x0001_0700); // masked, ExtINT
        machine.device_manager.irq.pic_mut().master.imr = 0xFE;
        machine
            .device_manager
            .irq
            .raise(rusty_box_devices::api::IrqLine(0));

        let before = machine.device_manager.irq.acknowledge_count();
        machine.sync_event_flags();

        assert_eq!(
            machine.device_manager.irq.acknowledge_count(),
            before,
            "a masked LINT0 takes no INTA"
        );
        assert!(machine.engine().routed.is_empty(), "and routes nothing");
        assert!(
            machine.device_manager.irq.int_pin_asserted(),
            "and does not spend the line — it is owed once the guest unmasks"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --release -p rusty_box --lib --features std a_backend_apic`
Expected: FAIL — no acknowledge and nothing routed, because the machine does not do this yet.

- [ ] **Step 3: Implement the routing**

In `rusty_box/src/emulator/scheduler.rs`, add the method and call it from `sync_final_event_levels` where the deleted `pic_pin_changed` block used to sit:

```rust
    /// Hand the 8259's asserted line to an engine that owns the guest's local
    /// APIC, as a resolved vector.
    ///
    /// A local APIC takes numbers, not wires — nothing on this seam asserts
    /// LINT0 — so the acknowledge that resolves the vector happens HERE, on the
    /// side that owns the controller, and the number travels. Bochs takes the
    /// same branch wherever the line reaches an APIC rather than a processor:
    /// `iodev/ioapic.cc service_ioapic` reads `DEV_pic_iac()` for a mode-7
    /// entry before delivering it, while `pc_system.cc raise_INTR` — the line
    /// wired straight at a processor — carries no vector at all.
    ///
    /// Nothing here runs for a machine whose own model APIC is the guest's;
    /// that line reaches the processor through `set_legacy_intr_level` and is
    /// acknowledged only when the processor can take it.
    ///
    /// Runaway is the 8259's own business, not a gate here. Its in-service and
    /// priority logic will not present another interrupt of equal or lower
    /// priority until the guest writes EOI, and that EOI is a port write this
    /// machine already services — so a boundary that finds the pin high
    /// acknowledges exactly one vector, and the next finds it low.
    fn route_the_legacy_line(&mut self) {
        if !self.device_manager.irq.int_pin_asserted()
            || !self.device_manager.irq.lint0_admits_ext_int()
        {
            return;
        }
        let vector = self.device_manager.irq.acknowledge();
        let delivery = crate::iodev::irq::IoApicDelivery {
            vector,
            // Said out loud rather than folded to `Fixed` here: the engine's own
            // `requested_kind` is the one place that decides what an ExtINT
            // becomes on a given backend, and a message that lied about where
            // its vector came from would take that decision away from it.
            delivery_mode: crate::iodev::ioapic::IoApicDeliveryMode::ExtInt,
            // The virtual wire is edge-triggered and reaches one processor: the
            // boot processor, which is the only one that holds it
            // (`BxLocalApic::preset_lint0`, divergence D6).
            trigger_mode: crate::iodev::irq::IoApicTrigger::Edge,
            dest: 0,
            dest_mode: crate::iodev::irq::IoApicDestinationMode::Physical,
        };
        match <E as SliceEngine<T>>::route_ioapic_delivery(&mut self.engine, delivery) {
            // The backend has it; its APIC holds the vector until the guest can
            // take it, which is the whole reason the line travels this way.
            DeliveryRoute::Backend => {}
            // An engine that owns the guest's APIC and then declines its own
            // message has already been asked the wrong question — and the
            // vector is spent, so there is nowhere to put it back.
            DeliveryRoute::Model | DeliveryRoute::Undelivered => {
                tracing::error!(
                    "the backend APIC would not take legacy vector {vector:#04x}, which is \
                     already acknowledged and cannot be returned to the 8259"
                );
            }
            DeliveryRoute::Refused(fault) => self.engine_fault = Some(fault),
        }
    }
```

Call it from `sync_final_event_levels`. It REPLACES the level publication rather
than joining it — the two are alternatives, not a sequence, because a line
published to both APICs is delivered twice. Replace the existing
`self.cpu_mut().set_legacy_intr_level(asserted);` line (Task 1 already deleted
the `pic_pin_changed` block that followed it) with:

```rust
        // Exactly one APIC is the guest's, so exactly one of these runs. The
        // model's takes a level and defers the acknowledge until its processor
        // can be asked; a backend's takes a resolved vector, because there is
        // no verb for asserting LINT0 on it and nobody to ask.
        if <E as SliceEngine<T>>::owns_the_guests_local_apic(&self.engine) {
            self.route_the_legacy_line();
        } else {
            self.cpu_mut().set_legacy_intr_level(asserted);
        }
```

Note `asserted` is read into a local ABOVE this block and stays there — the
`irq_pending`/`irq_cleared` clearing that follows it is unconditional on both
paths, and `route_the_legacy_line` re-reads the pin itself.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test --release -p rusty_box --lib --features std a_backend_apic
cargo test --release -p rusty_box --lib --features std
cargo check --release -p rusty_box --no-default-features
```
Expected: the two new tests PASS; the whole lib suite PASSES; the no-default build is clean.

- [ ] **Step 5: Commit**

```bash
git add rusty_box/src/emulator/scheduler.rs rusty_box/src/emulator/tests.rs
git commit -m "feat(emulator): the 8259's line reaches a backend APIC as a vector, not a pin

A local APIC is given numbers and not wires, and nothing on the engine seam
asserts LINT0. So a machine whose engine owns the guest's APIC acknowledges the
8259 at its own boundary and routes the resolved vector through the path its
I/O APIC messages already take.

Bochs takes the same branch for the same reason: \`iodev/ioapic.cc
service_ioapic\` reads \`DEV_pic_iac()\` for a mode-7 entry before delivering it,
while \`pc_system.cc raise_INTR\` — the line wired straight at a processor —
carries no vector at all.

Divergence D6 is unchanged: a masked LINT0 refuses the line and does not spend
it.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: Delete the staging machinery

With the machine routing the line, the vCPU thread's hand-written readiness gate has no caller. This task removes it and the two flags it turned on.

**Files:**
- Modify: `rusty_box_whp_engine/src/vcpu_thread.rs` (delete `stage_the_legacy_interrupt`, its call from the run loop, `ext_int_request`, its two tests, and `VcpuControl::{ext_int_pending, ext_int_blocked, raise_ext_int}`)
- Modify: `rusty_box_whp_engine/src/engine.rs` if a `PendingExtIntEvent` import or helper is left unused

**Interfaces:**
- Consumes: nothing new.
- Produces: nothing. This task only removes.

- [ ] **Step 1: Delete the staging step and its call**

Delete `VcpuThread::stage_the_legacy_interrupt` entirely, and the block in `run_loop` that calls it:

```rust
            if let Continue::Park(why) = self.stage_the_legacy_interrupt() {
                if self.park(why) == Woke::ToStop {
                    return;
                }
                continue;
            }
```

- [ ] **Step 2: Delete the request path**

Delete the free function `ext_int_request` with its whole doc comment, `VcpuControl::raise_ext_int`, and the `ext_int_pending` and `ext_int_blocked` fields with their docs and their initialisers in `VcpuThread::spawn`.

Delete the two tests `a_pin_reported_at_every_boundary_costs_one_cancel_per_vector` and `a_blocked_guest_is_fetched_out_again_at_every_boundary`. They test a mechanism that no longer exists; the property that replaced them — a legacy vector reaching a guest on hardware — is Task 4's test. Name both in the commit message.

- [ ] **Step 3: Let the compiler find the rest**

Run: `cargo check --release -p rusty_box_whp_engine --all-targets`

Delete whatever it reports as unused that belonged to this path (a `PendingExtIntEvent` import, a `Reg::PendingEvent` helper). Do **not** silence a warning with an `#[allow]`; if something is genuinely still needed by a test, gate it `#[cfg(test)]` and say why in its doc, as `ext_int_permitted` and `WhpEngine::controls` already are.

- [ ] **Step 4: Run the suite**

```bash
cargo test --release -p rusty_box_whp_engine
```
Expected: PASS, with two fewer tests than before.

- [ ] **Step 5: Commit**

```bash
git add rusty_box_whp_engine/src/vcpu_thread.rs rusty_box_whp_engine/src/engine.rs
git commit -m "refactor(whp): the vCPU thread no longer stages the legacy interrupt by hand

The machine hands the 8259's vector to the backend APIC now, and the APIC holds
it until the guest can take it. So the staging step, the pending and blocked
flags, and the cancel they drove all go — and with them the last caller that
fetched a processor out of its run for an interrupt.

Two tests are removed with the mechanism they covered:
\`a_pin_reported_at_every_boundary_costs_one_cancel_per_vector\` and
\`a_blocked_guest_is_fetched_out_again_at_every_boundary\`. Both asserted the
dedup of a cancel that is no longer issued. The property that replaces them —
a legacy vector reaching a guest running on hardware — is asserted on hardware
in the test this commit's successor rewrites.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: Prove it on hardware, and boot DLX

The acceptance criterion. Task 1.8 of the parent plan could not meet it.

**Files:**
- Modify: `rusty_box_whp_engine/src/lib.rs` (rewrite `a_legacy_8259_vector_reaches_a_hardware_guest_as_a_placed_ext_int`)

**Interfaces:**
- Consumes: `FastMachine::{adopt, step, with_machine, engine_census}` (`fast_machine.rs`). There is no `fast_machine_with_devices` helper — build the machine with `machine_with_devices_on(DeviceClock::HostTime, ..)` (`lib.rs`, `pub(crate)`) and hand it to `FastMachine::adopt`, which takes `Box<Emulator<T, WhpEngine>>` and returns `Result<Self, FastMachineFault>`. The census field is spelled `canceled`, one `l`.
- Produces: nothing.

- [ ] **Step 1: Write the hardware test**

Task 1 retired `a_legacy_8259_vector_reaches_a_hardware_guest_as_a_placed_ext_int`
rather than leaving it red across three commits: it asserts a delivery that
Task 1 disconnects and Task 2 restores, so it could not pass in between. Its
subject is what survives, and you recreate it here. The old body — including
the toy guest that raises IRQ0 and writes `MARK` to the debug port — is at
`git show 0329d76:rusty_box_whp_engine/src/lib.rs`; read it for the guest and
the assertions, not for the harness. Its `ThreadedRun`/`drive_on_a_thread`
helpers were retired with it and are NOT to be restored: `FastMachine` already
owns its vCPU thread.

Name it `a_legacy_8259_vector_reaches_a_hardware_guest` — "as a placed ext int" named the mechanism, and the mechanism has changed while the subject has not. Drive it on a `FastMachine` with the same guest, keep the assertion that the guest's own handler ran (the `MARK` at the debug port), and add the assertion the old path could not make:

```rust
        let census = machine.engine_census();
        assert!(
            census.exits.canceled <= 2,
            "the vector arrives without fetching the processor out of its run — \
             the same guest under the staged path cost 104,563,825 cancels: {:?}",
            census.exits
        );
```

- [ ] **Step 2: Run it**

Run: `cargo test --release -p rusty_box_whp_engine --lib a_legacy_8259_vector -- --nocapture --test-threads=1`
Expected: PASS. If it skips, this host has no hypervisor and the rest of this task cannot be verified — say so and stop rather than reporting success.

- [ ] **Step 3: Boot DLX**

```bash
DLX_WHP_PATIENCE_SECS=60 cargo run --release -p rusty_box_whp_engine --example dlx_whp
```
Expected: **3 of 3 milestones.** Run it three times and record each wall time.

If it reaches its interrupt but still does not boot, that is the separate defect the spec's last section names. Stop, report the new symptom with its census line, and do **not** patch it here.

- [ ] **Step 4: Gate**

```bash
cargo xtask ci > /tmp/gate.log 2>&1
echo "GATE_EXIT=$?"
```
Then read the `ci: N steps passed` line from the log. Expected: 24 steps.

- [ ] **Step 5: Commit**

```bash
git add rusty_box_whp_engine/src/lib.rs
git commit -m "test(whp): a legacy vector reaches a hardware guest without a cancel

The subject is unchanged and the mechanism is not, so the test keeps its
property and loses \`as a placed ext int\` from its name. It gains the assertion
the staged path could not make: the vector arrives without fetching the
processor out of its run. The same guest under that path cost 104,563,825
cancels and never took the interrupt at all.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: Register the divergence

**Files:**
- Modify: `docs/bochs-parity-divergences.md` (new `H9` entry, appended after `H8`)

- [ ] **Step 1: Write the entry**

Add `## H9 — The 8259's vector is acknowledged when its line asserts, not when the processor can take it`, following the shape every other entry uses: **Bochs:** (cite `pc_system.cc bx_pc_system_c::raise_INTR` and `cpu/event.cc`'s later `DEV_pic_iac()`, and note that `iodev/ioapic.cc service_ioapic` takes the eager branch for a mode-7 entry), **rusty_box in fast mode:**, **Provenance (R7):**, `### What the guest observes`, `### Why the divergence is the correct side`, `### Price of closing it`, `**Status:**`.

The argument to make: Bochs defers when the line travels as a wire and acknowledges eagerly when it travels as a number, and `WHV_INTERRUPT_TYPE` offers no ExtINT and no LocalInt0 — so under a hypervisor APIC the line can only travel as a number. What a guest can observe: a vector taken from the controller even if the guest masks that IRQ before it is delivered. What it cannot: anything at all on the interpreter, which is unchanged.

- [ ] **Step 2: Gate and commit**

```bash
cargo xtask ci > /tmp/gate.log 2>&1
echo "GATE_EXIT=$?"
git add docs/bochs-parity-divergences.md
git commit -m "docs: register H9 — the legacy vector is acknowledged at assert under a backend APIC

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage.** The design's five deletions map to Tasks 1 and 3 (`pic_pin_changed` and `pic_pin_published` in 1; the flags, `raise_ext_int`, `ext_int_request`, `stage_the_legacy_interrupt` and the `PendingEvent` write in 3). The `owns_the_guests_local_apic` question is Task 1. The acknowledge-and-route at `sync_final_event_levels` is Task 2. The three testing requirements are Task 2 (hypervisor-free, both halves) and Task 4 (on hardware, and DLX end to end). The named divergence is Task 5. The "what this does not address" clause is honoured by Task 4 Step 3, which stops rather than patching.

**Placeholder scan.** No TBD or TODO. Every code step carries the code. Task 5 describes a prose document rather than quoting it whole, and names the exact headings and the argument to make, which is the actual content an engineer needs for a divergence entry — the existing entries are the template.

**Type consistency.** `owns_the_guests_local_apic(&self) -> bool` is defined in Task 1 and called in Task 2 with the same name. `IoApicDelivery`'s five fields are used in Task 2 exactly as `iodev/irq.rs` declares them. `DeliveryRoute`'s four variants are matched exhaustively. `IrqFabric::acknowledge` returns `u8` and feeds `IoApicDelivery::vector: u8`. `set_bsp_lint0` takes `u64`, as Task 2's second test passes.

**One risk the plan does not remove.** Task 2's `Model | Undelivered` arm logs and drops a vector that is already spent. That is the honest handling — there is no way to return a vector to the 8259 — but if it ever fires in practice it means `owns_the_guests_local_apic` and `route_ioapic_delivery` disagree, which is a defect in this design rather than in a guest. The log names it precisely so it cannot be mistaken for a guest problem.
