use alloc::boxed::Box;
use alloc::vec::Vec;

use super::scalar_io::PendingScalarIo;
use super::{EntryMode, MemorySession, PhysicalMemory, ScalarIoRequest, SessionRun};
use crate::cpu::builder::BxCpuBuilder;
use crate::cpu::exec_ctx::ExecCtx;
use crate::cpu::registers::Registers;
use crate::cpu::{BreakpointRefusal, BxCpuC, CodeBreakpoint, CpuError, Instrumentation};
use crate::memory::MemoryError;
use crate::memory::code_writes::CodeWrites;
use crate::memory::execution::ExecutionMemory;
use crate::memory::flat::{FlatMemory, FlatParts};
use crate::memory::mapped::MemoryLoan;

/// Firmware-free x86 execution with owned dense RAM and the real interpreter.
///
/// Operations lend short views of the owned state. RAM, code stamps, and the
/// processor stay paired so translations and decoded traces can survive runs.
/// Use [`MemorySession`] for caller-owned storage and sparse memory.
pub struct CpuEmulator<T: Instrumentation = ()> {
    cpu: Box<BxCpuC<T>>,
    ram: Vec<u8>,
    stamps: Vec<u32>,
    writes: CodeWrites,
    pending: Option<PendingScalarIo>,
}

impl CpuEmulator {
    pub fn builder() -> CpuEmulatorBuilder {
        CpuEmulatorBuilder {
            ram_len: 64 * 1024,
            mode: EntryMode::Protected32,
            entry: 0x1000,
            stack: None,
        }
    }
}

impl<T: Instrumentation> CpuEmulator<T> {
    /// Inspect the installed observer between execution calls.
    pub fn instrumentation(&self) -> &T {
        &self.cpu.instrumentation.tracer
    }

    pub fn regs(&mut self) -> Registers<'_, T> {
        Registers::with_pending(&mut self.cpu, &mut self.pending)
    }

    pub fn memory(&mut self) -> PhysicalMemory<'_> {
        PhysicalMemory {
            memory: ExecutionMemory::Borrowed(FlatParts::dense(
                &mut self.ram,
                &mut self.stamps,
                &mut self.writes,
            )),
            pending: &mut self.pending,
        }
    }

    /// Run for an engine dispatch budget and report the stopping boundary.
    pub fn run(&mut self, dispatch_budget: u64) -> SessionRun {
        self.with_session(|session| session.run_controlled(dispatch_budget))
    }

    /// Use the one-dispatch semantics documented on [`MemorySession::step`].
    pub fn step(&mut self) -> SessionRun {
        self.run(1)
    }

    /// Reset the processor into a firmware-free layout using the existing RAM.
    /// See [`MemorySession::reset_at`] for entry and stack semantics.
    pub fn reset_at(&mut self, mode: EntryMode, entry: u32, stack: u32) -> crate::Result<()> {
        self.with_session(|session| session.reset_at(mode, entry, stack))
    }

    /// Handle the preflighted scalar port exit saved by the previous run.
    pub fn service_scalar_io(
        &mut self,
        service: impl FnOnce(ScalarIoRequest) -> Option<u32>,
    ) -> super::Result<bool> {
        self.with_session(|session| session.service_scalar_io(service))
    }

    /// Replace the persistent code breakpoint set.
    pub fn set_code_breakpoints(
        &mut self,
        breakpoints: &[CodeBreakpoint],
    ) -> Result<(), BreakpointRefusal> {
        self.with_session(|session| session.set_code_breakpoints(breakpoints))
    }

    /// Discard cached translation state after editing guest page tables.
    pub fn invalidate_translations(&mut self) {
        self.with_session(|session| session.invalidate_translations())
    }

    fn with_session<R>(&mut self, use_session: impl FnOnce(&mut MemorySession<'_, T>) -> R) -> R {
        let memory = MemoryLoan(FlatParts::dense(
            &mut self.ram,
            &mut self.stamps,
            &mut self.writes,
        ));
        let mut ctx = ExecCtx::owned_flat(&mut self.cpu, memory);
        ctx.pending_scalar_io = self.pending.take();
        let mut session = MemorySession { ctx };
        let result = use_session(&mut session);
        self.pending = session.ctx.pending_scalar_io.take();
        result
    }
}

