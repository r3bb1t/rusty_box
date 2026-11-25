#![no_std]

use core::iter::Empty;

use rusty_box_aarch64::board::address_space::{
    AddressRange, AddressSpace, ClockedDevice, DevicePair, MapError, MappedDevice, Ram, Rom,
};
use rusty_box_aarch64::{Aarch64Machine, AccessWidth, Cpu, CpuConfig, CpuModel, NoEl2, PhysAddr};
use rusty_box_core::{ClockHz, VmClock, VmDuration, VmInstant};
use rusty_box_devices::api::WindowOffset;
use rusty_box_devices::composition::{InterruptOutput, IrqMmioModel, MmioWrite};

const ENTRY: u64 = 0x1000;
const VECTOR: usize = 0x1280;
const SENSOR: u64 = 0x4000;
const GPIO: u64 = 0x5000;
const SAMPLE_DELAY: VmDuration = VmDuration::from_ticks(4);

// EL1 firmware. Arm DDI 0487 provides MOVZ, MSR, WFI, LDR/STR and ERET.
const START: &[u32] = &[
    0xD284_0000, // mov x0, #0x2000
    0xD518_C000, // msr vbar_el1, x0
    0xD288_000A, // mov x10, #0x4000
    0xD28A_000B, // mov x11, #0x5000
    0x5280_0AA0, // mov w0, #85
    0xB900_0540, // str w0, [x10, #4]
    0xD503_42FF, // msr daifclr, #2
    0xD503_207F, // wait: wfi
    0x17FF_FFFF, // b wait
];

const SENSOR_IRQ: &[u32] = &[
    0xB940_0941, // ldr w1, [x10, #8]
    0xB900_095F, // str wzr, [x10, #8]
    0xB940_0140, // ldr w0, [x10]
    0xB940_0162, // ldr w2, [x11]
    0x0B02_0000, // add w0, w0, w2
    0xB900_0560, // str w0, [x11, #4]
    0xD69F_03E0, // eret
];

pub const ROM_LEN: usize = 0x2000;
pub const RAM_LEN: usize = 0x1000;

pub fn firmware() -> [u8; ROM_LEN] {
    let mut bytes = [0; ROM_LEN];
    place(&mut bytes[..], START);
    place(&mut bytes[VECTOR..], SENSOR_IRQ);
    bytes
}

fn place(bytes: &mut [u8], words: &[u32]) {
    for (dest, word) in bytes.chunks_exact_mut(4).zip(words) {
        dest.copy_from_slice(&word.to_le_bytes());
    }
}

pub struct Temperature {
    degrees: u32,
    threshold: u32,
    alarm: bool,
    deadline: Option<VmInstant>,
}

impl Default for Temperature {
    fn default() -> Self {
        Self {
            degrees: 20,
            threshold: 85,
            alarm: false,
            deadline: None,
        }
    }
}

impl Temperature {
    pub fn set(&mut self, degrees: u32, clock: VmClock) {
        if self.degrees < self.threshold && degrees >= self.threshold {
            self.deadline = Some(clock.after(SAMPLE_DELAY));
        } else if degrees < self.threshold {
            self.deadline = None;
        }
        self.degrees = degrees;
    }
}

impl IrqMmioModel for Temperature {
    type Events = Empty<()>;

    fn read(&mut self, offset: WindowOffset, _: usize) -> u64 {
        match offset.0 {
            0 => self.degrees.into(),
            4 => self.threshold.into(),
            8 => self.alarm.into(),
            _ => 0,
        }
    }

    fn write(&mut self, access: MmioWrite, irq: &mut impl InterruptOutput) -> Self::Events {
        match access.offset.0 {
            4 => self.threshold = access.value as u32,
            8 => {
                self.alarm = false;
                irq.set_level(false);
            }
            _ => {}
        }
        core::iter::empty()
    }
}

impl ClockedDevice for Temperature {
    fn advance<I: InterruptOutput>(&mut self, clock: VmClock, irq: &mut I) -> Option<VmInstant> {
        if self
            .deadline
            .is_some_and(|deadline| deadline <= clock.now())
        {
            self.deadline = None;
            self.alarm = self.degrees >= self.threshold;
        }
        irq.set_level(self.alarm);
        self.deadline
    }

    fn reset(&mut self, _: VmClock) {
        *self = Self::default();
    }
}

#[derive(Default)]
pub struct Gpio {
    input: u32,
    output: u32,
}

impl Gpio {
    pub fn set_input(&mut self, value: u32) {
        self.input = value;
    }

    pub fn output(&self) -> u32 {
        self.output
    }
}

impl IrqMmioModel for Gpio {
    type Events = Empty<()>;

    fn read(&mut self, offset: WindowOffset, _: usize) -> u64 {
        match offset.0 {
            0 => self.input.into(),
            4 => self.output.into(),
            _ => 0,
        }
    }

    fn write(&mut self, access: MmioWrite, _: &mut impl InterruptOutput) -> Self::Events {
        if access.offset.0 == 4 {
            self.output = access.value as u32;
        }
        core::iter::empty()
    }
}

impl ClockedDevice for Gpio {
    fn reset(&mut self, _: VmClock) {
        *self = Self::default();
    }
}

pub type Board<'mem> =
    AddressSpace<'mem, DevicePair<MappedDevice<Temperature>, MappedDevice<Gpio>>>;
pub type Controller<'mem> = Aarch64Machine<Board<'mem>, NoEl2>;

pub fn controller<'mem>(
    ram: &'mem mut [u8; RAM_LEN],
    rom: &'mem [u8; ROM_LEN],
    rate: ClockHz,
) -> Result<Controller<'mem>, MapError> {
    let sensor = MappedDevice::new(
        AddressRange::new(PhysAddr::new(SENSOR), 12)?,
        AccessWidth::Word,
        Temperature::default(),
    )?;
    let gpio = MappedDevice::new(
        AddressRange::new(PhysAddr::new(GPIO), 8)?,
        AccessWidth::Word,
        Gpio::default(),
    )?;
    let board = AddressSpace::new(
        Ram::new(PhysAddr::new(0), ram.as_mut_slice())?,
        Some(Rom::new(PhysAddr::new(ENTRY), rom.as_slice())?),
        DevicePair::new(sensor, gpio)?,
    )?;
    let cpu = Cpu::new(
        CpuModel::CortexA57,
        CpuConfig {
            ips: rate,
            reset_pc: ENTRY,
        },
    );
    Ok(Aarch64Machine::new(cpu, board))
}

pub fn set_inputs(machine: &mut Controller<'_>, degrees: u32, input: u32) {
    let clock = machine.clock();
    machine.with_board_mut(|board| {
        board.devices_mut().first().device().set(degrees, clock);
        board.devices_mut().second().device().set_input(input);
    });
}
