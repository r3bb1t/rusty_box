//! What actually executes guest instructions.
//!
//! A machine schedules; an engine runs. The distinction matters because only
//! one of the two changes when the guest stops being interpreted and starts
//! executing on the host's own processor.
//!
//! Everything Bochs `main.cc` does around its CPU loop — choosing which
//! processor gets the next quantum, dividing elapsed ticks among them, draining
//! the interrupt fabric, servicing timers at a boundary — is machine work, and
//! is unchanged by who ran the instructions. The port's LAPICs, its 8259 pair
//! and its device models stay exactly where they are. What an engine owns is
//! narrower than it first looks: one processor, for one bounded stretch, given
//! the machine parts that stretch may touch.
//!
//! ## Why the request is a struct
//!
//! The four numbers below used to travel as three positional arguments to one
//! of two near-identical methods, and the choice between the methods was a
//! fourth argument in disguise. Two of the three were bare integers whose
//! meaning came from position alone — `1` at one call site and the processor
//! count at the other, with nothing on the page saying which was which (R0).

use super::{PcIo, Progress};
use crate::cpu::{cpu::BxCpuC, exec_ctx::ExecCtx, instrumentation::Instrumentation, Result};
use crate::iodev::irq::IoApicDelivery;
#[cfg(doc)]
use crate::memory::plan::MemoryPlan;
use rusty_box_core::EngineFault;

/// One bounded stretch of guest execution, as the machine asks for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliceRequest {
    /// Guest instructions this stretch may retire.
    pub(crate) instructions: u64,
    /// Whether that count is a ceiling the engine may not exceed, or merely a
    /// cap on a stretch that ends at its own boundary first.
    ///
    /// The machine sets it when the count was derived from a timer deadline:
    /// overshooting then is not a rounding error but a device firing late.
    pub(crate) strict: bool,
    /// How many processors share one tick of machine time.
    ///
    /// Bochs `main.cc` credits a full round of `BX_SMP_PROCESSORS` slices with
    /// one `BX_TICKN`, so a processor's own retired count is divided by this to
    /// reach machine time. One on a uniprocessor, where the two are the same.
    pub(crate) tick_denominator: u64,
    /// Whether the stretch ends at the first instruction-trace boundary.
    ///
    /// Bochs `cpu.cc` has two entry points for this reason: `cpu_run_trace`
    /// returns after one trace so the SMP scheduler can switch processors,
    /// while `cpu_loop` runs on. A uniprocessor has nothing to switch to.
    pub(crate) yield_after_one_trace: bool,
}

impl SliceRequest {
    /// Guest instructions this stretch may retire.
    #[must_use]
    pub const fn instructions(self) -> u64 {
        self.instructions
    }

    /// Whether the count is a ceiling the engine may not exceed.
    #[must_use]
    pub const fn is_strict(self) -> bool {
        self.strict
    }

    /// How many processors share one tick of machine time.
    #[must_use]
    pub const fn tick_denominator(self) -> u64 {
        self.tick_denominator
    }

    /// Whether the stretch ends at the first instruction-trace boundary.
    #[must_use]
    pub const fn yields_after_one_trace(self) -> bool {
        self.yield_after_one_trace
    }
}

/// The unit an engine's slices report progress in.
///
/// Fixed per engine rather than chosen per slice, because it follows from how
/// the engine runs the guest and not from what it was asked to do. A machine
/// reads it before accepting a budget: a budget denominated in a unit its
/// engine never reports is one that would never be spent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressUnit {
    /// The engine retires the guest's instructions itself and counts them.
    Instructions,
    /// The engine reports the guest time a stretch took.
    ///
    /// What an engine running the guest on the host's own processor can say:
    /// the hardware retires the instructions, and the shadow processor handed
    /// to the engine holds no tally of them for the machine to read.
    Ticks,
}

/// Who puts a pending event into the processor.
///
/// Exactly one of them may, and which one is a property of the engine rather
/// than of the machine. Acknowledging an interrupt is irreversible — `iac()`
/// takes it off the 8259 and there is no putting it back — so a machine and an
/// engine that both deliver do not merely duplicate work: they hand the guest
/// a vector nobody is expecting. Linux says so out loud
/// (`unexpected_intr`, then `Aiee, killing interrupt handler`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventDelivery {
    /// The machine delivers, between batches. What this port's own interpreter
    /// expects: its loop is entered with the event already in the processor.
    Machine,
    /// The engine delivers, at the head of every stretch it runs.
    ///
    /// What an engine running the guest on real hardware must do, because the
    /// hardware has neither this machine's 8259 pair nor its local APIC, and
    /// because delivering through the interpreter is what makes the guest's
    /// interrupt frame identical either way.
    Engine,
}

/// Where an I/O APIC delivery went.
///
/// Three outcomes and not two, because the third is the one that must not be
/// lost: a backend that refused the message leaves it undelivered, and a
/// machine that read the answer as "not delivered by me, so delivered by
/// someone" would strand an interrupt with nothing said. The machine matches
/// all three (R5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeliveryRoute {
    /// The machine writes the model's IRR as it does today.
    Model,
    /// The engine delivered it elsewhere; the machine does nothing.
    Backend,
    /// Nobody received it, and that is not an error.
    ///
    /// The message is left on the I/O APIC's stuck path exactly as
    /// [`Self::Refused`] leaves it, and no fault is raised — because a message
    /// no local APIC matches is a thing hardware does every day. Bochs
    /// `ioapic.cc service_ioapic` reads the same answer from
    /// `apic_bus_deliver_interrupt` returning false and simply leaves the entry
    /// pending; a machine that stopped instead would halt on a guest doing
    /// something legal. Linux's `unlock_ExtINT_logic` writes precisely such an
    /// entry — a logical destination holding the boot processor's *physical*
    /// APIC id, which is 0 on a uniprocessor and matches no logical id
    /// anywhere — and expects the acknowledge without the delivery.
    Undelivered,
    /// The engine's backend refused it. The machine leaves the message pending
    /// (the I/O APIC's stuck path) and returns the fault from the boundary.
    ///
    /// For a message the backend cannot carry AT ALL, which is a gap in this
    /// port rather than an ordinary outcome. A message simply not matched is
    /// [`Self::Undelivered`].
    Refused(EngineFault),
}

