//! The engine that runs a slice on the host's hypervisor.
//!
//! ## Why a shadow processor is not an optimisation
//!
//! Measured on this host (`docs/whp-platform-probe-2026-08-27.md`): a memory
//! exit reports `InstructionLength = 0`, does not advance `RIP`, and for a
//! write to a read-only window carries no instruction bytes either. There is
//! therefore no way to finish a trapped access by stepping over it, and no
//! decoder-free variant of this design to fall back on. Every exit the platform
//! cannot complete itself is completed by executing ONE instruction on this
//! port's own processor, through the same `ExecCtx` the interpreter uses — which
//! is also what keeps a trapped access bit-identical to what Bochs would do.
//!
//! ## Why time comes from the host
//!
//! The hypervisor retires instructions this port never sees, so the shadow's
//! `icount` stands still while the guest runs. What did pass is host time, so
//! the slice reports [`Progress::Ticks`], converted at the machine's own
//! instructions-per-second rate — the same rate the software engine's ticks are
//! denominated in, which is what lets a snapshot cross between them.

use core::time::Duration;

use rusty_box_whp::{
    Exit, ExitReason, ExtendedVmExits, LocalApicMode, MsrExits, Partition, PartitionConfig,
    WhpError,
};

use super::alarm::Alarm;
use super::state::{self, VpRegisters};
use super::Vp;
use rusty_box::cpu::arch_state::VcpuArchState;
use rusty_box::cpu::{cpu::BxCpuC, instrumentation::Instrumentation, CpuError, Result};
use rusty_box::emulator::{
    EventDelivery, PcIo, Progress, ProgressUnit, SliceEngine, SliceRequest,
};
use rusty_box::memory::plan::MemoryPlan;
use rusty_box::memory::BxMemC;
use rusty_box::GpaWindow;

/// The processor this engine runs. SMP under a hypervisor is its own unit; a
/// machine with more processors than this refuses to start rather than running
/// one and pretending.
const BOOT_VP: u32 = 0;

/// Every `CPUID` leaf this engine takes away from the host.
///
/// A leaf that is not here executes on the host's own processor and answers
/// with the host's own values — the model, the stepping, the feature words,
/// and leaf 1's `HypervisorPresent` bit. That is the loudest divergence a
/// guest can hear, and the one that matters most to anything looking for an
/// emulator, so the list is generous rather than minimal: a leaf omitted by
/// oversight is a leaf the host answers.
///
/// The ranges are the architectural ones — the standard leaves, the
/// hypervisor range, the extended leaves and Centaur's — each taken well past
/// what this port implements, because an unimplemented leaf still has a
/// correct answer and it is this port's to give (Bochs `cpuid.cc` returns
/// zeros or the highest supported leaf, never the host's).
const TRAPPED_CPUID_LEAVES: &[u32] = &{
    let mut leaves = [0u32; 0x20 + 0x10 + 0x20 + 0x02];
    let mut at = 0;
    let mut leaf = 0x0000_0000u32;
    while leaf < 0x0000_0020 {
        leaves[at] = leaf;
        at += 1;
        leaf += 1;
    }
    leaf = 0x4000_0000;
    while leaf < 0x4000_0010 {
        leaves[at] = leaf;
        at += 1;
        leaf += 1;
    }
    leaf = 0x8000_0000;
    while leaf < 0x8000_0020 {
        leaves[at] = leaf;
        at += 1;
        leaf += 1;
    }
    leaves[at] = 0xC000_0000;
    leaves[at + 1] = 0xC000_0001;
    leaves
};

/// Which model-specific register accesses this engine takes away from the
/// platform.
///
/// Setting the MSR exit bit alone traps nothing: the platform answers a fixed
/// handful of MSRs itself and hands over only what this names — a fact that
/// cost a test which read `IA32_MISC_ENABLE` and got the host's `0x00851809`
/// while the interpreter answered zero.
///
/// Everything is taken EXCEPT the two time-stamp-counter entries, and that is
/// a decision rather than an oversight. This port derives its TSC from retired
/// instructions, and a shadow processor retires almost none while the guest
/// runs on hardware — so a trapped `RDMSR` of the TSC would answer from a
/// clock that barely moves, while `RDTSC`, which is a separate exit this
/// engine does not request, would keep answering from the host's. Two clocks
/// that disagree is worse for a calibrating guest than one clock that is not
/// this port's, so both stay with the hardware until the cross-engine TSC
/// bridge exists. That is the one MSR divergence this engine has, and it is
/// registered rather than hidden.
const TRAPPED_MSRS: MsrExits = MsrExits {
    tsc_read: false,
    tsc_write: false,
    ..MsrExits::ALL
};

/// Guest code this port has not been asked to run on hardware yet, and which
/// the platform cannot finish alone. Each is a real exit that a complete engine
/// services; refusing by name is what keeps a half-serviced one from looking
/// like a working machine.
fn unserviced(what: &'static str, exit: &Exit) -> CpuError {
    // Where the guest was matters more than what the exit was called: an exit
    // this engine cannot service is a report about guest code, and the report
    // is useless without an address to look at.
    tracing::error!(
        "WHP exit not serviced by this engine: {what} at {:#x}:{:#x} (cs base {:#x}, rflags {:#x})",
        exit.vp.cs.selector,
        exit.vp.rip,
        exit.vp.cs.base,
        exit.vp.rflags,
    );
    CpuError::UnsupportedCpuOperation { operation: "WHP exit not serviced by this engine" }
}

fn platform_failed(error: WhpError) -> CpuError {
    tracing::error!("WHP platform call failed: {error}");
    CpuError::UnsupportedCpuOperation { operation: "the hypervisor refused" }
}

