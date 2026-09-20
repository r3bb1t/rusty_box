//! What an exit moves between the partition and the shadow, and no more.
//!
//! A whole-state exchange costs the same whatever the exit was for, and the
//! measured accounting says that cost is what a slice is made of: 85.4% of
//! slices produce no exit at all, and under 1% of this engine's wall time is
//! the guest executing. So an exit here pays for what it uses. A plain port
//! write moves nothing — `RIP` and `RAX` go back directly. `CPUID` moves the
//! general registers, the instruction pointer, and the two groups its own
//! retirement width is derived from. Only a transfer of the whole processor
//! moves the whole processor.
//!
//! The mask is two flag sets and nothing else. `externalised` says which groups
//! the partition holds the live value of; `imported_this_exit` says which ones
//! were read for the exit being serviced. An import is the class's groups
//! intersected with `externalised`; an export is `imported_this_exit` exactly.
//!
//! The property that makes it safe: a group that was never imported is never
//! written back. The shadow's copy of it is stale — the guest has been running
//! on hardware — and writing that copy over the partition's live value is the
//! one thing a partial exchange must not do.
//!
//! A pause is the one time the shadow is read whole without an exit to
//! service: a caller reaching into a paused machine reads the processor there,
//! so [`Exchange::import_at_park`] brings every group home, and what the caller
//! writes goes back through [`Exchange::export_owed`] before the processor next
//! enters the partition. That export compares before it writes, so a pause
//! nobody wrote through costs the partition nothing.

use rusty_box::cpu::arch_state::{ArchGroups, ExitHeader, VcpuArchState};
use rusty_box::cpu::{cpu::BxCpuC, instrumentation::Instrumentation, Result};
use rusty_box_whp::{Reg, VpContext};

use crate::engine::{
    platform_failed, refused_import, refused_state, uncarried, InjectState, InterruptStateWord,
};
use crate::state::{self, VpRegisters};
use crate::xsave::XsaveArea;

/// What an exit needs in hand before the interpreter may finish it.
///
/// A closed set, matched exhaustively (R5): an exit reason this engine learns
/// to service has to say what it needs rather than inherit a catch-all, and
/// the catch-all that would be inherited is [`ExitClass::Full`] — correct and
/// the most expensive thing here.
///
/// A single `IN` or `OUT` the platform decoded for the host is not a class:
/// the port, the direction and the value arrive with the exit, nothing
/// retires on the shadow, and `service_port_access` writes `RIP` and `RAX`
/// back as two words without asking the exchange for anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExitClass {
    /// A string or repeated port access, which the interpreter executes.
    StringPort,
    /// An access to memory no partition window backs.
    Mmio,
    Cpuid,
    Msr,
    /// A fault the partition handed back rather than delivering.
    Exception,
    /// The whole processor: a transfer between engines, a snapshot, a
    /// diagnostic dump, an SMI run to completion on the shadow.
    Full,
}

/// The register groups `class` needs.
///
/// Each entry is the state the interpreter reads to finish that access, and
/// nothing beyond it. `Mmio` carries the segments because the operand is
/// `seg:offset`, and the tables because a fault it raises walks them.
///
/// Every class retires an instruction on the shadow, so every class carries
/// `CONTROL_REGS` and `MSRS` whatever else it needs, because those hold the
/// mode the retirement is measured in — `CR0.PE` and `EFER.LMA` decide how far
/// `RIP` advances.
pub(crate) const fn needs(class: ExitClass) -> ArchGroups {
    match class {
        // Every class below runs the interpreter over the faulting
        // instruction, and the interpreter delivers a fault ITSELF rather than
        // handing it back: `cpu_loop_n_slice` walks the IDT through `idtr` and
        // loads the handler's `CS` descriptor through `gdtr`
        // (`cpu/protected_interrupts.rs`, Bochs `exception.cc`). So any class
        // whose instruction can fault reads `TABLES` and `SEGMENTS` whether or
        // not the instruction itself names them — and the shadow's copies are
        // stale by construction, because a guest's `LIDT` or `LGDT` retires on
        // hardware without an exit. A delivery into a stale table is a jump
        // into whatever the descriptor happens to say, with no error anywhere.
        // `INS`/`OUTS` name a memory operand at `ES:DI` or `DS:SI`, so its
        // address is formed under the very paging mode `Mmio`'s comment names
        // — and EFER lives in `MSRS`, which no `MsrExits` bit covers, so the
        // shadow's copy is stale by construction. The same groups as `Mmio`
        // for the same reason; the classes differ in who services them, not in
        // what forms their address.
        ExitClass::StringPort => ArchGroups::GPRS
            .union(ArchGroups::RIP_RFLAGS)
            .union(ArchGroups::SEGMENTS)
            .union(ArchGroups::TABLES)
            .union(ArchGroups::CONTROL_REGS)
            .union(ArchGroups::MSRS),
        ExitClass::Mmio => ArchGroups::GPRS
            .union(ArchGroups::RIP_RFLAGS)
            .union(ArchGroups::SEGMENTS)
            .union(ArchGroups::TABLES)
            .union(ArchGroups::CONTROL_REGS)
            .union(ArchGroups::MSRS),
        // The cheapest class that still retires an instruction, and the two
        // groups below are the floor beneath which no such class can go.
        //
        // `CPUID` needs no `SEGMENTS` and no `TABLES`: it takes no memory
        // operand, is not privileged, and is supported by every processor this
        // port models, so no delivery can follow it and nothing walks a
        // descriptor. What it cannot do without is the state its own
        // RETIREMENT is measured in — `CR0.PE` says real or protected and
        // `EFER.LMA` says long, and between them they decide whether `RIP`
        // advances by sixteen bits, thirty-two or sixty-four. A guest changes
        // both on hardware without ever taking an exit, so a shadow that
        // skipped them would retire this instruction at whatever mode it was
        // last left in.
        ExitClass::Cpuid => ArchGroups::GPRS
            .union(ArchGroups::RIP_RFLAGS)
            .union(ArchGroups::CONTROL_REGS)
            .union(ArchGroups::MSRS),
        // `RDMSR`/`WRMSR` of an MSR this port does not model raises `#GP`,
        // which is routine rather than exotic — so this class delivers, and
        // needs the segments the handler's `CS` load reads.
        ExitClass::Msr => ArchGroups::GPRS
            .union(ArchGroups::RIP_RFLAGS)
            .union(ArchGroups::SEGMENTS)
            .union(ArchGroups::TABLES)
            .union(ArchGroups::CONTROL_REGS)
            .union(ArchGroups::MSRS),
        // A delivery walks the IDT and pushes a frame, so it reads the tables
        // and the debug registers a `#DB` reports through, and it is the one
        // class whose interruptibility the partition must be told about.
        ExitClass::Exception => ArchGroups::GPRS
            .union(ArchGroups::RIP_RFLAGS)
            .union(ArchGroups::SEGMENTS)
            .union(ArchGroups::TABLES)
            .union(ArchGroups::CONTROL_REGS)
            .union(ArchGroups::DEBUG_REGS)
            .union(ArchGroups::MSRS)
            .union(ArchGroups::INTERRUPT_STATE),
        ExitClass::Full => ArchGroups::all(),
    }
}

