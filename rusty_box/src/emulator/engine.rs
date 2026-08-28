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
#[cfg(doc)]
use crate::memory::plan::MemoryPlan;

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
    /// Whatever prevented the new map from being installed. A machine treats
    /// that as a boundary failure rather than continuing, because a guest
    /// running against a stale map is worse than a guest that stopped.
    fn memory_map_changed(&mut self, memory: &mut crate::memory::BxMemC) -> Result<()> {
        let _ = memory;
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