/// A partition that has been configured, given memory and given a processor.
///
/// FIELD ORDER IS LOad-BEARING: `alarm` is declared before `partition` because
/// Rust drops fields in declaration order, and the alarm's thread holds a
/// canceller naming this partition. Joining that thread before the partition
/// is destroyed is the whole of the ordering obligation, and this is where it
/// is discharged.
struct Started {
    alarm: Alarm,
    partition: Partition,
    /// Reused across slices so a state exchange allocates nothing per exit.
    state: VcpuArchState,
    /// The map the partition is currently holding.
    ///
    /// Kept because a borrowed mapping leaves no bookkeeping behind — the
    /// partition records the range only for memory it owns — so this is the
    /// only record of what is installed, and the only way to know which
    /// windows a new map removed rather than merely changed.
    installed: MemoryPlan,
    /// Whether the guest was inside an interrupt shadow when the hardware last
    /// handed it back.
    ///
    /// x86 blocks interrupts for exactly one instruction after `MOV SS`,
    /// `POP SS` and `STI`, so that a stack switch written as `MOV SS, x` /
    /// `MOV ESP, y` cannot be interrupted between its two halves. The
    /// interpreter tracks this itself and never delivers inside one; this
    /// engine cannot, because the instruction retired on the hardware and the
    /// fact lives in the partition's own interrupt state rather than in any
    /// architectural register the exchange carries.
    ///
    /// Delivering anyway pushes the interrupt frame with the NEW `SS` and the
    /// OLD `ESP` — a frame at an address the handler never agreed to — and the
    /// `IRET` that ends the handler pops whatever happened to be there. That
    /// is a `general protection: 0000` on `IRET`, which is exactly how a DLX
    /// boot died once IDE interrupts started flowing.
    shadowed: bool,
}

/// What the guest has been leaving the hardware for.
///
/// An exit is the unit of cost of this whole design — measured at roughly four
/// microseconds on this host — so how many of each a guest takes is the first
/// thing worth knowing about a machine that is slow, and the first thing worth
/// looking at when one is stuck: a boot that has taken no port exits has not
/// reached its firmware, whatever else it has been doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExitCounts {
    /// Port I/O, answered from the machine's device set.
    pub port: u64,
    /// An access the partition's map did not serve, finished on the shadow.
    pub memory: u64,
    /// `CPUID`, answered from this port's own model.
    pub cpuid: u64,
    /// An MSR access, answered from this port's own register file.
    pub msr: u64,
    /// The guest halted.
    pub halt: u64,
    /// The host asked for the processor back mid-run.
    pub canceled: u64,
    /// A device dispatch asked for the machine's boundary to be serviced.
    pub boundary: u64,
}

/// Runs guest code on the Windows Hypervisor Platform.
///
/// Starts unconfigured because a machine constructs its engine before it has
/// memory to map — the partition is built on the first slice, which is the
/// first moment the engine is handed [`PcIo`] and can see the machine's own
/// guest-physical map.
#[derive(Default)]
pub struct WhpEngine {
    started: Option<Started>,
    exits: ExitCounts,
}

impl WhpEngine {
    /// What the guest has been leaving the hardware for, since this machine
    /// was built. Reached through `Emulator::engine`.
    #[must_use]
    pub const fn exits(&self) -> ExitCounts {
        self.exits
    }

}

/// Build the partition, map the machine's memory into it and create the
/// processor — once, on the first slice.
///
/// Takes the slot rather than the engine so a caller can hold the engine's
/// other fields at the same time: a running slice counts its exits while the
/// partition is borrowed.
fn start<'a>(started: &'a mut Option<Started>, io: &mut PcIo<'_>) -> Result<&'a mut Started> {
    {
        if started.is_none() {
            let mut config = PartitionConfig::new().map_err(platform_failed)?;
            config
                .processor_count(1)
                .map_err(platform_failed)?
                // No APIC in the partition: this port's own `cpu/apic.rs` is
                // the guest's local APIC, its 8259 pair stays in host memory,
                // and injection is therefore ours end to end. Measured
                // (probe finding 10): under `XApic` a halted processor never
                // leaves `WHvRunVirtualProcessor`, which would strand the
                // timer wheel on a thread that never returns.
                .local_apic(LocalApicMode::None)
                .map_err(platform_failed)?
                // What a guest asks ABOUT the processor is this port's to
                // answer, not the host's. Both are serviced on the shadow, so
                // a guest reading CPUID or an MSR under a hypervisor gets the
                // same bytes it would get with none — which is the whole of
                // what parity means here.
                .extended_vm_exits(ExtendedVmExits {
                    cpuid: true,
                    msr: true,
                    ..ExtendedVmExits::default()
                })
                .map_err(platform_failed)?
                .cpuid_exit_list(TRAPPED_CPUID_LEAVES)
                .map_err(platform_failed)?
                .msr_exits(TRAPPED_MSRS)
                .map_err(platform_failed)?;
            let mut partition = config.setup().map_err(platform_failed)?;

            let installed = install_the_machines_map(&mut partition, io.memory(), None)?;
            partition.create_processor(BOOT_VP).map_err(platform_failed)?;

            let alarm = Alarm::watching(partition.canceller(BOOT_VP));
            *started = Some(Started {
                alarm,
                partition,
                state: VcpuArchState::default(),
                installed,
                // A processor that has never run cannot be mid-instruction.
                shadowed: false,
            });
        }
    }
    // Established just now, or by an earlier slice. Reported rather than
    // asserted: a library says what it cannot do instead of ending the
    // host's process to say it.
    started.as_mut().ok_or(CpuError::UnsupportedCpuOperation {
        operation: "the partition did not start",
    })
}

/// Why a processor came back to the machine.
///
/// Both endings hand the machine work only it can do, which is why neither is
/// an error and why they are told apart: a halt changes what the machine knows
/// about the processor, a cancellation does not.
enum Yielded {
    /// The guest executed `HLT`. The machine's own fast-forward advances time
    /// to the next device deadline from here and decides when the processor
    /// may run again.
    Halted,
    /// The host asked for the processor back mid-run.
    Canceled,
    /// A device dispatch asked for the machine's boundary to be serviced.
    ///
    /// The guest must not run on until it has been. A chipset write that
    /// re-routes guest-physical space — a PAM flip, an SMRAM open, a relocated
    /// BAR — only takes effect when the machine applies it, and this engine
    /// then has a new map to install; a guest that kept running would be
    /// running against the layout the write was replacing.
    Boundary,
    /// The stretch the machine asked for is over.
    ///
    /// The machine bounds a slice by the time to its next device deadline, and
    /// a device timer fires only when the machine has the processor back. An
    /// engine that ran until the guest happened to stop would starve every
    /// timer in the machine: the first guest to find out is the BIOS, whose
    /// keyboard self-test polls the 8042 a bounded number of times and panics
    /// when the controller — waiting on a timer that never fires — does not
    /// answer.
    Budget,
}

