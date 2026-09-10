# Plan review — Rust feasibility, ownership, concurrency, doctrine

Target: `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md` (Stage 1, Tasks 1.1–1.9).
Read-only review; no build, no test, no edit.

## 1 — Is `Emulator<T, WhpEngine>: Send`, and does the existing assertion cover it?

**Send holds, structurally, but nothing asserts it and nothing would notice if it stopped.**

- `rusty_box/src/emulator/mod.rs:2188-2191` asserts `assert_send::<Emulator<()>>()`. `Emulator`'s
  parameter list is `Emulator<T: Instrumentation = (), E = SoftwareEngine>` (mod.rs:451), so
  `Emulator<()>` is `Emulator<(), SoftwareEngine>`. **The assertion covers `SoftwareEngine` only.**
  mod.rs:2196-2199 generalises over `T`, not over `E`. Neither `rusty_box_whp_engine` nor
  `rusty_box_whp` asserts anything about the engine-parameterised machine (`grep assert_send` in
  `rusty_box_whp_engine/src` returns nothing; `rusty_box_whp/src/lib.rs:69-76` asserts
  `Partition`/`PartitionConfig`/`HostPages` Send and `Canceller`/`InterruptRequester` Send+Sync).
- Field-by-field the property does hold today:
  `WhpEngine { started: Option<Started>, exits, census, inject_census, history }`
  (engine.rs:756-766); `Started { alarm: Alarm, partition: Partition, state: VcpuArchState,
  installed: MemoryPlan, shadowed, held_interrupt_state, held, ran_since_read_back,
  consecutive_mmio, xsave: XsaveArea, inject }` (engine.rs:475-567).
  `Alarm { Arc<Shared>, Option<JoinHandle<()>> }` (alarm.rs:73-77) — `Shared` is
  `Mutex<State> + Condvar + AtomicU64`, all Send+Sync.
  `Partition { OwnedPartition(RawPartition), Vec<Region>, Vec<u32> }` (partition.rs:422-426);
  `Region { u64, GpaPerms, HostPages }` (partition.rs:126-130); `HostPages { Vec<u8>, usize,
  usize }` (partition.rs:68-75). `RawPartition` is asserted Send+Sync at `sys.rs:234-235`.
  `XsaveArea { Box<[u8]>, usize, HostComponents }` (xsave.rs:236-245). `MemoryPlan` is
  `[GpaWindow; N] + len` (plan.rs:79-82).
  The `gui` field is `Option<Box<dyn BxGui>>` and `BxGui: Send + Sync` (gui/gui_trait.rs:28), so
  it does not break the property.
