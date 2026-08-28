//! Running a `rusty_box` machine's guest on the Windows Hypervisor Platform.
//!
//! The adapter, and the only crate that knows both sides. `rusty_box_whp` is a
//! thin leaf over `WinHvPlatform` that knows nothing about a machine, and
//! `rusty_box` is a machine that knows nothing about a hypervisor; neither
//! depends on the other, and a binary that wants this pairing depends on this.
//!
//! That is not tidiness. The leaf stays a leaf — its `unsafe` confined to one
//! file, its tests running on a host with no hypervisor, no emulator dragged in
//! behind it — and the emulator keeps building for wasm, UEFI and bare metal,
//! none of which can link a host hypervisor. An engine is chosen where a
//! machine is built, which is the binary.
//!
//! ## What the machine had to offer for this to live outside it
//!
//! One verb: [`rusty_box::emulator::PcIo::emulate_one`]. An engine running the
//! guest on real hardware still meets accesses the hardware will not finish,
//! and finishing one means decoding it. The decoder belongs to the emulator, so
//! the emulator offers a single instruction executed against its own parts —
//! and an access serviced that way is bit-identical to the same access under
//! the interpreter, which is the property the whole design rests on.
//!
//! ## The one unsafe obligation
//!
//! Installing the machine's memory into a partition hands the hypervisor host
//! addresses that outlive the borrow they came from, and no lifetime can say
//! otherwise: a partition stored beside the memory it maps cannot borrow its
//! sibling. `rusty_box_whp` puts that contract in a signature; this crate is
//! where it is discharged, because this is the crate that knows the machine
//! owns both the allocation and the engine. It is one call, in one function,
//! and it says so.
//!
//! ## One machine on this engine per process
//!
//! Measured, in `rusty_box_whp`'s `a_process_holds_one_partition_at_a_time`:
//! the platform does not materialise its backing partition until memory is
//! mapped, and it names that partition after the process — so a second machine
//! on this engine is refused at its first map with
//! `ERROR_VID_PARTITION_ALREADY_EXISTS`, however the two are arranged. A fleet
//! on hardware is therefore a fleet of processes. Software-backed machines are
//! unaffected and any number may run alongside, which is what makes a
//! mixed-engine comparison in one process possible. Partitions ARE serially
//! reusable: dropping one frees the name.
#![expect(
    unsafe_code,
    reason = "UNSAFETY: mapping the machine's own memory into a partition — see map_window"
)]

mod engine;
mod state;

pub use engine::WhpEngine;

use rusty_box_whp::{Partition, Reg, SegmentRegister, TableRegister, WhpResult};

/// One virtual processor of a partition, as the state exchange addresses it.
///
/// The platform's register calls all take a partition and an index; binding the
/// two together is what lets [`state`] be written against a processor rather
/// than against a partition plus a number it has to carry everywhere.
pub(crate) struct Vp<'a> {
    partition: &'a Partition,
    index: u32,
}

impl<'a> Vp<'a> {
    pub(crate) const fn new(partition: &'a Partition, index: u32) -> Self {
        Self { partition, index }
    }
}

impl state::VpRegisters for Vp<'_> {
    fn read_words(&self, regs: &[Reg], out: &mut [u64]) -> WhpResult<()> {
        self.partition.read_regs(self.index, regs, out)
    }

    fn write_words(&self, regs: &[Reg], words: &[u64]) -> WhpResult<()> {
        self.partition.write_regs(self.index, regs, words)
    }

    fn read_segments(&self, regs: &[Reg], out: &mut [SegmentRegister]) -> WhpResult<()> {
        self.partition.read_segments(self.index, regs, out)
    }

    fn write_segments(&self, regs: &[Reg], segments: &[SegmentRegister]) -> WhpResult<()> {
        self.partition.write_segments(self.index, regs, segments)
    }

    fn read_tables(&self, regs: &[Reg], out: &mut [TableRegister]) -> WhpResult<()> {
        self.partition.read_tables(self.index, regs, out)
    }

    fn write_tables(&self, regs: &[Reg], tables: &[TableRegister]) -> WhpResult<()> {
        self.partition.write_tables(self.index, regs, tables)
    }
}

#[cfg(test)]
mod tests {
    use super::WhpEngine;
    use rusty_box::cpu::instrumentation::{CpuSetupMode, X86Reg};
    use rusty_box::emulator::{
        Emulator, EmulatorConfig, EngineRefusal, MemorySize, RunBudget, StopReason,
    };
    use rusty_box::Error;

    /// Where the guest's code goes, and the port it writes to.
    const CODE: u64 = 0x1000;
    /// Port 0xE9, the chipset debug console: a byte written here reaches the
    /// machine's own debug port, which is host-readable — so the assertion is
    /// about what the GUEST did, not about what the engine returned.
    const DEBUG_PORT: u8 = 0xE9;
    const MARK: u8 = 0x5A;

