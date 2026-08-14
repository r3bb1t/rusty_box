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

    /// Everything a scheduler slice hands to the CPU loop, in one borrow.
    ///
    /// This is also the proof the whole design rests on: a handler that
    /// translates an address and then touches the bytes needs the CPU and
    /// memory live at once, and here they are, from disjoint machine fields.
    ///
    /// The pin set is shared while the CPU and memory are mutable, which the
    /// borrow checker accepts only because all three are distinct fields of the
    /// machine. The scheduler previously rebuilt these from a raw pointer and a
    /// `from_raw_parts` because it could not name that split.
    #[inline]
    pub(crate) fn slice_parts(
        &mut self,
    ) -> (
        &mut BxCpuC<T>,
        &mut BxMemC,
        &mut BxDevicesC,
        &mut BxPcSystemC,
        &'a [CpuTlbPin],
        &'a CpuTlbPin,
    ) {
        (
            self.cpu,
            self.memory,
            self.devices,
            self.pc_system,
            self.pins,
            &self.pins[self.pin_index],
        )
    }

}

/// Machine parts for tests that drive instructions directly.
///
/// The dispatcher lives on [`ExecCtx`], so a test can no longer execute an
/// instruction against a bare `BxCpuC`. This owns the smallest machine that
/// satisfies the context and hands one out on demand — `ctx()` borrows from
/// `self`, which is why the parts must live in the caller's frame rather than
/// being returned alongside the context.
///
/// The CPU is still built exactly as these tests built it, so per-test model
/// selection and reset state are unchanged; only the surrounding bus is new.
#[cfg(test)]
pub(crate) struct TestMachine {
    cpu: alloc::boxed::Box<BxCpuC<()>>,
    memory: BxMemC,
    devices: BxDevicesC,
    pc_system: BxPcSystemC,
    pins: alloc::vec::Vec<CpuTlbPin>,
}

#[cfg(test)]
impl TestMachine {
    /// A machine whose CPU uses the default model.
    pub(crate) fn new() -> Self {
        Self::from_cpu(super::builder::BxCpuBuilder::new().build().unwrap())
    }

    /// A machine whose CPU uses `model`, matching `BxCpuBuilder::new_with_model`.
    pub(crate) fn with_model(model: super::CpuModel) -> Self {
        Self::from_cpu(
            super::builder::BxCpuBuilder::new_with_model(model)
                .build()
                .unwrap(),
        )
    }

    fn from_cpu(cpu: alloc::boxed::Box<BxCpuC<()>>) -> Self {
        const MIB: usize = 1024 * 1024;
        let memory = BxMemC::new(
            crate::memory::BxMemoryStubC::create_and_init(MIB, MIB, 4096).unwrap(),
            false,
        );
        let pins = alloc::vec![CpuTlbPin::new(&*cpu)];
        Self {
            cpu,
            memory,
            devices: BxDevicesC::new(),
            pc_system: BxPcSystemC::new(),
            pins,
        }
    }

    /// Borrow the machine as an execution context.
    pub(crate) fn ctx(&mut self) -> ExecCtx<'_, ()> {
        ExecCtx::new(
            &mut self.cpu,
            &mut self.memory,
            &mut self.devices,
            &mut self.pc_system,
            &self.pins,
            0,
        )
    }
}

/// Run `f` against a context built over a caller-supplied CPU and memory.
///
/// For tests that prepare their own guest memory — page tables, specific
/// contents — and so cannot use [`TestMachine`]'s. The device bus and PC system
/// are supplied here because the context requires them, not because these tests
/// use them.
#[cfg(test)]
pub(crate) fn exec_with<T: Instrumentation, R>(
    cpu: &mut BxCpuC<T>,
    memory: &mut BxMemC,
    pins: &[CpuTlbPin],
    f: impl FnOnce(&mut ExecCtx<'_, T>) -> R,
) -> R {
    let mut devices = BxDevicesC::new();
    let mut pc_system = BxPcSystemC::new();
    let mut ctx = ExecCtx::new(cpu, memory, &mut devices, &mut pc_system, pins, 0);
    f(&mut ctx)
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

                // Memory and the device bus are reachable in the same scope,
                // with no pointer stored on the CPU and nothing wired first.
                ctx.memory.set_a20_mask(u64::MAX);
                let _ = ctx.pc_system.get_enable_a20();

                // All four at once — the CPU and memory mutable, the pin set
                // shared. A handler that translates an address and then touches
                // the bytes needs exactly this, and it is the combination the
                // raw wiring existed to fake.
                let (cpu, memory, _devices, _pc_system, pins, current_pin) = ctx.slice_parts();
                cpu.install_memory_bases(memory);
                assert!(!cpu.mem_alloc_base.is_null());
                assert_eq!(cpu.rip(), rip, "deref must reach the same cpu");
                assert!(!pins.is_empty());
                assert!(
                    core::ptr::eq(current_pin, &pins[0]),
                    "cpu 0's sidecar is the first in the set"
                );
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
