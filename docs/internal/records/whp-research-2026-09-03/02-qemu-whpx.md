# QEMU WHPX accelerator: exact design (read from source, 2026-09-02)

Sources read in full (raw from github.com/qemu/qemu):
- **Classic** = tag `v10.2.0`: `target/i386/whpx/{whpx-all.c (2791 l), whpx-apic.c, whpx-accel-ops.c, whpx-internal.h}`. Uses WinHvEmulation.dll.
- **Master** = `11.1.50` (VERSION file), after Mohamed Mediouni's Feb-Aug 2026 rework (72 commits): `accel/whpx/{whpx-common.c, whpx-accel-ops.c}`, `include/system/whpx-{internal,common,all,accel-ops}.h`, `target/i386/whpx/{whpx-all.c (3372 l), whpx-apic.c, whpx-cpu-legacy.c}`. WinHvEmulation is gone; `target/i386/emulate` is used.
- Glue: `system/cpus.c`, `system/cpu-timers.c`, `util/qemu-timer.c`, `hw/core/cpu-common.c`, `hw/i386/x86-cpu.c`, `hw/i386/x86-common.c`, `hw/intc/{ioapic,apic,apic_common,i8259}.c`, `target/i386/cpu-apic.c`, `system/physmem.c`, `docs/system/whpx.rst`, 59 commit messages via GitHub API, 7 cover letters via patchew.org.
Where classic and master differ, both are stated. Line refs are to the files as downloaded.

## 1. Threading model

**One thread per vCPU, main loop owns devices+timers.** `accel/whpx/whpx-accel-ops.c:whpx_cpu_thread_fn` (identical in v10.2.0):
```c
    bql_lock();  ... r = whpx_init_vcpu(cpu); ...
    do {
        qemu_process_cpu_events(cpu);          /* sleeps on halt_cond while cpu_thread_is_idle() */
        if (cpu_can_run(cpu)) {
            r = whpx_vcpu_exec(cpu);           /* loops whpx_vcpu_run() until exception_index >= EXCP_INTERRUPT */
            if (r == EXCP_DEBUG) cpu_handle_guest_debug(cpu);
        }
    } while (!cpu->unplug || cpu_can_run(cpu));
```
The thread is created holding the BQL and *nominally* holds it everywhere except inside the run loop. `whpx_vcpu_run` (`whpx-all.c`): `g_assert(bql_locked());` ... `whpx_vcpu_process_async_events(cpu);` ... `bql_unlock();` ... `cpu_exec_start(cpu);` then the `do { ... WHvRunVirtualProcessor ... } while (!ret);` loop runs **BQL-free**; on loop exit `cpu_exec_end(cpu); bql_lock(); current_cpu = cpu;`. Exit handlers re-take the BQL only where device state is touched: `whpx_handle_halt` (`bql_lock()` around the halted decision), `whpx_vcpu_pre_run` (`bql_lock()` around all `cpu_test_interrupt`/`cpu_get_pic_interrupt` bookkeeping, released before the `WHvSetVirtualProcessorRegisters` call), `whpx_vcpu_post_run` (BQL only if CR8/TPR changed), APIC MSR read/write on master. MMIO/PIO handlers call `address_space_rw` without the BQL; `system/physmem.c:prepare_mmio_access` takes it per access: `if (!bql_locked() && !mr->lockless_io) { bql_lock(); release_lock = true; }`. So a device access from a vCPU thread costs one BQL acquire/release, and the iothread's timer callbacks (which run under the BQL) serialize against it.

**Kick = WHvCancelRunVirtualProcessor.** `whpx_kick_vcpu_thread(cpu) { if (!qemu_cpu_is_self(cpu)) whpx_vcpu_kick(cpu); }`, `whpx_vcpu_kick` -> `whp_dispatch.WHvCancelRunVirtualProcessor(whpx->partition, cpu->cpu_index, 0)`. Registered as `ops->kick_vcpu_thread`; `ops->handle_interrupt = generic_handle_interrupt`. Generic layer (`system/cpus.c`):
```c
void generic_handle_interrupt(CPUState *cpu, int mask) { cpu_set_interrupt(cpu, mask); if (!qemu_cpu_is_self(cpu)) qemu_cpu_kick(cpu); }
void cpu_interrupt(CPUState *cpu, int mask) { g_assert(bql_locked()); cpus_accel->handle_interrupt(cpu, mask); }
void qemu_cpu_kick(CPUState *cpu) { qemu_cond_broadcast(cpu->halt_cond); if (cpus_accel->kick_vcpu_thread) cpus_accel->kick_vcpu_thread(cpu); else cpus_kick_thread(cpu); }
/* hw/core/cpu-common.c */ void cpu_exit(CPUState *cpu) { qatomic_store_release(&cpu->exit_request, true); qemu_cpu_kick(cpu); }
```
The cancelled VP returns `WHvRunVpExitReasonCanceled` -> `cpu->exception_index = EXCP_INTERRUPT; ret = 1;` -> back to `whpx_cpu_thread_fn` -> `qemu_process_cpu_events` (`process_queued_cpu_work`, stop requests) -> `whpx_vcpu_exec` again. The run loop also self-kicks: after `pre_run`, `if (qatomic_load_acquire(&cpu->exit_request)) whpx_vcpu_kick(cpu);` so a pending request makes the *next* `WHvRunVirtualProcessor` return immediately (a cancel issued before Run is latched by WHP).

**When a kick actually happens** (exhaustive, from the callers of `cpu_interrupt`/`cpu_exit`/`run_on_cpu`):
- Any `cpu_interrupt(cpu, mask)` from a non-vCPU thread: `CPU_INTERRUPT_HARD` from the PIC (`hw/i386/x86-cpu.c:pic_irq_request`) in *both* irqchip modes; `CPU_INTERRUPT_POLL` from the userspace APIC (`hw/intc/apic.c:apic_update_irq`: `if (!qemu_cpu_is_self(cpu)) cpu_interrupt(cpu, CPU_INTERRUPT_POLL);`) only with kernel-irqchip=off; NMI/SMI/INIT/SIPI from the userspace APIC (`apic_bus_deliver`, `apic_local_deliver`, `apic_startup`) only with kernel-irqchip=off. With kernel-irqchip=on, device interrupts never call `cpu_interrupt` (see Q2a), so a running guest is never cancelled for a device IRQ.
- `cpu_exit` paths: `cpu_pause`/`pause_all_vcpus` (vm_stop, reset, migration), `qemu_cpu_stop`, gdb.
- `run_on_cpu` (`cpu_synchronize_state`, `whpx_apic_put` for APIC reset/post_load) -> `qemu_cpu_kick` -> cancel.
- There is **no TLB-flush kick**: WHPX has no shadow TLB in QEMU; there is no `tlb_flush` hook.
- Timers never kick a vCPU (Q6/7): `system/cpu-timers.c:qemu_timer_notify_cb`: `if (!icount_enabled() || type != QEMU_CLOCK_VIRTUAL) { qemu_notify_event(); return; }` -- with WHPX icount is off, so a timer re-arm only wakes the main loop's poll.

## 2. Interrupt delivery in both modes

Mode selection: `whpx_accel_instance_init`: `whpx->kernel_irqchip_allowed = true;` ("Turn on kernel-irqchip, by default"). `whpx_accel_init` (v10.2.0:2629): `if (whpx->kernel_irqchip_allowed && features.LocalApicEmulation && whp_dispatch.WHvSetVirtualProcessorInterruptControllerState2) { mode = WHvX64LocalApicEmulationModeXApic; WHvSetPartitionProperty(..., WHvPartitionPropertyCodeLocalApicEmulationMode, ...); ... whpx->apic_in_platform = true; }`. Master uses `WHvX64LocalApicEmulationModeX2Apic` and adds `!(whpx_is_legacy_os() && pic_enabled && !whpx->kernel_irqchip_required)` (Windows 10 + PIC present -> user-mode LAPIC by default; docs: "a legacy PIC interrupt injected does not wake the guest from an HLT when using the Hyper-V provided interrupt controller"). `split` is rejected: `error_setg(errp, "WHPX: split irqchip currently not supported")`. APIC device type: `target/i386/cpu-apic.c:apic_get_class`: `else if (whpx_irqchip_in_kernel()) apic_type = "whpx-apic";`.

