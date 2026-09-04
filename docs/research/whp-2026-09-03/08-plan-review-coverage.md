# Plan review — spec coverage, internal consistency, placeholders

Reviewed read-only: PLAN `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md` (1019 lines) against
SPEC `docs/superpowers/specs/2026-09-03-whp-vmm-shape-design.md`, with symbol claims checked against
the tree at `wip/atom-execctx`. No file under the repo was modified; no build was run.

## 1. Spec coverage (§8's Stage 0 + Stage 1 assignments)

Covered, with the task that implements it:
§3.1 vCPU thread → 1.5; device thread → 1.7; machine lock → 1.5 Step 3; teardown canceller-before-partition
→ 1.7 `Drop`; engine-crate `[lints] workspace = true` → 1.5 Step 3.
§3.2 tick from VmTime → 1.2 + 1.7; `tickn` catch-up (not a single `countdown_event`) → 1.3 Step 3;
pause = VmTime stop + `WHvSuspendPartitionTime` → 1.7 `FastMachine::pause`; `HARDWARE_SPEED` removed → 1.7;
all four divergences (H5 tick-unit, H6 8042 one-shot, H7 transfer TSC rate, H8 EOI resample) → 0.4.
§3.3 X2Apic/XApic → 1.6; `ApicBaseMsrWrite` → 1.6; IOAPIC backend → 1.3 + 1.6; EOI resample → 1.6;
ExtINT window protocol → 1.6; LVT0 tracking → 1.6; SMI trap → 1.5/1.6; halt per P3 → 1.5;
dead `LOCAL_APIC_REGION` removed → 1.6 Step 3.
§3.4 (Stage-1 parts only) interpreter finishes trapped instructions → 1.5; externalised mask → 1.4.
§3.7 probes P1–P8 → 0.3 Steps 1–8 (each probe has a step); failure handling → 1.5 + 1.7;
G1/G5/G6 → 1.9.

- [blocking] Task 1.6, Interfaces "Produces (engine)" — `pic_pin_changed` calls `self.controls[0].raise_ext_int()`, but **no task gives `WhpEngine` a `controls` field**. `VcpuControl` is produced by 1.5 and the threads are spawned by 1.7's `FastMachine::adopt`; nothing plumbs the controls back into the engine value that lives inside the `Emulator`. Without it the whole legacy 8259 path (spec §3.3's ExtINT bullet, §8's "the ExtINT window protocol") has no producer. — Fix: add an explicit step in 1.7 (`adopt` installs the spawned controls into `engine_mut().controls`) and state the field in 1.5's or 1.6's Produces block, including what `pic_pin_changed` does before any thread exists.
- [major] Spec §3.2 "A guest read of the PIT, PM timer or HPET on a vCPU thread computes the wheel's position from VmTime at that instant, so counters no longer freeze for a slice" — **GAP**. No task advances the wheel (or answers a counter read) from VmTime at a port/MMIO exit; the wheel only moves on the device thread's wake (1.7). A guest polling the PM timer or PIT counter between two device-thread wakes sees it frozen, which is the coupling §2(3) names as a root cause. — Fix: add a step to 1.7 (or 1.5's service path) that catches the wheel up to `clock.now()` under the lock before dispatching a timer-device port read, and a test asserting a PM-timer read advances between two reads with no wheel deadline in between.
- [major] Spec §3.7 hypervisor-free test list — "the ExtINT readiness predicate from header bits" has **no test**. 1.6's hypervisor-free tests are the EOI resample and the LVT0 gate only; the IF/`InterruptShadow`/`InterruptionPending` decision (the freshness contract's whole point) is exercised only through gated guests. — Fix: 1.6 Step 1 gains a pure test over a `VpContext`-shaped input asserting permitted/not-permitted for each of the four header states.
- [major] Spec §3.7 hypervisor-present test list — "level-triggered EOI resample" is listed as a hypervisor-present test; the plan has only the hypervisor-free `irq.rs` unit test (1.6 Step 1). No gated test proves the platform actually raises `X64ApicEoi` for a level vector and that the re-issued request lands. — Fix: name it in 1.6 Step 1 or record in 1.9 that the property is unproven on hardware until Stage 2.
- [minor] Spec §3.4 "Unfinishable exits … a bounded interpreter stretch and re-entry; a recurrence at the same RIP surfaces as an engine fault" — 1.5 makes `UnsupportedFeature`/`InvalidVpRegisterValue`/`UnrecoverableException` an immediate `Fault(Unserviced)` with no stretch, because the stretch is Stage 2's clustering. Defensible, but the Stage 1 boot gate then aborts on the first such exit. — Fix: say so explicitly in 1.5 (one sentence: "in Stage 1 these are faults; Stage 2's clustering supplies the stretch").
- [minor] Spec §3.3 "the machine-profile knob hides the whole bank, in which case CPUID.1:ECX[31] is authored by us" — deferred to Stage 2 with the enlightenments, consistent with §8. No action.