/// Install the machine's guest-physical map into the partition, replacing
/// whatever `installed` describes, and report what is now installed.
///
/// The map is derived from the machine's own memory, so the hypervisor and
/// this port's interpreter serve the same bytes at the same addresses — which
/// is what makes an exit serviced by the shadow processor land where the guest
/// expects it.
///
/// Two things make the replacement more than a loop of maps. A mapping
/// REPLACES any prior one over the same range, so a window that merely changed
/// its permissions or its backing needs no unmap — but a window the new map
/// DROPS has to be taken out explicitly, or a range the chipset just turned
/// into device space would keep being served from RAM and never exit. And a
/// borrowed mapping leaves the partition no bookkeeping to consult, so what
/// the old map was has to be remembered rather than asked for.
///
/// Windows that did not change are left alone. That is not only for the cost
/// of the call: re-mapping a range discards the second-level translations the
/// hypervisor built for it, and a BIOS flipping one PAM area has no business
/// making the guest fault its way back through all of RAM.
///
/// # Errors
/// A machine whose memory has no stable map, a window outside the allocation,
/// or a platform call that refused.
fn install_the_machines_map(
    partition: &mut Partition,
    memory: &mut BxMemC,
    installed: Option<&MemoryPlan>,
) -> Result<MemoryPlan> {
    let plan = MemoryPlan::derive(memory).map_err(|error| {
        tracing::error!("this machine has no stable guest-physical map: {error:?}");
        CpuError::UnsupportedCpuOperation {
            operation: "a partially resident machine has no map to install",
        }
    })?;
    let old: &[GpaWindow] = installed.map_or(&[], MemoryPlan::windows);
    if old == plan.windows() {
        return Ok(plan);
    }

    for window in old {
        if plan.windows().contains(window) {
            continue;
        }
        partition
            .unmap_subrange(window.gpa, window.len)
            .map_err(platform_failed)?;
    }

    for window in plan.windows() {
        if old.contains(window) {
            continue;
        }
        let host = memory
            .allocation_slice(window.host.get(), window.len)
            .ok_or(CpuError::UnsupportedCpuOperation {
                operation: "a plan window fell outside the machine's allocation",
            })?;
        // The machine owns this allocation in a boxed slice that never moves,
        // and owns this engine beside it, so the mapping cannot outlive the
        // memory — which is the obligation `map_borrowed` names. Discharging it
        // is why this call is `unsafe` and this crate is not: see the wrapper
        // in `rusty_box_whp`, whose signature carries the contract.
        map_window(partition, window.gpa, host, window.perms)?;
    }
    Ok(plan)
}

/// Install one window, discharging the contract `map_borrowed` names.
///
/// The single place this crate reaches for `unsafe`, and it holds no unsafe
/// operation of its own — what it does is assert an ownership fact the type
/// system cannot: the machine owns this allocation, keeps it in a boxed slice
/// that never moves for its lifetime, and owns the engine holding this
/// partition alongside it. The mapping therefore cannot outlive the memory it
/// points at, and the partition is torn down with the machine that made it.
fn map_window(
    partition: &mut Partition,
    gpa: u64,
    host: &mut [u8],
    perms: rusty_box_whp::GpaPerms,
) -> Result<()> {
    // SAFETY: as above — the mapped bytes belong to the machine that owns this
    // partition, and neither the allocation nor its address changes while the
    // machine lives.
    unsafe { partition.map_borrowed(gpa, host, perms) }.map_err(|error| {
        // Mapping is where the platform materialises its backing partition,
        // and it names that partition after the process — so a second machine
        // on this engine in one process is refused HERE, with
        // `ERROR_VID_PARTITION_ALREADY_EXISTS`, and not at any earlier call.
        // Measured in `rusty_box_whp`'s `a_process_holds_one_partition_at_a_time`.
        tracing::error!(
            "installing guest memory at {gpa:#x} failed: {error}. One process runs one \
             machine on this engine; a second needs a second process."
        );
        CpuError::UnsupportedCpuOperation {
            operation: "the hypervisor would not take this machine's memory",
        }
    })
}

impl<T: Instrumentation> SliceEngine<T> for WhpEngine {
    // The hardware retires the guest's instructions and this port never sees
    // them, so what a slice can report is the time it took. See `ticks_elapsed`.
    const PROGRESS_UNIT: ProgressUnit = ProgressUnit::Ticks;

    // Delivery is this engine's: the partition has no local APIC of its own
    // and the hardware knows nothing of this machine's 8259 pair, so a vector
    // reaches the guest only by the shadow taking it at the head of a slice.
    const EVENT_DELIVERY: EventDelivery = EventDelivery::Engine;

    fn memory_map_changed(&mut self, memory: &mut BxMemC) -> Result<()> {
        // Before the first slice there is no partition and nothing installed;
        // the map this would have installed is the one `start` derives, so a
        // change now is not a change to anything.
        let Some(started) = self.started.as_mut() else {
            return Ok(());
        };
        started.installed =
            install_the_machines_map(&mut started.partition, memory, Some(&started.installed))?;
        Ok(())
    }

