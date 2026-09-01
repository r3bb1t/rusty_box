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
    cpu::{cpu::BxCpuC, exec_ctx::ExecCtx, instrumentation::Instrumentation, AcknowledgedInterrupt},
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

    /// Move what a device dispatch latched onto the processor that caused it.
    ///
    /// A device answering a port write cannot reach a processor — it is handed
    /// what it may touch, and a CPU is not on the list — so what it wants said
    /// is latched on the bus: the PIC's interrupt line, the 8237's hold
    /// request, and a request for the machine to service its boundary. This is
    /// where those become the processor's business, and it is the only place
    /// (R5) — a second drain would leave whichever ran first with nothing and
    /// the other with a latch it had already consumed.
    ///
    /// Every engine has to do it, which is why it lives here rather than on
    /// the interpreter's execution context: an engine running the guest on
    /// hardware answers a port exit out of these same devices, and a boundary
    /// request it dropped would strand a PAM flip or a relocated BAR.
    pub fn sync_io_events<T: Instrumentation>(&mut self, cpu: &mut BxCpuC<T>) {
        let pic_intr_level = self.devices.take_pic_intr_level();
        let hrq_level = self.devices.take_hrq_level();
        let scheduler_boundary_requested = self.devices.take_scheduler_boundary_requested();

        if let Some(level) = pic_intr_level {
            // The moment the bus's interrupt line becomes the processor's
            // business. Watching it alongside the device that raised it and
            // the INTA that takes it is what turns "an interrupt arrived that
            // nobody expected" into a sequence with an order.
            tracing::debug!(target: "irq", "BUS: PIC line -> {level}, onto the processor");
            if level {
                cpu.signal_event(BxCpuC::<T>::BX_EVENT_PENDING_INTR);
            } else {
                cpu.clear_event(BxCpuC::<T>::BX_EVENT_PENDING_INTR);
            }
        }
        if let Some(level) = hrq_level {
            // Bochs pc_system.cc set_HRQ: `HRQ = val; if (val)
            // BX_CPU(0)->async_event = 1;` — the OUT that unmasked a pending
            // DRQ makes HRQ visible at this CPU's very next instruction
            // boundary, where handle_async_event services HLDA.
            self.pc_system.set_hrq(level);
            if level {
                cpu.raise_async_event();
            }
        }
        if scheduler_boundary_requested {
            cpu.request_scheduler_boundary();
        }
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

    /// Take one interrupt-acknowledge moment on behalf of an execution engine.
    ///
    /// The same INTA cycle the interpreter takes at an instruction boundary —
    /// LAPIC before 8259, the fabric's counted acknowledge, spurious vectors
    /// included — because it IS the same body
    /// ([`acknowledge_external_interrupt`](ExecCtx::acknowledge_external_interrupt)),
    /// which is what keeps the two engines acknowledging in one order. An
    /// engine about to inject an external interrupt into its partition pops
    /// the vector here; `None` says nothing was deliverable and the stale
    /// pin was reconciled.
    pub fn pop_deliverable_vector<T: Instrumentation>(
        &mut self,
        cpu: &mut BxCpuC<T>,
    ) -> Option<u8> {
        let mut ctx = ExecCtx::new(cpu, self.reborrow());
        match ctx.acknowledge_external_interrupt() {
            AcknowledgedInterrupt::Lapic(vector) => Some(vector),
            AcknowledgedInterrupt::Pic(vector) => Some(vector),
            AcknowledgedInterrupt::None => None,
        }
    }

    /// Execute up to `instructions` guest instructions on `cpu`, as one run.
    ///
    /// The same verb as [`Self::emulate_one`] with the batch left intact.
    /// Building an execution context is not free, and the interpreter's own
    /// loop chains traces inside one — so asking for a thousand instructions
    /// once is not the same work as asking for one instruction a thousand
    /// times, by a factor this port can measure. An engine that emulates in
    /// bulk rather than to finish a single trapped access wants this one.
    ///
    /// Returns the number of instructions actually retired, which may be fewer
    /// than asked for: the loop stops at a halt, at an event to deliver, or at
    /// whatever else ends a trace.
    ///
    /// # Errors
    /// Whatever the guest raised that the processor could not take.
    pub fn emulate_batch<T: Instrumentation>(
        &mut self,
        cpu: &mut BxCpuC<T>,
        instructions: u64,
    ) -> crate::cpu::Result<u64> {
        let mut ctx = ExecCtx::new(cpu, self.reborrow());
        // A tick denominator of one, so an instruction is a tick — the same
        // denomination the software engine reports its progress in.
        ctx.cpu_loop_n_slice(instructions, false, 1)
    }

    /// Whether the machine has work queued that only the machine can do.
    ///
    /// EVERY source, which is the whole point of it being one question: the
    /// PIC's final interrupt level, the 8237's hold request, an explicit
    /// boundary request, and — the one easy to forget — a device timer armed
    /// during dispatch.
    ///
    /// That last one is what an engine cannot afford to miss. A disk arms its
    /// completion timer while answering a port write, and raises its interrupt
    /// when that timer fires; a timer fires only when the machine has the
    /// processor back. An engine that kept running would deliver the interrupt
    /// a whole slice late, and a driver that has meanwhile polled the data and
    /// finished takes an interrupt it armed no handler for — which is how
    /// Linux says `hda: unexpected_intr` and then dies in its own interrupt
    /// return path.
    #[must_use]
    pub fn needs_boundary(&self) -> bool {
        self.devices.has_pending_boundary_work()
    }

    /// Execute the guest instruction at `RIP` until the processor is past it.
    ///
    /// A repeated string instruction is ONE instruction that moves many items,
    /// and asking for one instruction moves one item: `RIP` stays where it is
    /// and `RCX` comes down by one. That is not a quirk — it is how x86 makes
    /// such an instruction interruptible — but it is ruinous for an engine
    /// that traps to get here. A guest handed back mid-`REP INSW` traps again
    /// for the next word, so reading one disk sector costs 256 exits and 512
    /// architectural state exchanges instead of one.
    ///
    /// Stops early when the processor has an event to attend to, because an
    /// interruptible instruction is precisely what may be interrupted there,
    /// and stops after [`Self::ITEM_CEILING`] items regardless — the processor
    /// is left mid-instruction in the state the architecture defines for one,
    /// so the only cost of stopping early is trapping again.
    ///
    /// # Errors
    /// Whatever the instruction raised that the processor could not take.
    pub fn finish_the_instruction<T: Instrumentation>(
        &mut self,
        cpu: &mut BxCpuC<T>,
    ) -> crate::cpu::Result<()> {
        let began_at = cpu.rip();
        // The interpreter's trace bookkeeping would stop the instruction
        // between items, and here that buys nothing: an engine servicing a
        // trap cannot act on a queued scheduler boundary until its whole slice
        // ends. Parked, not dropped — put back below, along with anything that
        // arrived while the instruction ran.
        let parked = cpu.park_trace_bookkeeping();
        // And the external interrupt, for the length of this instruction. The
        // interpreter's loop delivers one at the head of every iteration
        // regardless of the budget, so without this a trapped `REP INSW` can
        // enter an interrupt handler halfway through servicing a disk sector —
        // delivery in a place the engine's own contract says delivery does not
        // happen. See `park_deliverable_interrupt`.
        let held = cpu.park_deliverable_interrupt();
        let mut outcome = Ok(());
        for _ in 0..Self::ITEM_CEILING {
            outcome = self.emulate_one(cpu);
            if outcome.is_err() || cpu.rip() != began_at {
                break;
            }
            // What the architecture allows to interrupt a repeated
            // instruction, and one thing it does not: a device deadline coming
            // due. Stopping for that is deliberate and registered (divergence
            // D3) — this port delivers a timer interrupt on time rather than
            // at the end of a burst, and an engine that ran on would be the
            // one diverging.
            if cpu.has_an_event_to_deliver()
                || self.pc_system.get_num_cpu_ticks_left_next_event() == 0
            {
                break;
            }
        }
        cpu.resume_trace_bookkeeping(parked);
        cpu.resume_deliverable_interrupt(held);
        outcome
    }

    /// How many items of one repeated instruction are executed before the
    /// machine gets a look in.
    ///
    /// Well past the 256 words of a disk sector, which is the case this
    /// exists for, and far short of the four billion a `REP` may name — a
    /// guest that asked for that has not stopped being interruptible.
    pub const ITEM_CEILING: u32 = 1 << 16;

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
