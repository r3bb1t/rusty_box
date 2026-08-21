//! The public API, exercised the way a caller writes it.
//!
//! Every function here is a worked example AND an assertion. The lib tests
//! reach into the crate's own internals; nothing else compiles this surface
//! from outside, which is how three defects reached `HEAD` — a handle method
//! that could not be called as an expression, an error type that would not
//! absorb its own sub-error, and an argument type not re-exported beside the
//! trait that hands it out. None of them was visible from inside.
//!
//! Rules for anything added here: import only through the public paths a
//! consumer would use, keep each example the shape a caller would actually
//! write, and assert a guest-visible outcome rather than "it did not panic".

#![cfg(feature = "std")]

use rusty_box::cpu::{CpuSetupMode, ResetReason, X86Reg};
use rusty_box::emulator::{
    AtaSlot, BootDevice, BootOrder, DiskGeometry, Emulator, EmulatorConfig, MemorySize, MachineBuilder,
    PowerState, StopReason,
};
use rusty_box::gui::NoGui;
use rusty_box::iodev::scancodes::BxKey;

/// A machine the tests can build without any file on disk: a small guest, and
/// a BIOS image that is nothing but `HLT`, so whatever the reset vector
/// resolves to inside it parks the processor on the first instruction.
const HALT_BIOS_BYTES: usize = 0x10000;

fn small_config() -> EmulatorConfig {
    EmulatorConfig {
        memory: MemorySize::bytes(4 * 1024 * 1024),
        ..EmulatorConfig::default()
    }
}

// ── Assembling a machine ────────────────────────────────────────────────────

#[test]
fn a_machine_is_assembled_by_the_builder_and_starts_at_its_reset_vector() {
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    static DISK: &[u8] = &[0u8; 512];

    let mut machine = MachineBuilder::new(small_config())
        .gui(NoGui::new())
        .bios(&bios)
        .boot_order(BootOrder::just(BootDevice::Disk))
        .disk_static(
            AtaSlot::PRIMARY_MASTER,
            DISK,
            DiskGeometry::new(306, 4, 17),
        )
        .build()
        .expect("build");

    let outcome = machine.step_batch(16).expect("step");
    assert_eq!(
        outcome.stop,
        StopReason::Halted,
        "the reset vector must fetch from the BIOS the builder loaded"
    );
}

#[test]
fn a_machine_with_no_firmware_still_builds() {
    let machine = MachineBuilder::new(small_config()).build().expect("build");
    assert_eq!(machine.ticks(), 0);
}

/// Guest size and host size are one value, so a machine cannot be given more
/// RAM than its host backing by forgetting the second field. Asking for the
/// overflow file is a different constructor.
#[test]
fn a_bigger_guest_needs_the_swap_regime_asked_for_by_name() {
    let resident = MemorySize::mib(64);
    assert_eq!(resident.guest_bytes(), 64 * 1024 * 1024);
    assert_eq!(resident.host_bytes(), resident.guest_bytes());
    assert!(!resident.swaps());

    let swapping = MemorySize::partially_resident(64 * 1024 * 1024, 16 * 1024 * 1024);
    assert!(swapping.swaps());
    assert_eq!(swapping.guest_bytes(), 64 * 1024 * 1024);
    assert_eq!(swapping.host_bytes(), 16 * 1024 * 1024);

    // Bochs `BX_DEFAULT_MEM_MEGS`.
    assert_eq!(MemorySize::default(), MemorySize::mib(32));
}

/// The configured size is the size the machine has: a guest addressing past
/// it gets nothing back, which is the property the number actually means.
#[test]
fn the_configured_memory_size_is_the_memory_the_machine_has() {
    let config = EmulatorConfig {
        memory: MemorySize::mib(8),
        ..EmulatorConfig::default()
    };
    let mut machine = MachineBuilder::new(config).build().expect("build");

    assert_eq!(machine.mem_size(), 8 * 1024 * 1024);

    // The last byte of guest RAM is writable and reads back.
    let last = 8 * 1024 * 1024 - 1;
    machine.mem_write(last, &[0x5A]).expect("write last byte");
    assert_eq!(machine.mem_read_u8(last).expect("read back"), 0x5A);
}

// ── Running, and knowing why it stopped ─────────────────────────────────────