/// The groups a park brings home: every group this exchange carries except the
/// interrupt state.
///
/// The interrupt shadow and the NMI mask stay with the partition across a
/// pause. They describe where the guest stopped rather than what it holds, the
/// machine's API names neither, and importing them would oblige the next entry
/// to write back the shadow bit the import lapses on the interpreter's side —
/// ending an `STI` window the guest never finished.
const AT_PARK: ArchGroups = ArchGroups::all().difference(ArchGroups::INTERRUPT_STATE);

/// What a hand-back wrote, and what it cost (R0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Handback {
    /// The groups written into the partition. A group a park read and nothing
    /// wrote since is not among them: the partition already holds it.
    pub(crate) written: ArchGroups,
    /// Platform calls made, for the census.
    pub(crate) calls: usize,
}

/// One processor's account of where its architectural state lives.
pub(crate) struct Exchange {
    /// The groups the PARTITION holds the live value of. A set bit means the
    /// shadow's copy is stale and must not be written back.
    externalised: ArchGroups,
    /// The groups the shadow holds the live value of and owes the partition:
    /// read for the exit being serviced, read at a park, or never handed to
    /// the partition at all. Exactly what the next export writes, less what
    /// [`Self::read_at_park`] finds unchanged.
    imported_this_exit: ArchGroups,
    /// The groups the last park read out of the partition, which the next
    /// export compares against [`Self::at_park`] rather than writing whole.
    /// Empty outside a pause.
    read_at_park: ArchGroups,
    /// The shadow's own export of [`Self::read_at_park`] the moment the park
    /// read it — the partition's values as a write-back would carry them, and
    /// so what a caller's write is told apart from.
    at_park: VcpuArchState,
    /// The buffer both directions move through, held for the life of the
    /// processor rather than built per exit: a [`VcpuArchState`] carries the
    /// whole vector file, and zeroing two and a half kilobytes at every exit
    /// would be a cost this type exists to remove. Only the fields of the
    /// groups being moved are ever read out of it.
    scratch: VcpuArchState,
    /// What the partition's `WHvRegisterInterruptState` holds, as far as this
    /// exchange knows. The NMI mask lives only here — no architectural
    /// register carries it — so a write of the shadow bit must carry the mask
    /// through unchanged rather than zeroing it under a guest's NMI handler.
    held_interrupt_state: InterruptStateWord,
}

impl Exchange {
    /// A processor the shadow has never read: everything is the partition's.
    ///
    /// What the exchange's own tests start from. A running engine starts from
    /// [`Self::installing`], because the machine it adopts already has a
    /// processor of its own.
    #[cfg(test)]
    pub(crate) fn at_reset() -> Self {
        Self {
            externalised: ArchGroups::all(),
            imported_this_exit: ArchGroups::empty(),
            read_at_park: ArchGroups::empty(),
            at_park: VcpuArchState::default(),
            scratch: VcpuArchState::default(),
            held_interrupt_state: InterruptStateWord::AT_RESET,
        }
    }

    /// A processor the machine built and the partition has never held: every
    /// group but the interrupt state is the shadow's, and the first export
    /// writes all of it.
    ///
    /// What adoption is. The machine was reset, set up or restored before the
    /// partition existed, and a processor the platform creates afterwards
    /// stands at the platform's own reset state — `F000:FFF0`, whatever the
    /// machine said. Installing the machine's processor whole before the
    /// first entry is what makes the guest start where the machine left it.
    /// The interrupt state is the one group left with the partition: a fresh
    /// processor holds no shadow and no NMI mask, and neither does a machine
    /// the interpreter has not run.
    pub(crate) fn installing() -> Self {
        Self {
            externalised: ArchGroups::INTERRUPT_STATE,
            imported_this_exit: AT_PARK,
            read_at_park: ArchGroups::empty(),
            at_park: VcpuArchState::default(),
            scratch: VcpuArchState::default(),
            held_interrupt_state: InterruptStateWord::AT_RESET,
        }
    }

    /// The machine was reset: the shadow holds the reset processor, and the
    /// partition still holds the old guest's.
    ///
    /// Every group becomes the shadow's again, as at adoption
    /// ([`Self::installing`]), so the next export writes the reset processor
    /// whole. The interrupt state is the one group an adoption leaves with the
    /// partition, and a reset cannot: the old guest may have left an interrupt
    /// shadow standing or NMIs masked, and a reset clears both (Bochs
    /// `BX_CPU_C::reset`). No architectural register carries the NMI mask for
    /// an export to compare against, so the reset word is written here.
    ///
    /// # Errors
    /// The platform's refusal of the interrupt-state write. The exchange is
    /// left as it was, so nothing it records is untrue.
    pub(crate) fn reinstall_after_reset<V: VpRegisters>(&mut self, vp: &V) -> Result<()> {
        vp.write_words(&[Reg::InterruptState], &[InterruptStateWord::AT_RESET.encode()])
            .map_err(platform_failed)?;
        *self = Self::installing();
        Ok(())
    }

