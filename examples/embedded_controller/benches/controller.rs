use std::time::Instant;

use rusty_box_aarch64::board::address_space::{
    AddressRange, AddressSpace, DeviceMap, DevicePair, MappedDevice, Ram,
};
use rusty_box_aarch64::machine::link::{ConnectedPair, Link, PairStop, Port, Receive, Transmit};
use rusty_box_aarch64::{
    Aarch64Machine, AccessWidth, ClockHz, Cpu, CpuConfig, CpuModel, NoEl2, PhysAddr, Reg,
    RunBudget, StopReason,
};
use rusty_box_core::{VmClock, VmDuration};
use rusty_box_devices::api::WindowOffset;
use rusty_box_devices::composition::{InterruptOutput, IrqMmioModel, MmioWrite};
use rusty_box_embedded_controller::{
    Digital, Gpio, RAM_LEN, controller, firmware, output, set_inputs,
};

const WARMUP: u64 = 40_000;
const WORK: u64 = 4_000_000;
const IDLE_SLICES: u64 = 250_000;
const IRQ_SAMPLES: u64 = 2_000;
const LINK_ROUNDS: u32 = 50_000;
const ROUNDS: usize = 7;
const FIRST_GPIO: u64 = 0x3000;

fn cpu() -> Cpu<NoEl2> {
    Cpu::new(
        CpuModel::CortexA57,
        CpuConfig {
            ips: ClockHz::BOCHS_DEFAULT,
            reset_pc: 0,
        },
    )
}

fn load(ram: &mut [u8], words: &[u32]) {
    for (out, word) in ram.chunks_exact_mut(4).zip(words) {
        out.copy_from_slice(&word.to_le_bytes());
    }
}

fn gpio(base: u64) -> MappedDevice<Gpio> {
    MappedDevice::new(
        AddressRange::new(PhysAddr::new(base), 8).unwrap(),
        AccessWidth::Word,
        Gpio::default(),
    )
    .unwrap()
}

type GpioMount = MappedDevice<Gpio>;
type GpioPair = DevicePair<GpioMount, GpioMount>;
type GpioTriple = DevicePair<GpioMount, GpioPair>;
type GpioQuad = DevicePair<GpioPair, GpioPair>;

fn three() -> GpioTriple {
    DevicePair::new(
        gpio(0x3000),
        DevicePair::new(gpio(0x4000), gpio(0x5000)).unwrap(),
    )
    .unwrap()
}

fn quad(base: u64) -> GpioQuad {
    let first = DevicePair::new(gpio(base), gpio(base + 8)).unwrap();
    let second = DevicePair::new(gpio(base + 16), gpio(base + 24)).unwrap();
    DevicePair::new(first, second).unwrap()
}

type GpioFifteen = DevicePair<GpioTriple, DevicePair<GpioQuad, DevicePair<GpioQuad, GpioQuad>>>;

fn fifteen() -> GpioFifteen {
    let last_eight = DevicePair::new(quad(0x7000), quad(0x8000)).unwrap();
    let twelve = DevicePair::new(quad(0x6000), last_eight).unwrap();
    DevicePair::new(three(), twelve).unwrap()
}

fn access<D: DeviceMap>(devices: D, target: u64) -> f64 {
    let mut ram = [0; RAM_LEN];
    // ldr w0,[x1]; add w0,w0,#1; str w0,[x1]; b back to ldr
    load(
        &mut ram,
        &[0xB940_0020, 0x1100_0400, 0xB900_0020, 0x17FF_FFFD],
    );
    let board = AddressSpace::new(
        Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
        None,
        devices,
    )
    .unwrap();
    let mut machine = Aarch64Machine::new(cpu(), board);
    machine.reg_write(Reg::X1, target);
    assert_eq!(
        machine.step(RunBudget::Instructions(WARMUP)).unwrap().stop,
        StopReason::BudgetExhausted
    );
    let start = Instant::now();
    let outcome = machine.step(RunBudget::Instructions(WORK)).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(outcome.progress.instructions(), Some(WORK));
    assert_eq!(machine.reg_read(Reg::X0), (WARMUP + WORK) / 4);
    elapsed.as_secs_f64() * 1e9 / WORK as f64
}

