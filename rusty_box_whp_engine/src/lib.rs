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

mod device_thread;
mod engine;
mod fast_machine;
mod exchange;
mod state;
mod vcpu_thread;
mod vm_clock;
mod xsave;

/// Parts several test modules in this crate share.
///
/// Crate-level rather than nested in the module that first needed them: a
/// `#[cfg(test)] mod tests` item is reachable only from inside its own module,
/// and these are exercised from more than one.
#[cfg(test)]
pub(crate) mod fixtures {
    use crate::WhpEngine;
    use rusty_box::cpu::instrumentation::{CpuSetupMode, X86Reg};
    use rusty_box::emulator::{
        DeviceClock, Emulator, EmulatorConfig, MachineBuilder, MemorySize,
    };
    use rusty_box_core::time::{HostClock, HostInstant};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// Where the guest's code goes, and the port it writes to.
    pub(crate) const CODE: u64 = 0x1000;
    /// Port 0xE9, the chipset debug console: a byte written here reaches the
    /// machine's own debug port, which is host-readable — so the assertion is
    /// about what the GUEST did, not about what the engine returned.
    pub(crate) const DEBUG_PORT: u8 = 0xE9;
    pub(crate) const MARK: u8 = 0x5A;

    /// A real-mode machine on this engine, with `code` loaded at [`CODE`] and
    /// its processor pointed at it, ADOPTED and ready to step.
    ///
    /// What `machine_running` is for a fast machine. Bare, like it: no BIOS,
    /// no device initialisation, for a guest that only touches the debug port.
    /// A guest that programs the PIC or the PIT needs
    /// [`fast_machine_with_devices`] instead.
    pub(crate) fn fast_machine_running(code: &[u8]) -> crate::FastMachine<()> {
        crate::FastMachine::adopt(machine_running_on(DeviceClock::HostTime, code))
            .expect("a machine on hardware")
    }

    /// A real-mode machine on this engine, with `code` loaded at [`CODE`] and
    /// its processor pointed at it.
    pub(crate) fn machine_running(code: &[u8]) -> std::boxed::Box<Emulator<(), WhpEngine>> {
        machine_running_on(DeviceClock::Ticks, code)
    }

