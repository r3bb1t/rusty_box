# Plan review — CODE ACCURACY lens (HEAD 7728086, wip/atom-execctx)

## 1 paths and lines

Confirmed to exist (Modify/Read targets): docs/superpowers/specs/2026-09-03-whp-vmm-shape-design.md,
rusty_box_whp_engine/examples/{alpine_probe,dlx_whp,alpine_bench,step_bench,compute_bench}.rs,
rusty_box_whp/examples/whp_probe.rs, docs/whp-platform-probe-2026-08-27.md,
docs/bochs-parity-divergences.md, rusty_box_whp/src/{sys.rs,sys/windows.rs,sys/unsupported.rs,vcpu.rs,caps.rs,partition.rs,lib.rs},
rusty_box_whp_engine/src/{engine.rs,state.rs,xsave.rs,alarm.rs,lib.rs},
rusty_box_core/src/time.rs, rusty_box/src/emulator/{engine.rs,mod.rs,scheduler.rs,timers.rs,tests.rs},
rusty_box/src/iodev/{irq.rs,ioapic.rs,keyboard.rs,mod.rs,devices.rs}, rusty_box/src/memory/plan.rs,
rusty_box/src/cpu/{api_bridge.rs,apic.rs,event.rs}, rusty_box/src/pc_system.rs, xtask/src/ci.rs.

Confirmed citations:
- `scheduler.rs:972` service_scheduler_boundary (pub fn, scheduler.rs:972) — correct.
- `scheduler.rs:1220` sync_final_event_levels (private `fn`, scheduler.rs:1220) — correct.
- `scheduler.rs:1142-1166` the tick loop — correct: loop at 1149, `self.pc_system.tickn(step as u32)` at 1166.
- `scheduler.rs:574-597` deliver_lapic_bus_interrupt — correct (fn header 574, body ends 597).
- `mod.rs:634` run_slice destructuring — correct (`pub(crate) fn run_slice` at 634).
- `ioapic.rs:307` PendingIoApicDelivery — correct (struct at 307).
- `irq.rs:170` IrqFabric::mmio_write — correct.
- `run.rs:655` `pub fn step` — correct. `builder.rs:460` `pub fn build_on<E>` — correct.
- `mod.rs:241` `pub struct EmulatorConfig` — correct.
- `emulator_api.rs:960` with_engine, `:138` set_stop_flag, `:796` stop_handle — correct, but the file is
  `rusty_box/src/emulator_api.rs`, NOT under `rusty_box/src/emulator/`.

Findings:
- [minor] plan §Task 1.3 Files — plan says `rusty_box/src/emulator/scheduler.rs` holds `deliver_ioapic_to_lapics` —
  tree agrees, but the fn is at scheduler.rs:714-773 (`fn deliver_ioapic_to_lapics(&mut self, delivery: crate::iodev::ioapic::PendingIoApicDelivery) -> bool`),
  a detail the plan never cites; no fix needed, recorded for the implementer.

More confirmed citations:
- `engine.rs ~344` `struct InjectState` — exact (engine.rs:344, private `struct`, in-crate only).
- `engine.rs ~2780` `fn service_port_access(` — exact (engine.rs:2780).
- `event.rs:661` `acknowledge_external_interrupt` — exact (`pub(crate) fn`, cpu/event.rs:661).
- `lib.rs:631` `a_device_interrupt_reaches_a_hardware_guest_by_injection` — exact.
- `partition.rs ~558` Canceller lifetime rule — the doc comment runs 556-569, `pub fn canceller` at 570. Close enough.
- `sys.rs:104` `pub enum RegisterValue` — exact. `vcpu.rs:14` `pub enum Reg` — exact. `caps.rs:115` `pub struct ExtendedVmExits` — exact.
- `pc_system.rs:692` `pub fn tickn`, `:694-699` the countdown_event loop, `:744-746` the continuous-timer coalescing `while time_to_fire <= ticks_total` — all exact.
- `apic.rs:1972-1981` get_current_timer_count delta math; `:2620-2637` the snapshot "activation disagrees with its programming epoch" check — both exact.
- `plan.rs:45` `const LOCAL_APIC_REGION`, `:115` its `carve_outs.add` — exact.
- `ioapic.rs:756-761` `pub fn receive_eoi` (log-only) — exact. `irq.rs:212` `pub(crate) fn acknowledge` — exact.
- `keyboard.rs:1439` `fn activate_timer(&mut self)`; `:753 :778 :1184 :1213 :1884 :1896` are all `self.activate_timer();` call sites — exact.
- `interactive.rs:39` `pub fn run_interactive` — exact. `xtask/src/ci.rs` baselines `("rusty_box_whp/src", 38)` line 111 and `("rusty_box_whp_engine/src", 1)` line 119 — exact.
- ci step names "WHP leaf tests" (521), "WHP probe builds" (527), "WHP engine tests" (538) — all present.