### 2a. kernel-irqchip=on (`apic_in_platform` / `whpx_irqchip_in_kernel()`)
Path for every IOAPIC-routed device IRQ (PIT/RTC/HPET/serial/IDE/PCI INTx/MSI), all on the **iothread**, no vCPU involvement:
`qemu_set_irq` -> `hw/i386/x86-common.c:gsi_handler` (`qemu_set_irq(s->i8259_irq[n], level)` then falls through to `qemu_set_irq(s->ioapic_irq[n], level)`) -> `hw/intc/ioapic.c:ioapic_set_irq` -> `ioapic_service`: edge clears `irr`, level sets `IOAPIC_LVT_REMOTE_IRR` (and skips if already set: "guest should be still working on previous one"), then `address_space_stl_le(ioapic_as, info.addr, info.data, ...)` i.e. an MSI-format store to 0xFEExxxxx -> `whpx-apic.c:whpx_apic_mem_write` -> `whpx_send_msi`:
```c
    WHV_INTERRUPT_CONTROL interrupt = { .Type = delivery, .DestinationMode = dest_mode ? Logical : Physical,
        .TriggerMode = trigger_mode ? WHvX64InterruptTriggerModeLevel : WHvX64InterruptTriggerModeEdge,
        .Vector = vector, .Destination = dest, };
    HRESULT hr = whp_dispatch.WHvRequestInterrupt(whpx_global.partition, &interrupt, sizeof(interrupt));
```
`WHvRequestInterrupt` is called from whatever thread runs the device (iothread); it needs no vCPU cancel; the hypervisor wakes a halted VP itself. `whpx_apic_mem_read` returns `~0` (LAPIC MMIO is never read by QEMU). PCI MSI goes through the same `k->send_msi = whpx_send_msi`. Master adds `if (vector == 0) { warn_report(...); return; }`.
**EOI for level-triggered:** exit `WHvRunVpExitReasonX64ApicEoi` -> `assert(whpx_apic_in_platform()); ioapic_eoi_broadcast(vcpu->exit_ctx.ApicEoi.InterruptVector);` (`ioapic.c:230`: clears Remote-IRR on matching level entries; if `irr` still set re-runs `ioapic_service`, with a 10 ms `timer_mod_anticipate` back-off after `SUCCESSIVE_IRQ_MAX_COUNT` 10000 storms). This is the only per-interrupt vCPU exit in this mode, and only for level IRQs.
**PIC / ExtINT:** `hw/i386/x86-cpu.c:pic_irq_request`: `if (cpu_is_apic_enabled(...) && !kvm_irqchip_in_kernel() && !whpx_irqchip_in_kernel()) { ...apic_deliver_pic_intr... } else { if (level) cpu_interrupt(cs, CPU_INTERRUPT_HARD); else cpu_reset_interrupt(cs, CPU_INTERRUPT_HARD); }` -> kick (cancel) of `first_cpu`. Then in `whpx_vcpu_pre_run` (both versions): `else if (vcpu->ready_for_pic_interrupt && cpu_test_interrupt(cpu, CPU_INTERRUPT_HARD)) { cpu_reset_interrupt(cpu, CPU_INTERRUPT_HARD); irq = cpu_get_pic_interrupt(env); if (irq >= 0) { reg_names[n] = WHvRegisterPendingEvent; reg_values[n].ExtIntEvent = (WHV_X64_PENDING_EXT_INT_EVENT){ .EventPending = 1, .EventType = WHvX64PendingEventExtInt, .Vector = irq }; } }`. `cpu_get_pic_interrupt` with irqchip in kernel skips the APIC and does `intno = pic_read_irq(isa_pic)` (acks the 8259). `ready_for_pic_interrupt` is set only by the `WHvRunVpExitReasonX64InterruptWindow` exit, which is armed by writing `WHvX64RegisterDeliverabilityNotifications.InterruptNotification = 1` whenever `!vcpu->window_registered && CPU_INTERRUPT_HARD` (i.e. first pass arms the window, the InterruptWindow exit sets `ready_for_pic_interrupt = 1; window_registered = 0;`, the next pre_run injects). Master adds after the ExtInt injection: `if (whpx_irqchip_in_kernel()) whpx_vcpu_kick_out_of_hlt(cpu);` = read `WHvRegisterInternalActivityState`, clear `InternalActivity.HaltSuspend`, write back (commit ca7a6add3b: "interrupts processed through the cancel vCPU and inject path will not cause the vCPU to go out of its halt state"). LINT0/LINT1 registers are not used; ExtINT is injected as a pending event directly. INIT/SIPI (v10.2.0 only): `ExtendedVmExits.X64ApicInitSipiExitTrap = 1` -> exit `WHvRunVpExitReasonX64ApicInitSipiTrap` decodes the ICR and re-issues `WHvRequestInterrupt` with `WHvX64InterruptTypeInit`/`Sipi` per destination "Assuming that APIC Ids are identity mapped"; master removed it (6ef6a0f04e "The implementation in Hyper-V works fine").
**LAPIC state exchange:** `whpx_apic_get` -> `WHvGetVirtualProcessorInterruptControllerState2` into a 256x16-byte `struct whpx_lapic_state` (fields 0x02 ID, 0x08 TPR, 0x10-0x27 ISR/TMR/IRR, 0x30/0x31 ICR, 0x32.. LVTs, 0x38 initial count, 0x3e divide) then `apic_next_timer(s, qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL))`; `whpx_apic_put` (via `run_on_cpu`) writes `WHvX64RegisterApicBase` then `WHvSetVirtualProcessorInterruptControllerState2`; used by `whpx_apic_reset`, `whpx_apic_post_load`, and by `whpx_get_registers` (FULL level only).

### 2b. kernel-irqchip=off (userspace `apic`): `whpx_vcpu_pre_run`, v10.2.0 lines 1454-1574, verbatim
```c
static void whpx_vcpu_pre_run(CPUState *cpu)
{
    ... int irq; uint8_t tpr; WHV_X64_PENDING_INTERRUPTION_REGISTER new_int;
    UINT32 reg_count = 0; WHV_REGISTER_VALUE reg_values[3]; WHV_REGISTER_NAME reg_names[3];
    memset(&new_int, 0, sizeof(new_int)); memset(reg_values, 0, sizeof(reg_values));
    bql_lock();
    /* Inject NMI */
    if (!vcpu->interruption_pending &&
        cpu_test_interrupt(cpu, CPU_INTERRUPT_NMI | CPU_INTERRUPT_SMI)) {
        if (cpu_test_interrupt(cpu, CPU_INTERRUPT_NMI)) {
            cpu_reset_interrupt(cpu, CPU_INTERRUPT_NMI);
            vcpu->interruptable = false;
            new_int.InterruptionType = WHvX64PendingNmi;
            new_int.InterruptionPending = 1;
            new_int.InterruptionVector = 2;
        }
        if (cpu_test_interrupt(cpu, CPU_INTERRUPT_SMI)) {
            cpu_reset_interrupt(cpu, CPU_INTERRUPT_SMI);          /* SMI is silently dropped */
        }
    }
    /* Force the VCPU out of its inner loop to process any INIT requests or commit pending TPR access. */
    if (cpu_test_interrupt(cpu, CPU_INTERRUPT_INIT | CPU_INTERRUPT_TPR)) {
        if (cpu_test_interrupt(cpu, CPU_INTERRUPT_INIT) && !(env->hflags & HF_SMM_MASK)) {
            qatomic_set(&cpu->exit_request, true);
        }
        if (cpu_test_interrupt(cpu, CPU_INTERRUPT_TPR)) {
            qatomic_set(&cpu->exit_request, true);
        }
    }
    /* Get pending hard interruption or replay one that was overwritten */
    if (!whpx_apic_in_platform()) {
        if (!vcpu->interruption_pending &&
            vcpu->interruptable && (env->eflags & IF_MASK)) {
            assert(!new_int.InterruptionPending);
            if (cpu_test_interrupt(cpu, CPU_INTERRUPT_HARD)) {
                cpu_reset_interrupt(cpu, CPU_INTERRUPT_HARD);
                irq = cpu_get_pic_interrupt(env);          /* apic_get_interrupt() then pic_read_irq() */
                if (irq >= 0) {
                    new_int.InterruptionType = WHvX64PendingInterrupt;
                    new_int.InterruptionPending = 1;
                    new_int.InterruptionVector = irq;
                }
            }
        }
        /* Setup interrupt state if new one was prepared */
        if (new_int.InterruptionPending) {
            reg_values[reg_count].PendingInterruption = new_int;
            reg_names[reg_count] = WHvRegisterPendingInterruption;
            reg_count += 1;
        }
    } else if (vcpu->ready_for_pic_interrupt &&
               cpu_test_interrupt(cpu, CPU_INTERRUPT_HARD)) {
        cpu_reset_interrupt(cpu, CPU_INTERRUPT_HARD);
        irq = cpu_get_pic_interrupt(env);
        if (irq >= 0) {
            reg_names[reg_count] = WHvRegisterPendingEvent;
            reg_values[reg_count].ExtIntEvent = (WHV_X64_PENDING_EXT_INT_EVENT)
            { .EventPending = 1, .EventType = WHvX64PendingEventExtInt, .Vector = irq, };
            reg_count += 1;
        }
     }
    /* Sync the TPR to the CR8 if was modified during the intercept */
    tpr = whpx_apic_tpr_to_cr8(cpu_get_apic_tpr(x86_cpu->apic_state));
    if (tpr != vcpu->tpr) {
        vcpu->tpr = tpr;
        reg_values[reg_count].Reg64 = tpr;
        qatomic_set(&cpu->exit_request, true);
        reg_names[reg_count] = WHvX64RegisterCr8;
        reg_count += 1;
    }
    /* Update the state of the interrupt delivery notification */
    if (!vcpu->window_registered &&
        cpu_test_interrupt(cpu, CPU_INTERRUPT_HARD)) {
        reg_values[reg_count].DeliverabilityNotifications =
            (WHV_X64_DELIVERABILITY_NOTIFICATIONS_REGISTER) { .InterruptNotification = 1 };
        vcpu->window_registered = 1;
        reg_names[reg_count] = WHvX64RegisterDeliverabilityNotifications;
        reg_count += 1;
    }
    bql_unlock();
    vcpu->ready_for_pic_interrupt = false;
    if (reg_count) {
        hr = whp_dispatch.WHvSetVirtualProcessorRegisters(whpx->partition, cpu->cpu_index, reg_names, reg_count, reg_values);
        ...
    }
}
```
Notes: at most 3 registers, one hypercall, and **zero hypercalls when nothing changed** (the common case). Interrupt-window exit handler: `case WHvRunVpExitReasonX64InterruptWindow: vcpu->ready_for_pic_interrupt = 1; vcpu->window_registered = 0; ret = 0;` -> loop back to `pre_run`, which now sees `interruptable` (post_run cleared `InterruptShadow`) and injects. `whpx_vcpu_post_run` (verbatim, v10.2.0):
```c
    env->eflags = vcpu->exit_ctx.VpContext.Rflags;
    uint64_t tpr = vcpu->exit_ctx.VpContext.Cr8;
    if (vcpu->tpr != tpr) { vcpu->tpr = tpr; bql_lock(); cpu_set_apic_tpr(x86_cpu->apic_state, whpx_cr8_to_apic_tpr(vcpu->tpr)); bql_unlock(); }
    vcpu->interruption_pending = vcpu->exit_ctx.VpContext.ExecutionState.InterruptionPending;
    vcpu->interruptable = !vcpu->exit_ctx.VpContext.ExecutionState.InterruptShadow;
```
i.e. `post_run` reads only the exit context; **no register fetch on the common exit path**. How an IRQ reaches the vCPU in this mode: iothread timer -> device -> IOAPIC MSI store -> `apic_send_msi`/`apic_deliver_irq` -> `apic_set_irq` (sets IRR) -> `apic_update_irq`: `if (!qemu_cpu_is_self(cpu)) cpu_interrupt(cpu, CPU_INTERRUPT_POLL)` -> `generic_handle_interrupt` -> `WHvCancelRunVirtualProcessor`; vCPU exits `Canceled`, `whpx_vcpu_process_async_events`: `if (CPU_INTERRUPT_POLL) { cpu_reset_interrupt(POLL); apic_poll_irq(apic) }` -> now on the vCPU thread `apic_update_irq` -> `cpu_interrupt(cpu, CPU_INTERRUPT_HARD)` (no kick, self) -> `pre_run` injects or arms the window. So **every device interrupt costs one VP cancel + one re-entry** in this mode; that is the structural reason kernel-irqchip=on is the default. Master's `pre_run` differences: computes `irr = apic_get_highest_priority_irr(apic_state)` (PIC output counts as `irr = 0`), injects only if `(vcpu->tpr < irr || irr == 0)`, arms the window with `.InterruptPriority = irr >> 4` and re-arms when a higher-priority IRR arrives (`window_priority`), and CR8 is used only when `!whpx_irqchip_in_kernel()` (commit fad1e8a98e "Hyper-V is aware of interrupt priorities and implements CR8/TPR, with the InterruptPriority field being followed"). Also `if (irr == -1) { if (isa_pic && pic_get_output(isa_pic)) irr = 0; else if (CPU_INTERRUPT_HARD) abort(); }`.