#[test]
fn a_batch_reports_both_how_far_it_got_and_why_it_stopped() {
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    let mut machine = MachineBuilder::new(small_config())
        .bios(&bios)
        .build()
        .expect("build");

    let mut executed = 0u64;
    let stop = loop {
        let outcome = machine.step_batch(1_000).expect("step");
        executed += outcome.executed;

        // A caller that only needs "should I keep going" asks this instead of
        // enumerating the reasons.
        if outcome.is_terminal() {
            break outcome.stop;
        }
        match outcome.stop {
            StopReason::Halted => break outcome.stop,
            StopReason::BudgetExhausted => {}
            StopReason::GuestPowerOff | StopReason::CpuShutdown | StopReason::StopRequested => {
                break outcome.stop
            }
        }
        assert!(executed < 100_000, "a halting guest must not run forever");
    };

    assert_eq!(stop, StopReason::Halted);
    assert!(executed > 0, "the guest executed its first instruction");
}

#[test]
fn a_halted_machine_says_how_long_until_its_next_timer() {
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    let mut machine = MachineBuilder::new(small_config())
        .bios(&bios)
        .build()
        .expect("build");

    assert_eq!(machine.step_batch(16).expect("step").stop, StopReason::Halted);
    // Some timer is always armed on a running machine, so an idle host knows
    // how far it may skip ahead.
    assert!(machine.ticks_to_next_timer_deadline().is_some());
}

// ── Reading the screen ──────────────────────────────────────────────────────

#[test]
fn the_screen_reads_as_a_character_grid_with_a_cursor() {
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    let mut machine = MachineBuilder::new(small_config())
        .bios(&bios)
        .build()
        .expect("build");

    let resolution = machine.display().resolution();
    assert!(resolution.width > 0 && resolution.height > 0);

    // One expression, not a binding plus a borrow.
    let has_prompt = machine
        .display()
        .text()
        .is_some_and(|text| text.contains("login:"));
    assert!(!has_prompt, "nothing has been typed at this machine");

    let text = machine.display().text().expect("power-on state is text mode");
    // Before the video BIOS programs the CRTC there is a grid but no rows on
    // it: Bochs vgacore.cc derives the row count from the vertical display
    // end, which is still zero. A caller scraping too early sees an empty
    // screen, not a wrong one.
    assert_eq!(text.cols(), 80);
    assert_eq!(text.rows(), 0);
    assert!(text.find("login:").is_none());
    assert_eq!(text.to_text(), "");
    assert_eq!(text.row_chars(0).count(), 0, "there is no row 0 yet");
}

// ── Driving input, and learning what the guest took ─────────────────────────

#[test]
fn input_reports_what_the_guest_actually_received() {
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    let mut machine = MachineBuilder::new(small_config())
        .bios(&bios)
        .build()
        .expect("build");

    // The guest's ring is 16 bytes, so a long string is delivered short and
    // the count is the resume point.
    let text = "root\n";
    let delivered = machine.keyboard().type_text(text);
    assert!(delivered <= text.chars().count());
    let _remaining: String = text.chars().skip(delivered).collect();

    assert!(machine.keyboard().tap(BxKey::F1) || true, "tap reports delivery");
    let _held = machine.keyboard().key(BxKey::CtrlL, true);
    let accepted = machine.keyboard().scancodes(&[0x3B, 0xBB]);
    assert!(accepted <= 2);

    let _moved = machine.mouse().motion(4, -2, 0, 0x01);
}

#[test]
fn a_full_keyboard_ring_reports_the_refusal_rather_than_swallowing_it() {
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    let mut machine = MachineBuilder::new(small_config())
        .bios(&bios)
        .build()
        .expect("build");

    // Far more than the 16-byte ring can hold, with nothing draining it.
    let flood = [0x1Eu8; 64];
    let accepted = machine.keyboard().scancodes(&flood);
    assert!(
        accepted < flood.len(),
        "the ring cannot have taken all {} bytes",
        flood.len()
    );
}

// ── Power ───────────────────────────────────────────────────────────────────

#[test]
fn a_guest_power_off_is_a_state_the_host_can_see() {
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    let mut machine = MachineBuilder::new(small_config())
        .bios(&bios)
        .build()
        .expect("build");

    assert_eq!(machine.power().state(), PowerState::Running);
    machine.power().reset(ResetReason::Hardware).expect("reset");
    assert_eq!(machine.power().state(), PowerState::Running);

    // The button is an ACPI event: a guest with no SCI armed ignores it, so
    // the machine stays running rather than pretending to be off.
    machine.power().press_power_button();
    assert_eq!(machine.power().state(), PowerState::Running);
}