    fn run_slice(
        &mut self,
        cpu: &mut BxCpuC<T>,
        mut io: PcIo<'_>,
        request: SliceRequest,
    ) -> Result<Progress> {
        let ips = io.pc_system.ips();

        // DIAGNOSTIC BISECTION, not a shipping mode. `WHP_ALL_SHADOW=1` keeps
        // every part of this engine except the hardware: the same slice
        // budgeting, the same delivery-at-slice-entry policy, the same device
        // servicing through `PcIo` — but the guest's instructions are retired
        // on the shadow instead of the partition, so no state ever crosses to
        // the platform and back.
        //
        // It splits the remaining search space in half. If a DLX boot survives
        // this, everything machine-side is sound and the fault lives in
        // hardware execution or the state handoff around it. If it still dies
        // with the same `general protection` on `IRET`, the fault is in this
        // engine's own slice and delivery logic, which the interpreter's loop
        // does differently.
        if std::env::var_os("WHP_ALL_SHADOW").is_some() {
            return run_slice_on_the_shadow(cpu, &mut io, request);
        }

        // An interrupt to deliver, a halt to wake from, any work the
        // interpreter does at a trace boundary: it happens HERE, on the
        // shadow, before the hardware is given the processor. The partition
        // was created with no local APIC of its own precisely so that delivery
        // stays on this side (REPLAN decision 6), and running it through the
        // interpreter means a guest's interrupt frame is what it would be with
        // no hypervisor in the picture.
        //
        // Which is also why this engine never writes
        // `WHvRegisterPendingInterruption`: there is nothing for the platform
        // to inject that the shadow has not already delivered.
        // Make the processor's view of the bus current BEFORE asking whether
        // it has anything to deliver. The 8259's interrupt line is a level,
        // and what the processor holds is a latched copy of it: a line the
        // guest's own handler dropped while this engine was elsewhere leaves
        // that copy asserted, and the next slice delivers a vector the PIC no
        // longer has. The interpreter never has this problem because it syncs
        // inside its own loop, at every instruction boundary.
        io.sync_io_events(cpu);
        // An interrupt shadow the hardware is still holding forbids delivery
        // here, however ready the machine's own controller is. The shadowing
        // instruction retires the moment the guest runs again, so the vector
        // waits exactly one slice — which is what the architecture asks for
        // and what the interpreter does with its own inhibit mask.
        let shadowed = self.started.as_ref().is_some_and(|started| started.shadowed);
        if shadowed {
            tracing::debug!(target: "irq", "CPU: delivery held off — the guest is in an interrupt shadow");
        }
        if !shadowed && cpu.has_an_event_to_deliver() {
            // The head of a slice is the only place this engine delivers, so
            // it is also the only place the delay between a device raising a
            // line and a guest seeing it can be measured. Both RIPs, because a
            // delivery that happened shows as a jump to a handler and one that
            // did not shows as an ordinary instruction.
            let before = cpu.rip();
            io.emulate_one(cpu)?;
            tracing::debug!(
                target: "irq",
                "CPU: had an event at slice entry; rip {:#x} -> {:#x}, IF={}",
                before,
                cpu.rip(),
                cpu.interrupts_enabled()
            );
            // Delivery acknowledged the interrupt, which changes the line.
            io.sync_io_events(cpu);
        }
        // An SMI the shadow just took puts the processor in a mode the
        // hardware has no equivalent for, so the handler runs to completion
        // here rather than being handed over half-entered.
        run_the_shadow_out_of_smm(cpu, &mut io)?;

        let Self { started, exits } = self;
        let started = start(started, &mut io)?;

        // The shadow describes the processor; the platform runs it.
        install_the_shadow(started, cpu)?;

        let outcome = run_until_the_machine_is_needed(
            started,
            cpu,
            &mut io,
            deadline(request, ips),
            request.instructions(),
            ips,
            exits,
        );

        // Whatever ended the run, the shadow must describe the processor again
        // before the machine looks at it: the scheduler reads activity state,
        // the interrupt fabric reads IF, and a snapshot reads all of it.
        read_back_into_the_shadow(started, cpu)?;

        let SliceOutcome { yielded, ran } = outcome?;
        match yielded {
            // Halting is not architectural state, so it does not arrive in the
            // exchange above; the platform reports it as the reason the run
            // ended, and the shadow is where the machine reads it.
            Yielded::Halted => cpu.record_halt(),
            // A boundary request is already on the processor, where the
            // scheduler takes it the moment this slice returns; ending the
            // slice is the whole of what this engine owes it. The other two
            // leave the processor exactly as the hardware left it.
            Yielded::Canceled | Yielded::Boundary | Yielded::Budget => {}
        }
        // What the guest earned, whole.
        //
        // NOT clamped to what the machine asked for, which was tried and
        // measured: a slice overshoots its budget by however much guest time
        // the last run bought, and discarding that overshoot rather than
        // banking it drops the machine's clock 836-fold behind the guest that
        // is driving it. The boot went 539 times slower and reached less of
        // the kernel than before. The overshoot is fixed by ending the slice
        // sooner — the budget check above — never by under-reporting a slice
        // that has already run.
        Ok(Progress::Ticks(ticks_elapsed(ran, ips)))
    }
}

/// Run a slice entirely on the shadow, touching no partition.
///
/// The other half of the `WHP_ALL_SHADOW` bisection described at its call
/// site. Everything this engine does around the guest is kept — the delivery
/// decision at slice entry, the SMM drain, the boundary questions, the budget
/// denominated in the machine's own ticks — and only the executor changes.
///
/// One instruction per step rather than a batch, because that is the verb
/// `PcIo` offers and this path is a diagnostic: it is slower than the
/// interpreter's own loop and does not need not to be.
///
/// # Errors
/// Whatever the guest raised that the shadow could not take.
fn run_slice_on_the_shadow<T: Instrumentation>(
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    request: SliceRequest,
) -> Result<Progress> {
    io.sync_io_events(cpu);
    if cpu.has_an_event_to_deliver() {
        io.emulate_one(cpu)?;
        io.sync_io_events(cpu);
    }
    run_the_shadow_out_of_smm(cpu, io)?;

    let budget = request.instructions();
    let mut retired = 0u64;
    while retired < budget {
        io.emulate_one(cpu)?;
        retired += 1;
        if cpu.wants_a_machine_boundary() || io.needs_boundary() {
            break;
        }
        if cpu.has_an_event_to_deliver() {
            break;
        }
    }
    // One instruction is one tick here, which is the software engine's own
    // denomination — this path retires instructions and can count them.
    Ok(Progress::Ticks(retired))
}

/// Describe the platform's processor from the shadow.
///
/// One of the two halves of the state exchange, named because both halves run
/// in two places now — around a whole slice, and around each exit the shadow
/// has to finish. Writing them out twice is how the two could drift.
///
/// # Errors
/// A register the platform refused to take.
fn install_the_shadow<T: Instrumentation>(
    started: &mut Started,
    cpu: &BxCpuC<T>,
) -> Result<()> {
    // Named rather than elided, so a field added to `Started` has to be
    // considered here instead of silently ignored.
    let Started { alarm: _, partition, state, installed: _, shadowed: _ } = started;
    cpu.export_arch_state(state);
    state::import(&Vp::new(partition, BOOT_VP), state).map_err(platform_failed)
}

