# The Shadow Takes The System-Management Interrupt — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A chipset SMI reaches a guest running on the hypervisor, so `rombios32`'s `smm_init` completes and BIOS-booted guests get past `0xe0438`.

**Architecture:** The SMM sequence already exists and works on the `ApicSmiTrap` exit path. This plan factors it into one helper, adds a public predicate so the engine can ask whether the shadow owes an SMI, and asks that question at the tail of `service` — where the machine lock is already held, so it costs no extra locking on a path that runs 70 million times.

**Tech Stack:** Rust, `rusty_box` (machine), `rusty_box_whp_engine` (WHP engine), Windows Hypervisor Platform.

**Spec:** `docs/superpowers/specs/2026-09-08-the-shadow-takes-the-system-management-interrupt-design.md`

## Global Constraints

- Read `CLAUDE.md` first and follow it. **Edit with the Edit/Write tools, never shell heredocs, `sed -i`, or Python**; release builds only; **never run `cargo fmt`**; never `let _ = <Result>`; comments state today's invariant, never history; **Bochs citations name file + symbol, never line numbers**.
- **Never use the LSP tools** (`mcp__lsp__references`/`definition`/`diagnostics`) — they hang indefinitely in this workspace. The compiler is the oracle. rust-analyzer here also emits stale false-positive errors; reproduce every diagnostic with a real cargo command.
- Verification: `cargo check --release -p rusty_box --features std --lib`, **plus** `cargo check --release -p rusty_box --no-default-features` for anything under `cpu/` or `emulator/`.
- `cargo xtask ci` before every commit. **Never pipe it and never append to its line** — a pipeline's exit code masks the gate's. Redirect to a log, read it for `ci: N steps passed`, and grep it for `FAILED`.
- **Never stage** `ROADMAP.md`, `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md`, or the three untracked `docs/superpowers/specs/2026-08-22-*.md` files — someone else's work. Explicit `git add <path>` only; never `git add -A`. **Leave `stash@{0}` alone.**
- Commit messages end with: `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
- Branch: `wip/atom-execctx`. Do not create branches.
- **Known-flaky under host load:** `fast_machine::tests::step_in_ticks_runs_the_guest_for_that_much_vm_time_and_pauses`, the halt-errand test, and two siblings. Reproduce with the FULL `-p rusty_box_whp_engine --lib` suite — a single-test run filters out the others and is not a fair reproduction. Any other failure is yours.
- Other sessions share this repo; `cargo` may print `Blocking waiting for file lock`. Check `powershell -NoProfile -Command "Get-Process cargo | Select Id,StartTime,CPU"` — near-zero CPU means blocked, not compiling. **Never kill a process you did not start.**

---

### Task 1: The machine can be asked whether a processor owes an SMI

`BX_EVENT_SMI` and `pending_event` are both `pub(crate)`, so `rusty_box_whp_engine` cannot ask today. One public predicate closes that, and nothing else in this task changes behaviour.

**Files:**
- Modify: `rusty_box/src/cpu/event.rs` (add the predicate beside `deliver_smi`)
- Test: `rusty_box/src/cpu/event.rs` or the crate's existing CPU test module — put the test wherever `deliver_smi`'s neighbours are tested; if there is no such module, add the test in `rusty_box/src/emulator/tests.rs` using `TestMachine`.

**Interfaces:**
- Consumes: `BxCpuC::pending_event` (`cpu/cpu.rs`, `pub(crate)`), `BxCpuC::BX_EVENT_SMI` (`cpu/cpu.rs`, `pub(crate)`), `BxCpuC::deliver_smi` (`cpu/event.rs`, `pub(crate)`).
- Produces: `BxCpuC::owes_a_system_management_interrupt(&self) -> bool`, public. Task 2 calls it.

- [ ] **Step 1: Write the failing test**

The property is guest-visible in the sense that matters here: an SMI signalled and not yet taken must be reported, and one that has been taken must not be. Add:

```rust
    /// A processor reports an SMI it has been signalled and has not yet taken.
    ///
    /// The one question an engine that runs the guest on a hypervisor must ask.
    /// `deliver_smi` sets a bit in this processor's own event word, and a guest
    /// executing inside a partition never consults it — so without this the
    /// signal is raised and nothing ever acts on it.
    #[test]
    fn a_processor_reports_a_system_management_interrupt_it_has_not_taken() {
        let mut machine = furnished_machine();
        let cpu = machine.cpu_mut_at(0);

        assert!(
            !cpu.owes_a_system_management_interrupt(),
            "a processor that has been signalled nothing owes nothing"
        );

        cpu.deliver_smi();
        assert!(
            cpu.owes_a_system_management_interrupt(),
            "a signalled SMI is owed until it is taken"
        );

        cpu.clear_event(BxCpuC::<()>::BX_EVENT_SMI);
        assert!(
            !cpu.owes_a_system_management_interrupt(),
            "and taking it settles the debt"
        );
    }