Findings:
- [minor] Task 1.5 Step 3 — plan says the ExitReason service match is `engine.rs:1991–2138` — the service
  `match exit.reason {` is at engine.rs:2005 and closes at 2143; line 1991 is inside a DIFFERENT match
  (the ExitHistory character classification, engine.rs:1986-2003) — fix: cite 2005–2143.
- [minor] Global Constraints / prompt list — `emulator_api.rs` is `rusty_box/src/emulator_api.rs`, not under
  `rusty_box/src/emulator/`. `with_engine` :960, `set_stop_flag` :138, `stop_handle` :796 are all exact there.

## 2 consumed symbols

Confirmed present with the assumed shape: `Partition::{map, remap, unmap, unmap_subrange, mapped_regions,
create_processor, run, cancel_run, read_regs, write_regs, read_registers, write_registers, read_xsave,
write_xsave, inject, internal_activity, set_internal_activity, request_interrupt, canceller,
interrupt_requester, set_property_late}` (partition.rs:448-895); `Canceller` (906), `InterruptRequester` (936);
`InterruptRequest{kind,destination_mode,trigger_mode,destination,vector}` (vcpu.rs:246);
`InterruptKind::{Fixed,LowestPriority,Nmi,Init,Sipi,LocalInt1}` (203); `InternalActivity{startup_suspend,
halt_suspend,idle_suspend}` (283); `PendingInterruption{kind,vector,error_code}` (175);
`Exit.vp: VpContext{rip,rflags,cs,instruction_length,cr8,execution_state}` (vcpu.rs:414-431) — every field
name the plan's `take_header` test uses is real; `MsrExits.apic_base_write` (caps.rs:49);
`Capabilities{features,supported_exits,physical_address_width,processor_features}` (167) and
`Features.local_apic_emulation` (caps.rs:95, used by Task 1.6's mode selection);
`PartitionConfig::{new,processor_count,extended_vm_exits,msr_exits,exception_exits,cpuid_exit_list,
separate_security_domain,processor_features,local_apic,setup}`; `sys::set_property/get_words/set_words`
(sys.rs); `PcIo` fields all `pub` and `PcIo::{sync_io_events, emulate_one, emulate_batch,
finish_the_instruction, pop_deliverable_vector, needs_boundary, deliver_the_trap_owed}` all `pub`
(io.rs:101-324); `Emulator::step` pub (run.rs:655), `service_scheduler_boundary` pub (scheduler.rs:972),
`dispatch_timer_fires` pub (timers.rs:255), `debug_port`/`set_stop_flag`/`reg_write`/`mem_read_vec`/
`mem_write`/`stop_handle`/`with_engine`/`setup_cpu_mode` pub (emulator_api.rs), `display` pub
(display.rs:242), `save_snapshot`/`restore_snapshot` pub (snapshot.rs:460/490), `MachineBuilder::build_on`
pub (builder.rs:460); `BxCpuC::{discard_decoded_traces, is_in_smm, has_deliverable_ext_int,
has_non_ext_int_event, set_lapic_tpr_from_cr8, export_arch_state, import_arch_state, record_halt}` all `pub`
(cpu/arch_state.rs); `pc_system::next_timer_deadline_at` pub (1316); `TimerOwner::Keyboard` (pc_system.rs:101),
`DeviceTimerOwner::Keyboard` (timers.rs:687), `TimerRequest::Activate{..}` (iodev/mod.rs:193),
`request_timer_after_usec` (iodev/mod.rs:528); `BxKeyboardC::{activate_timer(1439, private),
timer_callback(1463, pub(crate)), periodic(1926, pub), timer_handle(554)}` and
`kbd_controller.{timer_pending(396), irq1_requested(398), irq12_requested(400)}` all `pub(crate)` — fine,
those callers are in-crate; engine-crate test helpers `CODE`(120) `DEBUG_PORT`(124) `MARK`(125)
`machine_running`(129) `machine_with_devices`(150) `trace_of`(171) `hypervisor_here`(195)
`a_turn_on_the_hardware`(215); probe example `Finding`(50) `Guest`(139) `run_with_rescue`(885)
`wake_attempt`(919) `install_handler`(996); `a_turn_on_the_hardware` DOES exist in rusty_box_whp
(partition.rs:1041), so Task 0.2 Step 5's parenthetical resolves to "already there";
`rusty_box_whp_engine/Cargo.toml` has no `[lints]` (Task 1.5's addition is correct) and the crate's one
`unsafe` lift is `#![expect(unsafe_code, …)]` at lib.rs:45 discharged in `map_window` (engine.rs:1084) —
the plan's baseline story is accurate; workspace pins `bitflags = "2"` (Cargo.toml:23).