/// Describe the shadow from the platform's processor.
///
/// # Errors
/// A register the platform refused to hand over, or a state this port will not
/// import — a segment whose attributes describe no descriptor it can build.
fn read_back_into_the_shadow<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
) -> Result<()> {
    let Started { alarm: _, partition, state, installed: _, shadowed } = started;
    let vp = Vp::new(partition, BOOT_VP);
    state::export(&vp, state).map_err(platform_failed)?;

    // Read alongside the architectural state, because it is not part of it:
    // the interrupt shadow is a property of where the guest stopped, and the
    // only place it exists is the partition. Bit 0 of
    // `WHvRegisterInterruptState` is the shadow; bit 1 is the NMI mask.
    let mut interrupt_state = [0u64; 1];
    vp.read_words(&[rusty_box_whp::Reg::InterruptState], &mut interrupt_state)
        .map_err(platform_failed)?;
    *shadowed = interrupt_state[0] & 1 != 0;

    cpu.import_arch_state(state).map_err(|error| {
        tracing::error!("the hypervisor returned a state this port refuses: {error:?}");
        CpuError::UnsupportedCpuOperation { operation: "hypervisor state refused on import" }
    })?;

    Ok(())
}

/// How a slice ended, and how much of its wall-clock time the guest was
/// actually executing for.
///
/// The two are not the same number, and treating them as one is what let a
/// guest outrun its own devices. A slice spends its wall clock on guest
/// execution AND on this engine's own work — the architectural state exchange
/// around a trapped instruction, a device answering a port, the shadow
/// finishing what the hardware handed back undone. None of that second part is
/// the guest running, so none of it may become guest time.
struct SliceOutcome {
    /// Why the processor came back.
    yielded: Yielded,
    /// Host time spent inside the platform's run call, and nowhere else.
    ran: Duration,
}

/// Run the processor, servicing what the platform cannot, until something the
/// machine owns has to happen.
fn run_until_the_machine_is_needed<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    deadline: Duration,
    budget: u64,
    ips: u64,
    counts: &mut ExitCounts,
) -> Result<SliceOutcome> {
    let mut ran = Duration::ZERO;
    let yielded = run_the_exit_loop(started, cpu, io, deadline, budget, ips, counts, &mut ran);
    // The alarm is armed per run inside the loop, so an error path can leave
    // one armed against a processor nobody is running. Disarming here is what
    // makes that impossible.
    started.alarm.disarm();
    yielded.map(|yielded| SliceOutcome { yielded, ran })
}