    /// A real-mode machine on this engine, with `code` loaded at [`CODE`] and
    /// its processor pointed at it.
    fn machine_running(code: &[u8]) -> std::boxed::Box<Emulator<(), WhpEngine>> {
        let config = EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            ..EmulatorConfig::default()
        };
        let mut machine =
            Emulator::<(), WhpEngine>::with_engine(config, CpuSetupMode::RealMode)
                .expect("machine");
        machine.mem_write(CODE, code).expect("load");
        machine.reg_write(X86Reg::Rip, CODE);
        machine
    }

    /// Whether this host can run the hypervisor-gated tests below.
    fn hypervisor_here() -> bool {
        if rusty_box_whp::hypervisor_present().unwrap_or(false) {
            return true;
        }
        eprintln!("skipped: this host has no Windows Hypervisor Platform");
        false
    }

    /// The turn a test takes before starting a machine on hardware.
    ///
    /// A process holds one partition at a time — measured in `rusty_box_whp`'s
    /// `a_process_holds_one_partition_at_a_time`, where a second live
    /// partition's first map is refused with
    /// `ERROR_VID_PARTITION_ALREADY_EXISTS`. Libtest runs tests on several
    /// threads, so without this two machines would start their engines at once
    /// and the platform would refuse one of them. That is a fact about the
    /// platform rather than about anything under test, so the tests take turns.
    ///
    /// Taken before the machine is built, so the machine — and with it the
    /// partition — is dropped before the turn passes on.
    fn a_turn_on_the_hardware() -> std::sync::MutexGuard<'static, ()> {
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        // A test that panicked while holding the turn poisoned nothing: the
        // guard protects an ordering, not a value.
        TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A machine running on the host's hypervisor executes a guest, and its
    /// output arrives in the machine's own devices.
    ///
    /// The end-to-end proof of the adapter: the partition is built from the
    /// machine's own memory plan, the shadow processor's state is installed on
    /// the platform, a real guest instruction runs on hardware, the port write
    /// it makes exits to this port's device set, and the halt hands the machine
    /// back. Nothing here is mocked.
    ///
    /// Skipped, with a reason, on a host without the platform — which is every
    /// CI runner, since none offers nested virtualisation.
    #[test]
    fn a_machine_on_the_hypervisor_runs_a_guest_and_its_output_reaches_the_devices() {
        if !hypervisor_here() {
            return;
        }

        let _turn = a_turn_on_the_hardware();
        // mov al, MARK ; out 0xE9, al ; hlt
        let mut machine = machine_running(&[0xB0, MARK, 0xE6, DEBUG_PORT, 0xF4]);
        let outcome = machine
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest");

        let written: std::vec::Vec<u8> = machine.debug_port().take_output().collect();
        assert_eq!(
            written,
            std::vec![MARK],
            "the byte the guest sent to port 0xE9 must arrive in this machine's devices"
        );
        assert_eq!(
            outcome.stop,
            StopReason::Halted,
            "the guest's HLT must reach the machine, or nothing stops scheduling a \
             processor that is no longer running"
        );
    }

    /// A machine on hardware reports how far it got as guest time.
    ///
    /// The unit is the engine's to state and the machine's to pass on. A
    /// machine that relabelled it as a count of instructions would report zero
    /// for every run — a number that is not merely useless but wrong, since
    /// the guest did advance.
    #[test]
    fn a_hardware_run_reports_its_progress_as_guest_time() {
        if !hypervisor_here() {
            return;
        }

        let _turn = a_turn_on_the_hardware();
        // out 0xE9, al ; hlt — enough to reach an exit and come back.
        let mut machine = machine_running(&[0xE6, DEBUG_PORT, 0xF4]);
        let outcome = machine
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest");

        assert!(
            outcome.progress.ticks().is_some(),
            "a machine whose engine counts no instructions must report time: got {:?}",
            outcome.progress
        );
    }

    /// An access the hardware cannot finish is finished by the shadow
    /// processor, against the machine's own devices.
    ///
    /// The guest stores a byte into video memory and loads it straight back.
    /// Neither access is in the partition's map — video memory is a device
    /// window — so each traps with no instruction length and, for the store,
    /// possibly no instruction bytes either; the platform cannot finish or
    /// even skip them. The byte arriving at the debug port is proof that both
    /// were executed on the shadow, that the machine's own VGA answered them,
    /// and that the processor the platform resumed was the one the shadow left
    /// behind.
    #[test]
    fn an_access_the_hardware_cannot_finish_is_serviced_by_the_machine() {
        if !hypervisor_here() {
            return;
        }

        let _turn = a_turn_on_the_hardware();
        let mut machine = machine_running(&[
            0xB8, 0x00, 0xB8, // mov ax, 0xB800   — the text-mode video segment
            0x8E, 0xC0, //       mov es, ax
            0x26, 0xC6, 0x06, 0x00, 0x00, MARK, // mov byte [es:0], MARK
            0x26, 0xA0, 0x00, 0x00, //           mov al, [es:0]
            0xE6, DEBUG_PORT, //                 out 0xE9, al
            0xF4, //                             hlt
        ]);
        machine
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest and the shadow finishes its accesses");

        let written: std::vec::Vec<u8> = machine.debug_port().take_output().collect();
        assert_eq!(
            written,
            std::vec![MARK],
            "the byte must survive a store and a load that only the shadow processor \
             could carry out"
        );
    }

    /// An instruction budget is refused, not silently never spent.
    ///
    /// Nothing this machine can measure advances per instruction, so the loop
    /// that spends such a budget would run until the guest stopped on its own.
    /// Needs no hypervisor: the ask is answered from what the engine is, before
    /// anything runs.
    #[test]
    fn an_instruction_budget_is_refused_by_an_engine_that_counts_none() {
        let mut machine = machine_running(&[0xF4]);
        match machine.step(RunBudget::Instructions(10)) {
            Err(Error::Engine(EngineRefusal::NoInstructionCount)) => {}
            other => panic!("an instruction budget must be refused, not accepted: {other:?}"),
        }
    }
}
