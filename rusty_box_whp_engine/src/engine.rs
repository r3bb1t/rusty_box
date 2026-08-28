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

use super::state::{self, VpRegisters};
use super::Vp;
use rusty_box::cpu::arch_state::VcpuArchState;
use rusty_box::cpu::{cpu::BxCpuC, instrumentation::Instrumentation, CpuError, Result};
use rusty_box::emulator::{PcIo, Progress, ProgressUnit, SliceEngine, SliceRequest};
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
fn unserviced(what: &'static str) -> CpuError {
    tracing::error!("WHP exit not serviced by this engine: {what}");
    CpuError::UnsupportedCpuOperation { operation: "WHP exit not serviced by this engine" }
}

fn platform_failed(error: WhpError) -> CpuError {
    tracing::error!("WHP platform call failed: {error}");
    CpuError::UnsupportedCpuOperation { operation: "the hypervisor refused" }
}

/// A partition that has been configured, given memory and given a processor.
struct Started {
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
}

impl WhpEngine {
    /// Build the partition, map the machine's memory into it and create the
    /// processor — once, on the first slice.
    fn start(&mut self, io: &mut PcIo<'_>) -> Result<&mut Started> {
        if self.started.is_none() {
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

            self.started =
                Some(Started { partition, state: VcpuArchState::default(), installed });
        }
        // Established just now, or by an earlier slice. Reported rather than
        // asserted: a library says what it cannot do instead of ending the
        // host's process to say it.
        self.started.as_mut().ok_or(CpuError::UnsupportedCpuOperation {
            operation: "the partition did not start",
        })
    }
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
        if cpu.has_an_event_to_deliver() {
            io.emulate_one(cpu)?;
        }

        let started = self.start(&mut io)?;

        // The shadow describes the processor; the platform runs it.
        install_the_shadow(started, cpu)?;

        let began = std::time::Instant::now();
        let outcome = run_until_the_machine_is_needed(started, cpu, &mut io, request);

        // Whatever ended the run, the shadow must describe the processor again
        // before the machine looks at it: the scheduler reads activity state,
        // the interrupt fabric reads IF, and a snapshot reads all of it.
        read_back_into_the_shadow(started, cpu)?;

        match outcome? {
            // Halting is not architectural state, so it does not arrive in the
            // exchange above; the platform reports it as the reason the run
            // ended, and the shadow is where the machine reads it.
            Yielded::Halted => cpu.record_halt(),
            // The request is already on the processor, where the scheduler
            // takes it the moment this slice returns; ending the slice is the
            // whole of what this engine owes it.
            Yielded::Canceled | Yielded::Boundary => {}
        }
        Ok(Progress::Ticks(ticks_elapsed(began.elapsed(), ips)))
    }
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
    let Started { partition, state, installed: _ } = started;
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
    let Started { partition, state, installed: _ } = started;
    state::export(&Vp::new(partition, BOOT_VP), state).map_err(platform_failed)?;
    cpu.import_arch_state(state).map_err(|error| {
        tracing::error!("the hypervisor returned a state this port refuses: {error:?}");
        CpuError::UnsupportedCpuOperation { operation: "hypervisor state refused on import" }
    })
}

