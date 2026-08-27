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

use super::PcIo;
use crate::cpu::{cpu::BxCpuC, exec_ctx::ExecCtx, instrumentation::Instrumentation, Result};

/// One bounded stretch of guest execution, as the machine asks for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SliceRequest {
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

/// Whatever runs guest code for one processor.
///
/// Not dyn-compatible and deliberately so (R8): a machine knows its engine at
/// compile time, and monomorphising is what keeps the interpreter's slice entry
/// as direct as it was before there was an engine at all.
pub(crate) trait SliceEngine<T: Instrumentation> {
    /// Run `cpu` against `io` for the stretch `request` describes, and report
    /// how many instructions it retired.
    ///
    /// # Errors
    /// Whatever ended the stretch other than its own budget: a fault the
    /// processor could not take, or an engine that failed underneath it.
    fn run_slice(
        &mut self,
        cpu: &mut BxCpuC<T>,
        io: PcIo<'_>,
        request: SliceRequest,
    ) -> Result<u64>;
}

/// This port's own interpreter.
///
/// Stateless: everything it needs is the processor and the parts it is handed,
/// which is the same property that lets a machine hold its processors itself
/// and hand out one at a time. An engine backed by a hypervisor partition owns
/// that partition and is not stateless — which is exactly why the machine holds
/// an engine value rather than calling free functions.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SoftwareEngine;

impl<T: Instrumentation> SliceEngine<T> for SoftwareEngine {
    #[inline]
    fn run_slice(
        &mut self,
        cpu: &mut BxCpuC<T>,
        io: PcIo<'_>,
        request: SliceRequest,
    ) -> Result<u64> {
        let mut ctx = ExecCtx::new(cpu, io);
        if request.yield_after_one_trace {
            ctx.cpu_run_trace_slice(request.instructions, request.strict, request.tick_denominator)
        } else {
            ctx.cpu_loop_n_slice(request.instructions, request.strict, request.tick_denominator)
        }
    }
}
