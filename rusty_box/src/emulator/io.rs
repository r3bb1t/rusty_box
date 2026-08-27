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
    iodev::{devices::DeviceManager, BxDevicesC},
    memory::BxMemC,
    pc_system::BxPcSystemC,
};

/// Everything a processor executes against, borrowed from one machine.
pub(crate) struct PcIo<'a> {
    /// Guest memory, including the routing that decides which accesses are
    /// served from RAM and which belong to a device.
    pub(crate) memory: &'a mut BxMemC,
    /// The port bus: which slot answers for a port, and the levels and requests
    /// a dispatch latched.
    pub(crate) devices: &'a mut BxDevicesC,
    /// The device models themselves. Disjoint from `devices`, which owns the
    /// tables that route to them — a port dispatch needs both.
    pub(crate) device_manager: &'a mut DeviceManager,
    /// Timers, the tick counter and the A20 gate.
    pub(crate) pc_system: &'a mut BxPcSystemC,
}

impl<'a> PcIo<'a> {
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
        }
    }
}
