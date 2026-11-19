//! CPU-only x86 execution over a caller's physical memory loan.

use super::exec_ctx::ExecCtx;
use super::{BxCpuC, Instrumentation, Result};
use crate::config::BxPhyAddress;
use crate::memory::mapped::MemoryLoan;

/// An exclusive loan of a processor and its physical memory map.
///
/// The caller supplies an initialized CPU and its current register state.
/// Reset and processor placement use the existing CPU APIs. Keep the session
/// across consecutive runs to reuse translations and decoded traces. Ending
/// it releases both borrows. A new session invalidates host mappings and
/// decoded code even if the arena was reused at the same address.
pub struct MemorySession<'a, T: Instrumentation = ()> {
    ctx: ExecCtx<'a, T>,
}

impl<'a, T: Instrumentation> MemorySession<'a, T> {
    pub fn new(cpu: &'a mut BxCpuC<T>, memory: MemoryLoan<'a>) -> Self {
        Self {
            ctx: ExecCtx::borrowed(cpu, memory),
        }
    }

    /// Run for up to `dispatch_budget` instruction dispatches. The return
    /// value is the engine's instruction count delta. REP elements and guest
    /// exception restarts follow the existing Bochs accounting conventions.
    /// HLT, an observer stop or a machine boundary may end a run early.
    /// Unbacked fetch returns `CpuError::VetoedDirectRead` before the
    /// instruction completes. To replace the map, end the session first.
    /// See [`super::CpuError`] for CPU-only exits and retirement semantics.
    pub fn run(&mut self, dispatch_budget: u64) -> Result<u64> {
        self.ctx.cpu_loop_n_slice(dispatch_budget, true, 1)
    }

    pub fn cpu(&self) -> &BxCpuC<T> {
        &self.ctx
    }

    /// Update writable physical RAM between runs using the engine's code
    /// stamp tracking. The whole write must fit in one RAM window. Updates
    /// to guest page tables still require guest TLB invalidation.
    pub fn write_ram(&mut self, address: BxPhyAddress, data: &[u8]) -> crate::Result<()> {
        self.ctx.memory.write_ram(address, data).map(|_| ())
    }
}
