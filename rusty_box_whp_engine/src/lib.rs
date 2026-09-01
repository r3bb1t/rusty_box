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

mod alarm;
mod engine;
mod state;
mod xsave;

pub use engine::{ExitCounts, PlatformCounters, SliceCensus, WhpEngine};

/// Re-exported so a caller reading [`WhpEngine::platform_counters`] need not
/// also name the platform crate to spell what it returns. These are that
/// crate's types, not this one's — the hypervisor's own accounting, which this
/// engine forwards rather than defines.
pub use rusty_box_whp::{InterceptCounter, InterceptCounters, RuntimeCounters};

/// Whether this host can run a guest on the hardware at all.
///
/// Re-exported because it is the first question a caller of this engine has,
/// and asking it should not require naming the platform crate underneath: a
/// machine that selects an engine wants to know whether the one it chose
/// exists before it builds anything.
///
/// # Errors
/// Whatever the platform said when asked.
pub use rusty_box_whp::hypervisor_present;

use rusty_box_whp::{Partition, Reg, RegisterValue, WhpResult};

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

    fn read_registers(&self, regs: &[Reg], out: &mut [RegisterValue]) -> WhpResult<()> {
        self.partition.read_registers(self.index, regs, out)
    }

    fn write_registers(&self, regs: &[Reg], values: &[RegisterValue]) -> WhpResult<()> {
        self.partition.write_registers(self.index, regs, values)
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

    /// A guest asking what processor it is on gets this port's answer, not the
    /// host's — and does not learn that it is virtualised.
    ///
    /// The guest reads `CPUID` leaf 1 on both engines and the answers must
    /// match — family, model and stepping, byte for byte, plus `ECX` bit 31,
    /// `HypervisorPresent`, which this port's model leaves clear because a
    /// guest that can see it is a guest that knows.
    ///
    /// Measured while writing it, so the assertion is known to bite rather
    /// than assumed to: with the trapping removed this host answers
    /// `0x000906A3` where the interpreter answers `0x00050654`. Worth knowing
    /// alongside it — this platform does NOT set `HypervisorPresent` for a
    /// partition configured like ours, so that bit alone would prove nothing;
    /// the model bytes are what carry the test.
    ///
    /// A host whose own processor happened to be this port's exact model and
    /// stepping would make it vacuous. Nothing can be done about that from
    /// inside the test, and the equality it asserts is the property that
    /// matters either way: two engines, one answer.
    #[test]
    fn a_guest_on_hardware_is_told_the_same_processor_the_interpreter_tells_it() {
        if !hypervisor_here() {
            return;
        }

        // mov eax,1 ; cpuid ; then EAX and ECX out a byte at a time
        let asking: &[u8] = &[
            0x66, 0xB8, 0x01, 0x00, 0x00, 0x00, //  mov eax, 1
            0x0F, 0xA2, //                          cpuid
            0xE6, DEBUG_PORT, //                    out 0xE9, al
            0x66, 0xC1, 0xE8, 0x08, //              shr eax, 8
            0xE6, DEBUG_PORT,
            0x66, 0xC1, 0xE8, 0x08,
            0xE6, DEBUG_PORT,
            0x66, 0xC1, 0xE8, 0x08,
            0xE6, DEBUG_PORT,
            0x66, 0xC1, 0xE9, 0x1F, //              shr ecx, 31
            0x88, 0xC8, //                          mov al, cl
            0xE6, DEBUG_PORT, //                    out 0xE9, al
            0xF4, //                                hlt
        ];

        let config = EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            ..EmulatorConfig::default()
        };
        let mut interpreted =
            Emulator::new_with_mode(config, CpuSetupMode::RealMode).expect("machine");
        interpreted.mem_write(CODE, asking).expect("load");
        interpreted.reg_write(X86Reg::Rip, CODE);
        interpreted
            .step(RunBudget::Ticks(1_000_000))
            .expect("the interpreter runs the guest");
        let by_the_interpreter: std::vec::Vec<u8> =
            interpreted.debug_port().take_output().collect();

        let _turn = a_turn_on_the_hardware();
        let mut on_hardware = machine_running(asking);
        on_hardware
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest");
        let by_the_hardware: std::vec::Vec<u8> =
            on_hardware.debug_port().take_output().collect();

        assert_eq!(
            by_the_interpreter.last(),
            Some(&0),
            "this port's own model does not claim to be running under a hypervisor"
        );
        assert_eq!(
            by_the_hardware, by_the_interpreter,
            "a guest must not be able to tell the two engines apart by asking the \
             processor about itself"
        );
    }

    /// A guest reading a model-specific register reads this port's, not the
    /// host's.
    ///
    /// `IA32_MISC_ENABLE` is a register every x86 has and no two agree on, so
    /// leaving it to the host would be as loud a divergence as `CPUID` — and
    /// unlike `CPUID` it needs no exit list, just the platform's MSR exit.
    /// Serviced on the shadow, which is the same path a trapped memory access
    /// and a trapped `CPUID` take.
    #[test]
    fn a_guest_on_hardware_reads_this_ports_model_specific_registers() {
        if !hypervisor_here() {
            return;
        }

        // mov ecx,0x1A0 ; rdmsr ; then EAX out a byte at a time
        let asking: &[u8] = &[
            0x66, 0xB9, 0xA0, 0x01, 0x00, 0x00, //  mov ecx, IA32_MISC_ENABLE
            0x0F, 0x32, //                          rdmsr
            0xE6, DEBUG_PORT,
            0x66, 0xC1, 0xE8, 0x08,
            0xE6, DEBUG_PORT,
            0x66, 0xC1, 0xE8, 0x08,
            0xE6, DEBUG_PORT,
            0x66, 0xC1, 0xE8, 0x08,
            0xE6, DEBUG_PORT,
            0xF4, //                                hlt
        ];

        let config = EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            ..EmulatorConfig::default()
        };
        let mut interpreted =
            Emulator::new_with_mode(config, CpuSetupMode::RealMode).expect("machine");
        interpreted.mem_write(CODE, asking).expect("load");
        interpreted.reg_write(X86Reg::Rip, CODE);
        interpreted
            .step(RunBudget::Ticks(1_000_000))
            .expect("the interpreter runs the guest");
        let by_the_interpreter: std::vec::Vec<u8> =
            interpreted.debug_port().take_output().collect();

        let _turn = a_turn_on_the_hardware();
        let mut on_hardware = machine_running(asking);
        on_hardware
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest");
        let by_the_hardware: std::vec::Vec<u8> =
            on_hardware.debug_port().take_output().collect();

        assert_eq!(
            by_the_interpreter.len(),
            4,
            "the guest must have completed its four writes on the interpreter"
        );
        assert_eq!(
            by_the_hardware, by_the_interpreter,
            "a model-specific register must read the same under both engines"
        );
    }

    /// A slice that services a port write and then halts is ONE slice with two
    /// exits, not two slices with one each.
    ///
    /// The distinction is the whole subject of this work: an engine that leaves
    /// the partition after every exit buys one VM entry per exit and runs the
    /// guest for approximately no time.
    #[test]
    fn a_port_write_and_a_halt_are_two_exits_in_one_slice() {
        if !hypervisor_here() {
            return;
        }

        let _turn = a_turn_on_the_hardware();
        // out 0xE9, al ; hlt
        let mut machine = machine_running(&[0xE6, DEBUG_PORT, 0xF4]);
        machine
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest");

        let census = machine.engine().census();
        assert_eq!(census.slices, 1, "one step is one slice; census was {census:?}");
        assert_eq!(
            census.exits_per_slice[2], 1,
            "that slice held two exits — the OUT and the HLT — so the 2-bucket has one \
             entry; census was {census:?}"
        );
        assert_eq!(
            census.ended_halted, 1,
            "the guest's own HLT is what ended it, so no boundary question may claim \
             the slice; census was {census:?}"
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