/// The exit loop proper, wrapped so the alarm is disarmed on every path out —
/// including an error.
fn run_the_exit_loop<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    deadline: Duration,
    budget: u64,
    ips: u64,
    counts: &mut ExitCounts,
    ran: &mut Duration,
) -> Result<Yielded> {
    loop {
        // Checked before running rather than after, so a slice whose budget is
        // already spent hands the machine back without another exit's worth of
        // guest time on top of it.
        //
        // Spent against the time the GUEST ran, not against the wall clock. A
        // slice dominated by exits — the IDE probe is nothing else — spends
        // most of its wall time in this engine rather than in the guest, and
        // charging that to the guest's budget ends the slice having executed
        // almost nothing while the machine's clock advances as though a full
        // slice had passed. That is how a disk seek modelled as three thousand
        // ticks was answered before its driver could poll once.
        //
        // NOT re-derived from the machine's countdown each time round, which
        // was tried and measured: that countdown does not decrement while a
        // slice runs — the machine is not ticking — so a small value at slice
        // entry stays small and clamps every iteration to the floor below,
        // turning every exit into its own 50-microsecond slice with a whole
        // architectural state exchange each way. It made the boot thirty times
        // slower and fixed nothing.
        // Spent in the unit the machine asked in. An exit is the one moment
        // this engine can stop the guest for free — it is already stopped —
        // and during any stretch that matters for device timing the guest is
        // exiting constantly, because polling a device IS a port exit. So the
        // budget is checked here, against guest time earned, and the alarm
        // below is left as the backstop for a guest that exits for nothing.
        //
        // Checking host time here instead is what let a slice overshoot by
        // three orders of magnitude: 45,000 ticks is 4.7 microseconds of host
        // time at this machine's rate, the floor rounds that to 50, and the
        // alarm — a condition variable, on an operating system whose timed
        // waits are granular to the millisecond — could not tell either from
        // four milliseconds. Every slice ran 836 times its budget, so every
        // device deadline inside it was serviced late, together, at the end.
        if ticks_elapsed(*ran, ips) >= budget {
            return Ok(Yielded::Budget);
        }
        let Some(left) = deadline.checked_sub(*ran).filter(|left| !left.is_zero()) else {
            return Ok(Yielded::Budget);
        };
        // The budget needs a voice that does not depend on the guest exiting:
        // a guest in a loop that touches no device and no unmapped page is
        // inside the platform's run call and never reaches the check above.
        // Armed per run rather than once around the loop, because what remains
        // of a budget denominated in guest time is known only here — an alarm
        // set once against the wall clock would fire while this engine was
        // servicing an exit, which is time the budget never bought.
        started.alarm.arm(std::time::Instant::now() + left);
        let entered = std::time::Instant::now();
        let exit = started.partition.run(BOOT_VP);
        *ran += entered.elapsed();
        // Disarmed before the error is propagated, so no path leaves an alarm
        // pointed at a processor that is no longer running. A cancellation
        // that lands between the run returning and this call is sticky and
        // would otherwise end the next run before it began.
        started.alarm.disarm();
        let exit = exit.map_err(platform_failed)?;
        // Counted before it is serviced, so the tally describes what the guest
        // asked for even when servicing it fails.
        match exit.reason {
            ExitReason::IoPortAccess(_) => counts.port += 1,
            ExitReason::MemoryAccess(_) => counts.memory += 1,
            ExitReason::Cpuid(_) => counts.cpuid += 1,
            ExitReason::MsrAccess(_) => counts.msr += 1,
            ExitReason::Halt => counts.halt += 1,
            ExitReason::Canceled { .. } => counts.canceled += 1,
            _ => {}
        }
        match exit.reason {
            // The one exit the platform pre-decodes: port, width, direction and
            // RAX all arrive in the exit, so no decoder is involved and the
            // machine's own device dispatch answers it directly.
            ExitReason::IoPortAccess(access) => {
                if access.string_op || access.rep_prefix {
                    // A string or repeated port access moves memory as well as
                    // a register, and the exit describes only the register.
                    // Finishing it from the exit alone would transfer one item
                    // and step over the rest — an IDE `REP INSW` reading a
                    // 512-byte sector would deliver two bytes and the guest
                    // would never know. The shadow runs the instruction whole,
                    // through the machine's own dispatch, exactly as the
                    // interpreter would.
                    finish_on_the_shadow(started, cpu, io, Trapped::Access)?;
                } else {
                    service_port_access(started, io, &exit, access)?;
                }
                // A device answering that write may have latched something on
                // the bus — an interrupt line, a hold request, a machine
                // boundary to service. The interpreter drains those at its own
                // next instruction boundary; this is that boundary here.
                io.sync_io_events(cpu);
            }
            // A halted processor is the machine's business: its HLT
            // fast-forward advances time to the next deadline and decides when
            // the guest may run again.
            ExitReason::Halt => return Ok(Yielded::Halted),
            // Someone asked for the processor back. Which someone matters:
            // past the deadline it is this slice's own alarm, which is the
            // budget running out and not an interruption.
            ExitReason::Canceled { .. } => {
                return Ok(if *ran >= deadline {
                    Yielded::Budget
                } else {
                    Yielded::Canceled
                });
            }
            // The platform documents this as one a run never returns.
            ExitReason::None => return Err(unserviced("the platform's own \"no reason\"", &exit)),
            // An access the partition's map does not answer: a device window,
            // a page the plan left out, or a write to a range mapped read-only.
            // The shadow finishes it, because nothing else can — see below.
            ExitReason::MemoryAccess(access) => {
                tracing::trace!(
                    "servicing a {:?} at gpa {:#x} on the shadow processor",
                    access.access,
                    access.gpa
                );
                finish_on_the_shadow(started, cpu, io, Trapped::Access)?;
            }
            // What the guest asked about the processor. Answered by executing
            // the instruction on the shadow, so the answer is this port's
            // model rather than the host's silicon — see the exit list above.
            ExitReason::Cpuid(access) => {
                let leaf = access.rax as u32;
                finish_on_the_shadow(started, cpu, io, Trapped::Cpuid { leaf })?;
            }
            ExitReason::MsrAccess(_) => {
                finish_on_the_shadow(started, cpu, io, Trapped::Access)?;
            }
            ExitReason::InterruptWindow => return Err(unserviced("interrupt window", &exit)),
            ExitReason::Exception => return Err(unserviced("exception", &exit)),
            ExitReason::Rdtsc => return Err(unserviced("RDTSC", &exit)),
            ExitReason::UnrecoverableException => {
                return Err(unserviced("unrecoverable exception", &exit))
            }
            ExitReason::InvalidVpRegisterValue => {
                return Err(unserviced("invalid processor register", &exit))
            }
            ExitReason::UnsupportedFeature { .. } => return Err(unserviced("unsupported feature", &exit)),
            ExitReason::ApicEoi { .. }
            | ExitReason::ApicSmiTrap
            | ExitReason::ApicInitSipiTrap
            | ExitReason::ApicWriteTrap
            | ExitReason::SynicSintDeliverable
            | ExitReason::Hypercall => return Err(unserviced("a partition-APIC exit", &exit)),
            ExitReason::Unrecognized(code) => {
                tracing::error!("the platform reported exit reason {code}, which is newer than this port");
                return Err(unserviced("an exit reason newer than this port", &exit));
            }
        }

        // Servicing that exit may have left the machine work only the machine
        // can do — most importantly a device timer, armed while the device was
        // answering, which raises the interrupt the guest is waiting for and
        // cannot fire until the machine has the processor back. Asked after
        // every serviced exit rather than only after a port write, because a
        // device reached through the shadow arms timers the same way.
        if cpu.wants_a_machine_boundary() || io.needs_boundary() {
            counts.boundary += 1;
            return Ok(Yielded::Boundary);
        }

        // Bring the bus's levels onto the processor for every exit, not only
        // for the port write that has its own drain above: a device reached
        // through the shadow raises interrupt lines the same way, and a line
        // nobody moved onto the processor is a line the guest never sees.
        io.sync_io_events(cpu);

        // An interrupt that has become deliverable ends the slice.
        //
        // This engine delivers only at the head of a slice, on the shadow, so
        // a vector raised mid-slice waits for whatever remains of it. Measured
        // on a DLX boot, that wait had a median of 158 microseconds of host
        // time — about 1.5 MILLION ticks of this machine's guest time, against
        // the one instruction boundary the interpreter takes. A disk driver
        // polls its status, finishes the command and tears down its handler
        // inside a window that size, and the interrupt then arrives for a
        // command nobody is waiting on: `hda: unexpected_intr`.
        //
        // Ending here costs one architectural state exchange and hands the
        // vector to the guest at the next slice entry, which is immediately.
        if cpu.has_an_event_to_deliver() {
            counts.boundary += 1;
            return Ok(Yielded::Boundary);
        }
    }
}

/// How many instructions a system-management handler may take before this
/// engine stops believing it is one.
///
/// The chipset's own handler is a few hundred instructions; a bound three
/// orders of magnitude above that costs a correct machine nothing and stops an
/// incorrect one from spinning inside a slice forever, where no timer of the
/// machine's could ever fire to end it.
const SMM_HANDLER_CEILING: u64 = 1_000_000;

/// Run the shadow until it leaves system-management mode.
///
/// SMM is the one mode this engine cannot hand to the hardware. No hypervisor
/// offers it, the state a processor saves on entry lives in SMRAM in a layout
/// the hardware would not produce, and `RSM` outside SMM is an invalid opcode —
/// so a partition given a half-entered handler executes it as ordinary code
/// and triple-faults, which is exactly what a Bochs BIOS does at
/// `SMBASE + 0x8000` once its chipset enables the SMI.
///
/// Running it on the shadow instead is not a workaround but the parity answer:
/// the handler executes on this port's own processor, against this port's own
/// SMRAM, exactly as it would with no hypervisor present. The guest is handed
/// back at `RSM`, in the state the handler left, and cannot tell.
///
/// # Errors
/// A fault the shadow could not take, or a handler that never returns.
fn run_the_shadow_out_of_smm<T: Instrumentation>(
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
) -> Result<()> {
    let mut executed = 0u64;
    while cpu.is_in_smm() {
        if executed >= SMM_HANDLER_CEILING {
            tracing::error!(
                "a system-management handler has run {SMM_HANDLER_CEILING} instructions \
                 without returning; the machine cannot be handed back mid-handler"
            );
            return Err(CpuError::UnsupportedCpuOperation {
                operation: "a system-management handler did not return",
            });
        }
        io.emulate_one(cpu)?;
        executed += 1;
    }
    Ok(())
}