```

If `furnished_machine` is not in scope in the module you choose, use whichever
constructor that module already uses for a `BxCpuC`; the assertions are what
matter, not the fixture.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --release -p rusty_box --lib --features std a_processor_reports_a_system_management`
Expected: FAIL to compile — `no method named owes_a_system_management_interrupt`.

- [ ] **Step 3: Add the predicate**

In `rusty_box/src/cpu/event.rs`, in the same `impl` block as `deliver_smi`:

```rust
    /// Whether this processor has been signalled a system-management interrupt
    /// it has not yet taken.
    ///
    /// The question an engine that runs the guest somewhere else must ask.
    /// [`Self::deliver_smi`] sets a bit in this processor's event word, which
    /// the interpreter consults in `handle_async_event` — Bochs
    /// `cpu/event.cc handleAsyncEvent`, where an SMI is a "Priority 3: External
    /// Hardware Intervention" tested ahead of `INTR`. A guest executing inside
    /// a hypervisor partition consults nothing here, so its engine has to.
    #[must_use]
    pub fn owes_a_system_management_interrupt(&self) -> bool {
        (self.pending_event & Self::BX_EVENT_SMI) != 0
    }
```

- [ ] **Step 4: Run it to verify it passes**

```bash
cargo test --release -p rusty_box --lib --features std a_processor_reports_a_system_management
cargo check --release -p rusty_box --no-default-features
```
Expected: the test PASSES; the no-default build is clean (this file is compiled in both configurations).

- [ ] **Step 5: Gate and commit**

```bash
cargo xtask ci > /tmp/gate-smi-predicate.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-smi-predicate.log
grep -c FAILED /tmp/gate-smi-predicate.log
```

```bash
git add rusty_box/src/cpu/event.rs
git commit -m "feat(cpu): a processor can be asked whether it owes an SMI

deliver_smi sets a bit in the processor's own event word. The interpreter reads
it in handle_async_event; a guest executing inside a hypervisor partition reads
nothing, so its engine must be able to ask. Both the bit and the word are
pub(crate), so an engine crate cannot see either.

No behaviour changes: this only makes an existing state observable.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

(If Step 1's test landed in `emulator/tests.rs`, stage that file too.)

---

### Task 2: The shadow takes a chipset SMI

The behavioural change. The SMM sequence is factored once and asked for at the tail of `service`, where the machine lock is already held.

**Files:**
- Modify: `rusty_box_whp_engine/src/vcpu_thread.rs` (factor the sequence; call it from `ApicSmiTrap` and from the tail of `service`)

**Interfaces:**
- Consumes: `BxCpuC::owes_a_system_management_interrupt` (Task 1); `Exchange::{import_everything, export_imported}`, `PcIo::{deliver_smi, emulate_one, sync_io_events}`, `run_the_shadow_out_of_smm` (`engine.rs`) — all already used by the `ApicSmiTrap` arm.
- Produces: `take_the_signalled_smi`, a private free function in `vcpu_thread.rs`.

- [ ] **Step 0: Correct the predicate Task 1 landed — it ignores the event mask**

Task 1 shipped `owes_a_system_management_interrupt` as
`(self.pending_event & Self::BX_EVENT_SMI) != 0`. That is a Bochs inaccuracy and
this task would build on it.

Bochs masks `BX_EVENT_SMI` on SMM **entry** and unmasks it on `RSM`
(`cpu/smm.cc`, and this port mirrors it in `cpu/smm.rs`), and
`cpu/event.cc handleAsyncEvent` therefore tests the event with
`is_unmasked_event_pending`, not a bare `pending_event` read. A predicate that
ignores the mask claims an SMI is owed by a processor already inside
system-management mode — a nested entry Bochs forbids.

In `rusty_box/src/cpu/event.rs`, change the body to the masked form and say why:

```rust
    #[must_use]
    pub fn owes_a_system_management_interrupt(&self) -> bool {
        // Masked, not a bare pending read: SMM entry masks this event and `RSM`
        // unmasks it (`cpu/smm.cc`), so a processor already inside
        // system-management mode owes nothing — Bochs
        // `cpu/event.cc handleAsyncEvent` tests it the same way.
        self.is_unmasked_event_pending(Self::BX_EVENT_SMI)
    }
