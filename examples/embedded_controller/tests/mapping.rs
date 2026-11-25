use rusty_box_aarch64::board::address_space::{
    AddressRange, AddressSpace, DevicePair, MapError, MappedDevice, MappingRefused, Mounted, Ram,
    Rom,
};
use rusty_box_aarch64::bus::{DescriptorSwap, FetchSource};
use rusty_box_aarch64::machine::custom::{
    BoardInputs, InvalidDeadline, MachineBoard, ProcessorOutputs,
};
use rusty_box_aarch64::{
    Aarch64Machine, AccessWidth, BusFault, ClockHz, Cpu, CpuConfig, CpuModel, NoEl2, PhysAddr,
    PhysBus, Reg, RunBudget, StopReason,
};
use rusty_box_core::events::{App, ExecutionRequest};
use rusty_box_core::{VmClock, VmInstant};
use rusty_box_embedded_controller::{Gpio, OutputChanged};

#[test]
fn typed_device_subscribers_stop_after_the_guest_write_commits() {
    let mut ram = [0; 4096];
    // mov w0,#42; str w0,[x1,#4]; mov w2,#7
    for (bytes, word) in ram
        .chunks_exact_mut(4)
        .zip([0x5280_0540u32, 0xB900_0420, 0x5280_00E2])
    {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    let app = App::new()
        .route(|event: &OutputChanged| {
            assert_eq!(event.0, 42);
            ExecutionRequest::Stop
        })
        .with_state(());
    let gpio = MappedDevice::new(
        AddressRange::new(PhysAddr::new(0x5000), 8).unwrap(),
        AccessWidth::Word,
        Gpio::default(),
    )
    .unwrap()
    .with_events(app);
    let board = AddressSpace::new(
        Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
        None,
        gpio,
    )
    .unwrap();
    let cpu = Cpu::<NoEl2>::new(
        CpuModel::CortexA57,
        CpuConfig {
            ips: ClockHz::BOCHS_DEFAULT,
            reset_pc: 0,
        },
    );
    let mut machine = Aarch64Machine::new(cpu, board);
    machine.reg_write(Reg::X1, 0x5000);
    let untouched = machine.reg_read(Reg::X2);
    let written = machine.step(RunBudget::Instructions(3)).unwrap();
    assert_eq!(written.stop, rusty_box_aarch64::StopReason::StopRequested);
    assert_eq!(written.progress.instructions(), Some(2));
    assert_eq!(machine.board().devices().device().output(), 42);
    assert_eq!(machine.reg_read(Reg::X2), untouched);
    assert_eq!(
        machine
            .step(RunBudget::Instructions(1))
            .unwrap()
            .progress
            .instructions(),
        Some(1)
    );
    assert_eq!(machine.reg_read(Reg::X2), 7);
}

#[test]
fn overlapping_ram_rom_or_device_intervals_cannot_be_mounted() {
    let range = AddressRange::new(PhysAddr::new(16), 8).unwrap();
    let device = || MappedDevice::new(range, AccessWidth::Word, Gpio::default()).unwrap();
    assert!(matches!(
        DevicePair::new(device(), device()),
        Err(MappingRefused {
            reason: MapError::Overlap,
            ..
        })
    ));
    assert_eq!(
        AddressRange::new(PhysAddr::new(u64::MAX), 1),
        Err(MapError::InvalidRange)
    );
    assert_eq!(
        AddressRange::new(PhysAddr::new(0), 0),
        Err(MapError::InvalidRange)
    );
    let mut ram = [0; 32];
    assert!(matches!(
        AddressSpace::new(
            Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
            None,
            device()
        ),
        Err(MappingRefused {
            reason: MapError::Overlap,
            ..
        }),
    ));
    let rom = [0; 8];
    assert!(matches!(
        AddressSpace::new(
            Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
            Some(Rom::new(PhysAddr::new(24), rom.as_slice()).unwrap()),
            (),
        ),
        Err(MappingRefused {
            reason: MapError::Overlap,
            ..
        }),
    ));
}

#[test]
fn refused_mounts_return_the_original_models_and_memory() {
    let mut gpio = Gpio::default();
    gpio.set_input(42);
    let misaligned = AddressRange::new(PhysAddr::new(17), 8).unwrap();
    let Err(refused) = MappedDevice::new(misaligned, AccessWidth::Word, gpio) else {
        panic!("misaligned device was accepted");
    };
    assert_eq!(refused.reason, MapError::InvalidRange);
    let range = AddressRange::new(PhysAddr::new(16), 8).unwrap();
    let first =
        MappedDevice::new(range, refused.resources.width, refused.resources.device).unwrap();
    let second = MappedDevice::new(range, AccessWidth::Word, Gpio::default()).unwrap();
    let Err(refused) = DevicePair::new(first, second) else {
        panic!("overlapping pair was accepted");
    };
    assert_eq!(refused.reason, MapError::Overlap);
    let mut high_ram = [0; 32];
    let ram = Ram::new(PhysAddr::new(0), high_ram.as_mut_slice()).unwrap();
    let Err(refused) = AddressSpace::new(ram, None, refused.resources.first) else {
        panic!("overlapping RAM was accepted");
    };
    assert_eq!(refused.reason, MapError::Overlap);
    let mut separate = AddressSpace::new(refused.resources.ram, refused.resources.rom, ()).unwrap();
    separate.write_ram(PhysAddr::new(0), &[77]).unwrap();

    let mut ram = [0; 16];
    let memory = Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap();
    let mut board = AddressSpace::new(memory, None, refused.resources.devices).unwrap();
    let clock = VmClock::new(VmInstant::from_ticks(0), ClockHz::BOCHS_DEFAULT);
    assert_eq!(
        board.read(PhysAddr::new(16), AccessWidth::Word, clock),
        Ok(42)
    );
    assert_eq!(separate.ram()[0], 77);
}

#[test]
fn editing_only_a_device_preserves_the_guest_exclusive_reservation() {
    let mut ram = [0; 4096];
    // ldxr x0, [x1]; stxr w3, x2, [x1]
    for (bytes, word) in ram.chunks_exact_mut(4).zip([0xC85F_7C20u32, 0xC803_7C22]) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    let gpio = MappedDevice::new(
        AddressRange::new(PhysAddr::new(0x5000), 8).unwrap(),
        AccessWidth::Word,
        Gpio::default(),
    )
    .unwrap();
    let board = AddressSpace::new(
        Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
        None,
        gpio,
    )
    .unwrap();
    let cpu = Cpu::<NoEl2>::new(
        CpuModel::CortexA57,
        CpuConfig {
            ips: ClockHz::BOCHS_DEFAULT,
            reset_pc: 0,
        },
    );
    let mut machine = Aarch64Machine::new(cpu, board);
    machine.reg_write(Reg::X1, 256);
    machine.reg_write(Reg::X2, 99);
    assert_eq!(
        machine.step(RunBudget::Instructions(1)).unwrap().stop,
        StopReason::BudgetExhausted
    );
    machine.with_devices_mut(|mut devices| devices.get::<Mounted>().set_input(7));
    assert_eq!(
        machine.step(RunBudget::Instructions(1)).unwrap().stop,
        StopReason::BudgetExhausted
    );
    assert_eq!(
        machine.reg_read(Reg::X3),
        0,
        "the guest store-exclusive succeeded"
    );
    assert_eq!(machine.board().ram()[256], 99);
}

#[test]
fn a_guest_translation_survives_a_device_edit_until_the_host_edits_ram() {
    let mut ram = [0; 0x6000];
    // Arm DDI 0487, EL1 stage-1 walk with a 4 KiB granule and T0SZ=25.
    // The first 2 MiB is identity mapped. VA 0x20_0000 has its own leaf.
    for (address, word) in [
        (0x1000, 0x2003u64),
        (0x2000, 0x701),
        (0x2008, 0x3003),
        (0x3000, 0x4703),
        (0x4000, 41),
        (0x5000, 62),
    ] {
        ram[address..address + 8].copy_from_slice(&word.to_le_bytes());
    }
    for (bytes, word) in ram.chunks_exact_mut(4).zip([
        0xD518_A20Au32, // msr mair_el1, x10
        0xD518_204B,    // msr tcr_el1, x11
        0xD518_200C,    // msr ttbr0_el1, x12
        0xD518_100D,    // msr sctlr_el1, x13
        0xD503_3FDF,    // isb
        0xF940_0024,    // ldr x4, [x1]
        0xF900_0062,    // str x2, [x3]; change the descriptor without TLBI
        0xF940_0025,    // ldr x5, [x1]
        0xF940_0026,    // ldr x6, [x1]
    ]) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    let gpio = MappedDevice::new(
        AddressRange::new(PhysAddr::new(0x6000), 8).unwrap(),
        AccessWidth::Word,
        Gpio::default(),
    )
    .unwrap();
    let board = AddressSpace::new(
        Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
        None,
        gpio,
    )
    .unwrap();
    let cpu = Cpu::<NoEl2>::new(
        CpuModel::CortexA57,
        CpuConfig {
            ips: ClockHz::BOCHS_DEFAULT,
            reset_pc: 0,
        },
    );
    let mut machine = Aarch64Machine::new(cpu, board);
    for (reg, value) in [
        (Reg::X1, 0x20_0000),
        (Reg::X2, 0x5703),
        (Reg::X3, 0x3000),
        (Reg::X10, 0xFF),
        (Reg::X11, 25 | (1 << 23) | (2 << 32)),
        (Reg::X12, 0x1000),
        (Reg::X13, 0x00C5_0839),
    ] {
        machine.reg_write(reg, value);
    }
    assert_eq!(
        machine.step(RunBudget::Instructions(7)).unwrap().stop,
        StopReason::BudgetExhausted
    );
    assert_eq!(machine.reg_read(Reg::X4), 41);
    machine.with_devices_mut(|mut devices| devices.get::<Mounted>().set_input(7));
    assert_eq!(
        machine.step(RunBudget::Instructions(1)).unwrap().stop,
        StopReason::BudgetExhausted
    );
    assert_eq!(
        machine.reg_read(Reg::X5),
        41,
        "the guest retained the cached translation"
    );
    machine
        .with_board_mut(|board| board.write_ram(PhysAddr::new(0x3000), &0x5703u64.to_le_bytes()))
        .unwrap();
    assert_eq!(
        machine.step(RunBudget::Instructions(1)).unwrap().stop,
        StopReason::BudgetExhausted
    );
    assert_eq!(
        machine.reg_read(Reg::X6),
        62,
        "the host edit invalidated the translation"
    );
}

#[test]
fn guest_refetches_modified_ram_and_rom_cannot_be_written() {
    let rate = ClockHz::BOCHS_DEFAULT;
    let mut ram = [0; 4096];
    let rom = [0; 8];
    // mov w0, #1; b back to mov
    ram[..4].copy_from_slice(&0x5280_0020u32.to_le_bytes());
    ram[4..8].copy_from_slice(&0x17FF_FFFFu32.to_le_bytes());
    let mem = Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap();
    let rom = Rom::new(PhysAddr::new(4096), rom.as_slice()).unwrap();
    let board = AddressSpace::new(mem, Some(rom), ()).unwrap();
    let cpu = Cpu::<NoEl2>::new(
        CpuModel::CortexA57,
        CpuConfig {
            ips: rate,
            reset_pc: 0,
        },
    );
    let mut machine = Aarch64Machine::new(cpu, board);
    assert_eq!(
        machine
            .step(RunBudget::Instructions(8))
            .unwrap()
            .progress
            .instructions(),
        Some(8)
    );
    assert_eq!(machine.reg_read(Reg::X0), 1);
    let clock = machine.clock();
    machine.with_board_mut(|board| {
        assert_eq!(
            board.write(PhysAddr::new(4096), AccessWidth::Word, 1, clock),
            Err(BusFault::Unmapped)
        );
        board
            .write_ram(PhysAddr::new(0), &0x5280_0040u32.to_le_bytes())
            .unwrap();
    });
    assert_eq!(
        machine
            .step(RunBudget::Instructions(1))
            .unwrap()
            .progress
            .instructions(),
        Some(1)
    );
    assert_eq!(machine.reg_read(Reg::X0), 2);
}

#[test]
fn descriptor_access_is_atomic_and_does_not_touch_device_registers() {
    let mut ram = [0; 16];
    let rom = [0; 16];
    let device = MappedDevice::new(
        AddressRange::new(PhysAddr::new(32), 8).unwrap(),
        AccessWidth::Word,
        Gpio::default(),
    )
    .unwrap();
    let mut board = AddressSpace::new(
        Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
        Some(Rom::new(PhysAddr::new(16), rom.as_slice()).unwrap()),
        device,
    )
    .unwrap();
    let clock = VmClock::new(VmInstant::from_ticks(0), ClockHz::BOCHS_DEFAULT);
    assert_eq!(
        board.swap_descriptor(PhysAddr::new(0), 0, 0x703, clock),
        Ok(DescriptorSwap::Swapped)
    );
    assert_eq!(
        board.swap_descriptor(PhysAddr::new(0), 0, 0x803, clock),
        Ok(DescriptorSwap::Changed)
    );
    assert_eq!(board.read_descriptor(PhysAddr::new(0), clock), Ok(0x703));
    assert_eq!(
        board.swap_descriptor(PhysAddr::new(16), 0, 1, clock),
        Err(BusFault::Unmapped)
    );
    assert_eq!(
        board.read_descriptor(PhysAddr::new(32), clock),
        Err(BusFault::Unmapped)
    );
    assert_eq!(
        board.fetch(PhysAddr::new(32), clock),
        Err(BusFault::Unmapped)
    );
    assert_eq!(
        board.read(PhysAddr::new(32), AccessWidth::Half, clock),
        Err(BusFault::Unmapped)
    );
    assert_eq!(board.devices().device().output(), 0);
}

struct DmaBoard<'a> {
    bus: AddressSpace<'a, ()>,
    deadline: Option<VmInstant>,
    fulfill: bool,
}

