//! The machine state one CPU needs while it executes, held as borrows.
//!
//! A CPU cannot reach its own bus. It is one field of a machine, and the
//! memory, devices and PC system it drives are siblings — so the thing that
//! executes an instruction is not the CPU alone but the CPU together with
//! those siblings, borrowed at once. That combination is this type.
//!
//! Saying it with borrows rather than with stored pointers is what makes the
//! aliasing rule checkable: a context is built per scheduler slice from
//! disjoint machine fields, so the borrow checker enforces what prose used to
//! promise, and a CPU sitting outside a slice has no bus to misuse.
//!
//! It `Deref`s to the CPU, so code written against `&mut BxCpuC` keeps reading
//! `self.rip()`, `self.get_gpr32(..)` and the rest verbatim; only the bus
//! accessors resolve to this type instead.
//!
//! Two hazards come with that `Deref`. A `BxCpuC` method named after a field
//! here (`memory`, `devices`, `pc_system`) would be silently shadowed, so those
//! names are reserved; and `Self::CONST` inside an `impl ExecCtx` block does
//! NOT reach the CPU's associated constants — `Deref` forwards methods, not
//! `Self` — so those are spelled `BxCpuC::<T>::CONST`.

use core::ops::{Deref, DerefMut};

use super::{cpu::BxCpuC, instrumentation::Instrumentation};
use crate::{
    emulator::PcIo,
    iodev::{devices::DeviceManager, BxDevicesC},
    memory::BxMemC,
    pc_system::BxPcSystemC,
};

/// One CPU plus the machine it is executing against.
pub(crate) struct ExecCtx<'a, T: Instrumentation> {
    cpu: &'a mut BxCpuC<T>,
    /// Guest memory. Disjoint from `cpu`, so both can be held at once — the
    /// property the raw `mem_bus` pointer existed to fake.
    pub(crate) memory: &'a mut BxMemC,
    pub(crate) devices: &'a mut BxDevicesC,
    /// The device models themselves. Disjoint from `devices`, which owns
    /// the port tables that route to them — port dispatch needs both.
    pub(crate) device_manager: &'a mut DeviceManager,
    pub(crate) pc_system: &'a mut BxPcSystemC,
    /// Guest RAM as one host slice, or null when residency is partial. The
    /// data TLB measures its cached page offsets from here.
    pub(crate) mem_host_base: *mut u8,
    pub(crate) mem_host_len: usize,
    /// The whole memory allocation — guest RAM, ROM and the bogus page. The
    /// instruction TLB, the fetch window and the VMCB measure from here,
    /// because fetch also runs out of ROM and out of relocated blocks.
    pub(crate) mem_alloc_base: *mut u8,
}

impl<'a, T: Instrumentation> ExecCtx<'a, T> {
    /// Borrow a CPU together with the machine it runs against.
    ///
    /// Both allocation bases are measured here, from this context's own
    /// `memory`, and they are measured together: the data TLB counts from the
    /// identity guest-RAM base and the instruction TLB from the allocation
    /// base, so a cached offset resolved against a base that was never taken
    /// is a wild pointer. Deriving them at assembly rather than storing them
    /// on the CPU is what makes a stale base unrepresentable — there is no
    /// window in which a context holds bases from a different memory.
    ///
    /// The machine parts arrive as one [`PcIo`] because they are one loan: what
    /// this pairs with a processor is the same group an execution engine that
    /// runs the guest on hardware borrows, and the two must not be able to
    /// disagree about what it contains.
    #[inline]
    pub(crate) fn new(cpu: &'a mut BxCpuC<T>, io: PcIo<'a>) -> Self {
        let PcIo {
            memory,
            devices,
            device_manager,
            pc_system,
        } = io;
        let (mem_host_base, mem_host_len) = memory.identity_guest_base();
        let (mem_alloc_base, _alloc_len) = memory.allocation_span();
        Self {
            cpu,
            memory,
            devices,
            device_manager,
            pc_system,
            mem_host_base,
            mem_host_len,
            mem_alloc_base,
        }
    }