/// What the guest was doing when the hardware handed it back.
///
/// Only `CPUID` needs telling apart: its answer is the one thing this engine
/// adjusts on the way out, because it is the one thing that describes what the
/// machine can do rather than reporting what it did.
#[derive(Clone, Copy, Debug)]
enum Trapped {
    /// A memory access, or a model-specific register. Whatever the shadow made
    /// of it is what the guest gets.
    Access,
    /// `CPUID` for this leaf.
    Cpuid { leaf: u32 },
}

/// Take the virtualization features out of an answer this engine cannot honour.
///
/// A guest on this engine cannot run a hypervisor of its own. The shadow
/// processor's nested-virtualization state — the VMCS and VMCB caches — has no
/// place in a `VcpuArchState` and cannot cross the seam, and the hardware would
/// refuse the instructions in any case. So the guest is not told it has them:
/// a machine that offers a feature it cannot honour is worse than one that
/// does not offer it.
///
/// Told otherwise, the first guest to notice is the firmware, and it notices
/// immediately. Bochs's own BIOS reads `CPUID.1:ECX[5]`, believes it, and
/// writes `IA32_FEATURE_CONTROL` to enable VMX — a write the platform refuses,
/// in firmware that has no interrupt descriptor table yet, which is a triple
/// fault before the boot loader has run a single instruction. That is how this
/// was found, at `0xE1E80` in `rombios32`.
///
/// Registered as divergence H3.
fn withhold_virtualisation_from(state: &mut VcpuArchState, leaf: u32) {
    /// `CPUID` leaves its answers in RAX, RBX, RCX, RDX; the state carries the
    /// register file in the processor's own order, where RCX is second.
    const RCX: usize = 1;
    /// `CPUID.1:ECX[5]`, Intel's VMX.
    const VMX: u64 = 1 << 5;
    /// `CPUID.80000001:ECX[2]`, AMD's SVM.
    const SVM: u64 = 1 << 2;

    match leaf {
        1 => state.gprs[RCX] &= !VMX,
        0x8000_0001 => state.gprs[RCX] &= !SVM,
        _ => {}
    }
}

/// Execute the trapped instruction on the shadow processor.
///
/// The reason a shadow processor is mandatory rather than an optimisation, and
/// the measurement is in `docs/whp-platform-probe-2026-08-27.md`: a memory
/// exit reports `InstructionLength = 0` and does NOT advance `RIP`, so there is
/// no stepping over it; and for a write to a read-only window it carries no
/// instruction bytes either, so there is nothing to decode from the exit. The
/// only thing that can finish it is a processor that reads the instruction out
/// of guest memory and executes it — which is this port's interpreter, running
/// one instruction against the very same machine parts.
///
/// That is also what makes the result right rather than merely possible, and
/// it is why the same treatment serves an access, a `CPUID` and an MSR alike.
/// The access lands through the machine's own routing, so a device window
/// answers exactly as it would under the interpreter; the `CPUID` answers out
/// of this port's own model rather than the host's silicon; the MSR reads and
/// writes this port's own register file. Bit-identical, which is the property
/// the whole design rests on.
///
/// The exchange around it is the whole architectural state each way. A trapped
/// instruction may read any register, and the shadow must be the processor,
/// not an approximation of it.
///
/// # Errors
/// A state the exchange refused, or a fault the shadow could not take.
fn finish_on_the_shadow<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    trapped: Trapped,
) -> Result<()> {
    read_back_into_the_shadow(started, cpu)?;
    // The trapped instruction, whole, on the machine's own dispatch path.
    // Whatever it does — completes the access, moves a sector, or raises a
    // fault and enters a handler — the processor it leaves behind is the one
    // the platform must continue from.
    io.finish_the_instruction(cpu)?;

    let Started { alarm: _, partition, state, installed: _, shadowed: _ } = started;
    cpu.export_arch_state(state);
    if let Trapped::Cpuid { leaf } = trapped {
        withhold_virtualisation_from(state, leaf);
    }
    state::import(&Vp::new(partition, BOOT_VP), state).map_err(platform_failed)
}

/// Answer a port access out of the machine's own device set, then step the
/// processor past the instruction that caused it.
fn service_port_access(
    started: &mut Started,
    io: &mut PcIo<'_>,
    exit: &Exit,
    access: rusty_box_whp::IoPortAccess,
) -> Result<()> {
    let ticks = io.pc_system.time_ticks();
    let port = access.port;
    let width = access.access_size;

    // The platform hands back RAX WHOLE, whatever the access width, so both
    // directions have to narrow it themselves. An `OUT DX, AL` that handed a
    // device the other three bytes of RAX would be telling it something the
    // guest never wrote, and the interpreter — whose handler passes `AL`,
    // `AX` or `EAX` and nothing else — would tell it something different for
    // the same guest instruction.
    let mask: u64 = match width {
        1 => 0xFF,
        2 => 0xFFFF,
        _ => 0xFFFF_FFFF,
    };

    let rax = if access.is_write {
        io.devices.outp(
            port,
            (access.rax & mask) as u32,
            width,
            ticks,
            io.pc_system,
            io.device_manager,
            io.memory,
        );
        access.rax
    } else {
        let value =
            io.devices
                .inp(port, width, ticks, io.pc_system, io.device_manager);
        // A narrower `IN` leaves the bytes above its width as the guest had
        // them — the same rule the interpreter's own `port_in` follows.
        (access.rax & !mask) | (u64::from(value) & mask)
    };

    let vp = Vp::new(&started.partition, BOOT_VP);
    // Unlike a memory exit, a port exit DOES report its instruction length and
    // does not advance RIP itself, so finishing it is arithmetic rather than a
    // decode (probe finding 2).
    let resume = exit.vp.rip + u64::from(exit.vp.instruction_length);
    vp.write_words(
        &[rusty_box_whp::Reg::Rip, rusty_box_whp::Reg::Rax],
        &[resume, rax],
    )
    .map_err(platform_failed)
}

/// How long a slice may run on the host, from what the machine asked for.
///
/// The machine sizes a slice by the ticks remaining to its next device
/// deadline. An interpreter honours that by counting instructions; this engine
/// counts none, so it honours the same number as a span of host time at the
/// machine's own rate — the same conversion `ticks_elapsed` performs in
/// reverse, and the reason both engines' timers fire at the same guest time.
///
/// A slice that ran past it would starve every timer in the machine, because a
/// device timer fires only when the machine has the processor back.
fn deadline(request: SliceRequest, ips: u64) -> Duration {
    host_time_for(request.instructions(), ips)
}