impl PhysBus for DmaBoard<'_> {
    fn read(
        &mut self,
        address: PhysAddr,
        width: AccessWidth,
        clock: VmClock,
    ) -> Result<u64, BusFault> {
        self.bus.read(address, width, clock)
    }

    fn write(
        &mut self,
        address: PhysAddr,
        width: AccessWidth,
        value: u64,
        clock: VmClock,
    ) -> Result<(), BusFault> {
        self.bus.write(address, width, value, clock)
    }

    fn fetch_source(&self, address: PhysAddr) -> FetchSource {
        self.bus.fetch_source(address)
    }

    fn devices_touched(&self) -> bool {
        self.bus.devices_touched()
    }
}

impl MachineBoard for DmaBoard<'_> {
    fn synchronize(&mut self, clock: VmClock, outputs: ProcessorOutputs) -> BoardInputs {
        let mut inputs = self.bus.synchronize(clock, outputs);
        if self.fulfill && self.deadline.is_some_and(|at| at <= clock.now()) {
            self.bus
                .write_ram(PhysAddr::new(0), &0x5280_0040u32.to_le_bytes())
                .unwrap();
            self.deadline = None;
            inputs.memory_changed = true;
        }
        inputs.deadline = self.deadline;
        inputs
    }

    fn reset(&mut self, clock: VmClock) {
        self.bus.reset(clock);
        self.deadline = None;
    }
}