fn ram_alone() -> f64 {
    access((), 256)
}

fn ram_with_one() -> f64 {
    access(gpio(FIRST_GPIO), 256)
}

fn ram_with_devices() -> f64 {
    access(three(), 256)
}

fn one_device() -> f64 {
    access(gpio(0x5000), 0x5004)
}

fn three_devices() -> f64 {
    access(three(), 0x5004)
}

fn ram_with_fifteen() -> f64 {
    access(fifteen(), 256)
}

fn first_of_fifteen() -> f64 {
    access(fifteen(), FIRST_GPIO + 4)
}

fn last_of_fifteen() -> f64 {
    access(fifteen(), 0x801C)
}

#[derive(Default)]
struct Wire {
    tx: Transmit<Digital>,
    rx: Receive<Digital>,
}

impl IrqMmioModel for Wire {
    type Events = core::iter::Empty<()>;

    fn read(&mut self, _: WindowOffset, _: usize) -> u64 {
        self.rx.take().unwrap_or(0).into()
    }

    fn write(&mut self, write: MmioWrite, _: &mut impl InterruptOutput) -> Self::Events {
        self.tx.send(write.value as u32).unwrap();
        core::iter::empty()
    }
}

impl rusty_box_aarch64::board::address_space::ClockedDevice for Wire {
    fn reset(&mut self, _: VmClock) {
        *self = Self::default();
    }
}

fn link_board(ram: &mut [u8]) -> AddressSpace<'_, MappedDevice<Wire>> {
    let range = AddressRange::new(PhysAddr::new(0x5000), 8).unwrap();
    let wire = MappedDevice::new(range, AccessWidth::Word, Wire::default()).unwrap();
    AddressSpace::new(Ram::new(PhysAddr::new(0), ram).unwrap(), None, wire).unwrap()
}

fn link_case(pending: bool) -> f64 {
    let mut ram_a = [0; RAM_LEN];
    let mut ram_b = [0; RAM_LEN];
    let mut write = 0xD503_201F; // nop
    let mut read = 0xD503_201F;
    if pending {
        write = 0xB900_0020; // str w0,[x1]
        read = 0xB940_0420; // ldr w0,[x1,#4]
    }
    load(&mut ram_a, &[write, 0x17FF_FFFF]);
    load(&mut ram_b, &[read, 0x17FF_FFFF]);
    let mut a = Aarch64Machine::new(cpu(), link_board(&mut ram_a));
    let mut b = Aarch64Machine::new(cpu(), link_board(&mut ram_b));
    a.reg_write(Reg::X0, 42);
    a.reg_write(Reg::X1, 0x5000);
    b.reg_write(Reg::X1, 0x5000);
    let wire: Link<MappedDevice<Wire>, MappedDevice<Wire>, Digital> = Link::new(
        Port::new(|devices| &mut devices.device().tx),
        Port::new(|devices| &mut devices.device().rx),
    );
    let mut pair = ConnectedPair::new(a, b, wire).unwrap();
    let quantum = VmDuration::from_ticks(2);
    assert_eq!(pair.run(16, quantum).unwrap().stop, PairStop::Completed);
    let before = pair.first().instructions();
    let start = Instant::now();
    let outcome = pair.run(LINK_ROUNDS, quantum).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(outcome.stop, PairStop::Completed);
    assert_eq!(outcome.frames, u64::from(pending) * u64::from(LINK_ROUNDS));
    assert_eq!(
        pair.first().instructions() - before,
        2 * u64::from(LINK_ROUNDS)
    );
    assert_eq!(pair.first().instructions(), pair.second().instructions());
    if pending {
        assert_eq!(pair.second().reg_read(Reg::X0), 42);
    }
    elapsed.as_secs_f64() * 1e9 / f64::from(LINK_ROUNDS)
}

