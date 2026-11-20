#![cfg(feature = "std")]

#[path = "../examples/composed_hardware/gpu.rs"]
mod gpu;
#[path = "../examples/hardware_topology/register_bank.rs"]
mod register_bank;

use std::num::NonZeroU16;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpu::{Gpu, GpuEvent};
use register_bank::{RegisterBank, RegisterChanged};
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::instrumentation::EmuStopReason;
use rusty_box::cpu::registers::{EAX, RIP};
use rusty_box::cpu::{CpuError, CpuSetupMode};
use rusty_box::emulator::MachineBuilder;
use rusty_box::emulator::hardware::{
    EventEndpoint, IsaIrq, MmioBusAttachRefusal, MmioSubscriberError, MmioTopologyError,
    MmioWindow, PcDeviceAttachRefusal, PcMmioDevice, Reaction, SubscriberError, Subscriptions,
    register_window,
};
use rusty_box::memory::mmio::MmioAttachRefusal;
use rusty_box::{Emulator, EmulatorConfig, Error};

const BASE: u32 = 0x90000;
const FIRST: u32 = BASE + 0x2000;
const SECOND: u32 = BASE + 0x3000;
const CODE: u32 = 0x10000;
const DMA_CODE: u32 = 0x18000;
const DMA_DATA: u32 = 0x19000;

fn phy(address: u32) -> BxPhyAddress {
    BxPhyAddress::new(address.into())
}

fn pc() -> Box<Emulator> {
    Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32).unwrap()
}

fn board() -> Box<Emulator> {
    let mut pc = MachineBuilder::new(EmulatorConfig::default())
        .build()
        .unwrap();
    pc.setup_cpu_mode(CpuSetupMode::FlatProtected32).unwrap();
    pc
}

fn store(code: &mut Vec<u8>, address: u32, value: u32) {
    code.extend_from_slice(&[0xC7, 0x05]);
    code.extend_from_slice(&address.to_le_bytes());
    code.extend_from_slice(&value.to_le_bytes());
}

fn start(pc: &mut Emulator, code: &[u8], count: u64) -> EmuStopReason {
    pc.mem_write(phy(CODE), code).unwrap();
    pc.emu_start(u64::from(CODE), None, None, Some(count))
        .unwrap()
}

fn read(pc: &mut Emulator, address: u32) -> u32 {
    let mut code = [0xA1; 5]; // mov eax,[address]
    code[1..].copy_from_slice(&address.to_le_bytes());
    start(pc, &code, 1);
    pc.regs().read(EAX)
}

fn irr(pc: &mut Emulator, port: u8) -> u8 {
    start(pc, &[0xB0, 0x0A, 0xE6, port, 0xE4, port], 3);
    pc.regs().read(EAX) as u8
}

fn gpu_window(
    line: u8,
    destination: u32,
    stop: bool,
    deliveries: Arc<AtomicUsize>,
) -> MmioWindow<impl PcMmioDevice> {
    MmioWindow::with_irq(
        NonZeroU16::MIN,
        IsaIrq::new(line).unwrap(),
        EventEndpoint::with_subscriptions(
            Gpu::default(),
            Subscriptions::<GpuEvent, SubscriberError>::new().try_on(move |event| {
                if let Some(event) = event.written() {
                    assert_eq!(event.destination, phy(destination));
                    deliveries.fetch_add(1, Ordering::Relaxed);
                    return Ok(if stop {
                        Reaction::Stop
                    } else {
                        Reaction::Continue
                    });
                }
                assert_eq!(event.rejected().unwrap().destination, phy(SECOND));
                Ok(Reaction::Continue)
            }),
        ),
    )
}