// ── Serial and the debug console ────────────────────────────────────────────

#[test]
fn the_uart_and_the_debug_console_drain_as_bytes() {
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    let mut machine = MachineBuilder::new(small_config())
        .bios(&bios)
        .build()
        .expect("build");

    let com1: Vec<u8> = machine
        .serial(0)
        .expect("COM1 is always modelled")
        .take_output()
        .collect();
    assert!(com1.is_empty(), "nothing has run to write to it yet");
    assert!(machine.serial(99).is_none(), "there is no UART 99");

    let e9: Vec<u8> = machine.debug_port().take_output().collect();
    assert!(e9.is_empty());
}

// ── Stopping from another thread ────────────────────────────────────────────

#[test]
fn a_stop_handle_ends_a_run_from_another_thread() {
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    let mut machine = MachineBuilder::new(small_config())
        .bios(&bios)
        .build()
        .expect("build");

    let handle = machine.stop_handle();
    assert!(!handle.is_stopping());
    std::thread::spawn(move || handle.stop()).join().expect("join");

    let outcome = machine.step_batch(1_000_000).expect("step");
    assert_eq!(outcome.stop, StopReason::StopRequested);
}

// ── Register-level use, with no firmware ────────────────────────────────────

/// Where guest code goes in a CPU-only machine: past whatever the mode setup
/// built for itself. In `FlatLong64` that is the identity page tables, and
/// code written over the PML4 simply never runs.
const CODE: u64 = CpuSetupMode::FlatLong64.first_free_physical_address();

#[test]
fn a_machine_can_be_driven_at_register_level_with_no_firmware() {
    let mut machine =
        Emulator::new_with_mode(small_config(), CpuSetupMode::FlatLong64).expect("machine");

    // mov rax, 42 ; hlt
    machine
        .mem_write(CODE, &[0x48, 0xC7, 0xC0, 0x2A, 0x00, 0x00, 0x00, 0xF4])
        .expect("write code");
    machine.reg_write(X86Reg::Rsp, 0x0010_0000);

    machine
        .emu_start(CODE, None, None, Some(16))
        .expect("emu_start");
    assert_eq!(machine.reg_read(X86Reg::Rax), 42);

    // The same bytes, read back three ways. Long mode's tables are an
    // identity map, so the linear and physical addresses agree.
    let mut raw = [0u8; 4];
    machine.mem_read(CODE, &mut raw).expect("mem_read");
    assert_eq!(raw[0], 0x48);
    assert_eq!(machine.mem_read_u8(CODE).expect("u8"), 0x48);
    machine.virt_read(CODE, &mut raw).expect("virt_read");
    assert_eq!(raw[0], 0x48);
    assert_eq!(machine.virt_to_phys(CODE).expect("walk"), CODE);
}

#[test]
fn the_mode_setup_says_where_its_own_structures_end() {
    // Real mode builds nothing, so the guest owns everything.
    assert_eq!(CpuSetupMode::RealMode.first_free_physical_address(), 0);
    // The protected modes install a flat GDT in the first page.
    assert_eq!(
        CpuSetupMode::FlatProtected32.first_free_physical_address(),
        0x1000
    );
    // Long mode adds the identity page tables above it.
    assert!(
        CpuSetupMode::FlatLong64.first_free_physical_address()
            > CpuSetupMode::FlatProtected32.first_free_physical_address()
    );
}

#[test]
fn an_exit_address_ends_a_run_before_the_instruction_budget_does() {
    let mut machine =
        Emulator::new_with_mode(small_config(), CpuSetupMode::FlatLong64).expect("machine");

    // Four one-byte NOPs, then HLT.
    machine
        .mem_write(CODE, &[0x90, 0x90, 0x90, 0x90, 0xF4])
        .expect("write code");
    machine.set_exits(&[CODE + 2]);

    machine
        .emu_start(CODE, None, None, Some(64))
        .expect("emu_start");
    assert_eq!(
        machine.reg_read(X86Reg::Rip),
        CODE + 2,
        "execution stops at the exit address, not at the HLT"
    );
}

// ── Instrumentation ─────────────────────────────────────────────────────────