    /// The instruction-fetch window as bytes.
    ///
    /// The returned lifetime is deliberately not tied to `&self`. The bytes
    /// live in the memory allocation, not in the CPU, and their validity rests
    /// on the window being set and on the residency epoch not having moved —
    /// not on any borrow of this context. Tying it to `&self` would forbid the
    /// fetch paths from touching `self.i_cache` while holding the window,
    /// which is exactly what they must do.
    #[inline(always)]
    pub(super) fn fetch_window_bytes<'w>(&self) -> Option<&'w [u8]> {
        let base = self.mem_alloc_base;
        self.eip_fetch_window.map(|w| {
            // SAFETY: the window is only ever set from a live directly-mapped
            // span, and `revalidate_allocation_caches` retires it whenever the
            // residency epoch moves, so the span still names those bytes.
            unsafe { core::slice::from_raw_parts(base.wrapping_add(w.start), w.len) }
        })
    }

    /// Everything a scheduler slice hands to the CPU loop, in one borrow.
    ///
    /// This is also the proof the whole design rests on: a handler that
    /// translates an address and then touches the bytes needs the CPU and
    /// memory live at once, and here they are, from disjoint machine fields.
    ///
    /// The scheduler previously rebuilt these from a raw pointer and a
    /// `from_raw_parts` because it could not name that split.
    #[inline]
    pub(crate) fn slice_parts(
        &mut self,
    ) -> (
        &mut BxCpuC<T>,
        &mut BxMemC,
        &mut BxDevicesC,
        &mut BxPcSystemC,
    ) {
        (self.cpu, self.memory, self.devices, self.pc_system)
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
    /// `devices` (~332 KiB) and `device_manager` (~788 KiB) are boxed so the
    /// struct stays small and, more importantly, so building one never has
    /// their sizes live in the frame at once. Together they take a machine
    /// to ~1.4 MiB, which overflows a default test thread stack as soon as a
    /// test builds one per loop iteration. Test-only; off every execution
    /// path.
    devices: alloc::boxed::Box<BxDevicesC>,
    device_manager: alloc::boxed::Box<DeviceManager>,
    pc_system: BxPcSystemC,
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
        Self::from_parts(cpu, memory)
    }

    /// A machine over caller-supplied memory, for tests that need a specific
    /// size, backing contents, or A20 setting.
    pub(crate) fn from_parts(cpu: alloc::boxed::Box<BxCpuC<()>>, memory: BxMemC) -> Self {
        Self {
            cpu,
            memory,
            devices: alloc::boxed::Box::new(BxDevicesC::new()),
            device_manager: alloc::boxed::Box::new(DeviceManager::new()),
            pc_system: BxPcSystemC::new(),
        }
    }

    /// Borrow the machine as an execution context.
    ///
    /// The A20 mask is taken here for the same reason `cpu_loop_n_impl` takes
    /// it at slice entry: it is CPU-side state mirroring a chipset line, and a
    /// test context that skipped it would mask guest addresses differently
    /// from a real slice.
    pub(crate) fn ctx(&mut self) -> ExecCtx<'_, ()> {
        self.cpu.a20_mask = self.memory.a20_mask();
        ExecCtx::new(
            &mut self.cpu,
            PcIo::new(
                &mut self.memory,
                &mut self.devices,
                &mut self.device_manager,
                &mut self.pc_system,
            ),
        )
    }

    /// The machine's memory, for a test that prepares or inspects it outside
    /// an execution context.
    pub(crate) fn memory_mut(&mut self) -> &mut BxMemC {
        &mut self.memory
    }

    /// The machine's PC system, for a test that arms timers or advances the
    /// clock before taking a context.
    pub(crate) fn pc_system_mut(&mut self) -> &mut BxPcSystemC {
        &mut self.pc_system
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
    f: impl FnOnce(&mut ExecCtx<'_, T>) -> R,
) -> R {
    // Boxed for the same reason `TestMachine` boxes them: together they are
    // over a megabyte, too much to place in a test's frame.
    let mut devices = alloc::boxed::Box::new(BxDevicesC::new());
    let mut device_manager = alloc::boxed::Box::new(DeviceManager::new());
    let mut pc_system = BxPcSystemC::new();
    let mut ctx = ExecCtx::new(
        cpu,
        PcIo::new(memory, &mut devices, &mut device_manager, &mut pc_system),
    );
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

                // Assembling the context is what takes the allocation bases —
                // no separate install step can be forgotten.
                assert!(!ctx.mem_alloc_base.is_null());

                // All four at once, the CPU and memory both mutable. A handler
                // that translates an address and then touches the bytes needs
                // exactly this, and it is the combination the raw wiring
                // existed to fake.
                let (cpu, memory, _devices, _pc_system) = ctx.slice_parts();
                memory.set_a20_mask(u64::MAX);
                assert_eq!(cpu.rip(), rip, "deref must reach the same cpu");
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
