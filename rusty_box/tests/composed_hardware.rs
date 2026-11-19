#![cfg(feature = "std")]

#[path = "../examples/composed_hardware/adapter.rs"]
mod adapter;
#[path = "../examples/composed_hardware/gpu.rs"]
mod gpu;

use adapter::{GpuAttachment, GpuEndpoint};
use gpu::{Gpu, TileRejected, TileWritten};
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuSetupMode;
use rusty_box::cpu::instrumentation::EmuStopReason;
use rusty_box::cpu::registers::{EAX, EBX, ECX, RIP};
use rusty_box::emulator::hardware::{
    DmaWrite, Emit, InterruptOutput, IsaIrq, MmioWrite, ObserveWrites, PcDevice,
    PcDeviceAttachRefusal, PcInterrupt, Reaction, Subscriptions, WindowOffset,
};
use rusty_box::memory::mmio::{MmioAttachRefusal, MmioDevice};
use rusty_box::{Emulator, EmulatorConfig};

const DEVICE: BxPhyAddress = BxPhyAddress::new(0x90000);
const CODE: u64 = 0x10000;
const RAM: u32 = 0x18000;
const TILE: u32 = 0x4433_2211;

fn access(offset: u64, value: u32) -> MmioWrite {
    MmioWrite {
        offset: WindowOffset(offset),
        len: 4,
        value: value.into(),
    }
}

#[derive(Default)]
struct FakeDma {
    writes: Vec<(BxPhyAddress, Vec<u8>)>,
    fail: bool,
}

impl DmaWrite for FakeDma {
    type Error = ();

    fn write(&mut self, address: BxPhyAddress, bytes: &[u8]) -> Result<(), ()> {
        if self.fail {
            return Err(());
        }
        self.writes.push((address, bytes.to_vec()));
        Ok(())
    }
}

#[derive(Default)]
struct FakeIrq {
    level: bool,
    edges: Vec<bool>,
}

impl InterruptOutput for FakeIrq {
    fn set_level(&mut self, asserted: bool) {
        self.level = asserted;
        self.edges.push(asserted);
    }
}

struct Capture<E>(Vec<(E, bool)>);

impl<E: Copy> Emit<E, FakeIrq> for Capture<E> {
    type Error = ();

    fn emit(&mut self, event: &E, irq: &mut FakeIrq) -> Result<Reaction, ()> {
        self.0.push((*event, irq.level));
        Ok(Reaction::Continue)
    }
}

#[test]
fn same_device_runs_with_fake_dma_irq_and_event_sinks() {
    let (mut gpu, mut dma, mut irq) = (Gpu::default(), FakeDma::default(), FakeIrq::default());
    let mut ok = Capture::<TileWritten>(Vec::new());
    let mut errors = Capture::<TileRejected>(Vec::new());
    assert_eq!(
        gpu::write(
            &mut gpu,
            access(0, RAM),
            &mut dma,
            &mut irq,
            &mut ok,
            &mut errors
        ),
        Ok(Reaction::Continue)
    );
    assert_eq!(
        gpu::write(
            &mut gpu,
            access(4, TILE),
            &mut dma,
            &mut irq,
            &mut ok,
            &mut errors
        ),
        Ok(Reaction::Continue)
    );
    assert_eq!(irq.edges, [true]);
    assert_eq!(
        dma.writes,
        [(BxPhyAddress::new(RAM.into()), TILE.to_le_bytes().to_vec())]
    );
    assert_eq!(
        ok.0,
        [(
            TileWritten {
                destination: BxPhyAddress::new(RAM.into()),
                pixels: TILE.to_le_bytes()
            },
            true
        )]
    );

    dma.fail = true;
    assert_eq!(
        gpu::write(
            &mut gpu,
            access(4, 99),
            &mut dma,
            &mut irq,
            &mut ok,
            &mut errors
        ),
        Ok(Reaction::Continue)
    );
    assert_eq!(gpu.read(WindowOffset(8), 4), 2);
    assert_eq!(ok.0.len(), 1);
    assert_eq!(dma.writes.len(), 1);
    assert_eq!(irq.edges, [true]);
    assert_eq!(
        errors.0,
        [(
            TileRejected {
                destination: BxPhyAddress::new(RAM.into())
            },
            true
        )]
    );
}

fn program(pc: &mut Emulator, destination: u32) {
    program_for(pc, DEVICE, destination);
}

fn program_for(pc: &mut Emulator, page: BxPhyAddress, destination: u32) {
    // mov dword [page],destination; mov dword [page+4],tile pattern
    let mut code = vec![
        0xC7, 0x05, 0, 0, 9, 0, 0, 0, 0, 0, 0xC7, 0x05, 4, 0, 9, 0, 0x11, 0x22, 0x33, 0x44,
        // mov ebx,[RAM]; mov al,0x0a; out 0x20,al; in al,0x20; movzx ecx,al
        0x8B, 0x1D, 0, 0x80, 1, 0, 0xB0, 0x0A, 0xE6, 0x20, 0xE4, 0x20, 0x0F, 0xB6, 0xC8,
    ];
    let page = u32::try_from(page.get()).unwrap();
    code[2..6].copy_from_slice(&page.to_le_bytes());
    code[6..10].copy_from_slice(&destination.to_le_bytes());
    code[12..16].copy_from_slice(&(page + 4).to_le_bytes());
    pc.mem_write(BxPhyAddress::new(CODE), &code).unwrap();
}

