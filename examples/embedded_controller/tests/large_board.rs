use core::cell::Cell;

use rusty_box_aarch64::board::address_space::{
    AddressRange, AddressSpace, ClockedDevice, DevicePair, First, MappedDevice, Mounted, Ram, Rom,
    Second,
};
use rusty_box_aarch64::{
    Aarch64Machine, AccessWidth, ClockHz, Cpu, CpuConfig, CpuModel, NoEl2, PhysAddr, Reg,
    RunBudget, StopReason,
};
use rusty_box_core::events::App;
use rusty_box_embedded_controller::{
    DigitalIo, Gpio, OutputChanged, RAM_LEN, Temperature, firmware,
};

type GpioPair = DevicePair<MappedDevice<Gpio>, MappedDevice<Gpio>>;
type GpioBank = DevicePair<GpioPair, GpioPair>;

type FirstInput = First<First<First<First<Mounted>>>>;
type LastInput = First<Second<Second<Second<Second<Mounted>>>>>;
type Sensor = Second<First<Mounted>>;

fn register<D: ClockedDevice>(base: u64, bytes: u64, model: D) -> MappedDevice<D> {
    let range = AddressRange::new(PhysAddr::new(base), bytes).unwrap();
    MappedDevice::new(range, AccessWidth::Word, model).unwrap()
}

fn gpio_bank(base: u64) -> GpioBank {
    let gpio = |offset| register(base + offset, 8, Gpio::default());
    let first = DevicePair::new(gpio(0), gpio(8)).unwrap();
    let second = DevicePair::new(gpio(16), gpio(24)).unwrap();
    DevicePair::new(first, second).unwrap()
}

#[test]
fn fifteen_peripherals_keep_independent_models_and_deliver_guest_events() {
    let low = gpio_bank(0x8000);
    let middle = gpio_bank(0x9000);
    let high = gpio_bank(0xA000);
    let auxiliary = DevicePair::new(low, DevicePair::new(middle, high).unwrap()).unwrap();

    let output_written = Cell::new(None);
    let events = App::new()
        .route(|event: &OutputChanged| output_written.set(Some(event.0)))
        .with_state(());
    let temperature = register(0x4000, 12, Temperature::default());
    let gpio = register(0x5000, 8, Gpio::default()).with_events(events);
    let wire = register(0x6000, 16, DigitalIo::default());
    let control = DevicePair::new(temperature, DevicePair::new(gpio, wire).unwrap()).unwrap();
    let devices = DevicePair::new(auxiliary, control).unwrap();

    let mut ram = [0; RAM_LEN];
    let mut rom = firmware();
    // Read the two named GPIO inputs before entering the normal IRQ firmware.
    // ldr w7,[x20]; ldr w8,[x21]; b 0x1000 (from 0x1808)
    for (bytes, word) in
        rom[0x800..]
            .chunks_exact_mut(4)
            .zip([0xB940_0287u32, 0xB940_02A8, 0x17FF_FDFE])
    {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    let board = AddressSpace::new(
        Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
        Some(Rom::new(PhysAddr::new(0x1000), rom.as_slice()).unwrap()),
        devices,
    )
    .unwrap();
    let cpu = Cpu::<NoEl2>::new(
        CpuModel::CortexA57,
        CpuConfig {
            ips: ClockHz::BOCHS_DEFAULT,
            reset_pc: 0x1800,
        },
    );
    let mut machine = Aarch64Machine::new(cpu, board);
    machine.reg_write(Reg::X20, 0x8000);
    machine.reg_write(Reg::X21, 0xA018);
    machine.with_devices_mut(|mut devices| {
        devices.get::<FirstInput>().set_input(17);
        devices.get::<LastInput>().set_input(61);
    });
    assert_eq!(
        machine.step(RunBudget::Ticks(32)).unwrap().stop,
        StopReason::Halted
    );
    assert_eq!(machine.reg_read(Reg::X7), 17);
    assert_eq!(machine.reg_read(Reg::X8), 61);
    let clock = machine.clock();
    machine.with_devices_mut(|mut devices| devices.get::<Sensor>().set(90, clock));
    assert_eq!(
        machine.step(RunBudget::Ticks(32)).unwrap().stop,
        StopReason::Halted
    );
    assert_eq!(output_written.get(), Some(90));
}