    /// The same bare machine, with the device clock said out loud.
    pub(crate) fn machine_running_on(
        clock: DeviceClock,
        code: &[u8],
    ) -> std::boxed::Box<Emulator<(), WhpEngine>> {
        let config = EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            device_clock: clock,
            ..EmulatorConfig::default()
        };
        let mut machine =
            Emulator::<(), WhpEngine>::with_engine(config, CpuSetupMode::RealMode)
                .expect("machine");
        machine.mem_write(CODE, code).expect("load");
        machine.reg_write(X86Reg::Rip, CODE);
        machine
    }

    /// A real-mode machine with the FULL device set, with the device clock
    /// said out loud.
    ///
    /// The clock is what decides who owns the guest's local APIC — see
    /// `choose_the_local_apic` — so a test that drives its machine from a
    /// thread of its own asks for [`DeviceClock::HostTime`] and gets the
    /// hypervisor's APIC with it.
    pub(crate) fn machine_with_devices_on(
        clock: DeviceClock,
        code: &[u8],
    ) -> std::boxed::Box<Emulator<(), WhpEngine>> {
        let config = EmulatorConfig {
            memory: MemorySize::bytes(8 * 1024 * 1024),
            device_clock: clock,
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

    /// Where the interrupt handler of the protected-mode guests below lives.
    pub(crate) const HANDLER: u64 = 0x1100;

    /// The prologue both protected-mode guests share: enter protected mode
    /// against the tables the test wrote, take a flat data segment and a stack,
    /// and software-enable the local APIC.
    ///
    /// Every byte carries its mnemonic, as `whp_probe.rs` annotates its own.
    /// Addresses are absolute because the far jump needs one: `pm32` is
    /// `CODE + 0x1B`, and the byte string below is what puts it there.
    ///
    /// ```text
    /// 0x1000  FA                       cli
    /// 0x1001  0F 01 16 00 1F           lgdt [0x1F00]
    /// 0x1006  0F 01 1E 08 1F           lidt [0x1F08]
    /// 0x100B  0F 20 C0                 mov eax, cr0
    /// 0x100E  0C 01                    or al, 1
    /// 0x1010  0F 22 C0                 mov cr0, eax
    /// 0x1013  66 EA 1B 10 00 00 08 00  jmp 0x08:0x101B
    /// 0x101B  66 B8 10 00              mov ax, 0x10          ; 32-bit from here
    /// 0x101F  8E D8                    mov ds, ax
    /// 0x1021  8E C0                    mov es, ax
    /// 0x1023  8E D0                    mov ss, ax
    /// 0x1025  BC 00 70 00 00           mov esp, 0x7000
    /// 0x102A  C7 05 F0 00 E0 FE ...    mov dword [0xFEE000F0], 0x1FF  ; SVR
    /// 0x1034                           <the guest's own body>
    /// ```
    const ENTER_PROTECTED_MODE: [u8; 0x34] = [
        0xFA, //
        0x0F, 0x01, 0x16, 0x00, 0x1F, //
        0x0F, 0x01, 0x1E, 0x08, 0x1F, //
        0x0F, 0x20, 0xC0, //
        0x0C, 0x01, //
        0x0F, 0x22, 0xC0, //
        0x66, 0xEA, 0x1B, 0x10, 0x00, 0x00, 0x08, 0x00, //
        0x66, 0xB8, 0x10, 0x00, //
        0x8E, 0xD8, //
        0x8E, 0xC0, //
        0x8E, 0xD0, //
        0xBC, 0x00, 0x70, 0x00, 0x00, //
        0xC7, 0x05, 0xF0, 0x00, 0xE0, 0xFE, 0xFF, 0x01, 0x00, 0x00, //
    ];

    /// A guest that routes the PIT through the I/O APIC and spins.
    ///
    /// Body from `CODE + 0x34`:
    /// ```text
    /// C7 05 00 00 C0 FE 14 ...   mov dword [0xFEC00000], 0x14  ; IOREGSEL → entry 2 low
    /// C7 05 10 00 C0 FE 40 ...   mov dword [0xFEC00010], 0x40  ;   vector 0x40, fixed,
    ///                                                          ;   physical, edge, unmasked
    /// C7 05 00 00 C0 FE 15 ...   mov dword [0xFEC00000], 0x15  ; IOREGSEL → entry 2 high
    /// C7 05 10 00 C0 FE 00 ...   mov dword [0xFEC00010], 0     ;   destination APIC 0
    /// B0 34 / E6 43              mov al, 0x34 ; out 0x43, al   ; PIT ch0, lo/hi, mode 2
    /// B0 00 / E6 40              count 0x1000 low
    /// B0 10 / E6 40              count 0x1000 high             ; 3.4 ms per tick
    /// FB                         sti
    /// EB FE                      spin: jmp spin                ; no HLT — a delivery must
    ///                                                          ; interrupt a RUNNING processor
    /// ```
    /// Pin 0 is where the PIT's line arrives and `BxIoApic::set_pin_level`
    /// remaps it to pin 2 (Bochs ioapic.cc, "timer connected to pin #2"), which
    /// is why entry 2 is the one programmed.
    pub(crate) fn ioapic_edge_guest() -> std::vec::Vec<u8> {
        let mut code = ENTER_PROTECTED_MODE.to_vec();
        code.extend_from_slice(&[
            0xC7, 0x05, 0x00, 0x00, 0xC0, 0xFE, 0x14, 0x00, 0x00, 0x00, //
            0xC7, 0x05, 0x10, 0x00, 0xC0, 0xFE, 0x40, 0x00, 0x00, 0x00, //
            0xC7, 0x05, 0x00, 0x00, 0xC0, 0xFE, 0x15, 0x00, 0x00, 0x00, //
            0xC7, 0x05, 0x10, 0x00, 0xC0, 0xFE, 0x00, 0x00, 0x00, 0x00, //
            0xB0, 0x34, 0xE6, 0x43, //
            0xB0, 0x00, 0xE6, 0x40, //
            0xB0, 0x10, 0xE6, 0x40, //
            0xFB, //
            0xEB, 0xFE, //
        ]);
        code
    }

    /// The same machine with the LEGACY path instead: no I/O APIC entry, IRQ0
    /// unmasked at the 8259, and LINT0 masked before interrupts are enabled.
    ///
    /// Body from `CODE + 0x34`:
    /// ```text
    /// C7 05 50 03 E0 FE 00 07 01 00   mov dword [0xFEE00350], 0x10700 ; LVT0 masked, ExtINT
    /// B0 FE / E6 21                   mov al, 0xFE ; out 0x21, al     ; unmask IRQ0 alone
    /// B0 34 / E6 43                   PIT ch0, lo/hi, mode 2
    /// B0 00 / E6 40                   count 0x1000 low
    /// B0 10 / E6 40                   count 0x1000 high
    /// FB                              sti
    /// EB FE                           spin: jmp spin
    /// ```
    /// Gate 8 of its IDT points at the same handler, so a vector that reached
    /// the guest despite the mask would write its MARK and be seen — a guest
    /// with an empty gate would fault instead and look like success.
    pub(crate) fn masked_lvt0_guest() -> std::vec::Vec<u8> {
        let mut code = ENTER_PROTECTED_MODE.to_vec();
        code.extend_from_slice(&[
            0xC7, 0x05, 0x50, 0x03, 0xE0, 0xFE, 0x00, 0x07, 0x01, 0x00, //
            0xB0, 0xFE, 0xE6, 0x21, //
            0xB0, 0x34, 0xE6, 0x43, //
            0xB0, 0x00, 0xE6, 0x40, //
            0xB0, 0x10, 0xE6, 0x40, //
            0xFB, //
            0xEB, 0xFE, //
        ]);
        code
    }

    /// The handler both protected-mode guests share: a MARK on the debug port,
    /// an EOI to the local APIC — which the hypervisor owns, so it costs no
    /// exit — and back.
    ///
    /// ```text
    /// mov al, MARK                    ; B0 5A
    /// out 0xE9, al                    ; E6 E9
    /// mov dword [0xFEE000B0], 0       ; C7 05 B0 00 E0 FE 00 00 00 00 — EOI
    /// iretd                           ; CF
    /// ```
    pub(crate) const APIC_HANDLER: [u8; 15] = [
        0xB0, MARK, //
        0xE6, DEBUG_PORT, //
        0xC7, 0x05, 0xB0, 0x00, 0xE0, 0xFE, 0x00, 0x00, 0x00, 0x00, //
        0xCF, //
    ];

    /// Everything the two protected-mode guests need in memory besides their
    /// code: a flat GDT, an IDT whose only filled gate is `vector`, and the
    /// two register images `lgdt`/`lidt` read.
    ///
    /// Written by the test rather than by the guest because a real-mode guest
    /// that assembled its own tables would be twice the hand-assembly for
    /// nothing under test.
    pub(crate) fn load_the_protected_mode_tables(
        machine: &mut Emulator<(), WhpEngine>,
        vector: u8,
    ) {
        // Null; code base 0 limit 4 GiB (0x9A, 0xCF); data (0x92, 0xCF).
        let mut gdt = [0u8; 24];
        gdt[8..16].copy_from_slice(&[0xFF, 0xFF, 0, 0, 0, 0x9A, 0xCF, 0]);
        gdt[16..24].copy_from_slice(&[0xFF, 0xFF, 0, 0, 0, 0x92, 0xCF, 0]);
        machine.mem_write(0x2000, &gdt).expect("the GDT is writable");
        // limit 0x17, base 0x2000
        machine
            .mem_write(0x1F00, &[0x17, 0x00, 0x00, 0x20, 0x00, 0x00])
            .expect("the GDTR image is writable");

        // 0x41 gates, so a vector up to 0x40 has one; only `vector`'s is
        // filled. An interrupt gate: offset 0x1100, selector 0x08, type 0x8E.
        let mut idt = [0u8; 0x208];
        let at = usize::from(vector) * 8;
        idt[at..at + 8].copy_from_slice(&[
            (HANDLER & 0xFF) as u8,
            (HANDLER >> 8) as u8,
            0x08,
            0x00,
            0x00,
            0x8E,
            0x00,
            0x00,
        ]);
        machine.mem_write(0x3000, &idt).expect("the IDT is writable");
        // limit 0x207, base 0x3000
        machine
            .mem_write(0x1F08, &[0x07, 0x02, 0x00, 0x30, 0x00, 0x00])
            .expect("the IDTR image is writable");
        machine.mem_write(HANDLER, &APIC_HANDLER).expect("the handler is writable");
    }

    /// Whether this host can run the hypervisor-gated tests.
    pub(crate) fn hypervisor_here() -> bool {
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
    pub(crate) fn a_turn_on_the_hardware() -> std::sync::MutexGuard<'static, ()> {
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        // A test that panicked while holding the turn poisoned nothing: the
        // guard protects an ordering, not a value.
        TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A `HostClock` two owners can advance — the test and the source under
    /// test. Every handle cloned from one reads and moves the same nanosecond
    /// counter, which is what a clock source taking its host clock by value
    /// leaves a test no other way to do.
    ///
    /// Atomic rather than a cell, so a handle can cross to the thread whose
    /// clock it is: a test that drives a device thread holds one side and the
    /// thread the other. Release on the advance pairs with acquire on the
    /// reading, so a reader that sees a nanosecond span also sees whatever the
    /// advancer set up before granting it.
    #[derive(Clone, Default)]
    pub(crate) struct SharedClock(Arc<AtomicU64>);

    impl SharedClock {
        /// Saturating, matching `ManualClock::advance_nanos` in
        /// `rusty_box_core::time` — a clock that wrapped would run backwards.
        pub(crate) fn advance_nanos(&self, n: u64) {
            let mut nanos = self.0.load(Ordering::Acquire);
            while let Err(seen) = self.0.compare_exchange_weak(
                nanos,
                nanos.saturating_add(n),
                Ordering::Release,
                Ordering::Acquire,
            ) {
                nanos = seen;
            }
        }
    }

    impl HostClock for SharedClock {
        fn now(&self) -> HostInstant {
            HostInstant::from_nanos(self.0.load(Ordering::Acquire))
        }
    }

    /// A clock source over this handle is what a device-thread test moves onto
    /// the thread, so the handle's `Send` is load-bearing rather than incidental.
    /// Pinned here, in the tree's own idiom, because the property is invisible
    /// until the test that needs it exists.
    const _: () = {
        const fn is_send<T: Send>() {}
        is_send::<crate::VmClockSource<SharedClock>>();
    };

    /// Start a machine's partition and hand back the machine behind a lock
    /// beside the processor a thread will run.
    ///
    /// The two halves a threaded machine is made of, and they must be produced
    /// together: the processor is taken out of the engine exactly once, and the
    /// machine that keeps the partition alive is what everyone else reaches
    /// through. The lock is the machine's, not the engine's — a vCPU thread
    /// takes it to service an exit and the test takes it to read a device.
    pub(crate) fn shared(
        mut machine: std::boxed::Box<Emulator<(), WhpEngine>>,
    ) -> (Arc<std::sync::Mutex<std::boxed::Box<Emulator<(), WhpEngine>>>>, rusty_box_whp::Vcpu)
    {
        let vcpu = crate::engine::bring_up(&mut machine).expect("the partition starts");
        (Arc::new(std::sync::Mutex::new(machine)), vcpu)
    }

    /// A vCPU thread that is stopped and joined however its test ends.
    ///
    /// A panicking test that left its thread running would leave the machine
    /// alive for the rest of the process — the thread holds a clone of the
    /// `Arc` — and with the machine goes the partition. One process holds one
    /// partition, so every later hardware test would fail at its first map with
    /// `ERROR_VID_PARTITION_ALREADY_EXISTS`, reporting a platform refusal where
    /// the real defect was an assertion in a test that had already finished.
    /// Measured: one failed assertion here took ten unrelated tests with it.
    pub(crate) struct RunningVcpu {
        control: crate::vcpu_thread::VcpuControl,
        /// Taken by [`Self::stop_and_join`], so the drop below is the net
        /// rather than a second join.
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl RunningVcpu {
        /// Move `vcpu` onto a thread and hold the means to end it.
        pub(crate) fn spawn(
            vcpu: rusty_box_whp::Vcpu,
            index: usize,
            machine: Arc<std::sync::Mutex<std::boxed::Box<Emulator<(), WhpEngine>>>>,
        ) -> Self {
            // A clock started at the machine's own wheel position: the thread
            // catches the wheel up to it at every exit, so one that ran
            // against a clock behind the wheel would earn nothing and one
            // ahead of it would jump the guest forward on its first exit.
            let clock = {
                let machine = machine.lock().expect("the machine's lock");
                let rate = rusty_box_core::time::ClockHz::new(
                    machine.config().ips.per_second_u64(),
                )
                .expect("a positive instruction rate");
                let mut clock = crate::VmClockSource::stopped_at(
                    rusty_box_core::time::VmInstant::from_ticks(machine.ticks()),
                    rate,
                    crate::StdClock::new(),
                );
                clock.start();
                Arc::new(std::sync::Mutex::new(clock))
            };
            let (join, control) =
                crate::vcpu_thread::VcpuThread::spawn(vcpu, index, machine.clone(), clock)
                    .expect("the vCPU thread starts");
            Self { control, join: Some(join) }
        }

        pub(crate) fn control(&self) -> &crate::vcpu_thread::VcpuControl {
            &self.control
        }

        /// Stop the thread and wait for it, which is what releases the
        /// processor handle before the partition it names is destroyed.
        pub(crate) fn stop_and_join(mut self) {
            self.control.stop().expect("the stop reaches the platform");
            if let Some(join) = self.join.take() {
                join.join().expect("the vCPU thread ended without panicking");
            }
        }
    }

    impl Drop for RunningVcpu {
        fn drop(&mut self) {
            let Some(join) = self.join.take() else {
                return;
            };
            match self.control.stop() {
                Ok(()) => {}
                // Reported rather than propagated: a drop running during an
                // unwind cannot panic again, and a stop the platform refused
                // is exactly what a reader chasing the hang below needs to see.
                Err(error) => eprintln!("stopping a vCPU thread failed: {error}"),
            }
            // Bounded, then DETACHED — never an unconditional join. A drop can
            // run while this scope still holds the machine lock (a test that
            // binds the guard before the fixture, or one that panics holding
            // it), and the thread cannot finish servicing an exit until that
            // lock is free, so joining here would deadlock the two against each
            // other. A thread wedged inside `run()` has the same shape. Letting
            // it go leaks a thread for the length of the test process, which is
            // what the suite can afford; hanging is what it cannot. Task 1.7's
            // `FastMachine::drop` detaches a wedged thread for the same reason.
            if !wait_until_thread_ends(&join, std::time::Duration::from_secs(5)) {
                eprintln!(
                    "a vCPU thread did not end within 5 s and was detached; its next platform \
                     call fails with an invalid handle and it returns"
                );
                return;
            }
            match join.join() {
                Ok(()) => {}
                Err(_) => eprintln!("a vCPU thread panicked"),
            }
        }
    }

    /// Whether `join` has finished within `within`, without consuming it.
    ///
    /// `JoinHandle::is_finished` is the only way to ask without committing to
    /// the wait that `join` is.
    fn wait_until_thread_ends(
        join: &std::thread::JoinHandle<()>,
        within: std::time::Duration,
    ) -> bool {
        let deadline = std::time::Instant::now() + within;
        while std::time::Instant::now() < deadline {
            if join.is_finished() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        join.is_finished()
    }

    /// Poll `ready` until it answers true, or fail by name once `within` has
    /// passed.
    ///
    /// Bounded rather than a bare loop because every caller waits on another
    /// THREAD: a condition that never arrives is a defect to be reported at the
    /// assertion that named it, and a test that hangs instead reports nothing
    /// at all and takes the suite with it.
    pub(crate) fn wait_until(
        mut ready: impl FnMut() -> bool,
        within: std::time::Duration,
        what: &str,
    ) {
        let deadline = std::time::Instant::now() + within;
        while std::time::Instant::now() < deadline {
            if ready() {
                return;
            }
            std::thread::yield_now();
        }
        panic!("{what}: never happened within {within:?}");
    }
}

pub use device_thread::DeviceThreadControl;
pub use engine::{ExitCounts, PlatformCounters, WhpEngine};
pub use fast_machine::{EngineCensus, FastMachine, FastMachineFault, StepOutcome, StepStop};
pub use vcpu_thread::VcpuCensus;
pub use vm_clock::{StdClock, VmClockSource};

/// What has to be true for a machine to be run by a thread of its own, pinned
/// where the machine and the thread meet.
///
/// `rusty_box`'s own assertions cover `Emulator<()>` — the software engine —
/// and cannot see this one: the engine is a type parameter, and a `Send` that
/// holds for an interpreter says nothing about a partition handle. The
/// `Emulator<(), WhpEngine>` is what goes into the `Arc<Mutex<_>>` every thread
/// reaches the machine through; the control is what they reach the vCPU thread
/// through, from several threads at once; and the thread itself is what moves
/// onto a thread in the first place.
///
/// The device thread's control is held by the machine's driver and by the
/// thread at once, and the driver itself is the object a caller keeps — so a
/// `FastMachine` that could not cross a thread boundary would be unusable from
/// any front end that owns its machine on a worker.
const _: () = {
    const fn s<M: Send>() {}
    const fn ss<M: Send + Sync>() {}
    s::<rusty_box::emulator::Emulator<(), WhpEngine>>();
    ss::<vcpu_thread::VcpuControl>();
    s::<vcpu_thread::VcpuThread<()>>();
    ss::<DeviceThreadControl>();
    s::<FastMachine<()>>();
};

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

#[cfg(test)]
mod tests {
    use crate::fixtures::{
        a_turn_on_the_hardware, fast_machine_running, hypervisor_here, ioapic_edge_guest,
        load_the_protected_mode_tables, machine_running, machine_with_devices_on,
        masked_lvt0_guest, shared, wait_until, RunningVcpu, CODE, DEBUG_PORT, MARK,
    };
    use crate::vcpu_thread::Parked;
    use rusty_box::cpu::instrumentation::{CpuSetupMode, X86Reg};
    use rusty_box::emulator::{
        DeviceClock, Emulator, EmulatorConfig, EngineRefusal, MemorySize, RunBudget, StopReason,
    };
    use rusty_box::Error;

    /// A machine running on the host's hypervisor executes a guest, and its
    /// output arrives in the machine's own devices.
    ///
    /// The end-to-end proof of the adapter: the partition is built from the
    /// machine's own memory plan, the shadow processor's state is installed on
    /// the platform, a real guest instruction runs on hardware, and the port
    /// write it makes exits to this port's device set. Nothing here is mocked.
    ///
    /// The guest's `HLT` is no longer asserted on. Under the hypervisor's own
    /// local APIC a halt produces no exit at all — measured, probe P3 — and a
    /// halted processor stays inside its run rather than handing the machine
    /// back, so there is nothing here for a step to report.
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
        let mut machine = fast_machine_running(&[0xB0, MARK, 0xE6, DEBUG_PORT, 0xF4]);
        machine
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest");

        let written: std::vec::Vec<u8> =
            machine.with_machine(|m| m.debug_port().take_output().collect());
        assert_eq!(
            written,
            std::vec![MARK],
            "the byte the guest sent to port 0xE9 must arrive in this machine's devices"
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
        let mut machine = fast_machine_running(&[
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

        let written: std::vec::Vec<u8> =
            machine.with_machine(|m| m.debug_port().take_output().collect());
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
        let mut on_hardware = fast_machine_running(asking);
        on_hardware
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest");
        let by_the_hardware: std::vec::Vec<u8> =
            on_hardware.with_machine(|m| m.debug_port().take_output().collect());

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
        let mut on_hardware = fast_machine_running(asking);
        on_hardware
            .step(RunBudget::Ticks(1_000_000))
            .expect("the hypervisor runs the guest");
        let by_the_hardware: std::vec::Vec<u8> =
            on_hardware.with_machine(|m| m.debug_port().take_output().collect());

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

        // A fixed number of steps rather than a loop that stops on a halt: a
        // halted processor under the hypervisor's own APIC produces no exit
        // and stays inside its run (measured, probe P3), so there is nothing
        // for a step to report and nothing to break on. Eight milliseconds of
        // guest time is far more than this guest needs to finish its trace.
        let _turn = a_turn_on_the_hardware();
        let (hw_base_count, hw_base) = {
            let mut machine = fast_machine_running(baseline);
            for _ in 0..8 {
                machine
                    .step(RunBudget::Ticks(1_000_000))
                    .expect("the hypervisor runs the guest");
            }
            machine.with_machine(read_trace)
        };
        let (hw_err_count, hw_err, hw_err_memory_exits) = {
            let mut machine = fast_machine_running(with_errand);
            for _ in 0..8 {
                machine
                    .step(RunBudget::Ticks(1_000_000))
                    .expect("the hypervisor runs the guest");
            }
            let (count, ips) = machine.with_machine(read_trace);
            (count, ips, machine.engine_census().exits.memory)
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

    /// A CPUID exit on the thread imports the groups it needs, answers on the
    /// shadow, and exports the answer back to the partition.
    ///
    /// The port test covers the one arm that moves NO architectural state.
    /// This covers the cheapest arm that does, and it covers it the only way
    /// worth having: the guest reads `CPUID`'s own answer back out of `EBX`
    /// and sends it to a device. A byte arriving proves all three halves —
    /// the import put the leaf in the shadow, the shadow answered it, and the
    /// export put the answer where the guest could read it. An export that
    /// silently moved nothing would leave `BL` holding whatever the register
    /// happened to contain, and the guest would spin having sent a zero.
    #[test]
    fn a_cpuid_exit_on_the_thread_answers_on_the_shadow_and_exports_the_answer() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        // mov eax, 0 ; cpuid ; mov al, bl ; out DEBUG_PORT, al ; jmp $
        // Leaf 0 is inside `TRAPPED_CPUID_LEAVES`, so it exits rather than
        // being answered by the hardware; its `EBX` is the first four bytes of
        // the vendor string, which no model leaves zero.
        let (machine, vcpu) = shared(machine_running(&[
            0x66, 0xB8, 0x00, 0x00, 0x00, 0x00, 0x0F, 0xA2, 0x88, 0xD8, 0xE6, DEBUG_PORT, 0xEB,
            0xFE,
        ]));
        let running = RunningVcpu::spawn(vcpu, 0, machine.clone());
        let control = running.control().clone();

        let mut seen: std::vec::Vec<u8> = std::vec::Vec::new();
        wait_until(
            || {
                seen.extend(machine.lock().expect("the machine's lock").debug_port().take_output());
                !seen.is_empty()
            },
            std::time::Duration::from_secs(2),
            "the guest sent CPUID's own EBX to the debug port",
        );
        assert_ne!(
            seen[0], 0,
            "the low byte of leaf 0's EBX reached the guest, so the answer crossed back out of \
             the shadow rather than the export moving nothing"
        );

        control.request_park(Parked::Paused).expect("the park request reaches the platform");
        assert_eq!(
            control.wait_parked_by(std::time::Duration::from_secs(5)),
            Some(Parked::Paused),
        );
        let census = control.census();
        assert_eq!(census.exits.cpuid, 1, "one CPUID exit, not a repeated one: {census:?}");
        assert_eq!(census.exits.port, 1, "and the one port write that carried its answer");
        assert!(
            census.export_calls >= 1,
            "a CPUID exit imports and exports register groups, unlike a plain port exit: \
             {census:?}"
        );
        running.stop_and_join();
    }

    /// An I/O APIC edge reaches a guest whose local APIC is the hypervisor's,
    /// and the local APIC's own registers are not trapped: the only memory
    /// exits are the four I/O APIC programming writes.
    ///
    /// The whole of Task 1.6's I/O APIC half in one guest. The message leaves
    /// the fabric as a `WHvRequestInterrupt` on the partition rather than a
    /// write to this machine's own model, and the guest's `SVR` and `EOI`
    /// stores — which under a partition with no APIC would each be a memory
    /// exit into the shadow — cost nothing at all, because the page belongs to
    /// the hypervisor.
    #[test]
    fn an_ioapic_edge_is_delivered_by_the_hypervisor_apic_without_an_exit() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = machine_with_devices_on(DeviceClock::HostTime, &ioapic_edge_guest());
        load_the_protected_mode_tables(&mut machine, 0x40);
        let ips = machine.config().ips.per_second_u64();
        let (machine, vcpu) = shared(machine);
        let running = RunningVcpu::spawn(vcpu, 0, machine.clone());
        let control = running.control().clone();

        // The test IS the device thread: one millisecond of wheel per poll,
        // which is what makes the PIT tick at all under `HostTime`.
        let mut seen: std::vec::Vec<u8> = std::vec::Vec::new();
        wait_until(
            || {
                let mut m = machine.lock().expect("the machine's lock");
                m.service_device_time(ips / 1_000).expect("the wheel turns");
                seen.extend(m.debug_port().take_output());
                seen.len() >= 3
            },
            std::time::Duration::from_secs(5),
            "three PIT ticks reached the guest's handler through the hypervisor's local APIC",
        );
        // Read while the thread still runs: the park's own cancel is an exit,
        // and "no cancel" is half the claim — a delivery through the
        // partition's APIC never fetches the processor out of its run.
        let running_census = control.census();
        assert_eq!(
            (
                running_census.exits.canceled,
                running_census.exits.window,
                running_census.exits.halt
            ),
            (0, 0, 0),
            "no cancel, no window, no halt: {running_census:?}"
        );

        control.request_park(Parked::Paused).expect("the park request reaches the platform");
        assert_eq!(
            control.wait_parked_by(std::time::Duration::from_secs(5)),
            Some(Parked::Paused),
            "the thread parked within 5 s"
        );
        // Drained after the park, so the counts below describe a machine
        // nothing is still running against.
        seen.extend(machine.lock().expect("the machine's lock").debug_port().take_output());
        assert!(seen.iter().all(|byte| *byte == MARK), "{seen:#04x?}");
        let census = control.census();
        assert_eq!(
            census.exits.memory, 4,
            "IOREGSEL/IOWIN twice each — and NOT the SVR and EOI writes to the local APIC \
             page, which the hypervisor owns: {census:?}"
        );
        assert_eq!(
            census.exits.port,
            3 + seen.len() as u64,
            "three PIT programming writes plus one MARK per tick; the deliveries themselves \
             cost no exit: {census:?}"
        );
        running.stop_and_join();
    }

    /// A masked LINT0 keeps the legacy path shut.
    ///
    /// The 8259's vector reaches the partition's APIC as a `WHvRequestInterrupt`,
    /// which no LVT entry masks: nothing in the hypervisor honours the guest's
    /// LINT0 mask, so this machine's interrupt fabric is the only thing that
    /// can — and this test is what stands between a masked line and a
    /// delivered interrupt.
    ///
    /// The guest is the positive control for itself: IRQ0 is unmasked at the
    /// 8259 and the PIT ticks, so the INT pin genuinely rises and stays owed.
    /// What must not happen is the acknowledge — and the vector's gate is
    /// filled, so one that arrived anyway would announce itself.
    #[test]
    fn a_masked_lvt0_keeps_the_legacy_path_closed() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        let mut machine = machine_with_devices_on(DeviceClock::HostTime, &masked_lvt0_guest());
        load_the_protected_mode_tables(&mut machine, 0x08);
        let ips = machine.config().ips.per_second_u64();
        let (machine, vcpu) = shared(machine);
        let running = RunningVcpu::spawn(vcpu, 0, machine.clone());
        let control = running.control().clone();

        // Half a second of wheel — around 145 PIT ticks at this count, every
        // one of which raises the 8259's INT pin.
        let mut seen: std::vec::Vec<u8> = std::vec::Vec::new();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < until {
            let mut m = machine.lock().expect("the machine's lock");
            m.service_device_time(ips / 1_000).expect("the wheel turns");
            seen.extend(m.debug_port().take_output());
            drop(m);
            std::thread::yield_now();
        }
        let census = control.census();
        assert!(
            seen.is_empty(),
            "a vector the guest masked at LINT0 reached its handler: {seen:#04x?}"
        );
        let mut guard = machine.lock().expect("the machine's lock");
        assert!(
            guard.processor(0).io.device_manager().has_interrupt(),
            "the 8259's INT pin must have risen and still be owed — without that this test \
             never reached the gate and proves nothing: {census:?}"
        );
        assert_eq!(
            guard.processor(0).io.device_manager().irq().acknowledge_count(),
            0,
            "nothing may be acknowledged at the controllers for a masked line"
        );
        drop(guard);
        running.stop_and_join();
    }

    /// A guest on the vCPU thread writes a byte to the debug port and spins;
    /// the byte reaches the machine's device and the thread parks on request.
    ///
    /// The whole shape of the engine from here on, in one test: the thread
    /// enters the partition and STAYS there, taking the machine's lock only to
    /// service the one exit the guest produces, and leaving the run only
    /// because the host asked for the processor back. The debug port is read
    /// from the test's own thread through that same lock, which is what makes
    /// the byte proof that the two threads met at the machine rather than
    /// racing past each other.
    #[test]
    fn a_vcpu_thread_services_a_port_exit_under_the_machine_lock() {
        if !hypervisor_here() {
            return;
        }
        let _turn = a_turn_on_the_hardware();
        // mov al, MARK ; out DEBUG_PORT, al ; jmp $
        let (machine, vcpu) =
            shared(machine_running(&[0xB0, MARK, 0xE6, DEBUG_PORT, 0xEB, 0xFE]));
        let running = RunningVcpu::spawn(vcpu, 0, machine.clone());
        let control = running.control().clone();

        let mut seen: std::vec::Vec<u8> = std::vec::Vec::new();
        wait_until(
            || {
                seen.extend(machine.lock().expect("the machine's lock").debug_port().take_output());
                seen.contains(&MARK)
            },
            std::time::Duration::from_secs(2),
            "the guest's MARK reached the debug port",
        );

        control.request_park(Parked::Paused).expect("the park request reaches the platform");
        assert_eq!(
            control.wait_parked_by(std::time::Duration::from_secs(5)),
            Some(Parked::Paused),
            "the thread parked within 5 s — a cancel that never stuck"
        );
        let census = control.census();
        assert!(
            census.runs >= 1 && census.exits.port == 1 && census.in_run_nanos > 0,
            "{census:?}"
        );
        assert_eq!(
            census.export_calls, 0,
            "a plain port exit moves no architectural state: the platform decoded it, the \
             device answered it, and RIP and RAX went back as two words — {census:?}"
        );
        assert!(
            census.platform_at_last_park.is_some(),
            "the park refreshed the platform's own counters, which only the thread holding the \
             processor can read: {census:?}"
        );

        // A resumed thread goes back into the partition rather than parking
        // again on the request that stopped it: the resume clears the flag
        // before it clears the park slot, and a thread that woke with the flag
        // still set would come straight back out without ever entering.
        //
        // Waited for rather than asserted straight after the resume, because
        // the thread has to be observed INSIDE the partition before the second
        // park request is made — a request that raced the wake would be
        // answered at the loop head, and the entry the test is about would
        // never happen. `runs` is counted at the entry, so it moves while the
        // guest is still spinning.
        let entries = census.runs;
        let cancels = census.exits.canceled;
        control.resume();
        wait_until(
            || control.census().runs > entries,
            std::time::Duration::from_secs(5),
            "the resumed thread entered the partition again",
        );
        control.request_park(Parked::Paused).expect("the second park request reaches the platform");
        assert_eq!(
            control.wait_parked_by(std::time::Duration::from_secs(5)),
            Some(Parked::Paused),
            "a resumed thread can be parked again"
        );
        assert!(
            control.census().exits.canceled > cancels,
            "the second park fetched the thread out of a run it was already inside, which is \
             what a cancel is for"
        );

        // A parked thread told to stop returns from its run loop, and the join
        // is what discharges the obligation the `Vcpu` carries onto the thread:
        // the handle names a partition this machine is about to drop.
        running.stop_and_join();
    }
}