/// Dense RAM and entry-state configuration for [`CpuEmulator`].
#[derive(Clone, Debug)]
pub struct CpuEmulatorBuilder {
    ram_len: usize,
    mode: EntryMode,
    entry: u32,
    stack: Option<u32>,
}

impl CpuEmulatorBuilder {
    pub fn ram(mut self, bytes: usize) -> Self {
        self.ram_len = bytes;
        self
    }

    pub fn mode(mut self, mode: EntryMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn entry(mut self, offset: u32) -> Self {
        self.entry = offset;
        self
    }

    /// Initial stack pointer. The default uses the top of RAM, aligned down
    /// to 16 bytes and capped within the entry mode's stack width.
    pub fn stack(mut self, offset: u32) -> Self {
        self.stack = Some(offset);
        self
    }

    pub fn build(self) -> crate::Result<CpuEmulator> {
        self.build_with_tracer(())
    }

    pub fn build_with_tracer<T: Instrumentation>(self, tracer: T) -> crate::Result<CpuEmulator<T>> {
        let ram_end = self.ram_len as u64;
        let max_offset = match self.mode {
            EntryMode::Real16 => u64::from(u16::MAX),
            _ => u64::from(u32::MAX),
        };
        let stack = self.stack.unwrap_or((ram_end.min(max_offset) & !15) as u32);
        if ram_end > (u64::from(u32::MAX) + 1)
            || u64::from(self.entry) >= ram_end
            || u64::from(self.entry) > max_offset
            || u64::from(stack) > ram_end.min(max_offset)
        {
            return Err(CpuError::UnsupportedCpuOperation {
                operation: "CPU-only RAM, entry or stack exceeds the configured address range",
            }
            .into());
        }

        let mut ram = Vec::new();
        ram.try_reserve_exact(self.ram_len)
            .map_err(|_| MemoryError::UnableToAllocateGuestMemory(self.ram_len))?;
        ram.resize(self.ram_len, 0);
        let stamp_count = FlatMemory::required_stamps(self.ram_len);
        let mut stamps = Vec::new();
        stamps.try_reserve_exact(stamp_count).map_err(|_| {
            MemoryError::UnableToAllocateGuestMemory(stamp_count * size_of::<u32>())
        })?;
        stamps.resize(stamp_count, 0);

        let mut emulator = CpuEmulator {
            cpu: BxCpuBuilder::new().build_with_tracer(tracer)?,
            ram,
            stamps,
            writes: CodeWrites::new(),
            pending: None,
        };
        emulator.reset_at(self.mode, self.entry, stack)?;
        Ok(emulator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BxPhyAddress;
    use crate::cpu::registers::{EAX, ECX};

    #[test]
    fn dense_owner_reborrows_keep_the_fetch_window_and_track_host_code_writes() {
        let mut emu = CpuEmulator::builder().build().unwrap();
        // inc eax; jmp back to inc
        emu.memory()
            .write_phys(BxPhyAddress::new(0x1000), &[0x40, 0xEB, 0xFD])
            .unwrap();
        assert_eq!(emu.run(2).instructions, 2);
        assert_eq!(emu.regs().read(EAX), 1);

        emu.regs().write(ECX, 7);
        emu.with_session(|session| assert!(session.ctx.eip_fetch_window.is_some()));

        // Keep the same instruction address and backing, replacing INC with DEC.
        emu.memory()
            .write_phys(BxPhyAddress::new(0x1000), &[0x48])
            .unwrap();
        assert_eq!(emu.run(2).instructions, 2);
        assert_eq!(emu.regs().read(EAX), 0);
    }
}
