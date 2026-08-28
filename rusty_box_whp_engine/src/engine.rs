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

use rusty_box_whp::{Exit, ExitReason, LocalApicMode, Partition, PartitionConfig, WhpError};

use super::state::{self, VpRegisters};
use super::Vp;
use rusty_box::cpu::arch_state::VcpuArchState;
use rusty_box::cpu::{cpu::BxCpuC, instrumentation::Instrumentation, CpuError, Result};
use rusty_box::emulator::{PcIo, Progress, ProgressUnit, SliceEngine, SliceRequest};
use rusty_box::memory::plan::MemoryPlan;

/// The processor this engine runs. SMP under a hypervisor is its own unit; a
/// machine with more processors than this refuses to start rather than running
/// one and pretending.
const BOOT_VP: u32 = 0;

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
                .map_err(platform_failed)?;
            let mut partition = config.setup().map_err(platform_failed)?;

            map_machine_memory(&mut partition, io)?;
            partition.create_processor(BOOT_VP).map_err(platform_failed)?;

            self.started = Some(Started { partition, state: VcpuArchState::default() });
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
}

/// Install the machine's guest-physical map into the partition.
///
/// The plan is derived from the machine's own memory, so the hypervisor and
/// this port's interpreter serve the same bytes at the same addresses — which
/// is what makes an exit serviced by the shadow processor land where the guest
/// expects it.
fn map_machine_memory(partition: &mut Partition, io: &mut PcIo<'_>) -> Result<()> {
    let plan = MemoryPlan::derive(io.memory()).map_err(|error| {
        tracing::error!("this machine has no stable guest-physical map: {error:?}");
        CpuError::UnsupportedCpuOperation {
            operation: "a partially resident machine has no map to install",
        }
    })?;

    for window in plan.windows() {
        let host = io
            .memory()
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
    Ok(())
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

    fn run_slice(
        &mut self,
        cpu: &mut BxCpuC<T>,
        mut io: PcIo<'_>,
        request: SliceRequest,
    ) -> Result<Progress> {
        let ips = io.pc_system.ips();
        let started = self.start(&mut io)?;

        // The shadow describes the processor; the platform runs it.
        cpu.export_arch_state(&mut started.state);
        let vp = Vp::new(&started.partition, BOOT_VP);
        state::import(&vp, &started.state).map_err(platform_failed)?;

        let began = std::time::Instant::now();
        let outcome = run_until_the_machine_is_needed(started, cpu, &mut io, request);

        // Whatever ended the run, the shadow must describe the processor again
        // before the machine looks at it: the scheduler reads activity state,
        // the interrupt fabric reads IF, and a snapshot reads all of it.
        let vp = Vp::new(&started.partition, BOOT_VP);
        state::export(&vp, &mut started.state).map_err(platform_failed)?;
        cpu.import_arch_state(&started.state).map_err(|error| {
            tracing::error!("the hypervisor returned a state this port refuses: {error:?}");
            CpuError::UnsupportedCpuOperation { operation: "hypervisor state refused on import" }
        })?;

        match outcome? {
            // Halting is not architectural state, so it does not arrive in the
            // exchange above; the platform reports it as the reason the run
            // ended, and the shadow is where the machine reads it.
            Yielded::Halted => cpu.record_halt(),
            Yielded::Canceled => {}
        }
        Ok(Progress::Ticks(ticks_elapsed(began.elapsed(), ips)))
    }
}

/// Run the processor, servicing what the platform cannot, until something the
/// machine owns has to happen.
fn run_until_the_machine_is_needed<T: Instrumentation>(
    started: &mut Started,
    _cpu: &mut BxCpuC<T>,
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
            }
            // A halted processor is the machine's business: its HLT
            // fast-forward advances time to the next deadline and decides when
            // the guest may run again.
            ExitReason::Halt => return Ok(Yielded::Halted),
            // The host asked for the processor back.
            ExitReason::Canceled { .. } => return Ok(Yielded::Canceled),
            // The platform documents this as one a run never returns.
            ExitReason::None => return Err(unserviced("the platform's own \"no reason\"")),
            ExitReason::MemoryAccess(_) => return Err(unserviced("memory access")),
            ExitReason::Cpuid(_) => return Err(unserviced("CPUID")),
            ExitReason::MsrAccess(_) => return Err(unserviced("MSR access")),
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