```

Extend Task 1's test in `rusty_box/src/emulator/tests.rs` with the case that
pins it, keeping the existing assertions:

```rust
        cpu.deliver_smi();
        cpu.mask_event(BxCpuC::<()>::BX_EVENT_SMI);
        assert!(
            !cpu.owes_a_system_management_interrupt(),
            "a processor inside system-management mode owes no further SMI: Bochs \
             masks the event on entry and unmasks it at RSM"
        );
        cpu.unmask_event(BxCpuC::<()>::BX_EVENT_SMI);
        assert!(
            cpu.owes_a_system_management_interrupt(),
            "and it is owed again once the mode is left"
        );
```

Confirm `mask_event`/`unmask_event` exist with those names in `cpu/cpu.rs`
before using them; if they differ, use whatever `cpu/smm.rs` calls at its entry
and `RSM` sites, which are the two places that actually move this bit.

Run `cargo test --release -p rusty_box --lib --features std a_processor_reports_a_system_management`
and confirm the new assertions fail before the body change and pass after.

Commit this separately from the rest of the task — it is a correction to
committed code and belongs in its own commit:

```bash
git add rusty_box/src/cpu/event.rs rusty_box/src/emulator/tests.rs
git commit -m "fix(cpu): an owed SMI respects the event mask, as Bochs tests it

SMM entry masks BX_EVENT_SMI and RSM unmasks it (cpu/smm.cc, mirrored in
cpu/smm.rs), so cpu/event.cc handleAsyncEvent tests the event through
is_unmasked_event_pending. The predicate read pending_event bare, and so claimed
an SMI was owed by a processor already inside system-management mode.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

- [ ] **Step 1: Write the failing test**

In `rusty_box_whp_engine/src/lib.rs`'s test module, beside the other hardware tests. This is the property `rombios32` waits on, expressed directly: a guest that raises a chipset SMI has its handler run.