fn link_idle() -> f64 {
    link_case(false)
}

fn link_pending() -> f64 {
    link_case(true)
}

fn idle() -> f64 {
    let mut ram = [0; RAM_LEN];
    load(&mut ram, &[0xD503_207F]); // wfi
    let board = AddressSpace::new(
        Ram::new(PhysAddr::new(0), ram.as_mut_slice()).unwrap(),
        None,
        (),
    )
    .unwrap();
    let mut machine = Aarch64Machine::new(cpu(), board);
    assert_eq!(
        machine.step(RunBudget::Ticks(32)).unwrap().stop,
        StopReason::Halted
    );
    let start = Instant::now();
    for _ in 0..IDLE_SLICES {
        assert_eq!(
            machine.step(RunBudget::Ticks(32)).unwrap().stop,
            StopReason::Halted
        );
    }
    let elapsed = start.elapsed();
    assert_eq!(machine.clock().now().ticks(), 32 * (IDLE_SLICES + 1));
    elapsed.as_secs_f64() * 1e9 / IDLE_SLICES as f64
}

fn timed_irq() -> f64 {
    let rom = firmware();
    let mut ram = [0; RAM_LEN];
    let mut machine = controller(&mut ram, &rom, ClockHz::BOCHS_DEFAULT).unwrap();
    let mut nanos = 0;
    for _ in 0..IRQ_SAMPLES {
        machine.reset();
        assert_eq!(
            machine.step(RunBudget::Ticks(32)).unwrap().stop,
            StopReason::Halted
        );
        set_inputs(&mut machine, 90, 5);
        let start = Instant::now();
        let outcome = machine.step(RunBudget::Ticks(32)).unwrap();
        nanos += start.elapsed().as_nanos();
        assert_eq!(outcome.stop, StopReason::Halted);
        assert_eq!(output(&machine), 95);
    }
    nanos as f64 / IRQ_SAMPLES as f64
}

fn main() {
    let cases: [(&str, fn() -> f64); 12] = [
        ("RAM only (ns/instruction)", ram_alone),
        ("RAM, one device mounted (ns/instruction)", ram_with_one),
        (
            "RAM, three devices mounted (ns/instruction)",
            ram_with_devices,
        ),
        ("MMIO, one device (ns/instruction)", one_device),
        (
            "MMIO, third of three devices (ns/instruction)",
            three_devices,
        ),
        (
            "RAM, fifteen devices mounted (ns/instruction)",
            ram_with_fifteen,
        ),
        (
            "MMIO, first of fifteen devices (ns/instruction)",
            first_of_fifteen,
        ),
        (
            "MMIO, last of fifteen devices (ns/instruction)",
            last_of_fifteen,
        ),
        ("idle link (ns/barrier)", link_idle),
        ("pending link (ns/barrier)", link_pending),
        ("idle, no devices (ns/32-tick batch)", idle),
        ("timed wake + guest IRQ (ns/32-tick batch)", timed_irq),
    ];
    if let Ok(case) = std::env::var("RB_CONTROLLER_CASE") {
        let (name, run) = cases[case.parse::<usize>().unwrap()];
        println!("{name}: {:.3}", run());
        return;
    }
    let mut results = [[0.0; ROUNDS]; 12];
    for (_, run) in cases {
        std::hint::black_box(run());
    }
    for round in 0..ROUNDS {
        for offset in 0..cases.len() {
            let index = (round + offset) % cases.len();
            results[index][round] = cases[index].1();
        }
    }
    for ((name, _), samples) in cases.into_iter().zip(results) {
        let mut ordered = samples;
        ordered.sort_unstable_by(f64::total_cmp);
        println!("{name}: {:.3}, raw {samples:?}", ordered[ROUNDS / 2]);
    }
}
