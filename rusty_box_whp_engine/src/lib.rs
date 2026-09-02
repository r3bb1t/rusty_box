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

pub use engine::{ExitCounts, InjectCensus, PlatformCounters, SliceCensus, WhpEngine};

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
        Emulator, EmulatorConfig, EngineRefusal, MachineBuilder, MemorySize, RunBudget,
        StopReason,
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

    /// A real-mode machine with the FULL device set — the 8259 pair, the PIT,
    /// their port registrations and their timers — on this engine.
    ///
    /// [`machine_running`] deliberately skips hardware initialisation, which
    /// serves a guest that only touches the debug port; a guest that programs
    /// the PIC and the PIT needs the machine [`MachineBuilder`] furnishes,
    /// where those ports are registered and answer. No BIOS is loaded — the
    /// processor is re-pointed at the test's own code instead.
    fn machine_with_devices(code: &[u8]) -> std::boxed::Box<Emulator<(), WhpEngine>> {
        let config = EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            ..EmulatorConfig::default()
        };
        let mut machine = MachineBuilder::new(config)
            .build_on::<WhpEngine>()
            .expect("machine");
        machine
            .setup_cpu_mode(CpuSetupMode::RealMode)
            .expect("real mode");
        machine.mem_write(CODE, code).expect("load");
        machine.reg_write(X86Reg::Rip, CODE);
        machine
    }

    /// Whether this host can run the hypervisor-gated tests below.
    /// The words a guest recorded at 0x6000, in order, once it has reported
    /// how many on the debug port. Bounded by steps rather than by a halt,
    /// because a guest that sleeps mid-way halts twice and only the second
    /// is final; a guest that never wakes fails here by name.
    fn trace_of<E: rusty_box::emulator::SliceEngine<()>>(
        machine: &mut Emulator<(), E>,
    ) -> std::vec::Vec<u16> {
        let mut reported: std::vec::Vec<u8> = std::vec::Vec::new();
        for _ in 0..256 {
            machine
                .step(RunBudget::Ticks(1_000_000))
                .expect("the engine runs the guest and its tick reaches it");
            reported.extend(machine.debug_port().take_output());
            if !reported.is_empty() {
                break;
            }
        }
        let count = *reported
            .last()
            .expect("the guest reported its record count — it never woke");
        machine
            .mem_read_vec(0x6000, usize::from(count) * 2)
            .expect("the record buffer is readable")
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    }

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

    /// A slice that ends on the tail of a shadow errand ends WITHOUT reading
    /// the processor back from the partition — the hardware has not run
    /// since the errand's imposition, so the shadow already describes it —
    /// and the machine reads the same processor the interpreter leaves.
    ///
    /// The guest stores into video memory eight times, one device-memory
    /// exit each, which is the run length at which the engine stops finishing
    /// them one at a time and bursts the rest onto the shadow; the burst then
    /// carries the load, the port write and the `HLT`. That halt is the
    /// shadow's own, not a hardware `Halt` exit — the halt-exit count says
    /// so — and the slice ends at the errand's tail. The byte at the debug
    /// port, the halt reaching the machine and a `RIP` equal to the
    /// interpreter's are the guest-visible half; `read_backs_skipped` is the
    /// proof that the skip fired rather than merely compiled. This engine
    /// once carried a skip that never fired, because it compared a register
    /// the exchange never sends.
    #[test]
    fn a_slice_ending_on_an_errand_skips_the_read_back_and_the_machine_reads_the_shadow() {
        if !hypervisor_here() {
            return;
        }

        let code: &[u8] = &[
            0xB8, 0x00, 0xB8, // mov ax, 0xB800 — the text-mode video segment
            0x8E, 0xC0, //       mov es, ax
            // Eight stores to device memory: one memory exit each, and the
            // eighth is the one the engine bursts from.
            0x26, 0xC6, 0x06, 0x00, 0x00, MARK, // mov byte [es:0], MARK
            0x26, 0xC6, 0x06, 0x00, 0x00, MARK,
            0x26, 0xC6, 0x06, 0x00, 0x00, MARK,
            0x26, 0xC6, 0x06, 0x00, 0x00, MARK,
            0x26, 0xC6, 0x06, 0x00, 0x00, MARK,
            0x26, 0xC6, 0x06, 0x00, 0x00, MARK,
            0x26, 0xC6, 0x06, 0x00, 0x00, MARK,
            0x26, 0xC6, 0x06, 0x00, 0x00, MARK,
            0x26, 0xA0, 0x00, 0x00, //           mov al, [es:0]
            0xE6, DEBUG_PORT, //                 out 0xE9, al
            0xF4, //                             hlt
        ];

        let config = EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            ..EmulatorConfig::default()
        };
        let mut interpreted =
            Emulator::new_with_mode(config, CpuSetupMode::RealMode).expect("machine");
        interpreted.mem_write(CODE, code).expect("load");
        interpreted.reg_write(X86Reg::Rip, CODE);
        interpreted
            .step(RunBudget::Ticks(1_000_000))
            .expect("the interpreter runs the guest");
        let by_the_interpreter: std::vec::Vec<u8> =
            interpreted.debug_port().take_output().collect();
        let rip_by_the_interpreter = interpreted.rip();

        let _turn = a_turn_on_the_hardware();
        let mut machine = machine_running(code);
        let outcome = machine
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest and the shadow bursts its accesses");

        let written: std::vec::Vec<u8> = machine.debug_port().take_output().collect();
        assert_eq!(written, by_the_interpreter, "the byte must arrive under both engines");
        assert_eq!(
            outcome.stop,
            StopReason::Halted,
            "the shadow's own halt must reach the machine exactly as a hardware halt does"
        );
        assert_eq!(
            machine.rip(),
            rip_by_the_interpreter,
            "with no read-back, the shadow is what the machine reads, and it must be \
             the processor the errand left — identical to the interpreter's"
        );

        let exits = machine.engine().exits();
        let census = machine.engine().census();
        assert_eq!(
            exits.halt, 0,
            "the halt must be the shadow's — retired inside the burst — or the slice \
             did not end on an errand's tail and the property under test was not \
             reached; exits were {exits:?}"
        );
        assert_eq!(census.slices, 1, "one step is one slice; census was {census:?}");
        assert_eq!(census.ended_halted, 1, "the shadow's halt ended it; census was {census:?}");
        assert_eq!(
            census.read_backs_skipped, 1,
            "a slice that ends on an errand's tail must skip the read-back — a skip \
             that never fires is a failed change, not a neutral one; census was {census:?}"
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

    /// A device interrupt reaches a guest running on hardware BY INJECTION —
    /// written into the partition's pending-event slot — not by ending the
    /// slice so the shadow can deliver it.
    ///
    /// The guest exercises the whole delivery stack end to end: it points the
    /// real-mode IVT entry for the PIT's vector (IRQ0 at the master 8259's
    /// power-on offset 8, so IVT slot 8 at physical 0x20) at its own ISR,
    /// unmasks IRQ0 alone, programs PIT channel 0 as a rate generator, and
    /// then runs TRIALS: each trial polls the 8259's IRR with interrupts
    /// disabled until a tick is pending — so the tick can never be taken at
    /// a slice head — and only then runs `STI`. Delivery becomes possible in
    /// the middle of a slice, which is exactly the moment the engine must
    /// arm a window, take the window exit, acknowledge, and inject.
    ///
    /// Many trials rather than one, because which processor runs a given
    /// stretch is the engine's own business: a slice whose budget is nearer
    /// than the hardware's resolution runs on the shadow (this machine's
    /// 8042 serial-delay timer ticks every 150 microseconds, so such slices
    /// are routine), and a trial the shadow happens to carry is delivered by
    /// the interpreter, legitimately, with nothing to inject. Each trial the
    /// HARDWARE carries must inject; one success proves the path, and thirty-two
    /// trials make the chance that every one landed on the shadow vanish.
    ///
    /// The first assertion — MARK at the debug port — proves the vector
    /// genuinely reached the guest's own handler. The census assertions
    /// prove HOW: through the partition's pending-event slot. They are what
    /// tell injection apart from the slice-ending shadow path, which
    /// delivers the same MARK and would pass the first assertion alone.
    #[test]
    fn a_device_interrupt_reaches_a_hardware_guest_by_injection() {
        if !hypervisor_here() {
            return;
        }

        let _turn = a_turn_on_the_hardware();
        // The ISR lives inside the loaded blob, at CODE + 0x35; the IVT entry
        // the guest writes names that absolute offset (CS base is 0).
        const ISR: [u8; 2] = [0x35, 0x10];
        let mut machine = machine_with_devices(&[
            0x31, 0xC0, //                   xor ax, ax
            0x8E, 0xD8, //                   mov ds, ax
            0xC7, 0x06, 0x20, 0x00, ISR[0], ISR[1], // mov word [0x20], isr
            0xC7, 0x06, 0x22, 0x00, 0x00, 0x00, //     mov word [0x22], 0
            0xB0, 0xFE, //                   mov al, 0xFE — unmask IRQ0 alone
            0xE6, 0x21, //                   out 0x21, al  (OCW1)
            0xB0, 0x34, //                   mov al, 0x34 — ch0, lo/hi, mode 2
            0xE6, 0x43, //                   out 0x43, al
            0xB0, 0x00, //                   mov al, 0x00 — count 0x0400, low
            0xE6, 0x40, //                   out 0x40, al
            0xB0, 0x04, //                   mov al, 0x04 — count 0x0400, high
            0xE6, 0x40, //                   out 0x40, al
            0xB0, 0x0A, //                   mov al, 0x0A — OCW3: reads answer IRR
            0xE6, 0x20, //                   out 0x20, al
            0xB9, 0x00, 0x40, //             mov cx, 0x4000 — many trials, so a
            //                               run is never starved of a hardware
            //                               slice by the shadow-vs-hardware race
            // trial (CODE+0x27):
            0xFA, //                         cli — the tick must wait for STI
            // poll (CODE+0x28):
            0xE4, 0x20, //                   in al, 0x20 — the IRR
            0xA8, 0x01, //                   test al, 1  — is IRQ0 pending?
            0x74, 0xFA, //                   jz poll
            0xFB, //                         sti — delivery opens MID-SLICE
            0x90, //                         nop — the STI shadow lapses; the
            //                               tick is taken here, one way or the
            //                               other
            0xE2, 0xF5, //                   loop trial
            // park (CODE+0x32):
            0xF4, //                         hlt
            0xEB, 0xFD, //                   jmp park
            // isr (CODE+0x35):
            0xB0, MARK, //                   mov al, MARK
            0xE6, DEBUG_PORT, //             out 0xE9, al
            0xB0, 0x20, //                   mov al, 0x20
            0xE6, 0x20, //                   out 0x20, al — non-specific EOI
            0xCF, //                         iret
        ]);
        // A budget-spent machine hands each budget back to its caller;
        // stepping again is the caller's half of that contract, and the
        // guest's trial loop resumes where it left off. Bounded, so a guest
        // whose ticks never arrive fails the assertion instead of hanging the
        // suite. Stops early once an injection has happened and the handler
        // has spoken — the properties under test. Generously bounded because
        // whether a given trial runs on hardware or the shadow is a real-time
        // race, and only a hardware trial injects.
        let mut written: std::vec::Vec<u8> = std::vec::Vec::new();
        for _ in 0..40 {
            machine
                .step(RunBudget::Ticks(2_000_000))
                .expect("the hypervisor runs the guest and its ticks reach it");
            written.extend(machine.debug_port().take_output());
            if !written.is_empty() && machine.engine().inject_census().injected >= 1 {
                break;
            }
        }
        assert!(
            !written.is_empty() && written.iter().all(|byte| *byte == MARK),
            "the PIT's ticks must reach the guest's own ISR, and nothing else may \
             write the debug port: {written:#04x?}"
        );
        let census = machine.engine().inject_census();
        assert!(
            census.injected >= 1,
            "the vector must have crossed as a register write — an injection — not \
             as a shadow delivery at a slice head; census: injected {}, windows \
             armed {}",
            census.injected,
            census.windows_armed,
        );
        assert!(
            census.injected_per_vector[8] >= 1,
            "what was injected must be the PIT's own vector — IRQ0 at the master \
             8259's power-on offset, 8"
        );
        assert!(
            census.windows_armed >= 1,
            "the tick became pending while IF was clear, so a deliverability \
             window must have been armed before the STI opened delivery"
        );
        assert!(
            machine.engine().exits().window >= 1,
            "the armed window must have been answered by an interrupt-window \
             exit — armed and never answered is a wedged guest"
        );
    }

    /// The `WHvRegisterInterruptState` word an errand's imposition writes
    /// does not disturb single-step delivery: the stepped traces of the two
    /// engines are identical across an errand, and the first
    /// hardware-retired instruction after the imposition delivers its own
    /// `#DB`, in order.
    ///
    /// The question this stands guard over: WHP exposes ONE `InterruptShadow`
    /// bit where VMX distinguishes STI-blocking from MOV-SS-blocking, and
    /// MOV-SS-type blocking also suppresses the single-step trap after the
    /// next instruction — a suppressed step is LOST, not deferred. The
    /// imposition writes the interpreter's own inhibit into that bit, and
    /// under `TF` that inhibit is clear by the time it is written: the
    /// errand's tail retires the instruction a `MOV SS` shadows and delivers
    /// the owed step before imposing (`PcIo::deliver_the_trap_owed`), and
    /// taking a trap ends any inhibit (Bochs exception.cc `interrupt`). So
    /// the word written after s3 carries no shadow, and s4 (trap frame IP
    /// 0x102B) — the first hardware-retired instruction after the
    /// imposition — steps exactly as the interpreter steps it.
    ///
    /// Two baselines make the probe honest. TF-stepping with no errand
    /// proves native real-mode `#DB` delivery agrees between the engines
    /// byte for byte. And the errand guest's SECOND device-memory touch,
    /// with TF already clear, is where a latch stranded in the shadow by the
    /// first errand would fire, misattributed, as a duplicate 0x1033 entry —
    /// which the whole-trace equality shows.
    ///
    /// The errand-retired instruction's OWN step (frame IP 0x102A) is part
    /// of the same equality. The interpreter latches it for a boundary a
    /// one-instruction errand never reaches, so the errand's tail takes that
    /// boundary itself (`PcIo::deliver_the_trap_owed`), and the hardware
    /// trace carries the step exactly where the interpreter's does.
    #[test]
    fn a_single_step_trap_survives_the_imposed_inhibit() {
        if !hypervisor_here() {
            return;
        }

        // Probe 1: TF-stepping across plain instructions, no errand — the
        // baseline that native real-mode #DB delivery works on both engines.
        let baseline: &[u8] = &[
            0x31, 0xC0, // xor ax,ax
            0x8E, 0xD8, // mov ds,ax
            0x8E, 0xD0, // mov ss,ax
            0xBC, 0x00, 0x70, // mov sp,0x7000
            0xC7, 0x06, 0x04, 0x00, 0x36, 0x10, // mov word[0x0004], isr(0x1036)
            0xC7, 0x06, 0x06, 0x00, 0x00, 0x00, // mov word[0x0006], 0
            0xBF, 0x00, 0x60, // mov di,0x6000
            0x9C, 0x58, // pushf; pop ax
            0x80, 0xCC, 0x01, // or ah,1
            0x50, 0x9D, // push ax; popf — TF=1
            0x43, // inc bx (0x1F)
            0x43, // inc bx (0x20)
            0x43, // inc bx (0x21)
            0x43, // inc bx (0x22)
            0x9C, // pushf (0x23)
            0x58, // pop ax (0x24)
            0x80, 0xE4, 0xFE, // and ah,0xFE (0x25)
            0x50, // push ax (0x28)
            0x9D, // popf — TF=0 (0x29)
            0x89, 0xF8, // mov ax,di
            0x2D, 0x00, 0x60, // sub ax,0x6000
            0xD1, 0xE8, // shr ax,1
            0xE6, DEBUG_PORT, // out 0xE9,al — trap count
            0xF4, // hlt (0x33)
            0xEB, 0xFD, // jmp hlt
            // isr (0x36):
            0x55, // push bp
            0x89, 0xE5, // mov bp,sp
            0x50, // push ax
            0x8B, 0x46, 0x02, // mov ax,[bp+2] — saved IP
            0x89, 0x05, // mov [di],ax
            0x83, 0xC7, 0x02, // add di,2
            0x58, // pop ax
            0x5D, // pop bp
            0xCF, // iret
        ];

        // Probe 2: identical stepping but one instruction (s3) reads VGA
        // memory — a memory exit, an errand, an imposition. s3 retires on the
        // shadow owing its step (saved IP 0x102A); s4 is the first
        // hardware-retired instruction after the imposition, owing its own
        // step (saved IP 0x102B) — the imposed-bit question.
        let with_errand: &[u8] = &[
            0x31, 0xC0, // xor ax,ax
            0x8E, 0xD8, // mov ds,ax
            0x8E, 0xD0, // mov ss,ax
            0xBC, 0x00, 0x70, // mov sp,0x7000
            0xC7, 0x06, 0x04, 0x00, 0x43, 0x10, // mov word[0x0004], isr(0x1043)
            0xC7, 0x06, 0x06, 0x00, 0x00, 0x00, // mov word[0x0006], 0
            0xBF, 0x00, 0x60, // mov di,0x6000
            0xB8, 0x00, 0xB8, // mov ax,0xB800
            0x8E, 0xC0, // mov es,ax
            0x9C, 0x58, // pushf; pop ax
            0x80, 0xCC, 0x01, // or ah,1
            0x50, 0x9D, // push ax; popf — TF=1
            0x43, // s1 inc bx (0x24) -> trap IP 0x25
            0x43, // s2 inc bx (0x25) -> 0x26
            0x26, 0xA0, 0x00, 0x00, // s3 mov al,[es:0] (0x26) ERRAND -> 0x2A
            0x43, // s4 inc bx (0x2A) -> 0x2B  <- the imposed-bit probe point
            0x43, // s5 inc bx (0x2B) -> 0x2C
            0x9C, // pushf (0x2C)
            0x58, // pop ax (0x2D)
            0x80, 0xE4, 0xFE, // and ah,0xFE (0x2E)
            0x50, // push ax (0x31)
            0x9D, // popf — TF=0 (0x32)
            0x26, 0xA0, 0x00, 0x00, // mov al,[es:0] (0x33) — second errand,
            // TF=0: if s3's undelivered step lingers in the interpreter's
            // trap latch, it fires HERE, misattributed — a duplicate 0x1033
            // entry — instead of being merely lost
            0x89, 0xF8, // mov ax,di
            0x2D, 0x00, 0x60, // sub ax,0x6000
            0xD1, 0xE8, // shr ax,1
            0xE6, DEBUG_PORT, // out 0xE9,al
            0xF4, // hlt (0x40)
            0xEB, 0xFD, // jmp hlt
            // isr (0x43):
            0x55, 0x89, 0xE5, 0x50, // push bp; mov bp,sp; push ax
            0x8B, 0x46, 0x02, // mov ax,[bp+2]
            0x89, 0x05, // mov [di],ax
            0x83, 0xC7, 0x02, // add di,2
            0x58, 0x5D, 0xCF, // pop ax; pop bp; iret
        ];

        fn trace_on_interpreter(code: &[u8]) -> (u8, std::vec::Vec<u16>) {
            let config = EmulatorConfig {
                memory: MemorySize::bytes(8 * 1024 * 1024),
                ..EmulatorConfig::default()
            };
            let mut machine =
                Emulator::new_with_mode(config, CpuSetupMode::RealMode).expect("machine");
            machine.mem_write(CODE, code).expect("load");
            machine.reg_write(X86Reg::Rip, CODE);
            for _ in 0..8 {
                let outcome = machine
                    .step(RunBudget::Ticks(1_000_000))
                    .expect("the interpreter runs the guest");
                if outcome.stop == StopReason::Halted {
                    break;
                }
            }
            read_trace(&mut machine)
        }

        fn read_trace<E: rusty_box::emulator::SliceEngine<()>>(
            machine: &mut Emulator<(), E>,
        ) -> (u8, std::vec::Vec<u16>) {
            let count: std::vec::Vec<u8> = machine.debug_port().take_output().collect();
            let count = *count.last().expect("the guest reported its trap count");
            let raw = machine
                .mem_read_vec(0x6000, usize::from(count) * 2)
                .expect("the record buffer is readable");
            let ips = raw
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            (count, ips)
        }

        let (int_base_count, int_base) = trace_on_interpreter(baseline);
        let (int_err_count, int_err) = trace_on_interpreter(with_errand);

        let _turn = a_turn_on_the_hardware();
        let (hw_base_count, hw_base) = {
            let mut machine = machine_running(baseline);
            for _ in 0..8 {
                let outcome = machine
                    .step(RunBudget::Ticks(1_000_000))
                    .expect("the hypervisor runs the guest");
                if outcome.stop == StopReason::Halted {
                    break;
                }
            }
            read_trace(&mut machine)
        };
        let (hw_err_count, hw_err, hw_err_memory_exits) = {
            let mut machine = machine_running(with_errand);
            for _ in 0..8 {
                let outcome = machine
                    .step(RunBudget::Ticks(1_000_000))
                    .expect("the hypervisor runs the guest");
                if outcome.stop == StopReason::Halted {
                    break;
                }
            }
            let (count, ips) = read_trace(&mut machine);
            (count, ips, machine.engine().exits().memory)
        };

        eprintln!("baseline  interpreter: count {int_base_count}, trace {int_base:#06x?}");
        eprintln!("baseline  hardware:    count {hw_base_count}, trace {hw_base:#06x?}");
        eprintln!("errand    interpreter: count {int_err_count}, trace {int_err:#06x?}");
        eprintln!(
            "errand    hardware:    count {hw_err_count}, memory exits \
             {hw_err_memory_exits}, trace {hw_err:#06x?}"
        );
        eprintln!(
            "errand's own step (frame IP 0x102a) on hardware: {}",
            if hw_err.contains(&0x102A) { "DELIVERED" } else { "LOST" }
        );
        eprintln!(
            "first hardware-retired step after the imposition (frame IP 0x102b): {}",
            if hw_err.contains(&0x102B) { "DELIVERED" } else { "LOST" }
        );

        assert_eq!(
            hw_base, int_base,
            "with no errand in the stepped window, the two engines must deliver \
             the identical single-step trace — native #DB delivery through the \
             partition's own IVT is the floor this probe stands on"
        );
        assert!(
            hw_err_memory_exits >= 2,
            "both device-memory touches must actually exit ({hw_err_memory_exits} \
             memory exits) — without the errands the probe measures nothing"
        );
        assert!(
            hw_err.contains(&0x102B),
            "the single-step #DB of the instruction that consumes the imposed \
             InterruptState inhibit must be delivered — its absence means the \
             platform's bit carries MOV-SS-type suppression and every imposition \
             silently eats a step: hardware trace {hw_err:#06x?}"
        );
        // The whole trace, in order. One equality catches a lost step, a
        // duplicate from a latch stranded in the shadow, and a double
        // delivery alike — anything a stepping guest could observe
        // differently between the engines.
        assert_eq!(
            hw_err, int_err,
            "with an errand in the stepped window, the two engines must still \
             deliver the identical single-step trace"
        );
    }

    /// A halt the shadow retires is a halt the guest gets — and the step the
    /// halting instruction owes waits for the wake, where Bochs delivers it.
    ///
    /// The guest steps with TF through `mov ss,[es:0]`, whose device-memory
    /// source makes it an errand; the errand's tail retires the `hlt` the
    /// `MOV SS` inhibit shadows, on the shadow, with the step still owed. What
    /// must follow, on both engines identically: the processor stays halted
    /// until the PIT's tick; the tick ends the wait; the owed `#DB` lands
    /// FIRST, with the frame's saved IP after the `hlt` (0x104B); then the
    /// tick's own handler runs (it records a 0xFFFF mark); then stepping
    /// resumes. A partition run past the halt shows as the `inc bx` step
    /// (0x104C) arriving before — or instead of — the halt's, and the kept
    /// latch firing later against the wrong instruction as a duplicate.
    ///
    /// Bochs event.cc handleAsyncEvent: handleWaitForEvent, then Priority 4,
    /// then Priority 5 — the order this trace encodes.
    #[test]
    fn a_halt_retired_by_an_errand_keeps_the_guest_halted_and_its_step_for_the_wake() {
        if !hypervisor_here() {
            return;
        }

        // The `MOV SS` source is the VGA text buffer, untouched before it: the
        // display's memory is zero out of reset on both engines, so SS loads
        // 0 either way, and touching the buffer earlier would latch display
        // work whose boundary shortens the slice that follows into one the
        // shadow carries — and then the `MOV SS` is no errand.
        //
        // The delay loop between the PIT load and the errand is load-bearing
        // too. A slice whose request is nearer than the hardware's resolution
        // is carried by the shadow — 2,500 ticks at the default 50 MIPS — and
        // the machine's device timers make such a request whenever a slice
        // begins within that distance of a deadline; a shadow slice then
        // retires up to its whole request, and whole traces, so a straight
        // line through the `mov ss` would be the interpreter's. The loop is
        // longer than any request the shadow may carry and each taken `loop`
        // ends a trace, so a shadow slice that begins before the errand ends
        // inside the loop, at its deadline, and the slice that reaches the
        // errand is the hardware's.
        const ISR: u16 = 0x1060;
        const TICK: u16 = 0x106F;
        let isr = ISR.to_le_bytes();
        let tick = TICK.to_le_bytes();
        let code: &[u8] = &[
            0x31, 0xC0, // xor ax,ax
            0x8E, 0xD8, // mov ds,ax
            0x8E, 0xD0, // mov ss,ax
            0xBC, 0x00, 0x70, // mov sp,0x7000
            0xC7, 0x06, 0x04, 0x00, isr[0], isr[1], // mov word [0x0004], isr
            0xC7, 0x06, 0x06, 0x00, 0x00, 0x00, // mov word [0x0006], 0
            0xC7, 0x06, 0x20, 0x00, tick[0], tick[1], // mov word [0x0020], tick (IRQ0)
            0xC7, 0x06, 0x22, 0x00, 0x00, 0x00, // mov word [0x0022], 0
            0xBF, 0x00, 0x60, // mov di,0x6000
            0xB8, 0x00, 0xB8, // mov ax,0xB800
            0x8E, 0xC0, // mov es,ax
            0xB0, 0xFE, 0xE6, 0x21, // out 0x21, 0xFE — unmask IRQ0 alone
            0xB0, 0x34, 0xE6, 0x43, // out 0x43, 0x34 — ch0, lo/hi, mode 2
            0xB0, 0x00, 0xE6, 0x40, // out 0x40, 0x00 — count 0x1000, low
            0xB0, 0x10, 0xE6, 0x40, // out 0x40, 0x10 — count 0x1000, high
            0xB9, 0x00, 0x40, // mov cx,0x4000 (0x39) — longer than a shadow slice
            0xE2, 0xFE, // loop $ (0x3C) — a trace boundary per iteration
            0x9C, 0x58, // pushf; pop ax (0x3E)
            0x80, 0xCC, 0x03, // or ah,3 — TF and IF
            0x50, 0x9D, // push ax; popf (0x44): stepping starts after the NEXT instruction
            0x26, 0x8E, 0x16, 0x00, 0x00, // mov ss,[es:0] (0x45) ERRAND; its step is inhibited
            0xF4, // hlt (0x4A) retired by the errand's tail; owes the merged step -> 0x4B
            0x43, // inc bx (0x4B) -> 0x4C
            0x9C, // pushf (0x4C) -> 0x4D
            0x58, // pop ax (0x4D) -> 0x4E
            0x80, 0xE4, 0xFE, // and ah,0xFE (0x4E) -> 0x51
            0x50, // push ax (0x51) -> 0x52
            0x9D, // popf — TF=0 (0x52) -> 0x53
            0xFA, // cli (0x53): no tick may land in the report below
            0x89, 0xF8, // mov ax,di
            0x2D, 0x00, 0x60, // sub ax,0x6000
            0xD1, 0xE8, // shr ax,1
            0xE6, DEBUG_PORT, // out 0xE9,al — record count
            0xF4, // hlt (0x5D)
            0xEB, 0xFD, // jmp hlt
            // isr (0x60): record the frame's saved IP
            0x55, 0x89, 0xE5, 0x50, // push bp; mov bp,sp; push ax
            0x8B, 0x46, 0x02, // mov ax,[bp+2]
            0x89, 0x05, // mov [di],ax
            0x83, 0xC7, 0x02, // add di,2
            0x58, 0x5D, 0xCF, // pop ax; pop bp; iret
            // tick (0x6F): record a mark, acknowledge the 8259
            0xC7, 0x05, 0xFF, 0xFF, // mov word [di],0xFFFF
            0x83, 0xC7, 0x02, // add di,2
            0x50, // push ax
            0xB0, 0x20, 0xE6, 0x20, // out 0x20, 0x20 — non-specific EOI
            0x58, // pop ax
            0xCF, // iret
        ];

        let config = EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            ..EmulatorConfig::default()
        };
        let mut interpreted = MachineBuilder::new(config).build().expect("machine");
        interpreted
            .setup_cpu_mode(CpuSetupMode::RealMode)
            .expect("real mode");
        interpreted.mem_write(CODE, code).expect("load");
        interpreted.reg_write(X86Reg::Rip, CODE);
        let int_trace = trace_of(&mut interpreted);

        eprintln!("interpreter: {int_trace:x?}");
        assert_eq!(
            int_trace,
            [0x104B, 0xFFFF, 0x104C, 0x104D, 0x104E, 0x1051, 0x1052, 0x1053],
            "the interpreter is the reference: the halt's step lands at the \
             wake, before the tick's own handler, and stepping resumes after"
        );

        // Whether a given stretch runs on the hardware or on the shadow is a
        // real-time race on a machine with the full device set: a request
        // nearer than the hardware's resolution is carried by the shadow, and
        // then the `MOV SS` is no errand and the halt is the interpreter's own.
        // Only a run in which the device-memory touch exited put the halt on
        // the shadow through the errand's tail, so trials repeat until one
        // does — and every trial's trace must equal the reference regardless.
        let _turn = a_turn_on_the_hardware();
        let mut qualifying = None;
        for trial in 0..16 {
            let mut on_hardware = machine_with_devices(code);
            let hw_trace = trace_of(&mut on_hardware);
            let memory_exits = on_hardware.engine().exits().memory;
            let census = on_hardware.engine().census();
            // The exit and slice counts say where the stretches fell: a trial
            // with few port exits and few slices ran mostly on the shadow.
            eprintln!(
                "trial {trial}: hardware {hw_trace:x?} ({memory_exits} memory exits, {} port \
                 exits, {} slices)",
                on_hardware.engine().exits().port,
                census.slices,
            );
            assert_eq!(
                hw_trace, int_trace,
                "on every trial, however the stretches fell, the two engines must \
                 deliver the identical trace"
            );
            if memory_exits >= 1 {
                qualifying = Some(trial);
                break;
            }
        }
        assert!(
            qualifying.is_some(),
            "no trial had the device-memory touch exit, so the halt was never \
             retired by an errand's tail on the hardware engine — the property \
             under test was not reached"
        );
    }

    /// An `MWAIT` that breaks on interrupts with IF clear wakes on the
    /// shadow the way it wakes anywhere: the processor runs on, and nothing
    /// is delivered.
    ///
    /// Three claims share this one guest. The slice tail must not overwrite
    /// a self-entered `MwaitIf` with a plain halt — a halt is never runnable
    /// on a masked interrupt, and the guest would sleep forever. The slice
    /// head must let `handle_wait_for_event` decide the wake rather than ask
    /// whether an event is deliverable — with IF clear nothing is, and a head
    /// that asked would hand the machine an empty slice forever. And a shadow
    /// the head woke goes on to the hardware; one it did not ends the slice.
    /// Linux's `intel_idle` sleeps exactly like this.
    ///
    /// The burst is what puts `MONITOR` and `MWAIT` on the shadow: eight
    /// consecutive device-memory reads make the engine hand the shadow a
    /// stretch of up to 4096 instructions, and the pair retires inside it
    /// with no state exchange in between. That matters: a read-back between
    /// them disarms the monitor (an import flushes the TLB, and a TLB flush
    /// wakes the monitor), after which `MWAIT` returns at once — permitted
    /// by the architecture, but not the sleep this test is about. The
    /// platform never executes either instruction; it has no way to.
    #[test]
    fn an_mwait_that_breaks_on_a_masked_interrupt_wakes_the_shadow_without_delivery() {
        if !hypervisor_here() {
            return;
        }

        // The real-mode setup leaves IF set (`set_rflags_for_api(0x202)`), so
        // the guest clears it first: with IF set the tick would be delivered
        // through an IVT slot this guest never fills.
        //
        // The record is two words. 0xAAAA is written by the instruction after
        // `MWAIT`, so it exists only if the processor woke and ran on; the
        // tick's own handler writes 0xFFFF, and it can run only after the
        // `sti; nop` that follows — pending all along, undelivered until
        // asked for.
        //
        // The PIT is in mode 0, one pulse ever, so the record cannot depend on
        // how much guest time the `sti; nop` window happens to span: as a
        // rate generator (mode 2) it put a SECOND pulse inside that window in
        // one run of twenty — the machine's clock moving a whole period
        // across a handful of instructions — and a second mark is not what
        // this test is about.
        const TICK: u16 = 0x1079;
        let tick = TICK.to_le_bytes();
        let code: &[u8] = &[
            0xFA, // cli
            0x31, 0xC0, // xor ax,ax
            0x8E, 0xD8, // mov ds,ax
            0x8E, 0xD0, // mov ss,ax
            0xBC, 0x00, 0x70, // mov sp,0x7000
            0xC7, 0x06, 0x20, 0x00, tick[0], tick[1], // mov word [0x0020], tick (IRQ0)
            0xC7, 0x06, 0x22, 0x00, 0x00, 0x00, // mov word [0x0022], 0
            0xBF, 0x00, 0x60, // mov di,0x6000
            0xB8, 0x00, 0xB8, // mov ax,0xB800
            0x8E, 0xC0, // mov es,ax
            0xB0, 0xFE, 0xE6, 0x21, // out 0x21, 0xFE — unmask IRQ0 alone; IF stays clear
            0xB0, 0x30, 0xE6, 0x43, // out 0x43, 0x30 — ch0, lo/hi, mode 0: ONE tick, ever
            0xB0, 0x00, 0xE6, 0x40, // out 0x40, 0x00 — count 0x1000, low
            0xB0, 0x10, 0xE6, 0x40, // out 0x40, 0x10 — count 0x1000, high
            0xB9, 0x00, 0x40, // mov cx,0x4000 (0x2E) — longer than a shadow slice
            0xE2, 0xFE, // loop $ (0x31)
            // Eight device-memory reads in a row (0x33): the eighth exit
            // engages the burst, and the shadow runs on from there.
            0x26, 0xA0, 0x00, 0x00, // mov al,[es:0]
            0x26, 0xA0, 0x00, 0x00, // mov al,[es:0]
            0x26, 0xA0, 0x00, 0x00, // mov al,[es:0]
            0x26, 0xA0, 0x00, 0x00, // mov al,[es:0]
            0x26, 0xA0, 0x00, 0x00, // mov al,[es:0]
            0x26, 0xA0, 0x00, 0x00, // mov al,[es:0]
            0x26, 0xA0, 0x00, 0x00, // mov al,[es:0]
            0x26, 0xA0, 0x00, 0x00, // mov al,[es:0]
            0xB8, 0x00, 0x61, // mov ax,0x6100 (0x53) — the address MONITOR arms
            0x31, 0xC9, // xor cx,cx — MONITOR's extensions, none
            0x31, 0xD2, // xor dx,dx — MONITOR's hints, none
            0x0F, 0x01, 0xC8, // monitor (0x5A)
            0xB9, 0x01, 0x00, // mov cx,1 — MWAIT breaks on interrupts with IF clear
            0x0F, 0x01, 0xC9, // mwait (0x60): the shadow sleeps here
            0xC7, 0x05, 0xAA, 0xAA, // mov word [di],0xAAAA (0x63) — the wake, undelivered
            0x83, 0xC7, 0x02, // add di,2
            0xFB, // sti (0x6A)
            0x90, // nop (0x6B) — the STI shadow; the tick lands after it -> 0xFFFF
            0xFA, // cli (0x6C)
            0x89, 0xF8, // mov ax,di
            0x2D, 0x00, 0x60, // sub ax,0x6000
            0xD1, 0xE8, // shr ax,1
            0xE6, DEBUG_PORT, // out 0xE9,al — record count
            0xF4, // hlt (0x76); IF is clear, so this one is final
            0xEB, 0xFD, // jmp hlt
            // tick (0x79): record a mark, acknowledge the 8259
            0xC7, 0x05, 0xFF, 0xFF, // mov word [di],0xFFFF
            0x83, 0xC7, 0x02, // add di,2
            0x50, // push ax
            0xB0, 0x20, 0xE6, 0x20, // out 0x20, 0x20 — non-specific EOI
            0x58, // pop ax
            0xCF, // iret
        ];

        let config = EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            ..EmulatorConfig::default()
        };
        let mut interpreted = MachineBuilder::new(config).build().expect("machine");
        interpreted
            .setup_cpu_mode(CpuSetupMode::RealMode)
            .expect("real mode");
        interpreted.mem_write(CODE, code).expect("load");
        interpreted.reg_write(X86Reg::Rip, CODE);
        let int_trace = trace_of(&mut interpreted);

        eprintln!("interpreter: {int_trace:x?}");
        assert_eq!(
            int_trace,
            [0xAAAA, 0xFFFF],
            "the interpreter is the reference: the processor wakes and runs on \
             with the interrupt pending and undelivered, and the tick lands only \
             once IF is set"
        );

        // Which stretch carries the wake is the same race as the halt's. The
        // head under test is a HARDWARE head over a sleeping shadow, and it
        // leaves a mark of its own: woken there with IF clear, the pending
        // tick is staged into a deliverability window and INJECTED after the
        // `sti; nop` — whereas a shadow slice that carries the wake delivers
        // the tick itself and injects nothing. So a trial qualifies by an
        // injection, and every trial's record must equal the reference
        // regardless. A head that did not wake the shadow retires nothing,
        // the machine hands out the same empty slice again, and the guest
        // never reports — which `trace_of` fails by name.
        let _turn = a_turn_on_the_hardware();
        let mut qualifying = None;
        // A hardware head over the sleeping shadow is the minority outcome
        // (measured about one trial in six), so the cap is generous; a trial
        // is a few milliseconds.
        for trial in 0..64 {
            let mut on_hardware = machine_with_devices(code);
            let hw_trace = trace_of(&mut on_hardware);
            let memory_exits = on_hardware.engine().exits().memory;
            let injected = on_hardware.engine().inject_census().injected;
            let census = on_hardware.engine().census();
            eprintln!(
                "trial {trial}: hardware {hw_trace:x?} ({memory_exits} memory exits, {} port \
                 exits, {} slices, {injected} injected)",
                on_hardware.engine().exits().port,
                census.slices,
            );
            assert_eq!(
                hw_trace, int_trace,
                "on every trial, however the stretches fell, the two engines must \
                 deliver the identical record"
            );
            if injected >= 1 {
                qualifying = Some(trial);
                break;
            }
        }
        assert!(
            qualifying.is_some(),
            "no trial woke the sleeping shadow at a hardware head — the tick was \
             never injected — so the property under test was not reached"
        );
    }

    /// A vector pending AT A SLICE HEAD reaches an active guest by injection,
    /// not by the interpreter delivering before the hardware runs.
    ///
    /// The guest is built so the head is the ONLY staging opportunity: after
    /// programming the PIT it spins in a pure computation loop — no port
    /// touches, no device memory — so a running slice takes no exits until
    /// its budget cancels it. Every tick therefore becomes deliverable while
    /// the machine owns the processor between slices, and is pending when the
    /// next slice starts. A head that delivers through the shadow shows MARKs
    /// with injections stalled at the incidental few — this guest measured
    /// 2 injections against 131 MARKs on the interpreter-delivering head,
    /// 1.5% of the traffic, all of it ticks that happened to land on windows
    /// armed earlier — so the discriminator is the SHARE injection carries,
    /// not its mere occurrence.
    ///
    /// HOW the head injects is part of the claim: the head reads the
    /// interpreter's own one-instruction inhibit and the partition's own
    /// shadow bit, so with `IF` set and neither live it holds positive
    /// evidence that delivery is permitted and injects THERE — no
    /// deliverability window, no window exit. `windows_armed` stays at a
    /// small fraction of `injected` (measured zero: this guest never leaves
    /// a shadower as the last instruction the shadow retires); a head that
    /// deferred every vector on a presumed inhibit would drive the two
    /// counters level.
    ///
    /// The ISR runs `STI` before anything else for the same reason the
    /// errand-tail test's does: after an injection the processor's latched
    /// pin is stale until the next acknowledge attempt reconciles it, and an
    /// `IF=0` handler tail would arm one spurious window per injection,
    /// muddying the very counters under assertion.
    ///
    /// Trials repeat across steps because a stretch the shadow legitimately
    /// carries (a budget nearer than the hardware's resolution) delivers
    /// through the interpreter with nothing to inject; only a head the
    /// HARDWARE follows exercises the path under test.
    #[test]
    fn a_vector_pending_at_a_slice_head_is_injected_not_shadow_delivered() {
        if !hypervisor_here() {
            return;
        }

        let _turn = a_turn_on_the_hardware();
        // isr at CODE+0x29 (CS base is 0).
        let mut machine = machine_with_devices(&[
            0x31, 0xC0, //             xor ax, ax
            0x8E, 0xD8, //             mov ds, ax
            0x8E, 0xD0, //             mov ss, ax
            0xBC, 0x00, 0x70, //       mov sp, 0x7000
            0xC7, 0x06, 0x20, 0x00, 0x29, 0x10, // mov word [0x20], isr (IVT[8])
            0xC7, 0x06, 0x22, 0x00, 0x00, 0x00, //  mov word [0x22], 0
            0xB0, 0xFE, //             mov al, 0xFE — unmask IRQ0 alone
            0xE6, 0x21, //             out 0x21, al  (OCW1)
            0xB0, 0x34, //             mov al, 0x34 — ch0, lo/hi, mode 2
            0xE6, 0x43, //             out 0x43, al
            0xB0, 0x00, //             mov al, 0x00 — count 0x0400, low
            0xE6, 0x40, //             out 0x40, al
            0xB0, 0x04, //             mov al, 0x04 — count 0x0400, high
            0xE6, 0x40, //             out 0x40, al
            0xFB, //                   sti (CODE+0x25) — IF=1 for the whole busy loop
            // busy (CODE+0x26): pure computation, exit-free — the tick can
            // only be found pending at a slice head
            0x40, //                   inc ax
            0xEB, 0xFD, //             jmp busy
            // isr (CODE+0x29): STI first — see the doc above — then report
            0xFB, //                   sti
            0x90, //                   nop — the STI shadow lapses
            0xB0, MARK, //             mov al, MARK
            0xE6, DEBUG_PORT, //       out 0xE9, al
            0xB0, 0x20, //             mov al, 0x20
            0xE6, 0x20, //             out 0x20, al — non-specific EOI
            0xCF, //                   iret
        ]);
        // The guest never halts, so each step returns on budget and the next
        // resumes it; the PIT's ticks land across the steps. Bounded so a
        // wedge fails assertions instead of hanging the suite; stops early
        // once several deliveries AND several injections stand, so the census
        // comparison below is made of more than one event.
        let mut written: std::vec::Vec<u8> = std::vec::Vec::new();
        for _ in 0..48 {
            machine
                .step(RunBudget::Ticks(2_000_000))
                .expect("the hypervisor runs the guest and its ticks reach it");
            written.extend(machine.debug_port().take_output());
            if written.len() >= 8 && machine.engine().inject_census().injected >= 6 {
                break;
            }
        }
        assert!(
            !written.is_empty() && written.iter().all(|byte| *byte == MARK),
            "the PIT's ticks must reach the guest's own ISR, and nothing else may \
             write the debug port: {written:#04x?}"
        );
        let census = machine.engine().inject_census();
        let exits = machine.engine().exits();
        let slices = machine.engine().census();
        // The measured numbers, for the record beside the assertions: MARKs
        // are the guest-visible acknowledge count — one per delivered vector.
        eprintln!(
            "head-injection census: marks {}, injected {} (vector 8: {}), windows \
             armed {}, window exits {}, slices {}, ended_event_to_deliver {}",
            written.len(),
            census.injected,
            census.injected_per_vector[8],
            census.windows_armed,
            exits.window,
            slices.slices,
            slices.ended_event_to_deliver,
        );
        assert!(
            census.injected >= 4 && census.injected_per_vector[8] >= 4,
            "a vector pending at a slice head must cross as a register write — \
             injected {} (vector 8: {}), windows armed {}: injections stalling \
             while MARKs pile up is the retired head delivering through the \
             interpreter",
            census.injected,
            census.injected_per_vector[8],
            census.windows_armed,
        );
        // The traffic split is the claim, not one lucky event. Deliveries the
        // shadow legitimately carries (a budget nearer than the hardware's
        // resolution converts the whole slice, and its head delivers
        // interpreted) keep this below 100%; measured here the hardware
        // carries a quarter to a third of the ticks, where the
        // interpreter-delivering head measured 1.5% — incidental injections
        // only. One-sixth keeps an order of magnitude over the broken head
        // and headroom under the measured mix.
        assert!(
            census.injected * 6 >= written.len() as u64,
            "injection must carry the hardware-slice traffic, not be incidental \
             to it: injected {} of {} deliveries",
            census.injected,
            written.len(),
        );
        // The head's evidence is read, not presumed: a vector pending with
        // IF set and no shadow live on either side is injected at the head
        // itself. A quarter, not zero, because a shadow-carried stretch can
        // end on an `STI` with the next tick already pending, and that head
        // is genuinely blocked for one instruction.
        assert!(
            census.windows_armed * 4 <= census.injected,
            "a head-pending vector must be injected on the head's own evidence, \
             not deferred to a window: windows armed {} against {} injections — \
             level counters are a head presuming an inhibit it could have read",
            census.windows_armed,
            census.injected,
        );
    }

    /// A vector is never delivered into a context whose `IF` a shadow errand
    /// cleared between the exit and the staging decision.
    ///
    /// This is the freshness contract the Critical review finding is about,
    /// proven end to end on hardware. The property is guest-visible and
    /// absolute: a maskable external interrupt only ever interrupts an `IF=1`
    /// context, so the FLAGS image an interrupt pushes always has bit 9 set.
    /// A gate that judged from the stale exit header would acknowledge and
    /// inject after a shadow errand had already cleared `IF` — delivering
    /// into an `IF=0` context, which the VM-entry checks reject and no real
    /// processor ever does.
    ///
    /// The guest makes every exit an IF-clearing errand: it loops on `WRMSR`
    /// to a reserved MSR, which `#GP`s. `#GP` is trapped, so each one is
    /// serviced by `finish_on_the_shadow`, where the shadow re-runs the
    /// `WRMSR`, faults, and ENTERS the real-mode `#GP` handler — and a
    /// real-mode gate entry clears `IF` in that single serviced step. So at
    /// the tail of every `WRMSR` exit the shadow sits at the handler entry
    /// with `IF` clear, while the exit header that preceded the errand still
    /// says `IF=1`. A periodic PIT tick, unmasked as IRQ0, becomes pending at
    /// one of those tails: the engine must see the shadow's cleared `IF` (arm
    /// a window, deliver later when `IF` reopens) and never the header's.
    ///
    /// The `#GP` handler steps the saved IP over the two-byte `WRMSR` and
    /// returns, restoring `IF=1`, so the loop runs on. The IRQ0 handler reads
    /// bit 9 off its own pushed FLAGS and reports the verdict: `0xF1` for a
    /// lawful `IF=1` delivery, `0xF0` for one forced into the `IF=0` handler.
    /// Whether a given stretch runs on hardware or the shadow is the engine's
    /// own business — a shadow-carried stretch delivers lawfully (`0xF1`)
    /// too — so the test collects many deliveries and fails on a single
    /// `0xF0`.
    ///
    /// The test also proves the errand-then-stage path ran on hardware, by
    /// `windows_armed >= 1`: the gate found a vector deliverable-but-blocked
    /// after an errand and armed a window to defer it. This is the end-to-end
    /// hardware exercise of the whole path this fix touches — the shadow
    /// errand, the republish, the window arm, the deferred delivery when `IF`
    /// reopens. The BITING proof of the gate's IF source is the unit test
    /// `engine::a_shadow_errand_republishes_if_and_the_inhibit_it_read`, which
    /// fails the instant the republish is removed; this test is the hardware
    /// witness that the same machinery delivers correctly on the metal, never
    /// landing a vector in an `IF=0` frame.
    #[test]
    fn a_delivery_never_interrupts_a_context_whose_if_a_shadow_errand_cleared() {
        if !hypervisor_here() {
            return;
        }

        let _turn = a_turn_on_the_hardware();
        // gp_isr is at CODE+0x3B, irq0_isr at CODE+0x44 (CS base 0).
        let mut machine = machine_with_devices(&[
            0x31, 0xC0, //                 xor ax, ax
            0x8E, 0xD8, //                 mov ds, ax
            0x8E, 0xD0, //                 mov ss, ax
            0xBC, 0x00, 0x70, //           mov sp, 0x7000
            0xC7, 0x06, 0x20, 0x00, 0x44, 0x10, // mov word [0x20], irq0_isr (IVT[8])
            0xC7, 0x06, 0x22, 0x00, 0x00, 0x00, //  mov word [0x22], 0
            0xC7, 0x06, 0x34, 0x00, 0x3B, 0x10, //  mov word [0x34], gp_isr  (IVT[13])
            0xC7, 0x06, 0x36, 0x00, 0x00, 0x00, //  mov word [0x36], 0
            0xB0, 0xFE, //                 mov al, 0xFE — unmask IRQ0 alone
            0xE6, 0x21, //                 out 0x21, al
            0xB0, 0x34, //                 mov al, 0x34 — PIT ch0, lo/hi, mode 2
            0xE6, 0x43, //                 out 0x43, al
            0xB0, 0x00, //                 mov al, 0x00 — count 0x0400, low
            0xE6, 0x40, //                 out 0x40, al
            0xB0, 0x04, //                 mov al, 0x04 — count 0x0400, high
            0xE6, 0x40, //                 out 0x40, al
            // loop_wrmsr (CODE+0x31): every iteration is an IF-clearing errand
            0x66, 0xB9, 0xFF, 0x0F, 0x00, 0x00, // mov ecx, 0x0FFF — a reserved MSR
            0x0F, 0x30, //                 wrmsr — #GP, serviced on the shadow
            0xEB, 0xF6, //                 jmp loop_wrmsr
            // gp_isr (CODE+0x3B): skip the two-byte WRMSR and return (IF back to 1)
            0x55, //                       push bp
            0x89, 0xE5, //                 mov bp, sp
            0x83, 0x46, 0x02, 0x02, //     add word [bp+2], 2 — saved IP past the WRMSR
            0x5D, //                       pop bp
            0xCF, //                       iret
            // irq0_isr (CODE+0x44): report bit 9 of the interrupted FLAGS image
            0x55, //                       push bp
            0x89, 0xE5, //                 mov bp, sp
            0x8B, 0x46, 0x06, //           mov ax, [bp+6] — the pushed FLAGS
            0xF6, 0xC4, 0x02, //           test ah, 0x02  — FLAGS bit 9, IF
            0xB0, 0xF0, //                 mov al, 0xF0
            0x74, 0x02, //                 jz +2 — keep the damning byte
            0xB0, 0xF1, //                 mov al, 0xF1
            0xE6, DEBUG_PORT, //           out 0xE9, al — the verdict
            0xB0, MARK, //                 mov al, MARK
            0xE6, DEBUG_PORT, //           out 0xE9, al
            0xB0, 0x20, //                 mov al, 0x20
            0xE6, 0x20, //                 out 0x20, al — non-specific EOI
            0x5D, //                       pop bp
            0xCF, //                       iret
        ]);
        // The guest never halts (its WRMSR loop is endless), so each step
        // returns on budget and the next resumes it; the PIT's periodic ticks
        // land across the steps. Bounded so a guest that delivers nothing
        // fails the assertion rather than hanging the suite.
        let mut written: std::vec::Vec<u8> = std::vec::Vec::new();
        for _ in 0..48 {
            machine
                .step(RunBudget::Ticks(2_000_000))
                .expect("the hypervisor runs the guest; a delivery into IF=0 would \
                         be refused here as WHV_E_INVALID_VP_STATE");
            written.extend(machine.debug_port().take_output());
            if written.iter().filter(|byte| **byte == MARK).count() >= 8 {
                break;
            }
        }
        assert!(
            written.iter().any(|byte| *byte == MARK),
            "the PIT's ticks must have reached the guest's own ISR at least once: \
             {written:#04x?}"
        );
        // The corruption signature is a single 0xF0 byte — a vector delivered
        // into a context whose IF a shadow errand had cleared. Its total
        // absence is the property; every verdict emitted was the lawful 0xF1.
        // (Checked byte-wise rather than as pairs because a step boundary may
        // split the ISR between its verdict write and its MARK write, leaving
        // a lone trailing 0xF1 — harmless, and not a 0xF0.)
        assert!(
            !written.iter().any(|byte| *byte == 0xF0),
            "a delivery reported IF=0 at its interrupt frame (0xF0) — a vector \
             forced into a context whose IF a shadow errand had cleared: \
             {written:#04x?}"
        );
        assert!(
            written.iter().any(|byte| *byte == 0xF1),
            "a lawful IF=1 verdict must have been emitted: {written:#04x?}"
        );
        assert!(
            written.iter().all(|byte| *byte == 0xF1 || *byte == MARK),
            "the debug port must carry only verdicts and MARKs: {written:#04x?}"
        );
        // The gate ran on hardware and took the correct IF=0 path: seeing the
        // shadow's cleared IF, it armed a window and deferred the vector. The
        // stale-header gate would have injected into the IF=0 handler instead
        // — no window, and a 0xF0 verdict or a refused entry above. So an
        // armed window is the signature that the fixed gate, not the shadow
        // fallback, is what carried these deliveries.
        assert!(
            machine.engine().inject_census().windows_armed >= 1,
            "the fixed gate must have armed a window for a vector it found \
             deliverable-but-blocked on hardware; without one this test never \
             reached the gate and proves nothing"
        );
    }

    /// A vector that becomes deliverable AT AN ERRAND'S TAIL is injected
    /// there, on the shadow's own evidence — no window armed, no window exit
    /// paid.
    ///
    /// After an errand the engine reads the interpreter's inhibit instead of
    /// presuming one: an `OUTSB` arms none, so the tail finds `IF=1` and no
    /// shadow, acknowledges, and injects directly. The guest manufactures
    /// exactly that moment, once per trial: a PIT tick is latched at the 8259
    /// but MASKED (`IF=1` the whole time, so every exit header carries `IF=1`
    /// and no shadow), and the instruction that unmasks it is `OUTSB` to port
    /// 0x21 — a string port write, which the engine always services as a
    /// shadow errand (`finish_on_the_shadow`). The vector becomes deliverable
    /// during that errand, and nothing else in the guest ever finds a vector
    /// deliverable-but-blocked, so every armed window would be one the tail
    /// armed on a presumption. `windows_armed * 4 <= injected` is the
    /// discriminator (measured zero windows): presuming the inhibit live
    /// drives the two counters level. Trials repeat because a stretch the
    /// shadow happens to carry delivers through the interpreter (no
    /// injection, legitimately); only a hardware trial exercises the errand
    /// tail, and many trials make missing them all vanishingly unlikely.
    ///
    /// The ISR runs `STI` before anything else, deliberately: after any
    /// injection the processor's latched interrupt pin is stale until the
    /// next acknowledge attempt reconciles it, and an `IF=0` handler exit
    /// would present stale-pin-plus-blocked to the gate and arm a spurious
    /// window, burying the discriminator. With `IF=1` inside the handler,
    /// the gate reaches the acknowledge, finds the 8259 empty, and the
    /// reconcile clears the stale pin without arming anything.
    #[test]
    fn a_vector_raised_inside_an_errand_is_injected_at_its_tail() {
        if !hypervisor_here() {
            return;
        }

        let _turn = a_turn_on_the_hardware();
        // isr at CODE+0x42; the 0xFE unmask image byte at CODE+0x4D.
        let mut machine = machine_with_devices(&[
            0x31, 0xC0, //             xor ax, ax
            0x8E, 0xD8, //             mov ds, ax
            0x8E, 0xD0, //             mov ss, ax
            0xBC, 0x00, 0x70, //       mov sp, 0x7000
            0xFC, //                   cld — OUTSB walks SI forward
            0xC7, 0x06, 0x20, 0x00, 0x42, 0x10, // mov word [0x20], isr
            0xC7, 0x06, 0x22, 0x00, 0x00, 0x00, // mov word [0x22], 0
            0xB0, 0x0A, //             mov al, 0x0A — OCW3: reads answer IRR
            0xE6, 0x20, //             out 0x20, al
            0xB9, 0x18, 0x00, //       mov cx, 24 — the trials
            // trial (CODE+0x1D):
            0xB0, 0xFF, //             mov al, 0xFF — mask everything
            0xE6, 0x21, //             out 0x21, al
            0xB0, 0x30, //             mov al, 0x30 — PIT ch0, lo/hi, mode 0
            0xE6, 0x43, //             out 0x43, al
            0xB0, 0x00, //             mov al, 0x00 — count 0x0400, low
            0xE6, 0x40, //             out 0x40, al
            0xB0, 0x04, //             mov al, 0x04 — count 0x0400, high
            0xE6, 0x40, //             out 0x40, al
            0xFB, //                   sti — IF=1; safe, the tick is masked
            // poll (CODE+0x2E): masked, so nothing is deliverable yet
            0xE4, 0x20, //             in al, 0x20 — the IRR
            0xA8, 0x01, //             test al, 1
            0x74, 0xFA, //             jz poll
            0xBE, 0x4D, 0x10, //       mov si, unmask_image
            0xBA, 0x21, 0x00, //       mov dx, 0x0021
            0x6E, //                   outsb — THE ERRAND: unmasks IRQ0, so the
            //                         vector becomes deliverable inside it
            0x90, //                   nop — the deferred delivery lands here
            0x90, //                   nop
            0xE2, 0xDE, //             loop trial
            // park (CODE+0x3F):
            0xF4, //                   hlt
            0xEB, 0xFD, //             jmp park
            // isr (CODE+0x42): STI first — see the doc above — then report
            0xFB, //                   sti
            0x90, //                   nop — the STI shadow lapses
            0xB0, MARK, //             mov al, MARK
            0xE6, DEBUG_PORT, //       out 0xE9, al
            0xB0, 0x20, //             mov al, 0x20
            0xE6, 0x20, //             out 0x20, al — non-specific EOI
            0xCF, //                   iret
            // unmask_image (CODE+0x4D): what OUTSB sends to port 0x21
            0xFE,
        ]);
        let mut written: std::vec::Vec<u8> = std::vec::Vec::new();
        for _ in 0..40 {
            machine
                .step(RunBudget::Ticks(2_000_000))
                .expect("the hypervisor runs the guest and its ticks reach it");
            written.extend(machine.debug_port().take_output());
            if written.len() >= 4 && machine.engine().inject_census().injected >= 2 {
                break;
            }
        }
        // Every tick arrives, whichever engine carried its trial; a wedge
        // would show here as missing MARKs.
        assert!(
            !written.is_empty() && written.iter().all(|byte| *byte == MARK),
            "every trial's tick must reach the guest's own ISR and nothing else \
             may write the debug port: {written:#04x?}"
        );
        let census = machine.engine().inject_census();
        eprintln!(
            "errand-tail census: marks {}, injected {}, windows armed {}",
            written.len(),
            census.injected,
            census.windows_armed,
        );
        assert!(
            census.injected >= 1,
            "at least one trial's errand must have run on the hardware and its \
             tail must have injected the vector it made deliverable; marks {}",
            written.len(),
        );
        assert!(
            census.windows_armed * 4 <= census.injected,
            "a vector that became deliverable inside an errand must be injected \
             at the errand's tail on the shadow's own evidence — a window per \
             injection is the tail presuming an inhibit the errand never armed; \
             census: windows armed {}, injected {}",
            census.windows_armed,
            census.injected,
        );
    }

    /// A vector pending behind an errand's `MOV SS` lands after the stack
    /// switch the `MOV SS` began — never on the new `SS` with the old `SP`.
    ///
    /// The one errand whose tail IS inside an interrupt shadow: the trapped
    /// instruction is `mov ss,[es:0]`, a device-memory load, so the shadow
    /// retires it and the interpreter's own inhibit is live for the `mov sp`
    /// that follows. The tail's gate reads that inhibit and defers the
    /// pending tick to a window; the hardware retires the `mov sp` with the
    /// imposed shadow bit set, clears it, and the window exit's header lets
    /// the tick in — on the new stack. A gate that read "no shadow" there
    /// (an accessor answering for the wrong instruction, or a tail that
    /// wrote the shadow bit clear beneath an injection) would inject between
    /// the two halves of the switch, and the interrupt frame would go to the
    /// old `SP` — the `general protection: 0000` on `IRET` that
    /// `Started::shadowed`'s doc records.
    ///
    /// The verdict is guest-visible: the ISR reports the high byte of `SP`
    /// at entry. `mov sp,0x9000` makes every lawful frame land at
    /// `0x8FFA` (0x8F); a frame pushed inside the `MOV SS` shadow lands at
    /// `0x6FFA` (0x6F), the old stack's top. `IF` is set only from the
    /// `sti` that immediately precedes the `mov ss`, so no delivery can
    /// lawfully precede the switch: 0x8F is the only lawful verdict, and a
    /// single 0x6F fails the test. The trial's `sti` also puts the hardware's
    /// own STI shadow over the errand's exit header, which is the second half
    /// of what the tail must get right: a shadow the HARDWARE entered is
    /// consumed by the errand retiring the instruction it protected, while
    /// the `MOV SS` shadow the SHADOW entered is live for the next one.
    ///
    /// Trials repeat until one `mov ss` has exited on the hardware AND a
    /// deferred tick has been injected: a stretch the shadow carries delivers
    /// lawfully through the interpreter's own inhibit, which exercises
    /// nothing here, and a tick deferred at a tail is delivered by whichever
    /// engine next finds it deliverable — the interpreter, when the slice
    /// ends at that tail — so a run can retire several hardware trials before
    /// one window exit lets the hardware inject. `exits().memory >= 1` and
    /// `injected >= 1` together prove the property was reached.
    #[test]
    fn a_vector_pending_behind_an_errands_mov_ss_lands_after_the_stack_switch() {
        if !hypervisor_here() {
            return;
        }

        let _turn = a_turn_on_the_hardware();
        // isr at CODE+0x4F (CS base is 0).
        let mut machine = machine_with_devices(&[
            0x31, 0xC0, //             xor ax, ax
            0x8E, 0xD8, //             mov ds, ax
            0x8E, 0xD0, //             mov ss, ax
            0xBC, 0x00, 0x70, //       mov sp, 0x7000
            0xC7, 0x06, 0x20, 0x00, 0x4F, 0x10, // mov word [0x20], isr (IVT[8])
            0xC7, 0x06, 0x22, 0x00, 0x00, 0x00, //  mov word [0x22], 0
            0xB8, 0x00, 0xB8, //       mov ax, 0xB800 — the text-mode video segment
            0x8E, 0xC0, //             mov es, ax
            0xB0, 0xFE, //             mov al, 0xFE — unmask IRQ0 alone
            0xE6, 0x21, //             out 0x21, al  (OCW1)
            0xB0, 0x34, //             mov al, 0x34 — ch0, lo/hi, mode 2
            0xE6, 0x43, //             out 0x43, al
            0xB0, 0x00, //             mov al, 0x00 — count 0x0400, low
            0xE6, 0x40, //             out 0x40, al
            0xB0, 0x04, //             mov al, 0x04 — count 0x0400, high
            0xE6, 0x40, //             out 0x40, al
            0xB0, 0x0A, //             mov al, 0x0A — OCW3: reads answer IRR
            0xE6, 0x20, //             out 0x20, al
            0xB9, 0x00, 0x40, //       mov cx, 0x4000 — many trials, so a run
            //                         is never starved of a hardware trial
            // trial (CODE+0x31): back on the old stack, interrupts closed
            0xFA, //                   cli
            0x31, 0xC0, //             xor ax, ax
            0x8E, 0xD0, //             mov ss, ax
            0xBC, 0x00, 0x70, //       mov sp, 0x7000
            // poll (CODE+0x39): wait for a tick to be pending at the 8259
            0xE4, 0x20, //             in al, 0x20 — the IRR
            0xA8, 0x01, //             test al, 1
            0x74, 0xFA, //             jz poll
            0xFB, //                   sti — IF=1; its shadow covers the errand
            0x26, 0x8E, 0x16, 0x00, 0x00, // mov ss,[es:0] — THE ERRAND: SS=0
            //                         from the display's memory, and the
            //                         MOV SS shadow over the next instruction
            0xBC, 0x00, 0x90, //       mov sp, 0x9000 — the other half of the
            //                         switch; the tick lands after it
            0x90, //                   nop
            0x90, //                   nop
            0xE2, 0xE5, //             loop trial
            // park (CODE+0x4C):
            0xF4, //                   hlt
            0xEB, 0xFD, //             jmp park
            // isr (CODE+0x4F): STI first (the stale-pin rule the other
            // injection tests follow), then the verdict, then a MARK
            0xFB, //                   sti
            0x90, //                   nop — the STI shadow lapses
            0x89, 0xE0, //             mov ax, sp — after the 6-byte frame
            0x88, 0xE0, //             mov al, ah — 0x8F lawful, 0x6F the old-SP frame
            0xE6, DEBUG_PORT, //       out 0xE9, al — the verdict
            0xB0, MARK, //             mov al, MARK
            0xE6, DEBUG_PORT, //       out 0xE9, al
            0xB0, 0x20, //             mov al, 0x20
            0xE6, 0x20, //             out 0x20, al — non-specific EOI
            0xCF, //                   iret
        ]);
        let mut written: std::vec::Vec<u8> = std::vec::Vec::new();
        for _ in 0..48 {
            machine
                .step(RunBudget::Ticks(2_000_000))
                .expect("the hypervisor runs the guest; an injection into the MOV SS \
                         shadow would be refused here as WHV_E_INVALID_VP_STATE");
            written.extend(machine.debug_port().take_output());
            let marks = written.iter().filter(|byte| **byte == MARK).count();
            if marks >= 8
                && machine.engine().exits().memory >= 1
                && machine.engine().inject_census().injected >= 1
            {
                break;
            }
        }
        let census = machine.engine().inject_census();
        let exits = machine.engine().exits();
        eprintln!(
            "mov-ss census: bytes {}, injected {}, windows armed {}, window exits {}, memory exits {}",
            written.len(),
            census.injected,
            census.windows_armed,
            exits.window,
            exits.memory,
        );
        assert!(
            written.iter().any(|byte| *byte == MARK),
            "the PIT's ticks must have reached the guest's own ISR at least once: \
             {written:#04x?}"
        );
        assert!(
            !written.iter().any(|byte| *byte == 0x6F),
            "a tick was delivered inside the MOV SS shadow — its frame went to \
             the old SP (0x6F) between the two halves of the stack switch: \
             {written:#04x?}"
        );
        assert!(
            written.iter().any(|byte| *byte == 0x8F),
            "a lawful verdict (0x8F, the frame on the switched stack) must have \
             been emitted: {written:#04x?}"
        );
        assert!(
            written.iter().all(|byte| *byte == 0x8F || *byte == MARK),
            "the debug port must carry only lawful verdicts and MARKs: {written:#04x?}"
        );
        assert!(
            exits.memory >= 1,
            "no trial's MOV SS exited on the hardware, so no errand tail ever \
             stood inside a MOV SS shadow — the property under test was not \
             reached"
        );
        assert!(
            census.injected >= 1,
            "the hardware trials must have injected their ticks; injected {}",
            census.injected,
        );
    }
}