## 3. Register exchange policy

**Dirty flag.** `cpu->vcpu_dirty` (generic `CPUState` field since 36ab216b81). Set to `true` by `whpx_init_vcpu`, by `do_whpx_cpu_synchronize_state` after a get, and by `pre_loadvm`. Consumed only at the top of the run loop:
```c
    do {
        if (cpu->vcpu_dirty) { whpx_set_registers(cpu, WHPX_SET_RUNTIME_STATE); cpu->vcpu_dirty = false; }
        ... whpx_vcpu_pre_run(cpu); ... WHvRunVirtualProcessor(...); ... whpx_vcpu_post_run(cpu); switch (ExitReason) ...
    } while (!ret);
```
**Get is on demand only.** `whpx_get_registers` is called from exactly: `do_whpx_cpu_synchronize_state` (via `run_on_cpu`, i.e. `cpu_synchronize_state()` from gdb/monitor/vmport/`do_cpu_init`/`do_cpu_sipi`/`apic_handle_tpr_access_report`), the `WHvRunVpExitReasonException` handler (debug) and the fatal `default:` exit. **Never on MMIO, PIO, MSR, CPUID, halt, interrupt-window, EOI or Canceled exits** in v10.2.0. `whpx_cpu_synchronize_state(cpu) { if (!cpu->vcpu_dirty) run_on_cpu(cpu, do_whpx_cpu_synchronize_state, ...); }` -- a second sync while dirty is free.

**Set** happens at three levels (v10.2.0 `WHPX_SET_RUNTIME_STATE` < `RESET_STATE` < `FULL_STATE`; master enum `WHPXStateLevel { WHPX_LEVEL_FAST_RUNTIME_STATE, RUNTIME_STATE, RESET_STATE, FULL_STATE }`): run loop (RUNTIME), `synchronize_post_reset` (RESET: adds `whpx_set_tsc`), `synchronize_post_init` (FULL: master adds `WHvX64RegisterApicBase`). `assert(cpu_is_stopped(cpu) || qemu_cpu_is_self(cpu));` guards both directions.

**Register counts per call (v10.2.0).** `whpx_register_names[]` = **69 names** on x86_64 (16 GPR, RIP, RFLAGS, 8 segment, IDTR, GDTR, CR0/2/3/4/8, 16 XMM, 8 FpMmx, FpControlStatus, XmmControlStatus, EFER, KernelGsBase, ApicBase, SysenterCs/Eip/Esp, STAR, LSTAR, CSTAR, SFMASK; DRs and PAT commented out) in **one** `WHvGetVirtualProcessorRegisters`/`Set` call, plus separate 1-register calls: `WHvX64RegisterXCr0` (`whpx_get_xcrs`/`set_xcrs`, only if partition `XsaveSupport`), `WHvX64RegisterTsc` (get: only `if (!env->tsc_valid)`; set: only at RESET+), and with kernel-irqchip `whpx_apic_get` -> `WHvGetVirtualProcessorInterruptControllerState2` (called **twice** inside one `whpx_get_registers`, lines 627-636 and 769-771). XMM/x87 ride in the same 69-register call; **no XSAVE area** is exchanged (YMM/ZMM state is not synced at all in v10.2.0; migration blocker text: "non-migratable CPUID feature support, dirty memory tracking support, and XSAVE/XRSTOR support").

**Master:** `whpx_register_names[]` shrinks to **42** (FP moved out, PAT added, CR8 removed); `whpx_register_names_legacy_fp[]` = 26 (XMM0-15, FpMmx0-7, 2 control) fetched separately; `whpx_register_names_for_vmexit[]` = **16 GPRs only**. `whpx_get_registers(cpu, WHPX_LEVEL_FAST_RUNTIME_STATE)` -> `whpx_get_registers_for_vmexit`: one 16-register call, then `env->eip = exit_ctx.VpContext.Rip; env->eflags = exit_ctx.VpContext.Rflags;`. FULL level additionally does XCR0, `WHvGetVirtualProcessorState(WHvVirtualProcessorStateTypeXsaveState)` when `CR4.OSXSAVE` (compacted buffer, `decompact_xsave_area`), *and* the 26 legacy FP registers (37ce6d8da7: "On Hyper-V looks like we need to fetch both the legacy and new state instead of being able to rely on xsave"), and `whpx_apic_get`. `whpx_set_registers(FAST)` writes GPR+RIP+RFLAGS = 18 names (`idx` stops at `WHvX64RegisterEs`; comment: "Skip those registers for synchronisation after MMIO accesses as they're not going to be modified in that case"). Segments/CRs are fetched **on demand** by the emulator callbacks: `whpx_read_segment_descriptor` returns CS from `exit_ctx.VpContext.Cs`, DS/ES from `exit_ctx.IoPortAccess.{Ds,Es}` on PIO exits, otherwise one `whpx_get_reg`; `read_cr` = one `whpx_get_reg`; mode predicates `is_protected_mode/is_long_mode/is_user_mode` read `exit_ctx.VpContext.ExecutionState.{Cr0Pe,EferLma,Cpl}` (no hypercall). Paolo Bonzini on the series: "Perhaps you can make target/i386/emulate remember the registers it has already queried, and cache them in to env?" -- not done yet.