    /// Bring every group the partition still holds into the shadow, and the
    /// time-stamp counter with them, for a caller that will reach into the
    /// paused machine.
    ///
    /// The groups [`AT_PARK`] names, less what the shadow already holds —
    /// an exit that could not finish, an adoption not yet installed, or an
    /// earlier park with no entry since — which stays owed to the partition
    /// as it was. What is read is also remembered as it stands, so
    /// [`Self::export_owed`] can tell a caller's write from the partition's
    /// own value. An earlier park's record is kept, not replaced: a second
    /// park before any entry reads nothing, and the partition still holds
    /// exactly what the first one recorded.
    ///
    /// The counter is read on its own and never written back: the hardware
    /// owns it on this engine (`state::IMPORTED_MSRS`), so the shadow is told
    /// the guest's value only so that a read through the machine answers it.
    ///
    /// # Errors
    /// A register the platform would not give up, or a state this port
    /// refuses.
    pub(crate) fn import_at_park<V: VpRegisters, T: Instrumentation>(
        &mut self,
        vp: &V,
        cpu: &mut BxCpuC<T>,
        xsave: &mut XsaveArea,
    ) -> Result<()> {
        let owed = self.imported_this_exit;
        self.import_groups(vp, cpu, AT_PARK, xsave)?;
        let read_now = self.imported_this_exit.difference(owed);
        cpu.export_arch_groups(&mut self.at_park, read_now);
        self.read_at_park.insert(read_now);
        let mut tsc = [0u64; 1];
        vp.read_words(&[Reg::Tsc], &mut tsc).map_err(platform_failed)?;
        cpu.take_time_stamp_counter(tsc[0]);
        Ok(())
    }

    /// Whether the shadow holds anything the partition must be given before
    /// the processor next enters it.
    ///
    /// Asked without the machine's lock, which the answer needs only when it
    /// is yes — after a park, or before a processor's first entry. After an
    /// ordinary exit the errand has already written back what it read.
    pub(crate) fn owes_the_partition(&self) -> bool {
        !self.imported_this_exit.is_empty()
    }

    /// Hand the partition every group the shadow holds, before the processor
    /// next enters it.
    ///
    /// The one place the shadow reaches the partition outside an exit's own
    /// write-back (R5): after a park, where a paused caller may have written
    /// registers, and before a processor's first entry, where the machine's
    /// processor is installed. A group the park read and nothing wrote since
    /// goes back unwritten — the partition's copy IS that value, so a pause
    /// nobody wrote through makes no platform call. Everything else the shadow
    /// holds goes through [`Self::export_imported`], the path every exit's
    /// write-back takes.
    ///
    /// # Errors
    /// As [`Self::export_imported`]. The groups not yet written stay owed, so
    /// the next attempt writes them again.
    pub(crate) fn export_owed<V: VpRegisters, T: Instrumentation>(
        &mut self,
        vp: &V,
        cpu: &mut BxCpuC<T>,
        xsave: &mut XsaveArea,
    ) -> Result<Handback> {
        let compared = self.read_at_park;
        cpu.export_arch_groups(&mut self.scratch, compared);
        let mut changed = state::groups_that_differ(&self.scratch, &self.at_park, compared);
        if compared.contains(ArchGroups::VECTOR)
            && crate::xsave::vector_file_differs(&self.scratch, &self.at_park)
        {
            changed.insert(ArchGroups::VECTOR);
        }
        let unchanged = compared.difference(changed);
        self.imported_this_exit.remove(unchanged);
        self.externalised.insert(unchanged);
        self.read_at_park = ArchGroups::empty();
        let written = self.imported_this_exit;
        let calls = self.export_imported(vp, cpu, xsave)?;
        Ok(Handback { written, calls })
    }

    /// Copy the exit header's own fields into the shadow.
    ///
    /// The free half of an exchange: `RIP`, `RFLAGS`, `CS` and `CR8` arrive
    /// with the exit, so taking them costs no platform call. They are taken
    /// whatever the class needs, because they are the fields most likely to
    /// have moved — every branch and every `MOV CR8` retires on the hardware
    /// without an exit — and because a group the class does NOT import then
    /// stands on the header's answer rather than on the shadow's stale one: an
    /// exit that imports no segments still decodes its instruction at the CS
    /// the guest is actually executing in.
    ///
    /// The mask is untouched. The header freshens part of three groups and the
    /// whole of none, and a group marked local on the strength of it would be
    /// a group the export then refuses to write back — which is how a serviced
    /// instruction's `RIP` would fail to reach the guest.
    ///
    /// # Errors
    /// A code segment this port will not build.
    pub(crate) fn take_header<T: Instrumentation>(
        &mut self,
        cpu: &mut BxCpuC<T>,
        vp: &VpContext,
        inject: &mut InjectState,
    ) -> Result<()> {
        inject.refresh_from(vp);
        cpu.take_exit_header(&ExitHeader {
            rip: vp.rip,
            rflags: vp.rflags,
            cs: state::from_platform_segment(vp.cs),
            cr8: vp.cr8,
        })
        .map_err(refused_import)
    }

    /// Read the groups `class` needs that are still the partition's, and mark
    /// them local.
    ///
    /// `bytes` are the instruction the exit stopped on, as much of it as the
    /// platform reported. [`ArchGroups::VECTOR`] is added when they decode to
    /// an instruction that touches the x87 or vector file, and when there are
    /// none at all — a read-only window write carries no instruction, and the
    /// file is then imported rather than trusted.
    ///
    /// # Errors
    /// A register the platform would not give up, or a state this port
    /// refuses.
    pub(crate) fn import_for<V: VpRegisters, T: Instrumentation>(
        &mut self,
        vp: &V,
        cpu: &mut BxCpuC<T>,
        class: ExitClass,
        bytes: &[u8],
        xsave: &mut XsaveArea,
    ) -> Result<()> {
        // In two phases, and the order is load-bearing. Deciding the vector
        // question first would decide it at whatever mode the shadow happened
        // to be left in: `long_mode()` reads EFER, so a shadow whose `MSRS` are
        // still externalised can believe a 64-bit guest is 16-bit, decode a
        // REX-prefixed vector instruction as something else entirely, and
        // answer `false` — handing the guest a stale vector file with no error
        // anywhere. The classes that carry an arbitrary guest instruction
        // (`Mmio`, `Exception`) import `MSRS`, so after phase one the mode is
        // the guest's own; the two that do not (`StringPort`, `Cpuid`) carry a
        // fixed instruction family that touches no vector state either way.
        self.import_groups(vp, cpu, needs(class), xsave)?;
        // The second call costs a platform call only when the vector file is
        // actually wanted, which is the rare case; the common exit stays at
        // one. An exit that carries no bytes takes it unconditionally —
        // there is nothing to decode, and trusting a stale file is a wrong
        // answer where importing an unneeded one is only a cost.
        if bytes.is_empty() || cpu.next_instruction_touches_vector_state(bytes) {
            self.import_groups(vp, cpu, ArchGroups::VECTOR, xsave)?;
        }
        Ok(())
    }