/// Whatever runs guest code for one processor.
///
/// Not dyn-compatible and deliberately so (R8): a machine knows its engine at
/// compile time, and monomorphising is what keeps the interpreter's slice entry
/// as direct as it was before there was an engine at all.
///
/// Unsealed, because an engine backed by a host hypervisor lives in its own
/// crate — a backend must never be something the machine crate depends on, so
/// the adapter that knows both sides is a third crate and implements this from
/// outside. That is what makes [`PcIo`]'s fields and `emulate_one` public:
/// an engine needs the machine's parts to service what its hardware could not
/// finish. Being unsealed binds this trait to the doctrine's rule for
/// user-extensible traits — it gains only defaulted methods from here on.
pub trait SliceEngine<T: Instrumentation> {
    /// The unit this engine's slices answer in. See [`ProgressUnit`].
    const PROGRESS_UNIT: ProgressUnit;

    /// Who delivers this processor's pending events. See [`EventDelivery`].
    ///
    /// Defaulted to the machine, which is what an engine that says nothing
    /// gets and what this port's interpreter has always had.
    const EVENT_DELIVERY: EventDelivery = EventDelivery::Machine;

    /// Run `cpu` against `io` for the stretch `request` describes, and report
    /// how far the guest got.
    ///
    /// In whichever unit this engine can answer in. An interpreter retires
    /// instructions and counts them. An engine running the guest on the host's
    /// own processor counts none — the hardware does the work, and the shadow
    /// processor it is handed here holds no tally the machine could read — so
    /// it reports the time the stretch took instead. The machine divides a
    /// round and arms its timers off that either way.
    ///
    /// # Errors
    /// Whatever ended the stretch other than its own budget: a fault the
    /// processor could not take, or an engine that failed underneath it.
    fn run_slice(
        &mut self,
        cpu: &mut BxCpuC<T>,
        io: PcIo<'_>,
        request: SliceRequest,
    ) -> Result<Progress>;

    /// The machine's guest-physical map has changed; install the new one.
    ///
    /// Called at the machine boundary that applies chipset effects, which is
    /// the one place a PAM flip, an SMRAM open, a BIOS-write enable or a
    /// relocated BAR becomes visible (doctrine R5). Derive the map with
    /// [`MemoryPlan::derive`] — an engine is handed the memory rather than the
    /// map, because only an engine that installs one needs to build it.
    ///
    /// Defaulted to doing nothing, and that is the honest default: this port's
    /// interpreter consults the routing on every access, so there is nothing
    /// for it to install and no staleness for it to have. An engine that gave
    /// the map to hardware once is the one that must be told — a BIOS
    /// shadowing itself flips PAM three times before jumping into the copy,
    /// and a guest left running against the map it was given at reset would
    /// execute the ROM it thought it had replaced.
    ///
    /// # Errors
    /// [`EngineFault`] when the new map could not be installed — normally
    /// [`Memory`](rusty_box_core::EngineFaultKind::Memory), carrying the
    /// backend's own code. A machine
    /// treats that as a boundary failure rather than continuing, because a
    /// guest running against a stale map is worse than a guest that stopped:
    /// it raises the stop through the same choke point as a refused pin edge,
    /// so a caller with no error channel of its own still stops.
    fn memory_map_changed(
        &mut self,
        memory: &mut crate::memory::BxMemC,
    ) -> core::result::Result<(), EngineFault> {
        let _ = memory;
        Ok(())
    }

    /// Where an I/O APIC delivery goes.
    ///
    /// Defaulted to [`DeliveryRoute::Model`] — this port's Local APICs live on
    /// its processors, and the machine writes them itself. An engine whose
    /// backend owns the Local APICs answers [`DeliveryRoute::Backend`] and
    /// takes the message; one whose backend refused it answers
    /// [`DeliveryRoute::Refused`], and the machine leaves the message pending
    /// and surfaces the fault from the boundary.
    fn route_ioapic_delivery(&mut self, delivery: IoApicDelivery) -> DeliveryRoute {
        let _ = delivery;
        DeliveryRoute::Model
    }

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
}

/// This port's own interpreter.
///
/// Stateless: everything it needs is the processor and the parts it is handed,
/// which is the same property that lets a machine hold its processors itself
/// and hand out one at a time. An engine backed by a hypervisor partition owns
/// that partition and is not stateless — which is exactly why the machine holds
/// an engine value rather than calling free functions.
#[derive(Clone, Copy, Debug, Default)]
pub struct SoftwareEngine;

impl<T: Instrumentation> SliceEngine<T> for SoftwareEngine {
    const PROGRESS_UNIT: ProgressUnit = ProgressUnit::Instructions;

    #[inline]
    fn run_slice(
        &mut self,
        cpu: &mut BxCpuC<T>,
        io: PcIo<'_>,
        request: SliceRequest,
    ) -> Result<Progress> {
        let mut ctx = ExecCtx::new(cpu, io);
        let retired = if request.yield_after_one_trace {
            ctx.cpu_run_trace_slice(request.instructions, request.strict, request.tick_denominator)
        } else {
            ctx.cpu_loop_n_slice(request.instructions, request.strict, request.tick_denominator)
        }?;
        Ok(Progress::Instructions(retired))
    }
}