- `T: Instrumentation` already implies `'static` (`cpu/instrumentation/bochs.rs:84:
  pub trait Instrumentation: Default + 'static`), so the plan's `T: Instrumentation + Send +
  'static` bound is correct but the `'static` half is redundant. `Send` is *not* implied and is
  correctly added.
- Gate item 1.9 asks for Send assertions on `Vcpu`, `VcpuControl`, `DeviceThreadControl`,
  `FastMachine<()>` — but **not** on `Emulator<(), WhpEngine>`, which is the type actually going
  into the `Arc<Mutex<…>>`. `FastMachine<()>: Send` implies it transitively only if
  `FastMachine` is asserted Send *and* holds the Arc, which it does; still, the direct assertion
  is the one that names the failure.

- [minor] Task 1.5 Interfaces / Task 1.9 gate — no assertion covers `Emulator<(), WhpEngine>`;
  mod.rs:2190 pins `SoftwareEngine` only, so a future `WhpEngine` field that is `!Send` (a raw
  pointer, an `Rc`, a `*mut` VP context) breaks only at the `Arc::new` call site with a
  many-line trait error, not at a named assertion — fix: add
  `const _: () = { const fn a<M: Send>() {} a::<Emulator<(), WhpEngine>>(); };` to
  `rusty_box_whp_engine/src/lib.rs` in Task 1.5 and list it in the 1.9 gate beside the others.
- [minor] Global Constraints — the plan never states that `T: Instrumentation` already carries
  `'static`; three signatures repeat `+ 'static`. Harmless, but it hides that the real new bound
  is `Send` — fix: say so once, keep `+ Send` only.

## 2 — `Vcpu` as a `Copy` token used off the machine lock

**Sound at the WHP level; unsound as a Rust ownership design, and it dissolves the spec's own
"only one thread touches VP state" invariant.**

- Platform level: mapping while a VP runs is what QEMU's whpx does, and `Partition::map`
  (partition.rs:448-456) / `map_borrowed` take `&mut self` — the `Partition` stays inside the
  locked `Emulator`, so a map is serialised against exit servicing by the machine lock. A VP
  running concurrently with `WHvMapGpaRange` is the reference behaviour. No objection.
- Rust level: `Vcpu { handle: RawPartition, index: u32 }` derives `Copy` and (via `RawPartition`,
  `sys.rs:235`) `Send + Sync`. The plan gives it the whole per-VP verb set including
  `read_regs/write_regs/read_registers/write_registers/read_xsave/write_xsave/inject/
  set_internal_activity` (plan lines 449-469). Spec §3.1 says the vCPU thread is "the only
  thread that reads or writes VP registers or VP state (`WHV_E_INVALID_VP_STATE` exists for
  accesses from elsewhere)". With `Vcpu: Copy + Send + Sync` and a public
  `Partition::vcpu(&self, index)`, **nothing in the type system holds that invariant** — it
  becomes a convention. This is exactly what R2 ("states are types") and R5 ("one choke point per
  hazard") exist to prevent, and the crate already demonstrates the right shape: `Canceller`
  (partition.rs:906) and `InterruptRequester` (partition.rs:936) are the *narrow* copyable
  tokens precisely because only those two verbs are safe off-thread, and their doc comments say
  so.
- Lifetime: `Partition::vcpu(&self, index) -> WhpResult<Vcpu>` hands out a token with no
  lifetime tie. Today `Canceller`'s obligation is discharged by the field order of whatever owns
  both (`Started` at engine.rs:470-477 declares `alarm` before `partition` and says so). Task
  1.1's doc repeats that argument, but the plan's own test at line 823 writes
  `VcpuThread::spawn(machine.vcpu(0), 0, machine.clone())` where `machine` is the shared
  Arc-Mutex — i.e. a `Vcpu` is minted outside the partition's own borrow and handed to a thread.
  `let v = fm.vcpu(0); drop(fm); v.run();` is then a use-after-free of a reclaimed handle,
  compiling cleanly.
- `FastMachine::drop` (plan line 935) *is* sufficient for the threads it owns: it pauses, stops
  the device thread, joins every handle, and only then drops `shared` — but only if the join is
  in the `Drop` body (field order alone will not do it: `shared` is declared first in the struct
  at line 927, so plain field-order dropping releases the machine's `Arc` before the
  `vcpus`/`devices` handles).
- **Panic in a vCPU thread:** the workspace sets `[profile.release] panic = "abort"`
  (root `Cargo.toml`). Under the project's mandatory release builds a panic on any of these
  threads **aborts the process**; the partition is reclaimed by process teardown. Consequently
  spec §3.7's "a device panic on the device thread is a machine fault, not a thread death: the
  lock is poisoned and the front end sees the fault" is **false for every build the project
  ships**, and Task 1.5's `Parked::Fault(EngineFault::MachinePoisoned)` is unreachable outside
  `cargo test` (Cargo forces `panic=unwind` for the test/bench profiles).

- [blocking] Task 1.1 Interfaces (plan 439-471) — `Vcpu` carries every VP-state verb while being
  `Copy + Send + Sync` and obtainable from `&Partition`, so the spec's "only the vCPU thread
  touches VP state" invariant has no type behind it and a `Vcpu` can outlive its partition —
  fix: make `Vcpu` a non-`Copy`, non-`Sync` owned handle moved into the vCPU thread (the register
  verbs live there), and keep the off-thread verbs on the existing `Canceller` /
  `InterruptRequester` tokens plus a new `VpCounters` token for the census; make
  `Partition::vcpu` consuming-once (`take_vcpu(&mut self, index)`) so two threads cannot mint the
  same one.
- [major] Spec §3.7 / Task 1.5 Step 3 — the poisoned-lock fault path cannot occur under
  `panic = "abort"` (root `Cargo.toml`), so the design's stated failure handling for a device
  panic is not implemented by it — fix: either state that a panic on a machine thread is a
  process abort by design (and delete `EngineFault::MachinePoisoned`), or follow the crate's
  existing precedent in `alarm.rs:139-145`, which *recovers* poison with
  `poisoned.into_inner()` rather than propagating.
- [minor] Task 1.7 `FastMachine` (plan 927, 935) — `shared` is declared before the thread
  handles, so drop-by-field-order would release the machine before the joins; the ordering must
  be in the `Drop` body — fix: say so in the step, mirroring `Started`'s
  "FIELD ORDER IS LOAD-BEARING" comment at engine.rs:470-474, and add a comment that here it is
  the body, not the order, that discharges it.

## 3 — `processor_and_parts` and R3

**Writable as stated; the borrow is fine; the R3 objection does not apply, but R0 does.**

- The disjoint borrow is exactly `run_slice`'s existing destructure (mod.rs:634-652) minus the
  `engine` field, and `exec_ctx` (mod.rs:609-624) already proves the shape:
  `MachineCpus::get_mut(&mut self, index) -> &mut BxCpuC<T>` (cpu_store.rs:34, impls at 73 and
  109) borrows only the store, so `memory`, `devices`, `device_manager`, `pc_system` stay
  independently live. `processor_and_parts` is a strict subset of what compiles today.
- R3 is satisfied, not violated. `PcIo`'s doc (io.rs:46-58) states the rule as
  "the parts are assembled by a machine destructuring its own `&mut self`, and by nothing else",
  enforced by the private `assembled_by_a_machine: ()` field and `PcIo::new` being `pub(crate)`
  (io.rs:369-370). `PcIo` is already public (`pub use io::PcIo`, mod.rs:52, while `mod io` is
  `pub(crate)`, mod.rs:51) and its fields are already `pub` by design, because the external
  `rusty_box_whp_engine` reaches the machine through them today (engine.rs:2803-2816, 1133).
  A public `processor_and_parts` is the same loan `run_slice` already makes, reachable by the
  caller instead of only by the trait. **This is what R3 permits.**
- R0 does bite: `-> (&mut BxCpuC<T>, PcIo<'_>)` is a tuple in a public API. So is
  `engine_census(&self) -> (ExitCounts, SliceCensus, InjectCensus, PlatformCounters)` — and the
  plan's own test indexes it positionally (`machine.engine_census().3`, plan line 965), the exact
  failure mode R0 names. Spec §5 even says "the transfer result, the probe results, the census
  are structs, no tuples".

- [major] Task 1.7 (plan 933, 965) — `engine_census` returns a 4-tuple read by position, against
  R0 and against the spec's own §5 sentence — fix: `pub struct EngineCensus { exits, slices,
  injections, platform }` and return that.
- [minor] Task 1.3 (plan 605) — `processor_and_parts` returns a pair; R0 asks for a name —
  fix: `pub struct Processor<'a, T> { pub cpu: &'a mut BxCpuC<T>, pub io: PcIo<'a> }`, or record
  the exemption. (`ExecCtx::new(cpu, io)` already takes the two positionally, so a named struct
  also documents which is which at the call sites.)