#[test]
fn a_nested_bus_routes_dma_irqs_stops_and_keeps_instance_state() {
    let observed = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&observed);
    let registers = register_window(
        NonZeroU16::new(2).unwrap(),
        RegisterBank::new(7),
        RegisterBank::read,
        RegisterBank::write,
        Subscriptions::<RegisterChanged, SubscriberError>::new().on(move |event| {
            assert_eq!(event.offset.get(), 4096);
            assert_eq!(event.value, 42);
            count.fetch_add(1, Ordering::Relaxed);
        }),
    );
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let branch = gpu_window(5, DMA_CODE + 1, false, Arc::clone(&first))
        .then(gpu_window(10, DMA_DATA, true, Arc::clone(&second)))
        .unwrap();
    let root = registers.then(branch).unwrap();
    let mut pc = board();
    assert_eq!(irr(&mut pc, 0x20) & (1 << 5), 0);
    assert_eq!(irr(&mut pc, 0xA0) & (1 << 2), 0);
    let original = 0x7654_3210u32;
    pc.mem_write(phy(BASE + 4096), &original.to_le_bytes())
        .unwrap();
    assert_eq!(read(&mut pc, BASE + 4096), original);
    pc.mem_write(phy(DMA_CODE), &[0xB8, 1, 0, 0, 0]).unwrap();
    pc.emu_start(DMA_CODE.into(), None, None, Some(1)).unwrap();
    assert_eq!(pc.regs().read(EAX), 1);

    root.install(&mut pc, phy(BASE)).unwrap();
    assert_eq!(read(&mut pc, BASE + 4096), 7);
    let mut code = Vec::new();
    store(&mut code, BASE + 4096, 42);
    store(&mut code, FIRST, DMA_CODE + 1);
    store(&mut code, FIRST + 4, 0x4433_2211);
    store(&mut code, SECOND, DMA_DATA);
    store(&mut code, SECOND + 4, 0x8877_6655);
    code.push(0x40); // inc eax
    assert!(matches!(start(&mut pc, &code, 6), EmuStopReason::Stopped));
    let resume = u64::from(CODE) + 50;
    assert_eq!(pc.regs().read(RIP), resume);
    assert_eq!(observed.load(Ordering::Relaxed), 1);
    assert_eq!(first.load(Ordering::Relaxed), 1);
    assert_eq!(second.load(Ordering::Relaxed), 1);
    pc.regs().write(EAX, 11);
    pc.emu_start(resume, None, None, Some(1)).unwrap();
    assert_eq!(pc.regs().read(EAX), 12);
    assert_eq!(second.load(Ordering::Relaxed), 1);
    pc.emu_start(DMA_CODE.into(), None, None, Some(1)).unwrap();
    assert_eq!(pc.regs().read(EAX), 0x4433_2211);
    assert_eq!(read(&mut pc, DMA_DATA), 0x8877_6655);

    assert_ne!(irr(&mut pc, 0x20) & (1 << 5), 0);
    assert_ne!(irr(&mut pc, 0xA0) & (1 << 2), 0);
    code.clear();
    store(&mut code, FIRST + 12, 0);
    start(&mut pc, &code, 1);
    assert_eq!(irr(&mut pc, 0x20) & (1 << 5), 0);
    assert_ne!(irr(&mut pc, 0xA0) & (1 << 2), 0);

    code.clear();
    store(&mut code, FIRST, SECOND);
    store(&mut code, FIRST + 4, 0xDEAD_BEEF);
    start(&mut pc, &code, 2);
    assert_eq!(read(&mut pc, FIRST + 8), 2);
    assert_eq!(read(&mut pc, SECOND), DMA_DATA);
    assert_eq!(second.load(Ordering::Relaxed), 1);

    let tree = pc.detach_mmio_bus(phy(BASE)).unwrap();
    assert_eq!(irr(&mut pc, 0xA0) & (1 << 2), 0);
    assert_eq!(read(&mut pc, BASE + 4096), original);
    let moved = BASE + 0x4000;
    tree.install(&mut pc, phy(moved)).unwrap();
    assert_eq!(read(&mut pc, moved + 4096), 42);
    assert_eq!(read(&mut pc, moved + 0x2000 + 8), 2);
    assert_eq!(read(&mut pc, moved + 0x3000), DMA_DATA);
}

fn plain(initial: u32) -> MmioWindow<impl PcMmioDevice> {
    register_window(
        NonZeroU16::MIN,
        RegisterBank::new(initial),
        RegisterBank::read,
        RegisterBank::write,
        Subscriptions::<RegisterChanged, SubscriberError>::new(),
    )
}