    /// The whole processor — [`ExitClass::Full`]'s import, named for the
    /// callers that are not servicing an exit at all.
    ///
    /// # Errors
    /// As [`Self::import_for`].
    pub(crate) fn import_everything<V: VpRegisters, T: Instrumentation>(
        &mut self,
        vp: &V,
        cpu: &mut BxCpuC<T>,
        xsave: &mut XsaveArea,
    ) -> Result<()> {
        self.import_groups(vp, cpu, needs(ExitClass::Full), xsave)
    }

    fn import_groups<V: VpRegisters, T: Instrumentation>(
        &mut self,
        vp: &V,
        cpu: &mut BxCpuC<T>,
        wanted: ArchGroups,
        xsave: &mut XsaveArea,
    ) -> Result<()> {
        let importing = wanted.intersection(self.externalised);
        if self.imported_this_exit.is_empty() && !importing.contains(ArchGroups::INTERRUPT_STATE) {
            // An inhibit the shadow still holds at the HEAD of an exit was
            // armed by an instruction it retired at some earlier exit, and the
            // hardware has run the guest since — the one-instruction window
            // that inhibit named is long over. The interpreter cannot see
            // that, because its own retired-instruction count is what an
            // inhibit is anchored to and that count does not move while the
            // hardware holds the guest (Bochs cpu.h `inhibit_interrupts`).
            // Told here rather than read from the partition, because no read
            // is needed to know it. The exit that DOES import the group takes
            // the partition's own answer instead, just below.
            //
            // The head is where nothing has been imported yet, which is also
            // where no errand can have run: an inhibit armed by THIS exit's
            // own errand is the one thing that must survive to the export, and
            // a second import in the same exit must not take it away.
            cpu.lapse_interrupt_inhibit();
        }
        if importing.is_empty() {
            return Ok(());
        }

        // One call over the union of the named groups. The count it answers
        // with belongs to the export, which reports what an exit cost; an
        // import's own cost is not a number any caller acts on.
        state::read_groups(vp, importing, &mut self.scratch).map_err(platform_failed)?;
        // The x87 and vector file has no register name on this platform, so it
        // arrives as a whole area beside the named registers — and on its own
        // when it is the only group being imported, which is what an exit
        // whose class needs nothing but whose instruction touches the file
        // asks for.
        if importing.contains(ArchGroups::VECTOR) {
            xsave.refresh_from(vp).map_err(platform_failed)?;
            xsave.fill(&mut self.scratch);
        }
        if importing.contains(ArchGroups::INTERRUPT_STATE) {
            self.read_interrupt_state(vp)?;
            // The hardware's clear bit says the instruction the shadow's own
            // inhibit protected has retired. A SET bit is left with the
            // partition: the interpreter is not told of a shadow the hardware
            // entered, because the paths that retire the shadowed instruction
            // hold delivery off on their own account. VirtualBox anchors the
            // same shadow to `RIP` on import (`NEMAllNativeTemplate-win.cpp.h`
            // `nemHCWinCopyStateFromHyperV`, `CPUMUpdateInterruptShadowEx`).
            if !self.held_interrupt_state.shadow {
                cpu.lapse_interrupt_inhibit();
            }
        }

        // The interrupt state is not a [`VcpuArchState`] field, so it is not
        // this call's to write.
        cpu.import_arch_groups(&self.scratch, importing.difference(ArchGroups::INTERRUPT_STATE))
            .map_err(refused_import)?;

        self.externalised.remove(importing);
        self.imported_this_exit.insert(importing);
        Ok(())
    }

    /// Write back exactly the groups imported for this exit, and mark them the
    /// partition's again.
    ///
    /// Externalised again rather than left local, because the very next thing
    /// the processor does is run the guest on hardware: every group it just
    /// took back is one the guest may change before the next exit, so the
    /// shadow's copy is stale from the VM entry onwards.
    ///
    /// Answers how many platform calls it made — the accounting an engine's
    /// census is built from.
    ///
    /// # Errors
    /// A register the platform would not take, or extended state the
    /// partition's area cannot carry.
    pub(crate) fn export_imported<V: VpRegisters, T: Instrumentation>(
        &mut self,
        vp: &V,
        cpu: &mut BxCpuC<T>,
        xsave: &mut XsaveArea,
    ) -> Result<usize> {
        let exporting = self.imported_this_exit;
        cpu.export_arch_groups(&mut self.scratch, exporting);
        let mut calls = state::write_groups(vp, exporting, &self.scratch)
            .map_err(|error| refused_state(error, &self.scratch))?;
        if exporting.contains(ArchGroups::VECTOR) {
            xsave.patch(&self.scratch).map_err(uncarried)?;
            xsave.write_to(vp).map_err(platform_failed)?;
            calls += 1;
        }
        calls += self.export_interrupt_state(vp, cpu, exporting)?;
        self.externalised.insert(exporting);
        self.imported_this_exit = ArchGroups::empty();
        Ok(calls)
    }

    /// Publish the interrupt shadow the shadow processor now stands in.
    ///
    /// Imposed as 1 exactly when the interpreter's own inhibit is live —
    /// `BxCpuC::in_interrupt_shadow`, the test its delivery gate runs at this
    /// boundary (Bochs event.cc handleAsyncEvent, Priority 5) — which after an
    /// errand means the instruction it retired last was an `STI`, a `MOV SS`
    /// or a `POP SS`. The hardware then holds delivery off for the one
    /// instruction the architecture gives an inhibit and clears the bit
    /// itself. That is what VirtualBox's NEM backend writes here
    /// (`NEMAllNativeTemplate-win.cpp.h` `nemHCWinCopyStateToHyperV`,
    /// `CPUMIsInInterruptShadow`).
    ///
    /// Written whenever the inhibit is live, whether or not the class named
    /// the group: the errand armed it, and the partition has no other source
    /// for it — this is the errand's own result, not a stale copy of a value
    /// the partition already holds. The NMI mask rides along and must be the
    /// partition's current one, so a live inhibit whose group was never
    /// imported reads the register first; zeroing the mask would unmask NMIs
    /// under a guest's own handler. The write is skipped when it would change
    /// nothing, as VirtualBox skips it.
    fn export_interrupt_state<V: VpRegisters, T: Instrumentation>(
        &mut self,
        vp: &V,
        cpu: &BxCpuC<T>,
        exporting: ArchGroups,
    ) -> Result<usize> {
        let inhibit = cpu.in_interrupt_shadow();
        if !inhibit && !exporting.contains(ArchGroups::INTERRUPT_STATE) {
            return Ok(0);
        }
        let mut calls = 0;
        if self.externalised.contains(ArchGroups::INTERRUPT_STATE) {
            self.read_interrupt_state(vp)?;
            calls += 1;
        }
        let next = InterruptStateWord {
            shadow: inhibit,
            nmi_masked: self.held_interrupt_state.nmi_masked,
        };
        if next != self.held_interrupt_state {
            vp.write_words(&[Reg::InterruptState], &[next.encode()])
                .map_err(platform_failed)?;
            self.held_interrupt_state = next;
            calls += 1;
        }
        Ok(calls)
    }