```rust
    /// A guest that raises a chipset SMI has its handler run.
    ///
    /// `rombios32.c smm_init` writes the APM command port and then spins on the
    /// status port until its own relocation handler clears it:
    /// `outb(0xb3, 0x01); outb(0xb2, 0x00); while (inb(0xb3) != 0x00);`
    /// The write reaches `acpi.generate_smi`, the machine's boundary signals the
    /// shadow, and nothing on this path would ever act on that signal without
    /// the ask at the tail of `service`. Every BIOS-booted guest stops here
    /// without it.
    ///
    /// The guest below is that loop with a handler that clears the port, so it
    /// terminates only if the SMI is genuinely taken.
    #[test]
    fn a_chipset_smi_reaches_a_hardware_guests_handler() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let machine = machine_with_devices_on(
            DeviceClock::HostTime,
            &[
                0x31, 0xC0, //             xor ax, ax
                0x8E, 0xD8, //             mov ds, ax
                0x8E, 0xD0, //             mov ss, ax
                0xBC, 0x00, 0x70, //       mov sp, 0x7000
                // Install an SMM handler at SMBASE+0x8000 = 0x38000
                // (`cpu/init.rs`: SMBASE is 0x30000 at reset). The handler is
                // `mov al,0; out 0xb3,al; rsm` — it clears the status port the
                // poll below waits on, which is the whole property under test.
                0xFC, //                   cld
                0xB8, 0x00, 0x38, //       mov ax, 0x3800
                0x8E, 0xC0, //             mov es, ax
                0x31, 0xFF, //             xor di, di
                0xB0, 0xB0, 0xAA, //       mov al,0xB0; stosb   ┐ mov al, 0
                0xB0, 0x00, 0xAA, //       mov al,0x00; stosb   ┘
                0xB0, 0xE6, 0xAA, //       mov al,0xE6; stosb   ┐ out 0xb3, al
                0xB0, 0xB3, 0xAA, //       mov al,0xB3; stosb   ┘
                0xB0, 0x0F, 0xAA, //       mov al,0x0F; stosb   ┐ rsm
                0xB0, 0xAA, 0xAA, //       mov al,0xAA; stosb   ┘
                // Enable APMC. `acpi.rs generate_smi` raises nothing unless
                // `pci_conf[0x5b]` bit 1 is set, and the BIOS sets it through
                // PCI config space — the ACPI function is `devfunc: 0x0B`,
                // i.e. BX_PCI_DEVICE(1, 3), so the config address for register
                // 0x58 is 0x80000B58 and byte 3 of that dword is 0x5b.
                // WITHOUT THIS the `out 0xb2` below raises no SMI at all and
                // this test would fail for the wrong reason.
                0xBA, 0xF8, 0x0C, //       mov dx, 0x0CF8
                0x66, 0xB8, 0x58, 0x0B, 0x00, 0x80, // mov eax, 0x80000B58
                0x66, 0xEF, //             out dx, eax
                0xBA, 0xFF, 0x0C, //       mov dx, 0x0CFF   (= 0xCFC + 3 → reg 0x5b)
                0xB0, 0x02, //             mov al, 0x02     (APMC_EN)
                0xEE, //                   out dx, al
                // Arm the status port, then raise the chipset SMI.
                0xB0, 0x01, //             mov al, 1
                0xE6, 0xB3, //             out 0xb3, al
                0xB0, 0x00, //             mov al, 0
                0xE6, 0xB2, //             out 0xb2, al
                // wait: poll until the handler clears it
                0xE4, 0xB3, //             in al, 0xb3
                0x84, 0xC0, //             test al, al
                0x75, 0xFA, //             jnz wait
                // cleared: say so and stop
                0xB0, MARK, //             mov al, MARK
                0xE6, DEBUG_PORT, //       out 0xE9, al
                0xF4, //                   hlt
            ],
        );
        let ThreadedRun { written, .. } = drive_on_a_thread(
            machine,
            std::time::Duration::from_secs(10),
            "the guest's SMM handler cleared the status port",
            |_, seen| !seen.is_empty(),
        );
        assert_eq!(
            written,
            std::vec![MARK],
            "the guest must leave its poll loop, which only its SMM handler can \
             end: {written:#04x?}"
        );
    }
```

**Verify the two addresses before trusting a green run**, because either being wrong makes the test fail for the wrong reason and look like a defect in your implementation:

