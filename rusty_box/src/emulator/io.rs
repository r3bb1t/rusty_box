//! A machine's parts, minus its processors.
//!
//! A processor holds none of the machine it runs against: memory, the port bus,
//! the device models and the PC system are its siblings, four separate fields of
//! the same machine. Every path that executes guest code needs all four at once,
//! and until now each said so by listing them — five positional arguments into
//! [`ExecCtx::new`](crate::cpu::exec_ctx::ExecCtx::new), one per field.
//!
//! Naming the group once is what lets a machine *lend* it. This port's own
//! interpreter takes it together with a `BxCpuC` and becomes an `ExecCtx`; an
//! execution engine that runs the guest on real hardware never sees a `BxCpuC`
//! at all, and reaches back through exactly these four whenever a guest access
//! leaves it. Both are the same loan, so both are spelled the same way.
//!
//! It is a value holding four borrows rather than a borrow of a struct, because
//! the four are not stored together — the machine owns them as separate fields,
//! which is precisely what makes them borrowable at once (doctrine R3: the parts
//! are assembled by a machine destructuring its own `&mut self`, and by nothing
//! else).

use crate::{
    cpu::{cpu::BxCpuC, exec_ctx::ExecCtx, instrumentation::Instrumentation},
    iodev::{devices::DeviceManager, BxDevicesC},
    memory::BxMemC,
    pc_system::BxPcSystemC,
};

/// Everything a processor executes against, borrowed from one machine.
pub struct PcIo<'a> {
    /// Guest memory, including the routing that decides which accesses are
    /// served from RAM and which belong to a device.
    pub memory: &'a mut BxMemC,
    /// The port bus: which slot answers for a port, and the levels and requests
    /// a dispatch latched.
    pub devices: &'a mut BxDevicesC,
    /// The device models themselves. Disjoint from `devices`, which owns the
    /// tables that route to them — a port dispatch needs both.
    pub device_manager: &'a mut DeviceManager,
    /// Timers, the tick counter and the A20 gate.
    pub pc_system: &'a mut BxPcSystemC,
    /// Nobody outside the machine may assemble one of these.
    ///
    /// The fields are public because a port dispatch needs three of them at
    /// once, and an accessor per field cannot hand out three simultaneous
    /// borrows — that simultaneity is the entire reason this type exists.
    /// Construction is what stays closed (R3, and the doctrine's private-field
    /// rule): the parts are assembled by a machine destructuring its own
    /// `&mut self`, and by nothing else.
    #[expect(
        dead_code,
        reason = "its presence is the assertion — a struct with a private field cannot be built from outside"
    )]
    assembled_by_a_machine: (),
}

impl<'a> PcIo<'a> {
    /// Guest memory, including the routing that decides which accesses are
    /// served from RAM and which belong to a device.
    #[inline]
    pub fn memory(&mut self) -> &mut BxMemC {
        self.memory
    }

    /// The port bus.
    #[inline]
    pub fn devices(&mut self) -> &mut BxDevicesC {
        self.devices
    }

    /// The device models themselves.
    #[inline]
    pub fn device_manager(&mut self) -> &mut DeviceManager {
        self.device_manager
    }

    /// Timers, the tick counter and the A20 gate.
    #[inline]
    pub fn pc_system(&mut self) -> &mut BxPcSystemC {
        self.pc_system
    }

    /// Execute exactly one guest instruction on `cpu`, against these parts.
    ///
    /// The verb an execution engine needs and cannot write for itself. An
    /// engine that runs the guest on the host's own processor still meets
    /// accesses the hardware cannot finish — device memory, an instruction the
    /// platform traps and hands back without even an instruction length — and
    /// the only way to finish one is to decode and execute it. That decoder is
    /// this port's, so this is where it is offered.
    ///
    /// One instruction, on this port's own dispatch path, so an access
    /// serviced here is bit-identical to the same access under the interpreter
    /// — which is what keeps the two engines telling a guest the same story.
    ///
    /// # Errors
    /// Whatever the instruction raised that the processor could not take.
    pub fn emulate_one<T: Instrumentation>(
        &mut self,
        cpu: &mut BxCpuC<T>,
    ) -> crate::cpu::Result<()> {
        let mut ctx = ExecCtx::new(cpu, self.reborrow());
        // A budget of one, strictly, and a tick denominator of one: this is a
        // single instruction on one processor, not a slice of a round.
        ctx.cpu_loop_n_slice(1, true, 1).map(|_| ())
    }

    /// Lend the same parts onward for a shorter time.
    ///
    /// A `&mut` reborrows implicitly wherever one is expected; a value holding
    /// four of them does not, so a caller that must keep its loan — an engine
    /// servicing one exit after another out of the same parts — says so here
    /// rather than surrendering it to the first call that wants it.
    #[inline]
    pub fn reborrow(&mut self) -> PcIo<'_> {
        PcIo {
            memory: self.memory,
            devices: self.devices,
            device_manager: self.device_manager,
            pc_system: self.pc_system,
            assembled_by_a_machine: (),
        }
    }

    /// Assemble the loan from a machine's own fields.
    ///
    /// Taking the four separately is the whole point: they are distinct fields,
    /// so one destructure of `&mut self` yields all four live at once.
    #[inline]
    pub(crate) fn new(
        memory: &'a mut BxMemC,
        devices: &'a mut BxDevicesC,
        device_manager: &'a mut DeviceManager,
        pc_system: &'a mut BxPcSystemC,
    ) -> Self {
        Self {
            memory,
            devices,
            device_manager,
            pc_system,
            assembled_by_a_machine: (),
        }
    }
}
