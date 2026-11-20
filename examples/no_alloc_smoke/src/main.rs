#![no_std]
#![no_main]

use core::mem::MaybeUninit;
use core::panic::PanicInfo;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::BxCpuC;
use rusty_box::cpu::memory_session::{EntryMode, MemorySession, SessionRun};
use rusty_box::cpu::registers::AX;
use rusty_box::emulator::hardware::{
    EventEndpoint, InterruptOutput, IrqMmioModel, IrqWindow, IsaIrq, MmioBusAttachRefusal,
    MmioWindow, MmioWrite, PcAttachment, PcDevice, PcDeviceAttachError, RegisterWindow,
    SubscriberError, Subscriptions, WindowOffset, WindowPair,
};
use rusty_box::memory::mapped::{MappedMemory, PhysicalRegion, RegionKind};
use rusty_box::memory::mmio::{MmioAttachError, MmioDevice};

fn consume<T>(_value: T) {}

// This target checks that both the public map and its execution entry point
// compile with allocation disabled. The host selects and places the CPU.
fn execute(cpu: &mut BxCpuC) -> rusty_box::Result<SessionRun> {
    let mut bytes = [0; 4096];
    let mut stamps = [0; 1];
    let regions = [PhysicalRegion {
        guest_start: BxPhyAddress::new(0x1000),
        arena_offset: 0,
        len: bytes.len(),
        kind: RegionKind::Ram,
    }];
    let mut memory = MappedMemory::new(&mut bytes, &mut stamps, &regions).unwrap();
    let mut session = MemorySession::new(cpu, memory.loan());
    session.reset_at(EntryMode::Real16, 0x1000, 0x1FF0)?;
    session.regs().write(AX, 0x42);
    session
        .memory()
        .write_phys(BxPhyAddress::new(0x1000), &[0x90])?;
    let mut code = [0];
    session
        .memory()
        .read_phys(BxPhyAddress::new(0x1000), &mut code)?;
    Ok(session.step())
}

fn attach(
    machine: &mut rusty_box::Emulator,
    device: &'static mut dyn MmioDevice,
) -> Result<(), MmioAttachError> {
    machine.attach_mmio_device(BxPhyAddress::new(0x90000), device)
}

struct Pulse;

impl IrqMmioModel for Pulse {
    type Events = Option<()>;

    fn read(&mut self, _: WindowOffset, _: usize) -> u64 {
        0
    }

    fn write(&mut self, _: MmioWrite, irq: &mut impl InterruptOutput) -> Self::Events {
        irq.set_level(true);
        Some(())
    }
}

fn attach_composed(
    machine: &mut rusty_box::Emulator,
    storage: &'static mut MaybeUninit<EventEndpoint<Pulse>>,
) -> Result<(), PcDeviceAttachError> {
    let (page, line, endpoint) =
        PcAttachment::with_model(BxPhyAddress::new(0x91000), IsaIrq::new(5).unwrap(), Pulse)
            .into_parts();
    machine.attach_composed_mmio(page, line, PcDevice::Borrowed(storage.write(endpoint)))
}

fn read_byte(state: &mut u8, _: WindowOffset, _: usize) -> u64 {
    u64::from(*state)
}

fn write_byte(state: &mut u8, access: MmioWrite) -> Option<()> {
    *state = access.value as u8;
    None
}

type Register = RegisterWindow<
    u8,
    fn(&mut u8, WindowOffset, usize) -> u64,
    fn(&mut u8, MmioWrite) -> Option<()>,
    Subscriptions<(), SubscriberError>,
>;
type Bus = MmioWindow<WindowPair<Register, IrqWindow<EventEndpoint<Pulse>>>>;

fn attach_bus(
    machine: &mut rusty_box::Emulator,
    storage: &'static mut MaybeUninit<Bus>,
) -> Result<(), MmioBusAttachRefusal> {
    let register: MmioWindow<Register> = MmioWindow::registers(0, read_byte, write_byte);
    let irq = MmioWindow::with_model(IsaIrq::new(5).unwrap(), Pulse);
    let bus = register.then(irq).unwrap();
    storage
        .write(bus)
        .install_borrowed(machine, BxPhyAddress::new(0x90000))
        .map_err(|error| error.reason)
}

fn publish(input: u8, state: &mut u8) {
    use rusty_box::events::{Emit, ExecutionRequest, Subscriptions};
    let mut subscriptions = Subscriptions::<u8, core::convert::Infallible>::new().on_with(
        |data: &u8, state: &mut u8| {
            *state = state.wrapping_add(*data);
            Ok(ExecutionRequest::None)
        },
    );
    let _ = subscriptions.emit(&input, state).unwrap();
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    consume(rusty_box::EmulatorConfig::default());
    let mut bytes = [0; 4097];
    let mut stamps = [0; 2];
    let mut ram = rusty_box::memory::flat::FlatMemory::new(&mut bytes, &mut stamps).unwrap();
    ram.write(rusty_box::config::BxPhyAddress::new(4096), &[0x90])
        .unwrap();
    consume(ram);
    consume(core::hint::black_box(execute as fn(&mut BxCpuC) -> _));
    consume(core::hint::black_box(
        attach as fn(&mut rusty_box::Emulator, &'static mut dyn MmioDevice) -> _,
    ));
    consume(core::hint::black_box(
        attach_composed
            as fn(&mut rusty_box::Emulator, &'static mut MaybeUninit<EventEndpoint<Pulse>>) -> _,
    ));
    consume(core::hint::black_box(
        attach_bus as fn(&mut rusty_box::Emulator, &'static mut MaybeUninit<Bus>) -> _,
    ));
    consume(core::hint::black_box(publish as fn(u8, &mut u8)));
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