#[test]
fn cross_window_guest_accesses_route_both_bytes_in_order() {
    let mut pc = pc();
    let deliveries = Arc::new(AtomicUsize::new(0));
    let bank = |left| {
        let deliveries = Arc::clone(&deliveries);
        register_window(
            NonZeroU16::MIN,
            RegisterBank::new(0),
            RegisterBank::read,
            RegisterBank::write,
            Subscriptions::<RegisterChanged, SubscriberError>::new().try_on(move |event| {
                let previous = deliveries.fetch_add(1, Ordering::Relaxed);
                if left {
                    assert_eq!((previous, event.offset.get(), event.value), (0, 4095, 0x34));
                    return Ok(Reaction::Stop);
                }
                assert_eq!((previous, event.offset.get(), event.value), (1, 0, 0x12));
                Ok(Reaction::Continue)
            }),
        )
    };
    bank(true)
        .then(bank(false))
        .unwrap()
        .install(&mut pc, phy(BASE))
        .unwrap();
    let mut code = vec![0x66, 0xC7, 0x05]; // mov word [end of left window],0x1234
    code.extend_from_slice(&(BASE + 4095).to_le_bytes());
    code.extend_from_slice(&[0x34, 0x12, 0x90]);
    assert!(matches!(start(&mut pc, &code, 2), EmuStopReason::Stopped));
    assert_eq!(pc.regs().read(RIP), u64::from(CODE) + 9);
    assert_eq!(deliveries.load(Ordering::Relaxed), 2);
    assert_eq!(read(&mut pc, BASE), 0x34);
    assert_eq!(read(&mut pc, BASE + 4096), 0x12);
    let mut load = vec![0x0F, 0xB7, 0x05]; // movzx eax, word [end of left window]
    load.extend_from_slice(&(BASE + 4095).to_le_bytes());
    start(&mut pc, &load, 1);
    assert_eq!(pc.regs().read(EAX), 0x1234);
    let mut snapshot = Vec::new();
    assert_eq!(
        pc.save_snapshot(&mut snapshot).unwrap_err().kind(),
        std::io::ErrorKind::Unsupported
    );
    assert!(
        snapshot.is_empty(),
        "register-only routes also own device state"
    );
}

#[test]
fn topology_preflights_the_whole_aperture_and_irq_set_before_mounting() {
    let mut pc = board();
    let gpu = |line| gpu_window(line, DMA_DATA, false, Arc::new(AtomicUsize::new(0)));
    assert!(matches!(
        gpu(5).then(gpu(5)),
        Err(MmioTopologyError::DuplicateIrq)
    ));
    let tree = plain(1).then(gpu(5).then(gpu(10)).unwrap()).unwrap();
    // The base is clear, but the next page is inside the built-in VGA aperture.
    let error = tree.install(&mut pc, phy(0x9F000)).unwrap_err();
    assert_eq!(
        error.reason,
        MmioBusAttachRefusal::Placement(PcDeviceAttachRefusal::Address(MmioAttachRefusal::Overlap))
    );
    let tree = error.window;
    let error = tree
        .install(&mut pc, BxPhyAddress::new(u64::MAX - 4095))
        .unwrap_err();
    assert_eq!(error.reason, MmioBusAttachRefusal::AddressOverflow);
    let tree = error.window;
    gpu(10).install(&mut pc, phy(SECOND + 4096)).unwrap();
    let error = tree.install(&mut pc, phy(BASE)).unwrap_err();
    assert_eq!(
        error.reason,
        MmioBusAttachRefusal::Placement(PcDeviceAttachRefusal::IrqInUse)
    );
    let tree = error.window;
    // The failed install must not have reserved its first line.
    gpu(5).install(&mut pc, phy(SECOND + 8192)).unwrap();
    assert!(pc.detach_mmio_bus(phy(SECOND + 8192)).is_some());
    assert!(pc.detach_mmio_bus(phy(SECOND + 4096)).is_some());
    tree.install(&mut pc, phy(BASE)).unwrap();
    assert!(pc.detach_composed_mmio(phy(BASE)).is_none());
    assert_eq!(read(&mut pc, BASE), 1);
}

#[test]
fn a_child_subscriber_error_reports_the_physical_address_after_commit() {
    let mut pc = pc();
    let failing = register_window(
        NonZeroU16::MIN,
        RegisterBank::new(0),
        RegisterBank::read,
        RegisterBank::write,
        Subscriptions::<RegisterChanged, SubscriberError>::new()
            .try_on(|_| Err(SubscriberError("register observer failed"))),
    );
    plain(0)
        .then(plain(1).then(failing).unwrap())
        .unwrap()
        .install(&mut pc, phy(BASE))
        .unwrap();
    let target = BASE + 8192;
    let mut code = Vec::new();
    store(&mut code, target, 42);
    code.push(0x90);
    pc.mem_write(phy(CODE), &code).unwrap();
    let Err(Error::Cpu(CpuError::MmioSubscriber(error))) =
        pc.emu_start(CODE.into(), None, None, Some(2))
    else {
        panic!("expected a host subscriber error");
    };
    assert_eq!(
        error,
        MmioSubscriberError {
            address: phy(target),
            source: SubscriberError("register observer failed"),
        }
    );
    let resume = u64::from(CODE) + 10;
    assert_eq!(pc.regs().read(RIP), resume);
    pc.emu_start(resume, None, None, Some(1)).unwrap();
    assert_eq!(read(&mut pc, target), 42);
}