## 2. Type / signature consistency across tasks

- [blocking] Task 1.4 Step 3 — `BxCpuC::import_arch_groups(&mut self, state: &VcpuArchState, groups: Externalised)` is to be added in `rusty_box/src/cpu/api_bridge.rs`, but `Externalised` is created in `rusty_box_whp_engine/src/exchange.rs` (1.4 Files). `rusty_box_whp_engine` already depends on `rusty_box`; naming an engine-crate type in a `rusty_box` signature is a **dependency cycle** and also breaks REPLAN #12 (kept by spec §3.1). — Fix: either define the group set in `rusty_box`/`rusty_box_core` beside `VcpuArchState` (`rusty_box/src/cpu/arch_state.rs:324`) and re-export/alias it in the engine, or make `import_arch_groups` take a `rusty_box`-owned `ArchStateGroups` that `Externalised` converts into.
- [blocking] Task 1.4 — declared `Exchange::import_for(&mut self, vcpu: &Vcpu, …)` / `export_imported(&mut self, vcpu: &Vcpu, …)` (concrete `Vcpu` from 1.1), yet every test in the same task passes `&Recorder` (`let vp = Recorder::default(); ex.import_for(&vp, …)`). The tree already has the right seam — `pub(crate) trait VpRegisters` at `rusty_box_whp_engine/src/state.rs:138`, implemented by `Vp<'_>` (`lib.rs:91`) and by `Recorder` (`state.rs:476`). As written the tests do not compile. — Fix: declare `import_for<V: VpRegisters, T: Instrumentation>(&mut self, vp: &V, …)` (generic, R8-clean) and say that `Vcpu` gains a `VpRegisters` impl in 1.1; also say what happens to the existing `Vp<'_>` adapter.
- [major] Task 1.4 — `Recorder` lives inside `state.rs`'s `#[cfg(test)] mod tests` (private). 1.4's tests are in a **different module** (`exchange.rs`). The plan says "keep the `Recorder` test fake and extend it" but never says to move it to a shared `#[cfg(test)]` location or make it `pub(crate)`. — Fix: add a step: move `Recorder` to a `#[cfg(test)] pub(crate) mod test_vp;` (or `state::tests` → `state::test_vp`) and re-export.
- [major] Task 1.5 — `Parked::Fault(EngineFault)` with `EngineFault::MachinePoisoned` and `EngineFault::Unserviced { reason }`. In the tree `EngineFault` is a **struct** (`rusty_box_core/src/engine.rs:322`) built by `EngineFault::new(kind: EngineFaultKind, at: &'static str)`; the variants live on `EngineFaultKind` (line 303) and neither `MachinePoisoned` nor `Unserviced` exists there. — Fix: state the two new `EngineFaultKind` variants and construct through `EngineFault::new(kind, "…")` in 1.5's Produces block.
- [major] Task 1.5 `VcpuControl::request_park(&self)` takes no argument, but Task 1.7's device-thread loop calls `vcpu.request_park(GuestPowerOff)` and 1.5's own prose says "the device thread sets `Parked::GuestPowerOff`". No verb sets a non-`Paused` park reason. — Fix: `request_park(&self, reason: Parked)` (or a separate `park_with(reason)`), and fix the 1.5 signature and the 1.5 test that calls `control.request_park()`.
- [major] Task 1.6 gated test calls `control.census().exits_total()`, but `census: SliceCensus` is a field of `VcpuThread` (1.5), not of `VcpuControl`, and `VcpuControl` has no `census` method in 1.5's Produces block. `exits_total()` is likewise undeclared. — Fix: put a shared `Arc<…>` census on `VcpuControl` in 1.5 and declare `census()`/`exits_total()` there.
- [major] Task 1.7 — `VmClockSource<StdClock>` (in the spawn signature and `FastMachine`'s field). **`StdClock` does not exist** anywhere in the workspace, and Task 1.2 produces only `VmTime`/`VmClockSource` over `rusty_box_core::time::{HostClock, ManualClock, ClockHz}`. — Fix: 1.2 also produces the `HostClock` impl the engine uses; name it and say which crate it lands in (`rusty_box_core/src/time.rs` is std-gated already).
- [major] Task 1.3 — `service_device_time(&mut self, elapsed_ticks: u64) -> Result<Option<u64>>` returns **ticks**, but 1.7's device-thread test binds it through `service_once(&mut machine, &clock)` (declared in prose as `-> Option<VmTime>`) and then does `.unwrap()` twice before `.nanos()`. Three shapes for one value. — Fix: fix `service_once`'s declared return to `Result<Option<VmTime>>` and name where the ticks→VmTime conversion happens.
- [major] Task 1.3 Step 3 — `service_device_time` = `service_scheduler_boundary(elapsed)` plus the deadline. `service_scheduler_boundary` returns `CpuResult<bool>` (`scheduler.rs:972`); the plan's wrapper **discards the bool** (CLAUDE.md: never discard a meaningful return value). Separately, guest power-off is not that bool — it is `stop_cause = StopCause::GuestPowerOff` + `stop_flag`, set inside the boundary's port-0x8900 and ACPI-S5 arms — so 1.7's "if the machine raised GuestPowerOff" has no named channel. — Fix: return a named struct carrying the next deadline AND whether a stop was requested; say in 1.7 that the device thread reads `stop_cause`.
- [major] Task 1.3 `processor_and_parts(&mut self, index) -> (&mut BxCpuC<T>, PcIo<'_>)` and Task 1.7 `engine_census(&self) -> (ExitCounts, SliceCensus, InjectCensus, PlatformCounters)` are **tuples in public APIs**, forbidden by the plan's own Global Constraints (R0). The 1.7 test then indexes it (`engine_census().3`). — Fix: named structs.
- [major] Task 0.2 Step 2 — the APIC-page test asserts `page.0[0x80 << 4 - 4 + 4 ..][..4]`. In Rust `<<` binds looser than `+`/`-`, so this is `0x80 << 4` = **2048**, while the test's own comment computes byte offset 8 × 16 = **128**. It also compares a `[u8]` slice against a `[u8; 4]`. As written it neither compiles nor tests the stated layout. — Fix: `assert_eq!(&page.0[0x80..0x84], &0xF0u32.to_le_bytes());`.
- [major] Task 0.2 — `ApicStatePage::zeroed()` is used in Step 2's and Step 5's tests, but the Produces block declares only the tuple-struct, `register` and `set_register`. — Fix: declare `pub fn zeroed() -> Self`.
- [minor] Task 1.6 Step 3 — mode selection written as "`X2Apic` if `capabilities().features.local_apic_emulation`, else `XApic`" inverts the spec: §3.3 falls back to XApic *where the capability names only XApic*, and a host with no LAPIC emulation must not silently get `XApic`. — Fix: match on the capability's own value, R5-exhaustive, with a named refusal for "none".
- [minor] Task 0.2 vs 1.1 — 0.2 produces the four `Partition::{read,write}_apic_state/_words128(index, …)` verbs; 1.1 moves them onto `Vcpu` and drops them from `Partition`. Intended, but 1.1's "Modify every caller" list omits `rusty_box_whp/src/partition.rs`'s own gated tests, which 0.2 Step 5 just wrote against the `Partition` form. — Fix: name partition.rs's tests in 1.1's caller list.
- [minor] Task 1.7 — `engine_census(&self)` reads `Vcpu::runtime_counters`, a VP-state call, from the front-end thread, while spec §3.1 makes the vCPU thread the only thread that touches VP state (`WHV_E_INVALID_VP_STATE`). `with_machine` is marked paused-only; `engine_census` is not. — Fix: mark it paused-only, or serve it from a shared atomic snapshot.
- [minor] Task 1.7 — `step` returns `Result<BatchOutcome>` but its refusal is `Err(EngineRefusal::NoInstructionCount)`, an error type named nowhere else (the same task's `run_slice` correctly uses `CpuError::UnsupportedCpuOperation { operation }`, which matches `rusty_box/src/cpu/error.rs:49`). — Fix: name the error type once.
- [minor] Naming split noted, not a defect: `hypervisor_present()` (`rusty_box_whp/src/caps.rs:187`) in 0.2/1.1 vs `hypervisor_here()` (`rusty_box_whp_engine/src/lib.rs:195`) in 1.5–1.7. Both exist, in different crates; do not "unify" them.

## 3. Placeholder scan

No `TBD`/`TODO`/`FIXME`/"implement later"/"similar to Task N" anywhere. The real placeholders are
undefined test helpers and one body-less test.

- [major] Task 1.7 Step 1 — `a_guest_that_halts_stays_parked_in_the_hypervisor_and_wakes_for_a_device_interrupt()` has **no body**, only a comment describing what to assert. It is the plan's only test of spec §3.7's "a halted guest woken by a device-thread `WHvRequestInterrupt`" and of §3.3's halt rule — the behaviour P1 exists to de-risk. — Fix: write it, reusing the PIT guest from `a_device_interrupt_reaches_a_hardware_guest_by_injection` as the comment says.
- [major] Task 1.1 Step 1 — `a_real_mode_partition_running(&[0xF4])` is annotated "the file's fixture that maps a page of code at 0x1000". **It does not exist**: `rusty_box_whp/src/partition.rs` has no such fn; its gated tests each build a partition inline. — Fix: "write this fixture, factored out of `the_platform_counts_the_guests_port_writes_apart_from_its_halts` (partition.rs:1204)".
- [major] Task 1.5 Step 1 — `peek_output()` does not exist; the tree has `DebugPort::take_output(&mut self) -> DebugconDrain<'_>` (`rusty_box/src/emulator_api.rs:104`), which **drains**. A `wait_until(|| … peek_output().contains(&MARK))` poll written over `take_output` consumes the byte it waits for. — Fix: add a non-draining `peek_output` (say so), or accumulate into a local buffer.
- [major] Task 1.5 Step 1 — `VcpuControl::for_tests()` is described as "canceller that records calls", but `VcpuControl` holds a concrete `Canceller` over a `RawPartition`; there is no way to build a recording one, and `cancels_recorded()` is undeclared. — Fix: make the canceller a named seam (R2 enum or a generic) and declare both helpers in 1.5's Produces block.
- [minor] Undefined but explicitly instructed (acceptable): `program_ioapic_entry`, `furnished_machine` (1.3 Step 1 "if absent, write them in tests.rs"); `ManualClock::at_nanos/advance_nanos/instant_at_nanos` (1.2 Step 1 "add them to `rusty_box_core/src/time.rs`"); `a_turn_on_the_hardware` in `rusty_box_whp` (0.2 Step 5 — and it does exist, partition.rs:1041); `IOAPIC_EDGE_GUEST` (1.6 Step 1 "hand-assemble … the way `whp_probe.rs` annotates its byte strings").
- [minor] Undefined and NOT instructed: `test_cpu()`, `XsaveArea::for_tests()`, `VpContext::for_tests()`, `InjectState::at_reset()`, `Recorder::total_reads()`, `Recorder::xsave_writes()` (1.4 Step 1 instructs only `reads_of`/`writes_of`); `shared_machine_running`, `wait_until` (1.5); `shared_machine_with_devices`, `IrqFabric::new`, `fabric.program_ioapic_entry(pin, vec, Trigger::Level)` and the `Trigger` type (1.6); `furnished_software_machine`, `machine.keyboard_timer_fires_seen()`, `machine_with_devices`, `service_once` (1.7). — Fix: one line per task naming which are new helpers and where they go.
- [minor] Task 1.6 Step 3 says "re-run the pin's service so a fresh `PendingIoApicDelivery` is queued" without naming the entry point it calls; only the drain side (`take_pending_deliveries`, `ioapic.rs:883`) is named. — Fix: name the queueing function.

## 4. Ordering — does a task consume what a later task produces?

Backward dependencies are otherwise clean: 1.1 `Vcpu` → 1.4/1.5/1.6/1.7; 1.2 `VmTime` → 1.7;
1.3 `IoApicDelivery`/`DeliveryRoute`/`processor_and_parts`/`service_device_time` → 1.5/1.6/1.7;
1.4 `Exchange` → 1.5; 1.5 `VcpuControl`/`Parked` → 1.6/1.7; 1.7 `FastMachine::adopt` → 1.8.
P1's answer (0.3) is consumed by 1.6 and P3's by 1.5 — both after Stage 0. Two forward references are
handled explicitly and correctly: 1.5's `ApicEoi` arm ("until then this arm calls today's no-op
`receive_eoi`") and 1.5's pre-run hook ("Task 1.6's ExtINT staging").

- [blocking] **Stage 1 header contradicts itself and the tree is not bootable between 1.6 and 1.7.**
  Line 426: "the tree boots (on the old engine) after every task until Task 1.7 removes the old engine,
  and boots on the new one from Task 1.6" — both cannot hold at 1.6, and neither does. Task 1.6 switches
  `start()` to `LocalApicMode::X2Apic` and makes `route_ioapic_delivery` return `Backend` **while the
  slice engine is still the driver** (`FastMachine` and the device thread arrive in 1.7). Under X2Apic a
  halted VP does not leave `WHvRunVirtualProcessor` — the exact reason REPLAN v4 #6 chose `None`
  (spec §4, quoting `engine.rs:870-875`) — so the slice loop strands the wheel on the run thread; and
  with deliveries routed to a backend the model LAPIC the slice engine reads is no longer the source of
  truth. Result: the WHP engine boots nothing at the 1.6 commit, so 1.6's own gated tests
  (`an_ioapic_edge_is_delivered_by_the_hypervisor_apic_without_an_exit`) and `cargo xtask ci` at 1.6's
  Step 5 cannot pass. — Fix: either merge 1.6 and 1.7 into one atom (the mode switch, the backend and
  the device thread land together), or gate the mode/backend behind the `device_clock: HostTime`
  config so the slice engine keeps `None` + `Model` until 1.7 flips it.
- [minor] Task 1.5's `VcpuThread` consumes `Arc<Mutex<Box<Emulator<T, WhpEngine>>>>`, which nothing
  before 1.7 constructs; its gated test papers over this with the undefined `shared_machine_running`.
  Not a hard ordering break (the Arc can be built in the test), but it is the same seam 1.7 owns. —
  Fix: state in 1.5 that the shared-machine constructor is a test-only helper 1.7 replaces.
- [minor] Task 1.4's `import_arch_groups` lands in `rusty_box`, so Task 1.4 also modifies `rusty_box`
  while its Files list names only engine-crate files plus `api_bridge.rs` in the prose. — Fix: add
  `rusty_box/src/cpu/api_bridge.rs` and `rusty_box/src/cpu/error.rs` to 1.4's Files block, and add the
  `--no-default-features` check the Global Constraints require for `rusty_box/` edits.

## 5. Disposition table vs what the tasks do

Rows that match: T8 reused (1.6), T9/T14 superseded, T11 moot, T13 absorbed (1.4), carry-forward a
(1.6 Step 3's INTA), carry-forward d (1.6), Fix 1 moot (1.7), Fix 2 dropped, Fix 3 moot, Fix 4 Stage 2,
fast-path T5 superseded by `VmTime` (1.2), T6/T7/T8 split across Stage 2 / 1.4 / 1.9, gate-hygiene debt
(0.1 Step 1), examples kept and adapted (1.8), MONITORX / `exception()`-`interrupt()` out of scope.

- [minor] Row "Peer session's zeroed segment attributes" says **"Task 1.4, Step 6"**; the work is
  actually Task 1.4 **Step 5** ("Segment import validates every segment"). Step 6 is "Gates and commit".
  — Fix: correct the row to Step 5.
- [minor] Row "`WHP_ALL_SHADOW` bisection mode — Dropped with the slice engine (Task 1.7)". Task 1.7's
  delete list names `everything_on_the_shadow` but not the `WHP_ALL_SHADOW` env-var read that selects
  it, nor `WHP_MIN_SLICE_US` (spec §2 root cause 1 names both). — Fix: add both env reads to 1.7's
  DELETE list explicitly, so the "compiles clean ≠ reached" trap does not leave dead knobs behind.
- [minor] Row "T10 `ShadowFreshness` witness — Superseded by the externalised mask (Task 1.4)". Task 1.4
  never names `ShadowFreshness`; 1.7's DELETE list names `Started.shadowed` / `ran_since_read_back` but
  not the witness type. — Fix: name the type in one of the two delete lists.
- [minor] Row "T12 … machine writes to VP state removed structurally (only the vCPU thread touches VP
  state, Task 1.5)" — contradicted by 1.7's `FastMachine::engine_census(&self)` (a VP `runtime_counters`
  read off the vCPU thread) and by Stage 3's pause-time import. See §2's `engine_census` finding.

## 6. Gate criteria — is each measurable with an instrument the plan names?

Measurable as written: Stage 0's three gate lines (baseline cells filled, probe doc records P1–P8,
`cargo xtask ci` green); 1.9's DLX 3/3 milestones (`dlx_whp` prints them); 1.9's "zero fatal signatures
in the scraped screen" (`alpine_probe` scrapes); 1.9's `unsafe` token baselines (the `xtask ci` ratchet,
38 / 1); G6's `cargo xtask ci`.