**TPR/CR8 and the per-vCPU flags.** `AccelCPUState { WHV_EMULATOR_HANDLE emulator; bool window_registered; bool interruptable; bool ready_for_pic_interrupt; uint64_t tpr; uint64_t apic_base; bool interruption_pending; WHV_RUN_VP_EXIT_CONTEXT exit_ctx; }` (master: no `emulator`/`apic_base`, adds `int window_priority`). `vcpu->tpr` mirrors CR8 (`APIC.TPR[7:4] = CR8[3:0]`, `whpx_apic_tpr_to_cr8 = tpr >> 4`); `post_run` pushes `exit_ctx.VpContext.Cr8` into the userspace APIC, `pre_run` pushes APIC TPR changes back as `WHvX64RegisterCr8` and sets `exit_request` so the write lands before the next run. Master wraps every CR8 use in `if (!whpx_irqchip_in_kernel())` (4fdbad6c49: "When kernel-irqchip=on, manage TPR as part of the APIC state instead entirely"). `interruptable = !ExecutionState.InterruptShadow` and `interruption_pending = ExecutionState.InterruptionPending` gate injection (Q2b). There is no "LAPIC dirty" flag: with kernel-irqchip the userspace `APICCommonState` is a stale mirror refreshed only by `whpx_apic_get` inside a FULL get; writes go through `run_on_cpu(whpx_apic_put)` on reset/post_load only.

## 4. MMIO and PIO exits, memory mapping, VGA

**v10.2.0 (WinHvEmulation):** `whpx_init_vcpu` -> `WHvEmulatorCreateEmulator(&whpx_emu_callbacks, &vcpu->emulator)` per vCPU. Exit handlers are one call each: `whpx_handle_mmio` -> `WHvEmulatorTryMmioEmulation(vcpu->emulator, cpu, &vcpu->exit_ctx.VpContext, ctx, &emu_status)`, `whpx_handle_portio` -> `WHvEmulatorTryIoEmulation(...)`. Callbacks:
```c
static const WHV_EMULATOR_CALLBACKS whpx_emu_callbacks = {
    .WHvEmulatorIoPortCallback = whpx_emu_ioport_callback,        /* address_space_rw(&address_space_io, IoAccess->Port, ...) */
    .WHvEmulatorMemoryCallback = whpx_emu_mmio_callback,          /* address_space_rw(cpu_addressspace(cs), ma->GpaAddress, ...) */
    .WHvEmulatorGetVirtualProcessorRegisters = whpx_emu_getreg_callback,  /* forwards RegisterNames/RegisterCount verbatim */
    .WHvEmulatorSetVirtualProcessorRegisters = whpx_emu_setreg_callback,  /* forwards; then cpu->vcpu_dirty = false */
    .WHvEmulatorTranslateGvaPage = whpx_emu_translate_callback,   /* WHvTranslateGva */
};
```
The register count per emulated instruction is chosen by WinHvEmulation.dll, not QEMU (the callback is a pass-through), so each MMIO/PIO exit costs Run + N getreg hypercalls + the device access (under BQL via `prepare_mmio_access`) + a setreg hypercall. QEMU does not batch or coalesce MMIO on WHPX (no `coalesced` in either version). The setreg callback clears `vcpu_dirty` "so we avoid the double write on resume of the VP".

