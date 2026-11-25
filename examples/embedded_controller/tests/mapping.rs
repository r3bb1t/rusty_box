use rusty_box_aarch64::board::address_space::{
    AddressRange, AddressSpace, DevicePair, MapError, MappedDevice, Ram, Rom,
};
use rusty_box_aarch64::bus::{DescriptorSwap, FetchSource};
use rusty_box_aarch64::machine::custom::{
    BoardInputs, InvalidDeadline, MachineBoard, ProcessorOutputs,
};
use rusty_box_aarch64::{
    Aarch64Machine, AccessWidth, BusFault, ClockHz, Cpu, CpuConfig, CpuModel, NoEl2, PhysAddr,
    PhysBus, Reg, RunBudget,
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
    let written = machine.step(RunBudget::Instructions(3)).unwrap();
    assert_eq!(written.stop, rusty_box_aarch64::StopReason::StopRequested);
    assert_eq!(written.progress.instructions(), Some(2));
    assert_eq!(machine.board().devices().device().output(), 42);
    assert_eq!(machine.reg_read(Reg::X2), 0);
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
        Err(MapError::Overlap)
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
        Err(MapError::Overlap),
    ));
    let rom = [0; 8];
    assert!(matches!(
        AddressSpace::new(
            Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
            Some(Rom::new(PhysAddr::new(24), rom.as_slice()).unwrap()),
            (),
        ),
        Err(MapError::Overlap),
    ));
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
