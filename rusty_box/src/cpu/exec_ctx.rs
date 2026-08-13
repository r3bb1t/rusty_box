//! The machine state one CPU needs while it executes, held as borrows.
//!
//! `BxCpuC` currently stores its bus access as raw pointers (`mem_bus`,
//! `io_bus`, `pc_system_ptr`) installed for the duration of a scheduler slice.
//! Those fields are what force `unsafe impl Send for Emulator`: a struct with a
//! pointer into its sibling fields is self-referential, so the compiler cannot
//! derive `Send` no matter how disciplined the wiring is.
//!
//! `ExecCtx` says the same thing with borrows. It is built per slice from
//! disjoint `Emulator` fields, so the borrow checker enforces the aliasing rule
//! the raw wiring documents in prose, and nothing needs to be stored on the CPU
//! at all.
//!
//! It `Deref`s to the CPU, so code written against `&mut BxCpuC` keeps reading
//! `self.rip()`, `self.get_gpr32(..)` and the rest verbatim; only the bus
//! accessors resolve to this type instead.

use core::ops::{Deref, DerefMut};

use super::{cpu::BxCpuC, instrumentation::Instrumentation};
use crate::{iodev::BxDevicesC, memory::BxMemC, memory::CpuTlbPin, pc_system::BxPcSystemC};

/// One CPU plus the machine it is executing against.
pub(crate) struct ExecCtx<'a, T: Instrumentation> {
    cpu: &'a mut BxCpuC<T>,
    /// Guest memory. Disjoint from `cpu`, so both can be held at once — the
    /// property the raw `mem_bus` pointer existed to fake.
    pub(crate) memory: &'a mut BxMemC,
    pub(crate) devices: &'a mut BxDevicesC,
    pub(crate) pc_system: &'a mut BxPcSystemC,
    /// Every CPU's eviction sidecar, including this one's. Shared, because the
    /// allocator only ever reads them.
    pub(crate) pins: &'a [CpuTlbPin],
    /// Index of this CPU's own sidecar within `pins`.
    pub(crate) pin_index: usize,
}

impl<'a, T: Instrumentation> ExecCtx<'a, T> {
    #[inline]
    pub(crate) fn new(
        cpu: &'a mut BxCpuC<T>,
        memory: &'a mut BxMemC,
        devices: &'a mut BxDevicesC,
        pc_system: &'a mut BxPcSystemC,
        pins: &'a [CpuTlbPin],
        pin_index: usize,
    ) -> Self {
        debug_assert!(pin_index < pins.len(), "cpu has no sidecar in the pin set");
        Self {
            cpu,
            memory,
            devices,
            pc_system,
            pins,
            pin_index,
        }
    }

    /// This CPU's own eviction sidecar.
    #[inline]
    pub(crate) fn pin(&self) -> &CpuTlbPin {
        &self.pins[self.pin_index]
    }

    /// The CPU and its memory at the same time.
    ///
    /// Handlers that translate an address and then touch the bytes need both
    /// halves live at once. Returning them together from disjoint fields is
    /// what makes that expressible without a stored pointer.
    #[inline]
    pub(crate) fn cpu_and_memory(&mut self) -> (&mut BxCpuC<T>, &mut BxMemC) {
        (self.cpu, self.memory)
    }

    /// The CPU with the device bus and PC system, mirroring the existing
    /// `io_and_pc_system_mut` shape used by port I/O.
    #[inline]
    pub(crate) fn cpu_io_and_pc_system(
        &mut self,
    ) -> (&mut BxCpuC<T>, &mut BxDevicesC, &mut BxPcSystemC) {
        (self.cpu, self.devices, self.pc_system)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use crate::emulator::{Emulator, EmulatorConfig};

    /// `Emulator` is several MiB; tests need an explicit stack.
    const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;

    #[test]
    fn one_borrow_of_the_machine_yields_cpu_and_bus_at_once() {
        // The claim under test is not that this returns the right values, but
        // that it COMPILES while holding all of them: CPU, memory, devices and
        // PC system live simultaneously out of one `&mut Emulator`. That is
        // exactly what the stored raw pointers fake today, and it is the thing
        // that has to be true before the bus fields can leave `BxCpuC`.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(|| {
                let mut emu = Emulator::new(EmulatorConfig::default()).unwrap();
                let mut ctx = emu.exec_ctx(0);

                // Reached through `Deref`, so every existing `self.<cpu thing>`
                // in a handler body keeps working unchanged.
                let rip = ctx.rip();

                // Memory is reachable in the same scope, with no pointer stored
                // on the CPU and nothing wired beforehand.
                ctx.memory.set_a20_mask(u64::MAX);

                // And both halves together, which is what any handler that
                // translates an address and then touches bytes requires.
                let (cpu, memory) = ctx.cpu_and_memory();
                cpu.install_memory_bases(memory);
                assert!(!cpu.mem_alloc_base.is_null());
                assert_eq!(cpu.rip(), rip, "deref must reach the same cpu");

                // The device bus and PC system come out of the same borrow.
                let (_cpu, _devices, pc_system) = ctx.cpu_io_and_pc_system();
                let _ = pc_system.get_enable_a20();

                // The sidecar set is shared while the CPU is mutable, which is
                // the split the eviction check needs.
                assert!(ctx.pins.len() >= 1);
                assert!(!ctx.pin().is_alloc_range_pinned(usize::MAX, usize::MAX));
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

impl<T: Instrumentation> Deref for ExecCtx<'_, T> {
    type Target = BxCpuC<T>;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.cpu
    }
}

impl<T: Instrumentation> DerefMut for ExecCtx<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.cpu
    }
}