- **SMBASE is `0x30000`** (`rusty_box/src/cpu/init.rs`, and asserted by `cpu/smm.rs`'s `hardware-reset SMBASE` test), so the entry point is `0x38000` and segment `0x3800` is right. If a later change moves SMBASE, this guest must move with it.
- **APMC_EN is `pci_conf[0x5b]` bit 1** (`iodev/acpi.rs generate_smi`), and `acpi.rs`'s own tests `no SMI without APMC_EN` / `APMC_EN set: SMI delivered` are the pair that pins that behaviour. The ACPI function is `devfunc: 0x0B`.

If the test fails at its deadline with an empty debug port, distinguish the two causes before touching your implementation:

- `acpi.smi_request_pending` **never becomes true** → the PCI enable did not land, so no SMI was ever raised. The guest's prologue is wrong, not your code.
- it **becomes true but the handler never runs** → the SMI was raised and not taken. That is the defect this task fixes, and before Step 3 it is the expected red.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --release -p rusty_box_whp_engine --lib a_chipset_smi_reaches -- --test-threads=1`
Expected: FAIL at the 10 s deadline with an empty debug port — the guest never leaves its poll loop, exactly as DLX does not.

- [ ] **Step 3: Factor the sequence**

In `rusty_box_whp_engine/src/vcpu_thread.rs`, add a private free function beside the other helpers:

```rust
/// Take a system-management interrupt the shadow has ALREADY been signalled,
/// and hand the result back to the partition.
///
/// The processor is imported first because system-management mode saves the
/// state it finds: a shadow holding anything but the guest's current registers
/// would save the wrong ones, and `RSM` would restore them. The handler runs to
/// completion here — a processor cannot be returned to the hardware half-way
/// into a mode the hardware has no equivalent for.
fn take_the_signalled_smi<T: Instrumentation>(
    vcpu: &Vcpu,
    exchange: &mut Exchange,
    xsave: &mut XsaveArea,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
) -> Result<usize> {
    exchange.import_everything(vcpu, cpu, xsave)?;
    // Signalled, not yet taken: the event is processed when the processor next
    // runs, exactly as Bochs decides it.
    io.emulate_one(cpu)?;
    io.sync_io_events(cpu);
    run_the_shadow_out_of_smm(cpu, io)?;
    exchange.export_imported(vcpu, cpu, xsave)
}
```

Then rewrite the `ExitReason::ApicSmiTrap` arm to use it. That arm signals the SMI itself, so it keeps its `deliver_smi` and gains nothing else — the import moves inside the helper:

```rust
            ExitReason::ApicSmiTrap => {
                io.deliver_smi(cpu);
                let calls = take_the_signalled_smi(vcpu, exchange, xsave, cpu, &mut io)?;
                self.control
                    .census
                    .export_calls
                    .fetch_add(u64::try_from(calls).unwrap_or(u64::MAX), Ordering::Release);
                Ok(Continue::Run)
            }
```

**Careful with the borrow:** `service` destructures `self` (`let Self { vcpu, index, machine, clock, exchange, xsave, inject, control } = self;`), so inside it use the destructured `control`, not `self.control`. Match whatever the surrounding arm already does.

- [ ] **Step 4: Ask the question at the tail of `Servicer::answer`**

**Not in `service` — in `Servicer::answer`.** `Servicer` already holds `vcpu`, `exchange` and `xsave`, and `answer` already receives `cpu` and `io`, so every piece the sequence needs is in scope there with no restructuring. `service`'s own tail would have none of them: it consumes `servicer` in the `match` that produces its return value.

`answer` currently ends with one `match exit.reason { … }` whose arms each yield `Ok(...)`. Bind that to a local, ask, then return it:

```rust
        let carry_on = match exit.reason {
            // … every existing arm, unchanged …
        }?;
        // A chipset SMI is signalled onto this shadow by the machine's boundary
        // (`emulator/scheduler.rs` drains `acpi.smi_request_pending`), and a
        // guest running inside the partition never consults that word — so this
        // is the one place anything acts on it. Asked here, where the machine's
        // lock is already held and the shadow is already in hand, because a
        // guest can take tens of millions of exits and a lock taken per entry
        // to read one bit would be paid on every one of them.
        //
        // Only on the way back to the guest: a processor that is parking has a
        // fault or a power-off to report, and system-management mode is not
        // somewhere to send it on the way out.
        //
        // Bochs orders an SMI ahead of the 8259's line (`cpu/event.cc
        // handleAsyncEvent`, "Priority 3: External Hardware Interventions"),
        // and taking it here — before the entry stages a legacy vector — is
        // that order.
        if matches!(carry_on, Continue::Run) && cpu.owes_a_system_management_interrupt() {
            let calls = take_the_signalled_smi(self.vcpu, self.exchange, self.xsave, cpu, io)?;
            self.control
                .census
                .export_calls
                .fetch_add(u64::try_from(calls).unwrap_or(u64::MAX), Ordering::Release);
        }
        Ok(carry_on)
```

The `matches!(carry_on, Continue::Run)` guard is load-bearing, not defensive: without it a fault or a guest power-off would be followed by an SMM entry on a processor that is on its way to parking.

Note the arms hold `self.exchange` etc. as `&mut` fields of `Servicer`, so inside `answer` write `self.vcpu` / `self.exchange` / `self.xsave` — not the bare destructured names used in `service`.

```rust
        // A chipset SMI is signalled onto this shadow by the machine's boundary
        // (`emulator/scheduler.rs` drains `acpi.smi_request_pending`), and a
        // guest running inside the partition never consults that word — so this
        // is the one place anything acts on it. Asked here rather than before
        // an entry because the lock is already held: a guest can take tens of
        // millions of exits, and a lock acquisition per entry to read one bit
        // would be paid on every one of them.
        //
        // Bochs orders an SMI ahead of the 8259's line
        // (`cpu/event.cc handleAsyncEvent`, "Priority 3: External Hardware
        // Interventions"), and taking it here — before the entry stages a
        // legacy vector — is that order.
        if cpu.owes_a_system_management_interrupt() {
            let calls = take_the_signalled_smi(vcpu, exchange, xsave, cpu, &mut io)?;
            control
                .census
                .export_calls
                .fetch_add(u64::try_from(calls).unwrap_or(u64::MAX), Ordering::Release);
        }
```

If `service`'s structure makes a single tail placement awkward (for example if arms return early), restructure so the outcome is computed into a local and the check runs before it is returned. **Do not duplicate the check into several arms** — one site, per R5.

- [ ] **Step 5: Run the tests**

```bash
cargo test --release -p rusty_box_whp_engine --lib a_chipset_smi_reaches -- --test-threads=1
cargo test --release -p rusty_box_whp_engine --lib
cargo check --release -p rusty_box --no-default-features
```
Expected: the new test PASSES; the full suite is green; the no-default build is clean.

- [ ] **Step 6: Prove the ask is what carries it**

Comment out the `if matches!(carry_on, Continue::Run) && cpu.owes_a_system_management_interrupt() { … }` block you added at the tail of `Servicer::answer` and re-run the new test.

Expected: **FAIL** at the deadline with an empty debug port — the guest is back in the poll loop that no handler ends. Restore it and re-run to confirm PASS.

Report both outcomes verbatim. This is the only evidence the new code is what makes the test pass: the test is gated on `hypervisor_here()` with a bare `return`, so a host without a hypervisor gives an identical-looking green in the same ~0.03 s, and every hardware claim in this plan is worthless without a mutation behind it.

- [ ] **Step 7: Gate and commit**

```bash
cargo xtask ci > /tmp/gate-smi.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-smi.log
grep -c FAILED /tmp/gate-smi.log
```

```bash
git add rusty_box_whp_engine/src/vcpu_thread.rs rusty_box_whp_engine/src/lib.rs
git commit -m "fix(whp): the shadow takes a chipset SMI, so rombios32 gets past smm_init

A chipset SMI ends as a bit in the shadow's event word: the ACPI controller sets
smi_request_pending on an 0xb2 write, the machine's boundary drains it with
deliver_smi, and nothing on the hypervisor path ever read that word. rombios32's
smm_init raises exactly that SMI and spins on port 0xb3 until its relocation
handler clears it, so every BIOS-booted guest stopped there.

The sequence that answers it already existed on the ApicSmiTrap exit path. It is
factored once and asked for at the tail of service, where the machine's lock is
already held — a guest can take tens of millions of exits, and a lock per entry
to read one bit would be paid on every one.

This is applepie's model: it handles all interrupts in Bochs emulation itself,
which is what gives it SMIs the platform does not support.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: Boot DLX and Alpine, and say what actually happened

The acceptance task. It writes no production code.

**Files:**
- None modified. This task measures and reports.

**Interfaces:**
- Consumes: `cargo run --release -p rusty_box_whp_engine --example dlx_whp` and `--example alpine_bench`. The Alpine ISO is at the repo root as `alpine-virt-3.24.1-x86_64.iso`, which is the filename `alpine_bench` defaults to — no `ALPINE_ISO` is needed.

- [ ] **Step 1: Boot DLX, three runs**

```bash
cargo run --release -p rusty_box_whp_engine --example dlx_whp
```

Record for each run, verbatim: the milestone count, which milestones landed, the cancel count, the port-exit count, and the in-run share.

The first milestone, "the BIOS is alive", is the one this plan targets: it could not land while the BIOS sat in `smm_init`. **Report the real number.** If it is still 0/3, say so and give the RIP the guest now sits at — that is the next wall, and naming it is this task's most valuable output. Do not adjust the milestones and do not patch a second defect here.

- [ ] **Step 2: Boot Alpine on both engines**

```bash
cargo run --release -p rusty_box_whp_engine --example alpine_bench
```

Record both columns — interpreter and hypervisor — with the host time to each of the three milestones (`BIOS`, `ISOLINUX`, `login:`).

**The bar for this work:** Alpine reaches `login:` on the hypervisor **faster than the interpreter**, whose measured time is **76.4 s**. VMware parity is explicitly deferred and is not this task's bar.

- [ ] **Step 3: Report, and stop**

Write the numbers into the report. Then stop.

If Alpine reaches `login:` but is slower than the interpreter, that is a
performance problem and it gets its own investigation with a profile — do not
tune anything here. If it does not reach `login:`, name the milestone that failed
and the RIP it sits at.

Either outcome is a successful task. The failure mode to avoid is a number that
cannot be trusted: do not re-run until a good one appears, and report the spread
if runs disagree.

- [ ] **Step 4: Commit the measurements**

No source changes, so there is nothing to gate beyond what Task 2 already gated. Record the numbers in the task report only; do not commit a document unless the results change a claim in the spec, in which case amend the spec and say which claim moved.

---

## Self-Review

**1. Spec coverage.** The spec's public predicate is Task 1. Its factored `take_the_signalled_smi` and the tail-of-`service` ask are Task 2. Its four testing requirements map to: the hypervisor-free predicate test (Task 1 Step 1), the on-hardware SMM test (Task 2 Step 1), DLX end to end (Task 3 Step 1) and Alpine against the interpreter (Task 3 Step 2). The spec's "does not promise Alpine boots" is honoured by Task 3 Step 3, which treats naming the next wall as success. The spec's banked `EMULATE_STEPS` idea is deliberately in no task — it is for later work with a profile.

**2. Placeholder scan.** No TBD/TODO. Every code step carries its code. The one instruction to *investigate* rather than transcribe — checking `APMC_EN` before writing the guest — is explicit about what to look for, which existing test shows it, and what going wrong looks like (a vacuous pass).

**3. Type consistency.** `owes_a_system_management_interrupt(&self) -> bool` is defined in Task 1 and called in Task 2 Step 4 with that exact name. `take_the_signalled_smi` takes `(&Vcpu, &mut Exchange, &mut XsaveArea, &mut BxCpuC<T>, &mut PcIo<'_>) -> Result<usize>` and both callers pass that shape; its `usize` feeds `u64::try_from(...)` at both sites, matching what the existing `ApicSmiTrap` arm already does with `export_imported`'s return.

**4. Risks this plan does not remove.**

- ~~`service`'s shape may not admit a single tail check.~~ **Resolved before dispatch.** `service` ends in one `match servicer.answer(…)` and consumes `servicer` doing it, so its tail has none of the pieces; `Servicer::answer` ends in one `match exit.reason` and already holds `vcpu`/`exchange`/`xsave` with `cpu`/`io` in hand. Step 4 now names `answer` and needs no control-flow restructuring — only binding that match to a local. The `matches!(carry_on, Continue::Run)` guard is the one piece of judgement it adds.
- **The chipset-SMI latency is one poll iteration**, because the flag reaches the shadow only when the device thread drains it. `rombios32` polls tightly so this is microseconds, but a guest that raises an SMI and then runs exit-free would not be served at all. No such guest is known and none is in scope; recorded rather than hidden.
- **Task 3 may find another wall.** That is expected, not a plan defect — Alpine has never booted on `FastMachine`.