    /// Bring [`Self::held_interrupt_state`] up to date from the partition.
    ///
    /// The lapse rule is deliberately not here: an import applies it and an
    /// export must not, because at an export the inhibit being published is
    /// the shadow's own.
    fn read_interrupt_state<V: VpRegisters>(&mut self, vp: &V) -> Result<()> {
        let mut word = [0u64; 1];
        vp.read_words(&[Reg::InterruptState], &mut word)
            .map_err(platform_failed)?;
        self.held_interrupt_state = InterruptStateWord::decode(word[0]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{machine_running, CODE};
    use crate::state::test_vp::Recorder;
    use crate::xsave::{HostComponents, XsaveArea};
    use rusty_box::cpu::arch_state::{ArchGroups, VcpuArchState};
    use rusty_box_whp::{Reg, SegmentRegister, VpContext};

    /// An area the size a compacted one starts at, so [`XsaveArea::read_from`]
    /// has a header to read.
    fn recorder_with_an_area() -> Recorder {
        let vp = Recorder::default();
        vp.seed_xsave(&[0u8; 576]);
        vp
    }

    /// What each class costs, stated once so a change to the table is a change
    /// to a test rather than to a boot.
    ///
    /// Only a whole-processor transfer moves everything. Every other class
    /// names the state the interpreter reads and no more — and none of them
    /// but [`ExitClass::Exception`] names the interruptibility, which the
    /// partition is told about only where a delivery makes it matter.
    #[test]
    fn each_exit_class_asks_for_the_state_it_reads_and_no_more() {
        assert_eq!(needs(ExitClass::Full), ArchGroups::all());

        assert_eq!(
            needs(ExitClass::Cpuid),
            ArchGroups::GPRS
                | ArchGroups::RIP_RFLAGS
                | ArchGroups::CONTROL_REGS
                | ArchGroups::MSRS,
            "CPUID reads and writes four general registers and advances RIP — by a \
             width `CR0.PE` and `EFER.LMA` decide, so it carries those too"
        );
        assert_eq!(
            needs(ExitClass::StringPort),
            ArchGroups::GPRS
                | ArchGroups::RIP_RFLAGS
                | ArchGroups::SEGMENTS
                | ArchGroups::TABLES
                | ArchGroups::CONTROL_REGS
                | ArchGroups::MSRS
        );
        assert_eq!(
            needs(ExitClass::Mmio),
            needs(ExitClass::StringPort),
            "both name a memory operand, and an operand's address is formed under the paging \
             mode EFER names — the classes differ in who services them, not in what forms \
             their address"
        );
        assert_eq!(
            needs(ExitClass::Msr),
            ArchGroups::GPRS
                | ArchGroups::RIP_RFLAGS
                | ArchGroups::SEGMENTS
                | ArchGroups::TABLES
                | ArchGroups::CONTROL_REGS
                | ArchGroups::MSRS
        );
        // The rule the four interpreting classes share, asserted as a rule
        // rather than four times over: the interpreter delivers a fault
        // itself, so a class that can fault reads the tables it would walk and
        // the segments the handler's `CS` load reads. `CPUID` cannot fault and
        // is the deliberate exception.
        for class in [
            ExitClass::StringPort,
            ExitClass::Mmio,
            ExitClass::Msr,
            ExitClass::Exception,
        ] {
            assert!(
                needs(class).contains(ArchGroups::TABLES | ArchGroups::SEGMENTS),
                "{class:?} interprets an instruction that can fault, so it must carry the \
                 tables a delivery walks and the segments its handler loads"
            );
        }
        assert!(
            !needs(ExitClass::Cpuid).intersects(ArchGroups::TABLES | ArchGroups::SEGMENTS),
            "CPUID takes no memory operand and is not privileged, so no delivery follows it"
        );
        assert_eq!(
            needs(ExitClass::Exception),
            needs(ExitClass::Mmio) | ArchGroups::DEBUG_REGS | ArchGroups::INTERRUPT_STATE,
            "a delivery walks the IDT and is the one class whose \
             interruptibility the partition must be told"
        );

        for class in [
            ExitClass::StringPort,
            ExitClass::Mmio,
            ExitClass::Cpuid,
            ExitClass::Msr,
            ExitClass::Exception,
        ] {
            assert!(
                !needs(class).contains(ArchGroups::VECTOR),
                "{class:?} takes the vector file only when its instruction says so"
            );
        }
    }

    #[test]
    fn a_cpuid_exit_moves_only_what_cpuid_can_change() {
        let vp = recorder_with_an_area();
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_for(&vp, cpu, ExitClass::Cpuid, &[0x0F, 0xA2], &mut xs).unwrap();
        assert!(
            vp.reads_of(Reg::Rax) == 1 && vp.reads_of(Reg::Es) == 0,
            "the general registers CPUID answers in, and no segment it cannot name"
        );
        assert!(
            vp.reads_of(Reg::Efer) == 1 && vp.reads_of(Reg::Cr0) == 1,
            "and the two the width of its own retirement is derived from"
        );

        let calls = ex.export_imported(&vp, cpu, &mut xs).unwrap();
        assert!(
            vp.writes_of(Reg::Rax) == 1 && vp.writes_of(Reg::Es) == 0,
            "export writes exactly what was imported"
        );
        assert_eq!(calls, 1, "one batched write");
    }

    #[test]
    fn an_imported_group_is_not_read_twice_until_it_is_exported() {
        let vp = recorder_with_an_area();
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_for(&vp, cpu, ExitClass::Msr, &[0x0F, 0x30], &mut xs).unwrap();
        ex.import_for(&vp, cpu, ExitClass::Cpuid, &[0x0F, 0xA2], &mut xs).unwrap();
        assert_eq!(vp.reads_of(Reg::Rax), 1, "GPRS were already local");
    }

    /// The whole point of the mask: a group this exit never imported keeps the
    /// PARTITION's live value — the shadow's stale copy is never written over
    /// it.
    #[test]
    fn a_group_that_was_not_imported_is_not_written_back() {
        let vp = recorder_with_an_area();
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        vp.seed(Reg::Cr3, 0x0000_1000);
        vp.seed(Reg::Dr7, 0x0000_0400);
        let mut ex = Exchange::at_reset();

        ex.import_for(&vp, cpu, ExitClass::Cpuid, &[0x0F, 0xA2], &mut xs).unwrap();

        // The shadow drifts locally, through this task's own API.
        let mut drifted = VcpuArchState::default();
        cpu.export_arch_groups(&mut drifted, ArchGroups::all());
        drifted.cr3 = 0xDEAD_0000;
        drifted.dr7 = 0xDEAD_0400;
        drifted.segments[0].selector = 0x0020;
        cpu.import_arch_groups(
            &drifted,
            ArchGroups::CONTROL_REGS | ArchGroups::DEBUG_REGS | ArchGroups::SEGMENTS,
        )
        .unwrap();

        ex.export_imported(&vp, cpu, &mut xs).unwrap();
        // Both directions of the mask, over one exit. `CPUID` imports the
        // control registers — its retirement width is derived from them — so
        // the shadow's copy is the live one and goes back.
        assert_eq!(
            vp.value_of(Reg::Cr3),
            0xDEAD_0000,
            "CONTROL_REGS were imported, so the shadow's copy is written back"
        );
        // It imports neither the debug registers nor the segments, so the
        // partition keeps its own — which is the property this test exists for.
        assert_eq!(
            vp.value_of(Reg::Dr7),
            0x0000_0400,
            "DEBUG_REGS were not imported, so they are not exported"
        );
        assert_eq!(
            vp.writes_of(Reg::Es),
            0,
            "SEGMENTS were not imported, so none is written"
        );
    }

    #[test]
    fn the_tsc_is_never_written_and_the_xsave_area_crosses_only_for_vector_state() {
        let vp = recorder_with_an_area();
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");

        let mut ex = Exchange::at_reset();
        ex.import_everything(&vp, cpu, &mut xs).unwrap();
        ex.export_imported(&vp, cpu, &mut xs).unwrap();
        assert_eq!(vp.writes_of(Reg::Tsc), 0);
        assert_eq!(vp.xsave_writes(), 1, "Full imports the vector file, so Full exports it");

        let mut ex = Exchange::at_reset();
        ex.import_for(&vp, cpu, ExitClass::Cpuid, &[0x0F, 0xA2], &mut xs).unwrap();
        ex.export_imported(&vp, cpu, &mut xs).unwrap();
        assert_eq!(vp.xsave_writes(), 1, "unchanged: CPUID never touches the vector file");

        let mut ex = Exchange::at_reset();
        // movdqa xmm0, [mem] — the operand is a device window, the register is
        // the vector file.
        ex.import_for(&vp, cpu, ExitClass::Mmio, &[0x66, 0x0F, 0x6F, 0x05], &mut xs).unwrap();
        ex.export_imported(&vp, cpu, &mut xs).unwrap();
        assert_eq!(
            vp.xsave_writes(),
            2,
            "an MMIO exit whose instruction touches xmm0 moves the vector file"
        );
    }

    /// The NMI mask the partition holds rides through an exchange unchanged,
    /// and a shadow the exchange did not observe is not invented.
    ///
    /// The mask lives in no architectural register, so the only copy of it is
    /// the one the exchange carries; a write that zeroed it would unmask NMIs
    /// under a guest's own handler. And with nothing to change, nothing is
    /// written at all — a register write per exit to say what the partition
    /// already holds is the cost this whole type exists to remove.
    #[test]
    fn the_nmi_mask_rides_through_an_exchange_that_publishes_no_shadow() {
        let vp = recorder_with_an_area();
        // Bit 1 of `WHvRegisterInterruptState`: NMIs masked, which is the
        // hardware's record of a guest inside its own NMI handler.
        vp.seed(Reg::InterruptState, 0b10);
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_for(&vp, cpu, ExitClass::Exception, &[0x0F, 0xA2], &mut xs).unwrap();
        assert_eq!(
            vp.reads_of(Reg::InterruptState),
            1,
            "an exception is the class whose interruptibility is read"
        );

        ex.export_imported(&vp, cpu, &mut xs).unwrap();
        assert_eq!(
            vp.writes_of(Reg::InterruptState),
            0,
            "the shadow retired nothing that shadows, so there was nothing to write"
        );
        assert_eq!(vp.value_of(Reg::InterruptState), 0b10, "and the NMI mask still stands");
    }

    /// The vector question is decided at the GUEST's mode, not at whatever
    /// mode the shadow was left in by an earlier exit.
    ///
    /// `next_instruction_touches_vector_state` picks its decoder from
    /// `long64_mode()`, which reads EFER — so deciding before the import runs
    /// decides it on a shadow that may still believe a 64-bit guest is 16-bit.
    /// `F3 44 0F 6F 00` is `movdqu xmm8, [rax]` to a 64-bit decoder and a
    /// repeated `INC SP` to a 16-bit one: the first touches the vector file
    /// and the second does not, so a stale mode silently hands the guest a
    /// stale file. Falsified by deciding before the import instead of after.
    #[test]
    fn the_vector_question_is_decided_at_the_guests_mode_not_the_shadows() {
        let vp = Recorder::default();
        // The partition holds a processor in 64-bit mode.
        vp.seed(Reg::Efer, 0x500); // LME | LMA
        vp.seed(Reg::Cr0, 0x8000_0001); // PG | PE
        vp.seed_segment(
            Reg::Cs,
            SegmentRegister {
                base: 0,
                limit: 0xFFFF_FFFF,
                selector: 0x08,
                // present, non-system, code execute/read, and L.
                attributes: 0x209B,
            },
        );
        let mut area = [0u8; 576];
        area[512] = 0b11;
        area[160] = 0xA5;
        area[161] = 0x5A;
        vp.seed_xsave(&area);

        // The shadow starts in real mode, which is the stale mode that would
        // decode those bytes as something that touches nothing.
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_for(&vp, cpu, ExitClass::Mmio, &[0xF3, 0x44, 0x0F, 0x6F, 0x00], &mut xs)
            .expect("the import");

        let mut shadow = VcpuArchState::default();
        cpu.export_arch_groups(&mut shadow, ArchGroups::VECTOR);
        assert_eq!(
            (shadow.vector[0][0], shadow.vector[0][1]),
            (0xA5, 0x5A),
            "the guest is in 64-bit mode, so the REX-prefixed vector instruction was recognised \
             and the partition's vector file crossed"
        );
    }

    /// An exit that carries no instruction bytes imports the vector file
    /// unconditionally.
    ///
    /// A string port access is serviced with none — the platform decoded it
    /// and reported no bytes — and so is a trapped fault; nothing then says
    /// the instruction leaves the file alone. The file names no register, so
    /// asserting on the value in the shadow is what says the area actually
    /// crossed rather than the mask merely saying it did.
    #[test]
    fn an_exit_with_no_instruction_bytes_imports_the_vector_file() {
        let vp = Recorder::default();
        // A standard-form area at the architecture's own offsets: `XSTATE_BV`
        // at 512 marking x87 and SSE live, `XMM0` at 160 holding a value
        // nothing else would produce.
        let mut area = [0u8; 576];
        area[512] = 0b11;
        area[160] = 0xA5;
        area[161] = 0x5A;
        vp.seed_xsave(&area);
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_for(&vp, cpu, ExitClass::StringPort, &[], &mut xs).unwrap();

        let mut shadow = VcpuArchState::default();
        cpu.export_arch_groups(&mut shadow, ArchGroups::VECTOR);
        assert_eq!(
            (shadow.vector[0][0], shadow.vector[0][1]),
            (0xA5, 0x5A),
            "the partition's XMM0 reached the shadow"
        );

        let calls = ex.export_imported(&vp, cpu, &mut xs).unwrap();
        assert_eq!(calls, 2, "the class's register batch, and the area beside it");
        assert_eq!(vp.xsave_writes(), 1);
    }

    /// A pause writes back only what the caller changed.
    ///
    /// The park reads every group; the caller then writes one general
    /// register. Only that group crosses back — the control registers, the
    /// segments and the vector file the park read are the partition's own
    /// values, and writing them again would be a platform call to say nothing.
    /// A second pause whose caller writes the vector file moves that and
    /// nothing else.
    #[test]
    fn a_pause_writes_back_only_the_groups_the_caller_changed() {
        let vp = recorder_with_an_area();
        vp.seed(Reg::Rbx, 0x1111);
        vp.seed(Reg::Cr3, 0x3000);
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_at_park(&vp, cpu, &mut xs).unwrap();
        let mut state = VcpuArchState::default();
        cpu.export_arch_groups(&mut state, ArchGroups::GPRS);
        assert_eq!(state.gprs[3], 0x1111, "the park read the partition's RBX");
        state.gprs[3] = 0x5A;
        cpu.import_arch_groups(&state, ArchGroups::GPRS).unwrap();

        let handback = ex.export_owed(&vp, cpu, &mut xs).unwrap();
        assert_eq!(handback.written, ArchGroups::GPRS);
        assert_eq!(handback.calls, 1, "one batched register write");
        assert_eq!(vp.value_of(Reg::Rbx), 0x5A, "the caller's RBX reached the partition");
        assert_eq!(vp.writes_of(Reg::Cr3), 0, "a control register nobody wrote goes back unwritten");
        assert_eq!(vp.writes_of(Reg::Es), 0, "and so does a segment");
        assert_eq!(vp.xsave_writes(), 0, "and the vector file");
        assert!(!ex.owes_the_partition());

        ex.import_at_park(&vp, cpu, &mut xs).unwrap();
        cpu.export_arch_groups(&mut state, ArchGroups::VECTOR);
        state.vector[0][0] = 0x99;
        cpu.import_arch_groups(&state, ArchGroups::VECTOR).unwrap();
        let handback = ex.export_owed(&vp, cpu, &mut xs).unwrap();
        assert_eq!(handback.written, ArchGroups::VECTOR);
        assert_eq!(vp.xsave_writes(), 1, "a changed vector file crosses as its area");
        assert_eq!(vp.writes_of(Reg::Rbx), 1, "and the registers do not cross again");
    }

    /// A second park before any entry keeps what the first one read.
    ///
    /// A resume that lands and is withdrawn by the next pause before the
    /// thread reaches its hand-back parks the processor twice with no entry
    /// between. The second park reads nothing — every group is still the
    /// shadow's — and must not forget what the first one read, or the next
    /// hand-back writes every group whole for a caller who wrote one.
    #[test]
    fn a_second_park_before_any_entry_keeps_what_the_first_one_read() {
        let vp = recorder_with_an_area();
        vp.seed(Reg::Rbx, 0x1111);
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_at_park(&vp, cpu, &mut xs).unwrap();
        ex.import_at_park(&vp, cpu, &mut xs).unwrap();
        let mut state = VcpuArchState::default();
        cpu.export_arch_groups(&mut state, ArchGroups::GPRS);
        state.gprs[3] = 0x5A;
        cpu.import_arch_groups(&state, ArchGroups::GPRS).unwrap();

        let handback = ex.export_owed(&vp, cpu, &mut xs).unwrap();
        assert_eq!(
            handback.written,
            ArchGroups::GPRS,
            "only the group the caller wrote crosses back"
        );
        assert_eq!(vp.value_of(Reg::Rbx), 0x5A);
        assert_eq!(vp.xsave_writes(), 0);
    }

    /// A pause nobody writes through makes no platform write at all.
    #[test]
    fn a_pause_nobody_writes_through_writes_nothing() {
        let vp = recorder_with_an_area();
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_at_park(&vp, cpu, &mut xs).unwrap();
        assert!(ex.owes_the_partition(), "the park took every group");
        let handback = ex.export_owed(&vp, cpu, &mut xs).unwrap();
        assert_eq!(handback.written, ArchGroups::empty());
        assert_eq!(handback.calls, 0);
        assert_eq!(vp.writes_of(Reg::Rip), 0);
        assert_eq!(vp.xsave_writes(), 0);
        assert!(!ex.owes_the_partition(), "and every group is the partition's again");
    }

    /// Adoption installs the machine's processor whole before its first
    /// entry — everything but the counter the hardware owns and the interrupt
    /// state a fresh processor already holds.
    ///
    /// The partition a machine is adopted into was created after the machine
    /// was, and holds the platform's reset state; the machine's own processor
    /// stands wherever the machine left it.
    #[test]
    fn an_adopted_processor_is_installed_whole_before_its_first_entry() {
        let vp = recorder_with_an_area();
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::installing();
        assert!(ex.owes_the_partition(), "the machine's processor is owed before any entry");

        // A park before the first entry reads nothing: the machine's
        // processor is the one owed, not the platform's reset state.
        ex.import_at_park(&vp, cpu, &mut xs).unwrap();
        assert_eq!(vp.reads_of(Reg::Rip), 0, "an install is not overwritten by the partition");

        let handback = ex.export_owed(&vp, cpu, &mut xs).unwrap();
        assert_eq!(
            handback.written,
            ArchGroups::all().difference(ArchGroups::INTERRUPT_STATE)
        );
        assert_eq!(vp.value_of(Reg::Rip), CODE, "the partition starts where the machine stood");
        for reg in [Reg::Rax, Reg::Rip, Reg::Cr0, Reg::Dr7, Reg::Efer, Reg::Es, Reg::Gdtr] {
            assert_eq!(vp.writes_of(reg), 1, "{reg:?} is installed");
        }
        assert_eq!(vp.xsave_writes(), 1, "and the vector file");
        assert_eq!(vp.writes_of(Reg::Tsc), 0, "never the counter the hardware owns");
        assert_eq!(vp.writes_of(Reg::InterruptState), 0);
        assert!(!ex.owes_the_partition());
    }

    /// A pause leaves the interrupt shadow and the NMI mask with the
    /// partition, however the caller writes the flags.
    ///
    /// Reading them home would oblige the next entry to write back the shadow
    /// bit the import lapses, ending an `STI` window the guest never finished.
    #[test]
    fn a_pause_leaves_the_interrupt_state_with_the_partition() {
        let vp = recorder_with_an_area();
        // An STI window, and NMIs masked.
        vp.seed(Reg::InterruptState, 0b11);
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_at_park(&vp, cpu, &mut xs).unwrap();
        assert_eq!(vp.reads_of(Reg::InterruptState), 0);
        let mut state = VcpuArchState::default();
        cpu.export_arch_groups(&mut state, ArchGroups::RIP_RFLAGS);
        state.rflags = 0x0202;
        cpu.import_arch_groups(&state, ArchGroups::RIP_RFLAGS).unwrap();

        let handback = ex.export_owed(&vp, cpu, &mut xs).unwrap();
        assert_eq!(handback.written, ArchGroups::RIP_RFLAGS);
        assert_eq!(vp.value_of(Reg::Rflags), 0x0202);
        assert_eq!(vp.writes_of(Reg::InterruptState), 0);
        assert_eq!(vp.value_of(Reg::InterruptState), 0b11);
    }

    /// The counter a pause reads is the one the machine answers with, and
    /// nothing writes it back.
    #[test]
    fn the_counter_a_pause_reads_is_the_one_the_machine_answers_with() {
        let vp = recorder_with_an_area();
        vp.seed(Reg::Tsc, 0x1234_5678_9ABC);
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut xs = XsaveArea::read_from(&vp, HostComponents::of_this_host()).expect("an area");
        let mut ex = Exchange::at_reset();

        ex.import_at_park(&vp, cpu, &mut xs).unwrap();
        assert_eq!(cpu.time_stamp_counter(), 0x1234_5678_9ABC);
        ex.export_owed(&vp, cpu, &mut xs).unwrap();
        assert_eq!(vp.writes_of(Reg::Tsc), 0);
    }

    #[test]
    fn the_header_copy_costs_no_platform_call_and_lands_in_the_shadow() {
        let vp = recorder_with_an_area();
        let mut machine = machine_running(&[]);
        let cpu = machine.processor(0).into_cpu();
        let mut inject = InjectState::at_reset();
        let mut ex = Exchange::at_reset();

        let header = VpContext {
            rip: 0x1234,
            rflags: 0x202,
            cs: SegmentRegister {
                base: 0,
                limit: 0xFFFF,
                selector: 0,
                attributes: 0x93,
            },
            instruction_length: 2,
            cr8: 3,
            execution_state: 0,
        };
        ex.take_header(cpu, &header, &mut inject).unwrap();

        let mut shadow = VcpuArchState::default();
        cpu.export_arch_groups(
            &mut shadow,
            ArchGroups::RIP_RFLAGS | ArchGroups::CONTROL_REGS,
        );
        assert_eq!(
            (shadow.rip, shadow.rflags, shadow.cr8),
            (0x1234, 0x202, 3),
            "RIP, RFLAGS and CR8→TPR landed"
        );
        assert_eq!(vp.total_reads(), 0);
    }

    /// Every class that RETIRES an instruction on the shadow must import the
    /// state the processor's mode is derived from.
    ///
    /// The mode decides how wide the retirement is. `CR0.PE` says real or
    /// protected (`CONTROL_REGS`) and `EFER.LMA` says long (`MSRS`); a shadow
    /// missing either derives some other processor's mode and advances `RIP` by
    /// the wrong width. Measured, before this test existed: a `CPUID` exit
    /// imported neither, so the shadow still held `CR0 = 0x60000010` from reset
    /// — `PE` clear — believed itself in real mode, and retired `CPUID` at
    /// `0xe2adc` to `0x2ade` instead of `0xe2ade`. The guest resumed in the
    /// middle of nothing and triple-faulted; the whole BIOS boot died on one
    /// instruction, and the census said `cpuid 1`.
    ///
    /// Stated over the closed set rather than against a list of known-good
    /// classes, so a class added later has to answer it too. A plain port
    /// access is not among them because it is not a class: it retires nothing
    /// on the shadow — the platform decoded the access and
    /// `service_port_access` writes `RIP` and `RAX` back as two words — so no
    /// mode of any kind is consulted.
    #[test]
    fn every_class_that_retires_on_the_shadow_imports_what_its_mode_derives_from() {
        // `CR0` for real-versus-protected, `EFER` for long: the two groups the
        // fetch-mode derivation reads that a guest can change on hardware
        // without ever taking an exit.
        let mode_inputs = ArchGroups::CONTROL_REGS.union(ArchGroups::MSRS);
        for class in [
            ExitClass::StringPort,
            ExitClass::Mmio,
            ExitClass::Cpuid,
            ExitClass::Msr,
            ExitClass::Exception,
            ExitClass::Full,
        ] {
            assert!(
                needs(class).contains(mode_inputs),
                "{class:?} retires an instruction on the shadow but imports \
                 {:?}, which is missing {:?} — the shadow would derive its mode \
                 from stale state and advance RIP by the wrong width",
                needs(class),
                mode_inputs.difference(needs(class))
            );
        }
    }
}