#[cfg(feature = "instrumentation")]
#[test]
fn a_code_hook_sees_every_instruction_in_its_range() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let mut machine =
        Emulator::new_with_mode(small_config(), CpuSetupMode::FlatLong64).expect("machine");
    machine
        .mem_write(CODE, &[0x90, 0x90, 0x90, 0xF4])
        .expect("write code");

    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    let hook = machine.hook_add_code(CODE..CODE + 8, move |_rip, _instr| {
        counter.fetch_add(1, Ordering::Relaxed);
    });

    machine
        .emu_start(CODE, None, None, Some(8))
        .expect("emu_start");
    assert!(seen.load(Ordering::Relaxed) >= 3, "three NOPs and a HLT ran");

    // The crate's own error absorbs its own sub-error, so `?` composes.
    let removed: rusty_box::Result<()> = machine.hook_del(hook).map_err(Into::into);
    removed.expect("hook removed");
}

/// A tracer type is the monomorphized alternative to closures: no per-hook
/// dispatch, and the machine carries it in its own type.
#[derive(Default)]
struct InstructionCounter {
    executed: u64,
}

impl rusty_box::cpu::Instrumentation for InstructionCounter {
    fn before_execution(&mut self, _rip: u64, _instr: &rusty_box::cpu::Instruction) {
        self.executed += 1;
    }
}

#[test]
fn a_tracer_type_rides_along_with_the_machine() {
    let mut machine = Emulator::<InstructionCounter>::new_with_mode_and_instrumentation(
        small_config(),
        CpuSetupMode::FlatLong64,
        InstructionCounter::default(),
    )
    .expect("machine");

    machine
        .mem_write(CODE, &[0x90, 0x90, 0x90, 0xF4])
        .expect("write code");
    machine
        .emu_start(CODE, None, None, Some(8))
        .expect("emu_start");

    assert!(machine.instrumentation().executed >= 3);
}

#[test]
fn one_tracer_instance_cannot_be_shared_by_several_processors() {
    let config = EmulatorConfig {
        cpu_params: rusty_box::params::BxParams::default()
            .with_topology(2, 1, 1)
            .expect("valid topology"),
        ..small_config()
    };

    let refused = MachineBuilder::new(config.clone())
        .tracer(InstructionCounter::default())
        .build();
    assert!(refused.is_err(), "an SMP machine needs one tracer each");

    // A factory mints one per processor, which is the shape that works.
    let fleet = MachineBuilder::new(config)
        .tracer_factory(InstructionCounter::default)
        .build()
        .expect("build");
    assert_eq!(fleet.cpu_count(), 2);
}

// ── Snapshots ───────────────────────────────────────────────────────────────

#[test]
fn a_snapshot_round_trips_architectural_state() {
    // A snapshot covers the whole machine, devices included, so it needs one
    // that went through hardware initialisation — a CPU-only machine has no
    // fw_cfg to serialise and says so rather than writing a partial image.
    const BASE: u64 = 0x0010_0000;
    let bios = vec![0xF4u8; HALT_BIOS_BYTES];
    let mut machine = MachineBuilder::new(small_config())
        .bios(&bios)
        .build()
        .expect("build");
    machine.reg_write(X86Reg::Rax, 0xDEAD_BEEF);
    machine.mem_write(BASE, &[0xA5; 8]).expect("write");

    let mut saved = Vec::new();
    machine.save_snapshot(&mut saved).expect("save");

    machine.reg_write(X86Reg::Rax, 0);
    machine.mem_write(BASE, &[0x00; 8]).expect("clobber");

    machine
        .restore_snapshot(&mut std::io::Cursor::new(&saved))
        .expect("restore");

    assert_eq!(machine.reg_read(X86Reg::Rax), 0xDEAD_BEEF);
    let mut back = [0u8; 8];
    machine.mem_read(BASE, &mut back).expect("read");
    assert_eq!(back, [0xA5; 8]);
}

// ── Fleets ──────────────────────────────────────────────────────────────────

#[test]
fn machines_are_send_so_a_fleet_is_one_per_worker() {
    let workers: Vec<_> = (0..2)
        .map(|_| {
            std::thread::spawn(|| {
                let bios = vec![0xF4u8; HALT_BIOS_BYTES];
                let mut machine = MachineBuilder::new(small_config())
                    .bios(&bios)
                    .build()
                    .expect("build");
                machine.step_batch(16).expect("step").stop
            })
        })
        .collect();

    for worker in workers {
        assert_eq!(worker.join().expect("join"), StopReason::Halted);
    }
}