- [minor] Task 1.3 (plan 614) — `service_device_time(elapsed_ticks: u64) -> Result<Option<u64>>`
  passes and returns bare `u64` ticks against R4 while the same task introduces `VmTime` for
  exactly that reason — fix: take and return the tree's `Ticks` newtype.

## 4 — `route_ioapic_delivery` and where the `InterruptRequester` lives

**The hook shape is fine; the implementation sketch loses interrupts before `start()` and drops a
`Result`.**

- The trait addition is legal: `SliceEngine`'s doc (engine.rs:125-131) says it is unsealed and
  "gains only defaulted methods from here on"; both new methods are defaulted, and
  `SoftwareEngine` (engine.rs:198) and the test engine are untouched. Clean.
- The requester is **not** a `WhpEngine` field. It is minted from a live partition:
  `Partition::interrupt_requester(&self)` (partition.rs:581-583); the partition lives in
  `Started` (engine.rs:477), which is `Option` (engine.rs:757) and is created by
  `start(started, io)` (engine.rs:851) — a free function needing a `PcIo` to derive the memory
  plan, so it can only run from inside a slice/exit. The plan's sketch (line 843) writes
  `self.requester.request(…)` as though the field were flat and always present.
- **Before `start()` there is no partition.** `sync_final_event_levels` runs on the machine's own
  reset and restore paths (`sync_restored_event_levels`, mod.rs:696; and every
  `service_scheduler_boundary`) and drains `take_pending_deliveries` (scheduler.rs:1236-1258);
  those calls happen before any run. The sketch returns `DeliveryRoute::Backend`
  unconditionally, and `Backend` means "the machine does nothing" (plan 592-593), so a delivery
  routed then is **silently lost**. The `Model` fallback is required and the plan never says so.
- `InterruptRequester::request` returns `WhpResult<()>` (partition.rs:946); the sketch discards
  it. CLAUDE.md: "Never `let _ = call_returning_result()`", and `DeliveryRoute` has no failure
  variant to carry it.
- With `run_slice` deleted in Task 1.7, **nothing calls `start()` any more**, and Task 1.7 never
  says `FastMachine::adopt` starts the partition — yet `adopt` must, being the only place left
  that holds both the machine and a `PcIo`.

- [blocking] Task 1.6 Interfaces (plan 843) — `route_ioapic_delivery` returns `Backend`
  unconditionally, so an IOAPIC delivery raised before the partition exists (reset, snapshot
  restore via `sync_restored_event_levels`, mod.rs:696) is dropped, and the `WhpResult` from
  `InterruptRequester::request` (partition.rs:946) is discarded — fix: read
  `self.started.as_ref()`; `None` → `DeliveryRoute::Model`; a platform refusal → a third
  `DeliveryRoute::Refused(WhpError)` variant (R5 exhaustive), never a discarded `Result`.