/// How long the hardware takes to cover `ticks` of this machine's guest time.
///
/// The inverse of [`ticks_elapsed`], and they must stay inverses: a slice that
/// ran longer than the time it reports lets the guest outrun its own devices,
/// and one that ran shorter leaves the machine waiting for time that has
/// already passed.
fn host_time_for(ticks: u64, ips: u64) -> Duration {
    if ips == 0 {
        // A machine with no rate cannot say how long a tick is; give the
        // slice this engine's own resolution and let the machine decide.
        return SLICE_RESOLUTION;
    }
    let rate = u128::from(ips).saturating_mul(u128::from(HARDWARE_SPEED));
    let nanos = u128::from(ticks).saturating_mul(1_000_000_000) / rate;
    let asked = Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX));
    asked.max(SLICE_RESOLUTION)
}

/// The shortest stretch this engine can actually run.
///
/// An exit costs about four microseconds on this host, so a slice asked to
/// last less than that cannot run one instruction, let alone one exit's worth
/// — it would return having advanced no guest time at all, and a machine whose
/// budget is denominated in guest time would ask again, forever. That is not
/// hypothetical: a machine asks for a one-tick slice whenever a device
/// deadline is already due, and one tick at 300 MHz is three nanoseconds.
///
/// So the floor is this engine stating its resolution rather than pretending
/// to a precision it does not have. A slice never runs SHORTER than the
/// machine asked for by more than the machine can measure, and never returns
/// claiming that no time passed when it ran.
const SLICE_RESOLUTION: Duration = Duration::from_micros(50);

/// How much faster the host's own processor is than the rate this machine
/// nominally runs at.
///
/// A machine states its speed as instructions per second, and a guest on a
/// hypervisor runs on silicon retiring several billion a second. Equating a
/// host second with `ips` ticks — which is what a plain conversion does —
/// therefore pins the guest to real time: a `step` asking for 66 ms of guest
/// time has to burn 66 ms of wall clock to report it, however little the
/// guest had to do, and a boot takes exactly as long as a real machine's boot
/// no matter how fast the host is. Measured before this factor existed: 355
/// guest-seconds in 360 host-seconds, dead on 1:1.
///
/// This is the one number that says the two engines keep time differently, and
/// they may: an interpreter's tick is an instruction it retired, and there is
/// nothing for a hardware engine to count, so it converts the time it spent
/// instead and says by how much the hardware outruns the nominal rate.
///
/// Deliberately an under-estimate. Claiming less than the truth costs only
/// speed; claiming more would let guest time run ahead of the work the guest
/// actually did, and a guest polling a device would see it answer late.
const HARDWARE_SPEED: u64 = 32;

/// Host nanoseconds as machine ticks.
///
/// Both engines denominate time in ticks at `ips`, so a device deadline armed
/// under one is the same deadline under the other and a snapshot crosses
/// between them unchanged (REPLAN decision 1). What differs is how a tick is
/// earned — see [`HARDWARE_SPEED`].
fn ticks_elapsed(elapsed: Duration, ips: u64) -> u64 {
    let nanos = u128::from(elapsed.as_nanos().min(u128::from(u64::MAX)));
    let rate = u128::from(ips).saturating_mul(u128::from(HARDWARE_SPEED));
    let ticks = nanos.saturating_mul(rate) / 1_000_000_000;
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tick is a unit of guest time at the machine's own rate, so a second of
    /// host time is `ips` ticks — the same number the software engine would
    /// have retired in that second by construction.
    #[test]
    fn host_time_becomes_ticks_at_the_machines_own_rate() {
        let per_second = 300_000_000 * HARDWARE_SPEED;
        assert_eq!(ticks_elapsed(Duration::from_secs(1), 300_000_000), per_second);
        assert_eq!(
            ticks_elapsed(Duration::from_millis(1), 300_000_000),
            per_second / 1_000
        );
        assert_eq!(ticks_elapsed(Duration::from_secs(0), 300_000_000), 0);
    }

    /// A slice shorter than one tick is no ticks, not a rounded-up one: time
    /// the guest did not have must never be credited to it, or a device
    /// deadline arrives early.
    #[test]
    fn a_stretch_too_short_to_be_a_tick_is_no_ticks() {
        // Chosen so a tick is a whole number of nanoseconds and the assertion
        // is about the rounding rule rather than about the example.
        let rate = 250_000;
        let nanos_per_tick = 1_000_000_000 / (rate * HARDWARE_SPEED);
        assert_eq!(nanos_per_tick, 125, "the example must divide exactly");
        assert_eq!(ticks_elapsed(Duration::from_nanos(1), rate), 0);
        assert_eq!(ticks_elapsed(Duration::from_nanos(nanos_per_tick - 1), rate), 0);
        assert_eq!(ticks_elapsed(Duration::from_nanos(nanos_per_tick), rate), 1);
    }

    /// A slice runs for the time it will then report, and never longer.
    ///
    /// The two conversions are inverses up to the nanosecond both truncate to,
    /// and the direction of that truncation is the load-bearing part: a slice
    /// that ran LONGER than the time it reports would let the guest outrun its
    /// own devices, so a whole nanosecond is dropped rather than rounded up.
    /// The shortfall is bounded by what one nanosecond is worth.
    #[test]
    fn a_slice_runs_for_the_time_it_will_report_and_never_longer() {
        let ips = 300_000_000;
        let per_nanosecond = ips * HARDWARE_SPEED / 1_000_000_000 + 1;
        for asked in [1_000_000u64, 20_000_000, 100_000_000] {
            let reported = ticks_elapsed(host_time_for(asked, ips), ips);
            assert!(
                reported <= asked,
                "a slice reported {reported} ticks for a span asked to be {asked}"
            );
            assert!(
                asked - reported <= per_nanosecond,
                "a slice asked for {asked} ticks reported {reported}, short by more \
                 than the nanosecond both conversions truncate to"
            );
        }
    }

    /// An implausibly long stretch saturates rather than wrapping a counter the
    /// whole machine's clock is derived from.
    #[test]
    fn an_absurd_stretch_saturates_rather_than_wrapping() {
        assert_eq!(ticks_elapsed(Duration::from_secs(u64::MAX), u64::MAX), u64::MAX);
    }
}