**Master (target/i386/emulate):** `whpx_handle_mmio` -> `emulate_instruction(cpu, ctx->InstructionBytes, ctx->InstructionByteCount)`:
```c
    whpx_get_registers(cpu, WHPX_LEVEL_FAST_RUNTIME_STATE);   /* 1 hypercall: 16 GPRs; RIP/RFLAGS from exit_ctx */
    decode_instruction_stream(env, &decode, &stream);         /* decodes the exit-context InstructionBytes, no guest fetch */
    exec_instruction(env, &decode);                           /* memory via address_space_rw; segments/CRs via x86_emul_ops on demand */
    whpx_set_registers(cpu, WHPX_LEVEL_FAST_RUNTIME_STATE);   /* 1 hypercall: 18 regs */
```
PIO fast path (`whpx_handle_portio`): non-string IN = `whpx_get_reg(RAX)` + `handle_io` + `whpx_bump_rip` (set RIP) + `whpx_set_reg(RAX)` = 3 hypercalls; non-string OUT uses `ctx->Rax` from the exit context + `whpx_bump_rip` = **1 hypercall**; string ops fall back to `emulate_instruction`. If `cpu->vcpu_dirty` (vmport called `cpu_synchronize_state` inside the port read) the result is written into `env` instead. Measured on Windows 10 (cover letter, issue #3349): "QEMU 10.2: 2 minutes / QEMU 11.0rc0: 8 minutes / This series: back to 2 minutes -- performance parity with older QEMU using winhvemulation". Docs: MMX/SSE/AVX-based MMIO is unsupported by `target/i386/emulate`.

**RAM mapping:** `MemoryListener whpx_memory_listener { .region_add, .region_del, .log_sync = whpx_log_sync, .priority = MEMORY_LISTENER_PRIORITY_ACCEL }` on `address_space_memory`. v10.2.0 `whpx_process_section` maps only `memory_region_is_ram(mr)` sections, host-page aligned: `WHvMapGpaRange(partition, host_va, start_pa, size, WHvMapGpaRangeFlagRead | WHvMapGpaRangeFlagExecute | (rom ? 0 : WHvMapGpaRangeFlagWrite))`. Master `whpx_set_phys_mem`: `writable = !area->readonly && !area->rom_device`; non-RAM regions are skipped unless ROMD; ROM/ROMD mapped read+execute so guest writes trap. No `WHvMapGpaRangeFlagTrackDirtyPages`, no `WHvQueryGpaRangeDirtyBitmap` anywhere in either version.

**Dirty tracking / VGA:** `whpx_log_sync(listener, section) { if (!memory_region_is_ram(mr)) return; memory_region_set_dirty(mr, 0, int128_get64(section->size)); }` -- every sync marks the **whole RAM section** dirty, so `vga.c`'s `memory_region_snapshot_and_clear_dirty` sees a fully dirty VRAM every frame (full-frame redraw; correct, not incremental). This is also why migration is blocked ("missing dirty memory tracking support"). Legacy VGA (0xA0000 window, `vga_mem_ops` MMIO) is not RAM and traps per access -- docs: "Guests using legacy VGA modes ... performance will be quite suboptimal. Workaround: use a more modern graphics mode." Master's `ssd=off` (`WHvPartitionPropertyCodeSeparateSecurityDomain = 0` plus clearing `IbrsSupport/StibpSupport/IbpbSupport/SsbdSupport/IbrsAllSupport/PsfdSupport`) "results in a significant vmexit performance improvement by skipping speculative execution mitigations" -- i.e. per-exit cost on WHP is dominated by the security-domain switch, not by QEMU code.

## 5. TSC, time, enlightenments, MSR exits

**QEMU_CLOCK_VIRTUAL under WHPX is host monotonic time.** `util/qemu-timer.c:qemu_clock_get_ns`: `case QEMU_CLOCK_VIRTUAL: return cpus_get_virtual_clock();` -> `system/cpus.c`: `if (cpus_accel && cpus_accel->get_virtual_clock) return cpus_accel->get_virtual_clock(); return cpu_get_clock();`. WHPX registers no `get_virtual_clock` (only TCG/icount does), so it is `cpu_get_clock()` = `timers_state.cpu_clock_offset + get_clock()` (`system/cpu-timers.c`), i.e. `clock_gettime(CLOCK_MONOTONIC)` minus the time the VM spent stopped (`cpu_enable_ticks`/`cpu_disable_ticks` adjust the offset on vm_start/vm_stop). Device timers (PIT `hw/timer/i8254`, RTC, HPET, LAPIC timer in userspace mode via `apic_next_timer`) are `QEMU_CLOCK_VIRTUAL` timers on the main-loop `QEMUTimerList`; `timer_mod_ns` -> `timerlist_rearm` -> `timerlist_notify` -> `notify_cb` = `qemu_timer_notify_cb` -> `qemu_notify_event()` (wakes the main loop poll). Expired timers run in the main loop under the BQL (`qemu_clock_run_all_timers`). **Nothing in the timer path touches a vCPU**; a device that wants the CPU raises an IRQ (Q2). Guest TSC is the hardware TSC as virtualised by Hyper-V; QEMU never scales or offsets it at runtime.

**whpx_get_tsc / whpx_set_tsc** (identical in both versions):
```c
static int whpx_set_tsc(CPUState *cpu) {
    /* Suspend the partition prior to setting the TSC to reduce the variance in TSC across vCPUs.
       When the first vCPU runs post suspend, the partition is automatically resumed. */
    if (whp_dispatch.WHvSuspendPartitionTime) { hr = whp_dispatch.WHvSuspendPartitionTime(whpx->partition); if (FAILED(hr)) warn_report(...); }
    tsc_val.Reg64 = cpu_env(cpu)->tsc;
    hr = whp_dispatch.WHvSetVirtualProcessorRegisters(whpx->partition, cpu->cpu_index, &tsc_reg, 1, &tsc_val);
```
There is no `WHvResumePartitionTime` call; the comment relies on auto-resume at the next Run. Set only at `level >= WHPX_SET_RESET_STATE` (post_reset/post_init; commit 6785e76701: "Setting TSC at runtime is heavy and additionally can have side effects on the guest"). Get gated by `env->tsc_valid`: `if (!env->tsc_valid) { whpx_get_tsc(cpu); env->tsc_valid = !runstate_is_running(); }` and `whpx_cpu_update_state(running) { if (running) env->tsc_valid = false; }` -- while running, each FULL get re-reads TSC; while stopped it is cached. Master skips TSC and XCR0 reads entirely on the FAST level (fe8c0a8b7d, ccaa2fb7c2). TSC frequency: `WHvGetCapability(WHvCapabilityCodeProcessorClockFrequency)` -> `env->tsc_khz`; APIC bus: `WHvCapabilityCodeInterruptClockFrequency` (default `HYPERV_APIC_BUS_FREQUENCY` 200 MHz; master forces `1000000000` when `!whpx_irqchip_in_kernel()` because the userspace APIC timer runs at 1 GHz). No kvmclock analogue in v10.2.0; the frequencies are exposed through the VMware leaf 0x40000010 when `-cpu ...,vmware-cpuid-freq=on`.

**RDTSC:** never intercepted; there is no `RdtscExit` anywhere (`ExtendedVmExits` sets only `X64MsrExit`, `X64CpuidExit`, `ExceptionExit`, and v10.2.0 `X64ApicInitSipiExitTrap`).

**Hyper-V enlightenments.** v10.2.0: none; `hv-*` CPU flags are silently meaningless on whpx (GitLab #2063 "Poor performance with -accel whpx ... missing CPUID hypervisor ident": leaf 0x40000000 returned `eax=40000010 ebx=0 ecx=0 edx=0`). Master (ae9dcd6f26: per-CPU `hv-*` "happens too late", so it is an accelerator property `-accel whpx,hyperv=on|off|auto`, default auto=on when not legacy OS): `WHvPartitionPropertyCodeSyntheticProcessorFeaturesBanks` with `HypervisorPresent, Hv1, FastHypercallOutput, AccessVpRunTimeReg, AccessPartitionReferenceCounter, AccessPartitionReferenceTsc, AccessHypercallRegs, AccessFrequencyRegs, AccessVpIndex`, and only with kernel-irqchip: `AccessSynicRegs, AccessSyntheticTimerRegs, AccessIntrCtrlRegs, SyntheticClusterIpi, DirectSyntheticTimers, AccessGuestIdleReg, TbFlushHypercalls, EnableExtendedGvaRangesForFlushVirtualAddressList` ("These technically work without the Hyper-V LAPIC but behave oddly for multi-core VMs"). Hyper-V itself serves the reference-TSC page, synthetic timers, cluster IPIs and TLB-flush hypercalls -- that is the kvmclock/paravirt replacement, and it only exists with the LAPIC in the hypervisor. Also `WHvPartitionPropertyCodeProcessorFeaturesBanks` = all host features, `ProcessorPerfmonFeatures` (PMU), `NestedVirtualization` when `Bank1.NestedVirtSupport && kernel-irqchip`. Legacy-OS detection: `WHvCapabilityCodeProcessorPerfmonFeatures` failing => Windows 10 => `is_modern_os = false`.

**MSR exits.** v10.2.0: `prop.ExtendedVmExits.X64MsrExit = 1` only (the hypervisor emulates the architectural MSRs it knows; QEMU sees the rest). Handler: "For all unsupported MSR access we: ignore writes, return 0 on read." -- one `WHvSetVirtualProcessorRegisters` with RIP(+len) and, for reads, RAX=RDX=0. Master adds `WHvPartitionPropertyCodeX64MsrExitBitmap { .UnhandledMsrs = 1, .ApicBaseMsrWrite = 1 }` and handles: `HV_X64_MSR_APIC_FREQUENCY` (0x40000023, read -> `apic_bus_freq`, user-mode LAPIC only), `MSR_IA32_APICBASE` writes (validated by `cpu_set_apic_base`, #GP on reserved bits; "Read path unreachable on Hyper-V" -> `abort()`), x2APIC MSRs `0x800..0x8ff` -> `apic_msr_read/write` under BQL (user-mode LAPIC), `HV_X64_MSR_GUEST_IDLE` (0x400000f0, read => `whpx_handle_hyperv_guestidle` = HLT that ignores IF, `HF2_HYPERV_HLT_MASK`; "Windows 11 25H2 uses it even when not advertised"), `HV_X64_MSR_VP_ASSIST_PAGE` writes ignored. Unknown: `trace_whpx_unsupported_msr_access`, then `-accel whpx,ignore-unknown-msr=on` (default) keeps the 2018 behaviour, `off` injects #GP via `x86_emul_raise_exception` and leaves RIP unchanged. `intercept-msr-gp=on` adds `WHvX64ExceptionTypeGeneralProtectionFault` to the exception exit bitmap and re-emulates the faulting RDMSR/WRMSR (`whpx_handle_msr_from_gpf`) to log which MSR Hyper-V itself rejected. VMX capability MSRs are read via `WHvCapabilityCodeVmx*` (`whpx_get_supported_msr_feature`).

## 6. CPUID

**v10.2.0:** `whpx_accel_init`: `UINT32 cpuidExitList[] = {1, 0x80000001};` -> `WHvSetPartitionProperty(WHvPartitionPropertyCodeCpuidExitList)`; `whpx_init_vcpu` replaces it with `{1, 0x80000001, 0x40000000, 0x40000010}` when `x86_cpu->vmware_cpuid_freq && env->tsc_khz`. Exit handler `WHvRunVpExitReasonX64Cpuid`: `cpu_x86_cpuid(env, cpuid_fn, 0, ...)` (**subleaf hard-coded to 0**), then `case 0x40000000: rax = 0x40000010; rbx = rcx = rdx = 0;` (no vendor string at all -- the #2063 symptom), `case 0x40000010: rax = env->tsc_khz; rbx = env->apic_bus_freq / 1000;`, `case 0x80000001: rcx &= ~CPUID_EXT3_OSVW;`, then one 5-register `WHvSetVirtualProcessorRegisters` (RIP, RAX, RCX, RDX, RBX). Everything else is answered by Hyper-V's own CPUID model of the host. Code comment: "WHPX doesn't support setting CPUID values in the hypervisor once the partition has been setup, which is too late since VCPUs are realized later." `WHvPartitionPropertyCodeCpuidResultList` is **not used** in either version (grep: zero hits). The hypervisor bit: commit 7becac84fb (2018) trapped leaf 1 to OR in `CPUID_EXT_HYPERVISOR`; master derives it from the CPU model (`whpx_get_supported_cpuid` forces `CPUID_EXT_X2APIC | CPUID_EXT_HYPERVISOR` in ECX and `CPUID_HT` in EDX).

**Master:** 25-leaf exit list `{0x0, 0x1, 0x6, 0x7, 0xb, 0xd, 0x14, 0x24, 0x29, 0x1E, 0x40000000, 0x40000001, 0x40000010, 0x80000000..0x80000004, 0x80000007, 0x80000008, 0x8000000A, 0x80000021, 0x80000022, 0xC0000000, 0xC0000001}` (79b3cb1f01 "Unlike the implementation in QEMU 10.2, this one works. It's not optimal though as it doesn't use the Hyper-V support for this."). Handler answers from QEMU's CPU model `cpu_x86_cpuid(env, Rax, Rcx, ...)` (host model seeded via `whpx_get_supported_cpuid` -> `WHvGetVirtualProcessorCpuidOutput` on a temporary VP 0, or `host_cpuid()` masks in `whpx-cpu-legacy.c` on Windows 10), with dynamic bits copied from the exit context's `DefaultResult{Rax,Rbx,Rcx,Rdx}` (Hyper-V's answer): OSXSAVE, CET_IBT/SHSTK, OSPKE, leaf 0xD subleaf 1/2 EBX sizes; `CPUID[1].ECX.X2APIC` and `EDX.APIC` follow the QEMU APIC state. Hypervisor leaves: with enlightenments on, `0x40000000/1/10` are passed through as `DefaultResult*` (Hyper-V's "Microsoft Hv" identity, so Windows guests take the enlightened paths); with `hyperv=off`, `0x40000000` returns `0x40000010` + `"VMwareVMware"` (0x61774d56/0x4d566572/0x65726177) when `vmware_cpuid_freq`, else `0x40000001` + `"KVMKVMKVM\0\0\0"` (0x4b4d564b/0x564b4d56/0x4d) with `0x40000001` reporting bit 15 (KVM's `KVM_FEATURE_*` slot used to advertise x2APIC to Linux). No "TCGTCGTCG" string is ever produced under whpx.

## 7. Halt handling

**kernel-irqchip=on:** QEMU never sees the vCPU halt. The hypervisor parks the VP inside `WHvRunVirtualProcessor` on HLT and wakes it on `WHvRequestInterrupt`. v10.2.0 comment on the `WHvRunVpExitReasonX64Halt` case: "WARNING: as of build 19043.1526 (21H1), this exit reason is no longer used." Master: "Used for kernel-irqchip=off". The vCPU thread is declared never-idle so it will not park in QEMU: `whpx_vcpu_thread_is_idle(cpu) { return !whpx_apic_in_platform(); }` (master: `!whpx_irqchip_in_kernel()`), wired to `ops->cpu_thread_is_idle`. Consequence: `cpu->halted` is irrelevant in this mode and `whpx_vcpu_run` skips the halted check: `if (cpu->halted && !whpx_apic_in_platform()) { cpu->exception_index = EXCP_HLT; qatomic_set(&cpu->exit_request, false); return 0; }`. The only halt-related QEMU action is master's `whpx_vcpu_kick_out_of_hlt` after injecting a PIC ExtInt via `WHvRegisterPendingEvent` (Q2a), because that path bypasses `WHvRequestInterrupt` "which does not reset the HLT state".

**kernel-irqchip=off:** exit `WHvRunVpExitReasonX64Halt` -> `whpx_handle_halt` (verbatim, v10.2.0):
```c
static int whpx_handle_halt(CPUState *cpu)
{
    int ret = 0;
    bql_lock();
    if (!(cpu_test_interrupt(cpu, CPU_INTERRUPT_HARD) && (cpu_env(cpu)->eflags & IF_MASK)) &&
        !cpu_test_interrupt(cpu, CPU_INTERRUPT_NMI)) {
        cpu->exception_index = EXCP_HLT;
        cpu->halted = true;
        ret = 1;
    }
    bql_unlock();
    return ret;
}
```
(`eflags` is valid here because `post_run` copied `exit_ctx.VpContext.Rflags` into `env->eflags` before the switch.) `ret = 1` leaves the run loop; `whpx_vcpu_exec` returns `EXCP_HLT`; `whpx_cpu_thread_fn` calls `qemu_process_cpu_events`:
```c
    qatomic_set(&cpu->exit_request, false);
    while (cpu_thread_is_idle(cpu)) { ... qemu_cond_wait(cpu->halt_cond, &bql); }
```
with `cpu_thread_is_idle`: `if (cpu->stop || !cpu_work_list_empty(cpu)) return false; if (cpu_is_stopped(cpu)) return true; if (!cpu->halted || cpu_has_work(cpu)) return false; if (cpus_accel->cpu_thread_is_idle) return cpus_accel->cpu_thread_is_idle(cpu); return true;` and `cpu_has_work` = `x86_cpu_pending_interrupt(cs, cs->interrupt_request) != 0` (HARD only counts when `x86_cpu_interrupts_enabled(env)`, i.e. IF set; NMI/SMI/INIT/SIPI/POLL always count). Wake-up: any `cpu_interrupt` from the iothread -> `qemu_cpu_kick` -> `qemu_cond_broadcast(cpu->halt_cond)` (plus a harmless `WHvCancelRunVirtualProcessor` on a VP that is not running). Back in `whpx_vcpu_run`, `whpx_vcpu_process_async_events`: `if ((HARD && IF) || NMI) cpu->halted = false;` -- master also un-halts on `HF2_HYPERV_HLT_MASK` (the GUEST_IDLE MSR pseudo-halt that "wakes the vCPU even if EFLAGS.IF is set"). No halt polling: the thread sleeps on the condvar immediately.

## 8. Performance history (git log, cover letters, issues)

Chronology of everything perf-relevant in the WHPX history (`target/i386/whpx-all.c` 2018-2020, `target/i386/whpx/` 2020-2026):
- 2018-01 812d49f2a3 Justin Terry (Microsoft) "Introduce the WHPX impl": vPartition, vCPU, MMIO/PortIO via WinHvEmulation.
- 2018-02 eb1fe944a8 "WHPX improve interrupt notification registration": "skipping the additional call to WHvSetVirtualProcessorRegisters if we have already registered for the window exit" (origin of `window_registered`).
- 2018-03 4e286099fe "WHPX improve vcpu_post_run perf": "removes the additional call to WHvGetVirtualProcessorRegisters in whpx_vcpu_post_run now that the WHV_VP_EXIT_CONTEXT is returned in all WHV_RUN_VP_EXIT_CONTEXT structures" (origin of exit-context-only post_run).
- 2018-06 e7ca549fc8 "register for unrecognized MSR exits" (return 0 / ignore write, for Linux MSR probing).
- 2020-02 6785e76701 Sunil Muthuswamy "TSC get and set should be dependent on VM state": "Setting TSC at runtime is heavy"; adds `WHvSuspendPartitionTime`. 4df28c9352 "Use proper synchronization primitives while processing async events" (SMP fix).
- 2020-07 5c8e1e8328 "vmware cpuid leaf for tsc and apic frequency". 2020-10 faf20793b5 "support for the kernel-irqchip on/off" -- `whpx-apic.c`, `WHvX64LocalApicEmulationModeXApic`, `WHvRequestInterrupt`, ApicEoi exit, INIT/SIPI trap; "'split' value is not supported"; default on.
- 2022-03 d7482ffe97 Ivan Shcherbakov "Added support for breakpoints and stepping"; 2022-02/05 CR8/TPR fixes.
- 2023-12 GitLab #2063: Windows 10 guest on Server 2022 host "essentially unusable, compared to same image running under Hyper-V"; cause: no Hyper-V identity/enlightenments (`hv-*` "do not appear applicable to -accel WHPX"). Unfixed until 2026.
- 2026-02 (Mohamed Mediouni, v6 00/28 "whpx: x86 updates"): drop WinHvEmulation for `target/i386/emulate` ("I removed the reliance on the Hyper-V GVA translate call, which is a very slow one"), state levels, `WHPX_LEVEL_FAST_RUNTIME_STATE` ("Optimise vmexits by save/restoring less state"), remove CPUID trapping ("results in significantly inconsistent CPUID data"), x2APIC, exception injection. 2026-02-26: `whpx_vcpu_kick_out_of_hlt` ("Make legacy Windows guests (Windows XP) not run very slowly when kernel-irqchip=off"). 2026-02-28: all host features, synthetic (enlightenment) features, PMU.
- 2026-03 (v3 00/12 "Windows 10 and performance fixes"): "some state is really expensive to fetch or write with Hyper-V so switch some less-essential state to on demand. The effect of this is magnified on Windows 10 because Hyper-V enlightenments are not available there." Patches: exceptions exit only when needed, skip TSC/XCR read on MMIO exits, don't restore segments after MMIO, indirect CR access, segments on demand, GPR-only vmexit reads. Numbers (issue #3349, Windows 10 host): **"QEMU 10.2: 2 minutes / QEMU 11.0rc0: 8 minutes / This series: back to 2 minutes"**. Also 6ef6a0f04e remove SIPI trapping.
- 2026-04 (v3 00/37 "WHPX x86 updates for QEMU 11.1"): user-mode x2APIC "for better performance when it has to be used" ("The performance boost is quite visible for multicore guests" vs xAPIC MMIO), interrupt priority (`InterruptPriority` in DeliverabilityNotifications), IO-port fast path, GuestIdle MSR, Windows 10 defaults to kernel-irqchip=off when PIC present, `ssd=off` ("significantly higher MMIO performance"), xsave. Recommendation in cover letter: "On Windows 11, use WHP with kernel-irqchip=on"; docs: prefer `-M q35,pic=off` over `kernel-irqchip=on` on Windows 10.
- 2026-08: PAT sync, legacy+xsave FP fetch, `FastHypercallOutput`, re-inject #DB.
- No published whpx-vs-KVM-vs-HVF benchmark exists in the tree, docs, or the series; the only numbers are the 2 min / 8 min / 2 min boot above and #2063's qualitative report. Device choices that matter for exits, from the code: LAPIC in hypervisor (removes a cancel+re-entry per IRQ), MSI/IOAPIC edge over PIC (PIC = cancel + interrupt-window exit + ExtInt injection), `-M pic=off`, std/VBE VGA in a linear mode rather than legacy 0xA0000 banked modes (per-access MMIO trap), `ssd=off`, Hyper-V enlightenments for Windows guests (reference TSC, synthetic timers, IPI hypercalls instead of APIC MMIO), and avoiding anything that calls `cpu_synchronize_state` on a hot path (vmport does; hence the master PIO special case).

## 9. Single-step / debug (brief)

Mechanism (`whpx_vcpu_configure_single_stepping`, both versions): set `TF` in `WHvX64RegisterRflags` (read-modify-write, 2 hypercalls), then write `WHvRegisterInterruptState.InterruptShadow = 1` ("Suspend delivery of hardware interrupts during single-stepping"), run; after the exit clear TF, clear the shadow, and read `WHvX64RegisterPendingDebugException` -- if `.SingleStep` is set, clear it and write back ("hide the INT1 from the guest"). The trap arrives as `WHvRunVpExitReasonException` with `ExceptionType == WHvX64ExceptionTypeDebugTrapOrFault`, enabled through `WHvPartitionPropertyCodeExceptionExitBitmap` (`whpx_set_exception_exit_bitmap`, cached in `whpx->exception_exit_bitmap`). The bitmap is only changed in `whpx_first_vcpu_starting` (when `running_cpus` goes 0->1) because "whpx_set_exception_exit_bitmap() cannot be called if one or more VCPUs are already running"; `whpx->step_pending` comes from `AccelClass::pre_resume_vm`. Master (d2d6d91794) also drops `ExtendedVmExits.ExceptionExit` when the bitmap is empty ("The exceptions VM exit was enabled with an empty bitmask even when not used"). Breakpoints are software: `whpx_breakpoint_instruction = 0xF1` (INT1/ICEBP, "let the guest always handle INT3" because Linux uses int3 selftests) written via `cpu_memory_rw_debug`, state machine `WHPX_BP_{CLEARED,SET_PENDING,SET,CLEAR_PENDING}`, inserted on first vCPU start, removed when the last vCPU stops; stepping over a breakpoint uses `start_exclusive()` (`WHPX_STEP_EXCLUSIVE`, all other vCPUs parked). Master re-injects non-single-step #DB to the guest (`whpx_inject_back_db` via `WHvRegisterPendingEvent`, EventType `WHvX64PendingEventException`) and pauses the VM (`vm_stop(RUN_STATE_PAUSED)`) instead of `qemu_system_guest_panicked` on unexpected exits. The 75-line header comment in `whpx-all.c` lists the known TF-stepping limitations (PUSHF/POPF interplay, stepping over exceptions steps over the handler, guest debuggers see intercepted INT1s).

## 10. What QEMU/WHPX does NOT do that KVM does, and the consequences

| KVM facility | WHPX status (both versions unless noted) | Consequence |
|---|---|---|
| In-kernel PIT / PIC / IOAPIC | Only the **LAPIC** can be in the hypervisor (`WHvPartitionPropertyCodeLocalApicEmulationMode`). PIT/RTC/HPET/PIC/IOAPIC are always QEMU userspace. `split` rejected. | Every device tick is a host timer + BQL + IOAPIC service on the iothread; edge IRQs cost one `WHvRequestInterrupt`, level IRQs additionally one `ApicEoi` exit. PIC interrupts cost a cancel + interrupt-window exit + pending-event write. |
| irqfd / ioeventfd / MSI routes | None. `accel/accel-irq.c` dispatches only to KVM and MSHV; `ioapic_update_kvm_routes` is `#ifdef ACCEL_GSI_IRQFD_POSSIBLE` (KVM/MSHV). | MSI from virtio etc. is an MMIO store handled in the iothread -> `whpx_send_msi`; no kernel-bypass for vhost-style backends. |
| Coalesced MMIO | None (no `coalesced` in whpx). | Each VGA/legacy-MMIO write is a full exit + emulation round trip. |
| Dirty page logging | None: `whpx_log_sync` marks the whole section dirty; no `TrackDirtyPages`/`QueryGpaRangeDirtyBitmap`. | Migration blocked; VGA always redraws the full framebuffer (correctness ok). |
| kvmclock | None in v10.2.0. Master: Hyper-V reference TSC page + synthetic timers via `SyntheticProcessorFeaturesBanks` when enlightenments are on and the LAPIC is in the hypervisor. | Linux/Windows guests on master use the Hyper-V clocksource/timers; on v10.2.0 they fall back to TSC/HPET/PIT (more exits). |
| Posted interrupts / APICv control | Not exposed; whatever Hyper-V does internally. `WHvRequestInterrupt` is the only knob. | Fine; no tuning possible. |
| Halt polling | None; with LAPIC in hypervisor the VP halts inside `WHvRunVirtualProcessor`; without, the thread sleeps on `halt_cond` at once. | Wake-up latency is Hyper-V's (in-hypervisor mode) or condvar + Run re-entry (user-mode). |
| MSR filtering (`KVM_X86_SET_MSR_FILTER`) | Only `X64MsrExitBitmap { UnhandledMsrs, ApicBaseMsrWrite }` (master). | Cannot trap arbitrary architectural MSRs; RDTSC/RDTSCP never trap. |
| CPUID fully authored in kernel (`KVM_SET_CPUID2`) | `CpuidExitList` only (2 leaves classic, 25 master); `CpuidResultList` unused. | Every listed leaf is a full exit + 5-register write; unlisted leaves are Hyper-V's host model. |
| Save/restore, XSAVE (classic) | No dirty tracking; classic never syncs XSAVE; master syncs xsave + legacy FP. | Live migration impossible; snapshot of AVX state wrong on v10.2.0. |
| Nested virt | Master: `NestedVirtualization` only with kernel-irqchip (`89b624a9f7`: the other combination is rejected by build 26300.7939). | -- |
| TLB flush / `cpu_exit` on memory changes | No shadow TLB; no kick needed. | Memory-map changes (`WHvMapGpaRange`/`Unmap`) are applied from the iothread while VPs run. |
| SMI | `pre_run` silently clears `CPU_INTERRUPT_SMI` (`cpu_reset_interrupt(cpu, CPU_INTERRUPT_SMI)`) -- no SMM entry. | No SMM on WHPX. |

## Design pattern extracted -- the rules QEMU/WHPX follows that a new VMM on WHP should copy

1. **The vCPU thread lives inside `WHvRunVirtualProcessor`; nothing but the guest's own exits or an explicit cancel brings it out.** (`whpx_vcpu_run`'s `do { ... } while (!ret)`; `ret != 0` only on Canceled/Halt/debug/fatal.) Reason: on WHP the expensive event is the exit itself (the `ssd=off` note shows the security-domain switch dominates), so the design minimises exits, never slices.
2. **Device time is host monotonic time on a separate thread.** `QEMU_CLOCK_VIRTUAL == cpu_get_clock()` (CLOCK_MONOTONIC minus stopped time); device timers fire on the main loop; a timer re-arm only calls `qemu_notify_event()`. Reason: the hypervisor already virtualises TSC/APIC-timer time; the VMM must not own the CPU's notion of time and must never stop the CPU to advance device clocks.
3. **Interrupts are messages to the hypervisor, not state pushed into the CPU.** With the LAPIC in the platform, IRQ delivery = `WHvRequestInterrupt` from the device's thread (`whpx_send_msi`); no register write, no cancel, no BQL hand-off to the vCPU thread. Reason: this is the only path with zero vCPU exits per interrupt; `qemu_process_cpu_events`/`cpu_interrupt` never see device IRQs in this mode.
4. **Put the LAPIC in the hypervisor by default and route legacy IRQs through the IOAPIC as MSI.** `kernel_irqchip_allowed = true` by default; `ioapic_service` turns every pin into an MSI-format store; master even disables the LAPIC-in-hypervisor mode only where Hyper-V's PIC/HLT bug forces it (Windows 10 + PIC). Reason: the userspace-LAPIC path costs a cancel + re-entry per IRQ (`apic_update_irq` -> `CPU_INTERRUPT_POLL` -> `WHvCancelRunVirtualProcessor`), and enlightenments (reference TSC, synthetic timers, cluster IPIs) exist only with the hypervisor LAPIC.
5. **Kick only on cross-thread `cpu_interrupt`/`cpu_exit`/`run_on_cpu`, and coalesce it with `exit_request`.** `generic_handle_interrupt` kicks only if `!qemu_cpu_is_self`; the run loop checks `exit_request` after `pre_run` and pre-cancels the next Run so a request can never be lost. Reason: `WHvCancelRunVirtualProcessor` is the only asynchronous lever; it must be rare and race-free.
6. **Register state is fetched lazily and written only when dirty.** `vcpu_dirty` gates one `WHvSetVirtualProcessorRegisters` per resume; `whpx_get_registers` only on `cpu_synchronize_state` (gdb, monitor, vmport, INIT/SIPI, TPR-access report) or fatal exits; `post_run` reads the exit context only (`Rflags`, `Cr8`, `ExecutionState`). Master goes further: MMIO exits read 16 GPRs + exit-context RIP/RFLAGS/CS/ExecutionState, write 18 registers, and fetch segments/CRs one at a time only if the emulator asks. Reason: measured 4x boot-time regression (2 -> 8 min) when full state was exchanged per exit, recovered by going on-demand.
7. **Exchange interrupt-injection state in the same `WHvSetVirtualProcessorRegisters` call as everything else, and skip the call when nothing changed.** `pre_run` batches `PendingInterruption`/`PendingEvent`, `Cr8`, `DeliverabilityNotifications` into one <=3-register write, `if (reg_count)`. Track `window_registered` so the interrupt-window notification is armed once (2018 perf fix). Reason: hypercalls per resume must be zero on the hot path.
8. **Use the interrupt-window exit, never spin or poll, for delayed injection.** `WHvX64RegisterDeliverabilityNotifications.InterruptNotification = 1` -> `WHvRunVpExitReasonX64InterruptWindow` -> `ready_for_pic_interrupt` -> inject on the next `pre_run`; injection is gated by `ExecutionState.InterruptShadow`/`InterruptionPending` from the exit context. Master adds `InterruptPriority` so the window fires only when TPR/PPR would admit the vector. Reason: the hypervisor knows when the guest is interruptible; the VMM does not track IF/shadow/TPR per instruction.
9. **HLT belongs to whoever owns the LAPIC.** In-platform LAPIC: never observe HLT (`cpu_thread_is_idle` returns false, the VP idles inside Run). User-mode LAPIC: `whpx_handle_halt` -> `cpu->halted` -> `qemu_cond_wait(halt_cond)`, woken only by `cpu_interrupt`. After a `PendingEvent`-style ExtInt injection clear `InternalActivityState.HaltSuspend` yourself. Reason: sleeping in the VMM while the hypervisor thinks the VP is halted (or vice-versa) is the Windows-10 PIC bug and the "Windows XP runs very slowly" bug.
10. **Emulate MMIO/PIO from the exit context's instruction bytes with the VMM's own decoder; use the exit context's `Rax`/`Ds`/`Es`/`Cs` before asking for registers.** Master's `handle_portio` OUT path is one hypercall (`whpx_bump_rip`). Reason: the WinHvEmulation callback path made the register count opaque and used the "very slow" `WHvTranslateGva`.
11. **Map guest RAM once with `WHvMapGpaRange(Read|Execute|Write)` from a memory listener; ROM read+execute; never remap per slice.** Accept coarse dirty tracking (`log_sync` marks everything dirty) rather than trapping framebuffer writes; keep VGA in a linear RAM-backed mode.
12. **Partition-wide policy is set once before `WHvSetupPartition`:** `ProcessorCount`, `LocalApicEmulationMode`, `ExtendedVmExits { X64MsrExit, X64CpuidExit, ExceptionExit (only if bitmap non-empty) }`, `CpuidExitList`, `X64MsrExitBitmap`, processor/synthetic feature banks, `SeparateSecurityDomain`. Nothing is toggled at runtime except the exception bitmap while all VPs are stopped.
13. **Answer trapped MSRs/CPUID with a single register write of RIP+results; unknown MSRs read 0 / ignore write (or #GP by policy) -- never stop the world.**
14. **Set TSC only on reset/init, under `WHvSuspendPartitionTime`; read it only when someone needs the register file and cache it while the VM is stopped.** Never intercept RDTSC.

## What would break if you kept a lockstep Bochs-style device loop on WHP

Bochs lockstep = one thread: run CPU for N instructions (here: `WHvRunVirtualProcessor` bounded by the next emulated timer deadline), stop, service device timers, repeat. Measured today: 3.8 % of wall time inside `WHvRunVirtualProcessor`, 86 % of slices too short to enter the hypervisor. Mapping that onto what QEMU's code shows:

1. **There is no "run for N instructions / until deadline" primitive on WHP.** QEMU never asks for one: the only ways out of `WHvRunVirtualProcessor` are guest exits and `WHvCancelRunVirtualProcessor` from another thread. A lockstep loop therefore needs either a second thread that issues a cancel at every device deadline (each cancel = a `Canceled` exit = a full security-domain switch, exactly the cost `ssd=off` exists to reduce) or a deadline so short that the Run is not worth entering -- which is the 86 % figure. The Bochs instruction-count clock cannot be preserved at all: WHP's TSC and APIC timer run in host time regardless of how often the VMM re-enters, so `QEMU_CLOCK_VIRTUAL == host monotonic` is not a choice, it is the only consistent model.
2. **Every device timer becomes a vCPU exit.** In QEMU with LAPIC-in-platform a PIT/RTC/HPET tick costs the vCPU nothing (Q2a, rule 3); in lockstep it costs a cancel + `Canceled` exit + re-entry, plus whatever register traffic the resume does. At 1 kHz PIT + LAPIC timer + HPET that is thousands of forced exits per second on an otherwise idle guest.
3. **Interrupt latency inverts.** QEMU: `WHvRequestInterrupt` from the iothread wakes a halted VP immediately, inside the hypervisor. Lockstep: an IRQ raised while the vCPU is "between slices" waits for the loop to come around and for a `pre_run`-style injection, and a halted guest either spins the loop or sleeps the whole machine including devices.
4. **A shadow interpreter CPU forces full-state exchange.** QEMU's numbers: exchanging the 69-register set per exit (11.0rc0's regression) made a Windows 10 boot 4x slower; the fix was reading 16 GPRs and writing 18 registers on MMIO exits and nothing at all on interrupt/EOI/Canceled exits (Q3). A lockstep design that hands control to an interpreter (to run device code, or to "catch up" instructions) must do a FULL get (69+26 registers, XSAVE, TSC, LAPIC state) before and a FULL set after every slice -- the pattern QEMU reserves for gdb, INIT/SIPI and reset.
5. **The LAPIC cannot stay in the VMM without paying the userspace-LAPIC tax.** With `LocalApicEmulationMode=None` every device IRQ goes through the QEMU-off-mode path: `cpu_interrupt(POLL)` -> cancel -> `apic_poll_irq` -> `CPU_INTERRUPT_HARD` -> `PendingInterruption` write -> Run, with `DeliverabilityNotifications` + `InterruptWindow` exits whenever IF is clear, plus CR8 synchronisation on every exit (`post_run`) and every resume (`pre_run`). QEMU keeps this mode only as a fallback ("kernel-irqchip=off fixes: This was really... quite broken") and master had to add IRR-priority tracking and `GUEST_IDLE` handling to make it usable. It also forfeits the enlightenments (reference TSC page, synthetic timers, cluster IPI, TLB-flush hypercalls), which Hyper-V only offers with its own LAPIC ("These technically work without the Hyper-V LAPIC but behave oddly for multi-core VMs").
6. **EOI/level-IRQ semantics need the hypervisor's help.** QEMU relies on the `ApicEoi` exit (only for level-triggered vectors) to clear IOAPIC Remote-IRR and re-service; a VMM-owned LAPIC must instead see every EOI as an MMIO/MSR exit -- one more exit per interrupt on top of the injection exit.
7. **BQL-style serialisation is per access, not per slice.** QEMU takes the device lock inside `prepare_mmio_access` for the duration of one device access and releases it; the iothread runs timers under the same lock between accesses. A lockstep loop holds "device world" and "CPU world" alternately, so the vCPU cannot be inside the hypervisor while a timer callback runs -- which is precisely the 3.8 % duty cycle.
8. **Halt handling is undefined.** QEMU either never sees HLT (in-platform LAPIC) or parks the thread on a condvar until `cpu_interrupt`. In lockstep, a HLT exit must be handled by advancing device time to the next deadline (Bochs-style idle skip), then re-entering; but WHP's TSC keeps running in host time, so guest-visible time jumps relative to the device clocks and the LAPIC timer (owned by the VMM) drifts from the TSC the guest reads via RDTSC (never trapped).
9. **SMP is impossible to keep in lockstep.** Each VP must be in `WHvRunVirtualProcessor` on its own thread; there is no way to step VPs round-robin without a cancel per step. QEMU's design is one thread per VP from day one (`whpx_start_vcpu_thread`), and INIT/SIPI, IPIs and `start_exclusive()` all assume it.
10. **What survives from Bochs:** the device models themselves (PIT/RTC/HPET/PIC/IOAPIC/VGA/IDE logic), the interpreter as an *instruction emulator for MMIO/PIO exits* (QEMU's `target/i386/emulate` is exactly that: decode the exit's `InstructionBytes`, execute one instruction against `address_space_rw`, write back GPRs), and the interpreter as a *fallback engine* -- but not the scheduler, not the instruction-count clock, and not the VMM-owned LAPIC as the default.

Minimal restructure implied by the QEMU code: (a) vCPU thread(s) that never leave Run except on guest exits/cancel; (b) a device thread with host-monotonic timers that raises IRQs via `WHvRequestInterrupt` (LAPIC in platform; IOAPIC pins -> MSI); (c) per-access device locking; (d) `vcpu_dirty`-gated register writes and exit-context-only reads on the hot exits; (e) the interpreter reduced to a one-instruction emulator plus an on-demand register cache; (f) cancel only for pause/reset/debug/`run_on_cpu`.