- [blocking] Task 1.7 Step 3 (plan 975) — `WhpEngine::start()` (engine.rs:851) is reachable only
  through `run_slice`, which this step deletes; `FastMachine::adopt` is never said to start the
  partition, so an adopted machine has `started == None` and every thread it spawns has no VP —
  fix: name it: `adopt` takes `processor_and_parts(0)`, calls `start`, mints the
  `Vcpu`/`Canceller`/`InterruptRequester`, and only then spawns.

## 5 — Deadlock trace: device thread, vCPU thread, front end

**No deadlock on the cancel path (it is a wait). Four real hazards elsewhere.**

1. **Device → vCPU.** The device thread holds the machine lock inside `service_device_time` →
   `service_scheduler_boundary` → `sync_final_event_levels` (scheduler.rs:1220) →
   `pic_pin_changed` → `raise_ext_int` → `Canceller::cancel` (partition.rs:917-919 =
   `WHvCancelRunVirtualProcessor`, non-blocking, never waits for the VP). The vCPU thread returns
   from `run()` — correctly called with the lock RELEASED (plan 831) — and blocks on
   `machine.lock()` until the device thread finishes. **A wait, not a deadlock.** Correct.
2. **`FastMachine::pause`** (plan 930): `request_park` → `wait_parked` → `devices.pause()` →
   `clock.stop()` → `partition.suspend_time()`. That last verb needs the `Partition` inside the
   *locked* `Emulator`. A naive implementation that takes the machine lock at the top and holds
   it across `wait_parked()` deadlocks: the vCPU thread cannot service the exit that would let it
   park. The plan does not say the lock is taken only afterwards.
3. **`FastMachine::step(Ticks(n))`** (plan 931) "waits on the device thread's condvar". That
   condvar (`wake: Arc<(Mutex<DeviceWake>, Condvar)>`, plan 921) is notified only by
   `deadline_moved_earlier`/`pause`/`resume`/`stop` — the device thread itself never notifies it.
   **The sentinel has no producer** (CLAUDE.md: "A background wait must have a sentinel that can
   actually occur"). And if `step` held the machine lock while waiting, the device thread could
   never advance the clock at all.
4. **Two mutexes, no order.** Machine (`Arc<Mutex<Box<Emulator>>>`) and clock
   (`Arc<Mutex<VmClockSource<StdClock>>>`, plan 927). Device loop: clock alone in the wait phase,
   then machine→clock in the service phase (plan 924). `pause`: clock→machine
   (`clock.stop(); partition.suspend_time()`). Spec §3.2's "a guest read of the PIT, PM timer or
   HPET on a vCPU thread computes the wheel's position from VmTime at that instant" cements
   machine→clock, which `pause` then inverts.
5. **The cancel storm.** `sync_final_event_levels` publishes a **level**, not an edge: it runs on
   every commit and unconditionally does `if asserted { signal_event } else { clear_event }`
   (scheduler.rs:1220-1231). Task 1.3 hooks `pic_pin_changed(level)` there; Task 1.6 makes it
   `if asserted { raise_ext_int() }` (plan 843), which cancels the vCPU. A PIC pin stays asserted
   from raise until the guest's INTA, so **every device-thread wake while a legacy interrupt is
   unacknowledged cancels the running vCPU** — thousands of forced exits per second, straight
   against the ≥ 90 % in-run gate (G1) this stage is judged on. The method name promises an edge;
   the call site supplies a level.

- [blocking] Task 1.3 Step 3 + Task 1.6 Interfaces (plan 645, 843) — `pic_pin_changed` is wired
  to a level republished on every boundary and its `WhpEngine` body cancels the vCPU — fix:
  publish an edge (keep the last level in the machine, call the hook only on a transition) AND
  make `raise_ext_int` cancel only on a `false → true` `compare_exchange` of `ext_int_pending`.
  Both, not one.
- [blocking] Task 1.7 (plan 931) — `step(Ticks(n))` waits on a condvar nothing notifies — fix:
  the device thread `notify_all`s after every `service_device_time`; `step` uses a
  `wait_timeout` loop re-reading `clock.now()`; state that no machine lock is held across it.
- [major] Task 1.7 (plan 924, 927-930) — two mutexes with no documented acquisition order, and
  the plan's own `pause` uses the opposite order from the device loop — fix: state the order
  (machine → clock, since the vCPU exit path must read the clock under the machine lock per spec
  §3.2) and make `pause` obey it, or fold the clock into the machine so there is one lock.
- [major] Task 1.7 (plan 930) — `pause` must not hold the machine lock across `wait_parked()` —
  fix: say so; take the lock only for `partition.suspend_time()` after every vCPU has parked.