/// Run the processor, servicing what the platform cannot, until something the
/// machine owns has to happen.
fn run_until_the_machine_is_needed<T: Instrumentation>(
    started: &mut Started,
    cpu: &mut BxCpuC<T>,
    io: &mut PcIo<'_>,
    _request: SliceRequest,
) -> Result<Yielded> {
    loop {
        let exit = started.partition.run(BOOT_VP).map_err(platform_failed)?;
        match exit.reason {
            // The one exit the platform pre-decodes: port, width, direction and
            // RAX all arrive in the exit, so no decoder is involved and the
            // machine's own device dispatch answers it directly.
            ExitReason::IoPortAccess(access) => {
                service_port_access(started, io, &exit, access)?;
                // A device answering that write may have latched something on
                // the bus — an interrupt line, a hold request, a machine
                // boundary to service. The interpreter drains those at its own
                // next instruction boundary; this is that boundary here.
                io.sync_io_events(cpu);
                if cpu.wants_a_machine_boundary() {
                    return Ok(Yielded::Boundary);
                }
            }
            // A halted processor is the machine's business: its HLT
            // fast-forward advances time to the next deadline and decides when
            // the guest may run again.
            ExitReason::Halt => return Ok(Yielded::Halted),
            // The host asked for the processor back.
            ExitReason::Canceled { .. } => return Ok(Yielded::Canceled),
            // The platform documents this as one a run never returns.
            ExitReason::None => return Err(unserviced("the platform's own \"no reason\"")),
            // An access the partition's map does not answer: a device window,
            // a page the plan left out, or a write to a range mapped read-only.
            // The shadow finishes it, because nothing else can — see below.
            ExitReason::MemoryAccess(access) => {
                tracing::trace!(
                    "servicing a {:?} at gpa {:#x} on the shadow processor",
                    access.access,
                    access.gpa
                );
                finish_on_the_shadow(started, cpu, io)?;
            }
            // What the guest asked about the processor. Answered by executing
            // the instruction on the shadow, so the answer is this port's
            // model rather than the host's silicon — see the exit list above.
            ExitReason::Cpuid(_) | ExitReason::MsrAccess(_) => {
                finish_on_the_shadow(started, cpu, io)?;
            }
            ExitReason::InterruptWindow => return Err(unserviced("interrupt window")),
            ExitReason::Exception => return Err(unserviced("exception")),
            ExitReason::Rdtsc => return Err(unserviced("RDTSC")),
            ExitReason::UnrecoverableException => {
                return Err(unserviced("unrecoverable exception"))
            }
            ExitReason::InvalidVpRegisterValue => {
                return Err(unserviced("invalid processor register"))
            }
            ExitReason::UnsupportedFeature { .. } => return Err(unserviced("unsupported feature")),
            ExitReason::ApicEoi { .. }
            | ExitReason::ApicSmiTrap
            | ExitReason::ApicInitSipiTrap
            | ExitReason::ApicWriteTrap
            | ExitReason::SynicSintDeliverable
            | ExitReason::Hypercall => return Err(unserviced("a partition-APIC exit")),
            ExitReason::Unrecognized(code) => {
                tracing::error!("the platform reported exit reason {code}, which is newer than this port");
                return Err(unserviced("an exit reason newer than this port"));
            }
        }
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
) -> Result<()> {
    read_back_into_the_shadow(started, cpu)?;
    // One instruction, on the machine's own dispatch path. Whatever it does —
    // completes the access, or raises a fault and enters a handler — the
    // processor it leaves behind is the one the platform must continue from.
    io.emulate_one(cpu)?;
    install_the_shadow(started, cpu)
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

    let rax = if access.is_write {
        io.devices.outp(
            port,
            access.rax as u32,
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
        // The platform hands back RAX whole, so a narrower `IN` leaves the
        // bytes above its width as the guest had them — the same rule the
        // interpreter's own `port_in` follows.
        let mask = match width {
            1 => 0xFF,
            2 => 0xFFFF,
            _ => 0xFFFF_FFFF,
        };
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

/// Host nanoseconds as machine ticks, at the machine's own rate.
///
/// The conversion REPLAN decision 1 names: both engines denominate time in
/// ticks at `ips`, so a device deadline armed under one is the same deadline
/// under the other and a snapshot crosses between them unchanged.
fn ticks_elapsed(elapsed: Duration, ips: u64) -> u64 {
    let nanos = u128::from(elapsed.as_nanos().min(u128::from(u64::MAX)));
    let ticks = nanos.saturating_mul(u128::from(ips)) / 1_000_000_000;
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
        assert_eq!(ticks_elapsed(Duration::from_secs(1), 300_000_000), 300_000_000);
        assert_eq!(ticks_elapsed(Duration::from_millis(1), 300_000_000), 300_000);
        assert_eq!(ticks_elapsed(Duration::from_secs(0), 300_000_000), 0);
    }

    /// A slice shorter than one tick is no ticks, not a rounded-up one: time
    /// the guest did not have must never be credited to it, or a device
    /// deadline arrives early.
    #[test]
    fn a_stretch_too_short_to_be_a_tick_is_no_ticks() {
        assert_eq!(ticks_elapsed(Duration::from_nanos(1), 1_000_000), 0);
        assert_eq!(ticks_elapsed(Duration::from_nanos(999), 1_000_000), 0);
        assert_eq!(ticks_elapsed(Duration::from_nanos(1_000), 1_000_000), 1);
    }

    /// An implausibly long stretch saturates rather than wrapping a counter the
    /// whole machine's clock is derived from.
    #[test]
    fn an_absurd_stretch_saturates_rather_than_wrapping() {
        assert_eq!(ticks_elapsed(Duration::from_secs(u64::MAX), u64::MAX), u64::MAX);
    }
}