fn pc() -> Box<Emulator> {
    Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32).unwrap()
}

#[test]
fn x86_dma_completion_stops_after_the_write() {
    let written = Subscriptions::<TileWritten, ()>::new()
        .try_on(|event| {
            assert_eq!(event.pixels, TILE.to_le_bytes());
            Ok(Reaction::Continue)
        })
        .try_on(|event| {
            assert_eq!(event.destination, BxPhyAddress::new(RAM.into()));
            Ok(Reaction::Stop)
        });
    let rejected = Subscriptions::<TileRejected, ()>::new()
        .try_on(|_| panic!("ordinary RAM DMA must succeed"));
    let endpoint = ObserveWrites::new(
        GpuEndpoint {
            gpu: Gpu::default(),
            written,
            rejected,
        },
        |write, reaction| match write.offset.get() {
            0 => assert_eq!(reaction, Reaction::Continue),
            4 => assert_eq!(reaction, Reaction::Stop),
            _ => panic!("unexpected write"),
        },
    );

    let mut pc = pc();
    pc.attach_composed_mmio(DEVICE, IsaIrq::new(5).unwrap(), PcDevice::owned(endpoint))
        .unwrap();
    program(&mut pc, RAM);
    assert!(matches!(
        pc.emu_start(CODE, None, None, Some(9)).unwrap(),
        EmuStopReason::Stopped
    ));
    assert_eq!(pc.regs().read(RIP), CODE + 20);
    assert_eq!(pc.regs().read(EBX), 0);

    pc.emu_start(CODE + 20, None, None, Some(5)).unwrap();
    assert_eq!(pc.regs().read(EBX), TILE);
    assert_ne!(
        pc.regs().read(ECX) & (1 << 5),
        0,
        "guest sees the device's asserted IRQ"
    );
    let mut endpoint = pc.detach_composed_mmio(DEVICE).unwrap();
    assert_eq!(endpoint.endpoint().read(WindowOffset(8), 4), 1);
}

#[test]
fn typed_attachment_subscriptions_are_local_to_their_producer() {
    let mut pc = pc();
    let other = DEVICE + 4096;
    for (page, line, destination) in [(DEVICE, 5, RAM), (other, 10, RAM + 4)] {
        GpuAttachment::new()
            .on(move |event| assert_eq!(event.destination.get(), u64::from(destination)))
            .on_with_irq(|_, _| Reaction::Stop)
            .on_rejected(|_| panic!("DMA must succeed"))
            .at(page, IsaIrq::new(line).unwrap())
            .install(&mut pc)
            .unwrap();
    }
    for (page, destination) in [(DEVICE, RAM), (other, RAM + 4)] {
        program_for(&mut pc, page, destination);
        assert!(matches!(
            pc.emu_start(CODE, None, None, Some(2)).unwrap(),
            EmuStopReason::Stopped
        ));
        let mut bytes = [0; 4];
        pc.mem_read(BxPhyAddress::new(destination.into()), &mut bytes)
            .unwrap();
        assert_eq!(u32::from_le_bytes(bytes), TILE);
    }
}

#[test]
fn unsubscribed_device_still_writes_ram_and_asserts_a_guest_visible_irq() {
    let endpoint = GpuEndpoint {
        gpu: Gpu::default(),
        written: Subscriptions::<TileWritten, ()>::new(),
        rejected: Subscriptions::<TileRejected, ()>::new(),
    };
    let mut pc = pc();
    pc.attach_composed_mmio(DEVICE, IsaIrq::new(5).unwrap(), PcDevice::owned(endpoint))
        .unwrap();
    program(&mut pc, RAM);
    assert!(matches!(
        pc.emu_start(CODE, None, None, Some(7)).unwrap(),
        EmuStopReason::CountExhausted
    ));
    assert_eq!(pc.regs().read(EBX), TILE);
    assert_ne!(
        pc.regs().read(ECX) & (1 << 5),
        0,
        "guest sees the PIC's IRQ5"
    );

    // A CPU can be lent out of the PC through ProcessorParts. CPU-only
    // execution of that processor refuses its installed PC capabilities.
    use rusty_box::cpu::CpuError;
    use rusty_box::cpu::memory_session::MemorySession;
    use rusty_box::memory::flat::FlatMemory;
    let mut processor = pc.processor(0);
    let cpu = processor.parts().cpu;
    let mut ram = [0; 4096];
    let mut stamps = [0; 1];
    let mut memory = FlatMemory::new(&mut ram, &mut stamps).unwrap();
    let mut session = MemorySession::new(cpu, memory.loan());
    assert!(matches!(
        session.run(1),
        Err(CpuError::UnsupportedCpuOperation { .. })
    ));
}

