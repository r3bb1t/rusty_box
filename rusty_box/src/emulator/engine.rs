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
    /// The engine's backend refused it. The machine leaves the message pending
    /// (the I/O APIC's stuck path) and returns the fault from the boundary.
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

    /// The 8259's INT pin to the boot processor, as this boundary found it.
    ///
    /// Called at every boundary while the pin is ASSERTED, and once when it
    /// falls. Not once per transition: the pin is a level, this boundary is
    /// where the machine samples it, and two interrupts can arrive with no
    /// sample in between — the guest acknowledges the first on whatever thread
    /// is running it, a device raises the second, and the level never reads
    /// low. An engine told only about transitions would never hear of the
    /// second, and an engine that has to fetch a running processor out of its
    /// hardware to take a vector would owe that vector forever.
    ///
    /// **An engine that acts on this must dedup for itself.** The obligation
    /// is one act per vector, not one per boundary; a backend that cancelled a
    /// running processor on every call would cancel it thousands of times a
    /// second for one interrupt.
    ///
    /// Defaulted to nothing, which is what this port's interpreter needs: it
    /// reads the processor's event word, which the same boundary publishes.
    ///
    /// # Errors
    /// Whatever the engine's backend refused. The machine surfaces it from the
    /// boundary rather than continuing with an engine that missed the pin.
    fn pic_pin_changed(&mut self, asserted: bool) -> core::result::Result<(), EngineFault> {
        let _ = asserted;
        Ok(())
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
