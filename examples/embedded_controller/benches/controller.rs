use std::time::Instant;

use rusty_box_aarch64::board::address_space::{
    AddressRange, AddressSpace, DeviceMap, DevicePair, MappedDevice, Ram,
};
use rusty_box_aarch64::{
    Aarch64Machine, AccessWidth, ClockHz, Cpu, CpuConfig, CpuModel, NoEl2, PhysAddr, Reg,
    RunBudget, StopReason,
};
use rusty_box_embedded_controller::{Gpio, RAM_LEN, controller, firmware, output, set_inputs};

const WARMUP: u64 = 40_000;
const WORK: u64 = 4_000_000;
const IDLE_SLICES: u64 = 250_000;
const IRQ_SAMPLES: u64 = 2_000;
const ROUNDS: usize = 7;

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

fn three() -> DevicePair<MappedDevice<Gpio>, DevicePair<MappedDevice<Gpio>, MappedDevice<Gpio>>> {
    DevicePair::new(
        gpio(0x3000),
        DevicePair::new(gpio(0x4000), gpio(0x5000)).unwrap(),
    )
    .unwrap()
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

fn ram_with_devices() -> f64 {
    access(three(), 256)
}

fn one_device() -> f64 {
    access(gpio(0x5000), 0x5004)
}

fn three_devices() -> f64 {
    access(three(), 0x5004)
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
    let cases: [(&str, fn() -> f64); 6] = [
        ("RAM only (ns/instruction)", ram_alone),
        (
            "RAM, three devices mounted (ns/instruction)",
            ram_with_devices,
        ),
        ("MMIO, one device (ns/instruction)", one_device),
        (
            "MMIO, third of three devices (ns/instruction)",
            three_devices,
        ),
        ("idle, no devices (ns/32-tick batch)", idle),
        ("timed wake + guest IRQ (ns/32-tick batch)", timed_irq),
    ];
    let mut results = [[0.0; ROUNDS]; 6];
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