#[test]
fn failed_dma_has_no_prefix_or_success_event_and_subscriber_failure_stops() {
    let written = Subscriptions::<TileWritten, &'static str>::new()
        .try_on(|_| panic!("refused DMA cannot emit a completion"));
    let rejected = Subscriptions::<TileRejected, &'static str>::new()
        .on_with(|event: &TileRejected, irq: &mut PcInterrupt<'_>| {
            assert_eq!(event.destination, BxPhyAddress::new(0x18FFE));
            // The device raises IRQ only on success. This subscriber signals
            // the rejection through the board-assigned line.
            irq.set_level(true);
            Err("consumer refused notification")
        })
        .try_on(|_| panic!("an error stops delivery before the next handler"));
    let mut pc = pc();
    let endpoint = GpuEndpoint {
        gpu: Gpu::default(),
        written,
        rejected,
    };
    pc.attach_composed_mmio(DEVICE, IsaIrq::new(5).unwrap(), PcDevice::owned(endpoint))
        .unwrap();
    pc.mem_write(BxPhyAddress::new(0x18FFE), &[0x55; 4])
        .unwrap();
    program(&mut pc, 0x18FFE);
    assert!(matches!(
        pc.emu_start(CODE, None, None, Some(9)).unwrap(),
        EmuStopReason::Stopped
    ));
    assert_eq!(pc.regs().read(RIP), CODE + 20);
    let mut after = [0; 4];
    pc.mem_read(BxPhyAddress::new(0x18FFE), &mut after).unwrap();
    assert_eq!(after, [0x55; 4]);
    pc.emu_start(CODE + 20, None, None, Some(5)).unwrap();
    assert_ne!(
        pc.regs().read(ECX) & (1 << 5),
        0,
        "guest sees the subscriber's rejection IRQ"
    );
    let mut endpoint = pc.detach_composed_mmio(DEVICE).unwrap();
    assert_eq!(endpoint.endpoint().read(WindowOffset(8), 4), 3);
}

#[test]
fn dma_to_executed_code_invalidates_decoded_bytes() {
    let endpoint = GpuEndpoint {
        gpu: Gpu::default(),
        written: Subscriptions::<TileWritten, ()>::new().try_on(|_| Ok(Reaction::Stop)),
        rejected: Subscriptions::<TileRejected, ()>::new().try_on(|_| panic!("DMA must succeed")),
    };
    let mut pc = pc();
    pc.attach_composed_mmio(DEVICE, IsaIrq::new(5).unwrap(), PcDevice::owned(endpoint))
        .unwrap();
    pc.mem_write(BxPhyAddress::new(RAM.into()), &[0xB8, 1, 0, 0, 0])
        .unwrap();
    pc.emu_start(RAM.into(), None, None, Some(1)).unwrap();
    assert_eq!(pc.regs().read(EAX), 1);
    program(&mut pc, RAM + 1);
    pc.emu_start(CODE, None, None, Some(9)).unwrap();
    pc.emu_start(RAM.into(), None, None, Some(1)).unwrap();
    assert_eq!(pc.regs().read(EAX), TILE);
}

#[test]
fn dma_refuses_other_mmio_and_attachment_refusal_returns_ownership() {
    struct Plain;
    impl MmioDevice for Plain {
        fn read(&mut self, _: u64, _: usize) -> u64 {
            0
        }

        fn write(&mut self, _: u64, _: usize, _: u64) {}
    }
    fn empty_endpoint() -> PcDevice {
        PcDevice::owned(GpuEndpoint {
            gpu: Gpu::default(),
            written: Subscriptions::<TileWritten, ()>::new(),
            rejected: Subscriptions::<TileRejected, ()>::new().try_on(|_| Ok(Reaction::Stop)),
        })
    }
    let mut pc = pc();
    pc.attach_composed_mmio(DEVICE, IsaIrq::new(5).unwrap(), empty_endpoint())
        .unwrap();
    let error = pc
        .attach_composed_mmio(DEVICE + 4096, IsaIrq::new(5).unwrap(), empty_endpoint())
        .unwrap_err();
    assert_eq!(error.reason, PcDeviceAttachRefusal::IrqInUse);
    let error = pc
        .attach_composed_mmio(DEVICE, IsaIrq::new(10).unwrap(), error.device)
        .unwrap_err();
    assert_eq!(
        error.reason,
        PcDeviceAttachRefusal::Address(MmioAttachRefusal::Overlap)
    );
    pc.attach_mmio_device(BxPhyAddress::new(RAM.into()), Box::leak(Box::new(Plain)))
        .unwrap();
    pc.mem_write(BxPhyAddress::new(RAM.into()), &[0x55; 4])
        .unwrap();
    program(&mut pc, RAM);
    assert!(matches!(
        pc.emu_start(CODE, None, None, Some(9)).unwrap(),
        EmuStopReason::Stopped
    ));
    let mut after = [0; 4];
    pc.mem_read(BxPhyAddress::new(RAM.into()), &mut after)
        .unwrap();
    assert_eq!(after, [0x55; 4]);
}