- [major] G1 in 1.9 — "`in_run_share` **over the kernel phase (from the ISOLINUX milestone to
  `login:`)** ≥ 0.90", but the only instrument the plan builds is Task 1.8 Step 1's
  `in_run_share=<hypervisor_ms/wall_ms>`, added to "`alpine_probe`'s 10-second report and final line" —
  i.e. cumulative from process start (which includes BIOS/ISOLINUX) or a rolling 10 s window, never the
  milestone-to-milestone window the gate asks for. **No instrument computes the gated number.** — Fix:
  Task 1.8 Step 1 must latch `HypervisorRuntime100ns` and wall at the ISOLINUX milestone and again at
  `login:`, and print `kernel_phase_in_run_share=`.
- [major] 1.9 — "wall < **the interpreter's median** from the baseline document", but Task 0.1 Step 4
  runs the interpreter **once** ("one run") and the baseline template's interpreter row has only a Run 1
  cell. There is no median to compare against. — Fix: make Step 4 three runs and fill the row, or
  restate the gate against the single measured value.
- [major] Task 1.7's `step_in_ticks_runs_the_guest_for_that_much_vm_time_and_pauses` asserts
  `counters.runtime.hypervisor_100ns() * 10 >= 9 * 10_000 * 10 / 10`, whose right side is 90,000, so the
  assertion is `hypervisor_100ns() >= 9_000` = **0.9 ms**, while the comment claims "≥ 9 ms of
  hypervisor runtime" out of a 10 ms step. The written test passes at 9 % in-run, not 90 % — it would
  green-light exactly the failure the campaign exists to fix. — Fix: `assert!(counters.runtime.hypervisor_100ns() >= 90_000)`.
- [minor] 1.9 — "`Send` assertions present for `Vcpu`, `VcpuControl`, `DeviceThreadControl`,
  `FastMachine<()>`". Only `Vcpu`'s is instructed (1.1 Step 1). Tasks 1.5 and 1.7 never say to add the
  other three. — Fix: add the assertion to each producing task's Step 1.
- [minor] Stage 0 gate line "P1 = wakes (else the plan stops here)" is measurable, and 0.3 Step 1's
  Control A / Control B make the negative result distinguishable from a broken probe — good. No action.
- [minor] G0 (VMware) is user-supplied and the plan says G2/G3 cannot be judged until filled; Stage 1's
  gate does not depend on G0, so this does not block Stage 1. No action.
- [minor] Spec §3.7 G1 also asks for "idle-at-login exits/s in the low thousands or fewer"; §8's Stage 1
  gate quotes only the 90 % half, and 1.9 follows §8. Consistent — noted so it is not lost for Stage 2.