#[test]
fn a_board_deadline_can_patch_executable_ram_and_unsettled_deadlines_are_errors() {
    let mut ram = [0; 4096];
    ram[..4].copy_from_slice(&0x5280_0020u32.to_le_bytes()); // mov w0,#1
    ram[4..8].copy_from_slice(&0x17FF_FFFFu32.to_le_bytes()); // b back to mov
    let board = DmaBoard {
        bus: AddressSpace::new(
            Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
            None,
            (),
        )
        .unwrap(),
        deadline: Some(VmInstant::from_ticks(4)),
        fulfill: true,
    };
    let cpu = Cpu::<NoEl2>::new(
        CpuModel::CortexA57,
        CpuConfig {
            ips: ClockHz::BOCHS_DEFAULT,
            reset_pc: 0,
        },
    );
    let mut machine = Aarch64Machine::new(cpu, board);
    assert_eq!(
        machine
            .step(RunBudget::Ticks(4))
            .unwrap()
            .progress
            .instructions(),
        Some(4)
    );
    assert_eq!(machine.reg_read(Reg::X0), 1);
    assert_eq!(
        machine
            .step(RunBudget::Ticks(2))
            .unwrap()
            .progress
            .instructions(),
        Some(2)
    );
    assert_eq!(
        machine.reg_read(Reg::X0),
        2,
        "guest refetched the device-written opcode"
    );

    let now = machine.clock().now();
    machine.with_board_mut(|board| {
        board.fulfill = false;
        board.deadline = Some(now);
    });
    assert_eq!(
        machine.step(RunBudget::Ticks(1)),
        Err(InvalidDeadline { now, reported: now })
    );
    assert_eq!(machine.clock().now(), now);
}
