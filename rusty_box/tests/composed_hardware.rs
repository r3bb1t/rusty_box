#![cfg(feature = "std")]

#[path = "../examples/composed_hardware/gpu.rs"]
mod gpu;
#[path = "../examples/composed_hardware/mailbox.rs"]
mod mailbox;

use gpu::{Gpu, GpuEvent, TileRejected, TileWritten};
use mailbox::{DoorbellRequested, Mailbox, MailboxEvent, MessagePosted, MessageTaken};
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::instrumentation::EmuStopReason;
use rusty_box::cpu::registers::{EAX, EBX, ECX, RIP};
use rusty_box::cpu::{CpuError, CpuSetupMode};
use rusty_box::emulator::MachineBuilder;
use rusty_box::emulator::hardware::{
    DmaWrite, Emit, InterruptOutput, IsaIrq, MmioModel, MmioSubscriberError, MmioWrite,
    PcAttachment, PcDevice, PcDeviceAttachRefusal, Reaction, SubscriberError, WindowOffset,
};
use rusty_box::memory::mmio::{MmioAttachRefusal, MmioDevice};
use rusty_box::{Emulator, EmulatorConfig, Error};
use rusty_box_devices::composition::{Subscriptions, publish_events, write_and_publish};

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
    let mut events = Capture::<GpuEvent>(Vec::new());
    assert_eq!(
        write_and_publish(&mut gpu, access(0, RAM), &mut dma, &mut irq, &mut events),
        Ok(Reaction::Continue)
    );
    assert!(events.0.is_empty());
    assert_eq!(
        write_and_publish(&mut gpu, access(4, TILE), &mut dma, &mut irq, &mut events),
        Ok(Reaction::Continue)
    );
    assert_eq!(irq.edges, [true]);
    assert_eq!(
        dma.writes,
        [(BxPhyAddress::new(RAM.into()), TILE.to_le_bytes().to_vec())]
    );
    assert_eq!(
        events.0,
        [(
            GpuEvent::Written(TileWritten {
                destination: BxPhyAddress::new(RAM.into()),
                pixels: TILE.to_le_bytes()
            }),
            true
        )]
    );

    dma.fail = true;
    assert_eq!(
        write_and_publish(&mut gpu, access(4, 99), &mut dma, &mut irq, &mut events),
        Ok(Reaction::Continue)
    );
    assert_eq!(gpu.read(WindowOffset(8), 4), 2);
    assert_eq!(events.0.len(), 2);
    assert_eq!(dma.writes.len(), 1);
    assert_eq!(irq.edges, [true]);
    assert_eq!(
        events.0[1],
        (
            GpuEvent::Rejected(TileRejected {
                destination: BxPhyAddress::new(RAM.into()),
            }),
            true
        )
    );
}

#[test]
fn mailbox_uses_the_same_glue_and_never_needs_dma() {
    let (mut mailbox, mut dma, mut irq) =
        (Mailbox::default(), FakeDma::default(), FakeIrq::default());
    dma.fail = true;
    let mut events = Capture(Vec::new());
    assert_eq!(
        write_and_publish(&mut mailbox, access(0, 42), &mut dma, &mut irq, &mut events),
        Ok(Reaction::Continue)
    );
    assert_eq!(mailbox.read(WindowOffset(0), 4), 42);
    assert_eq!(
        events.0,
        [
            (MailboxEvent::Posted(MessagePosted(42)), true),
            (MailboxEvent::Doorbell(DoorbellRequested), true),
        ]
    );
    assert_eq!(
        write_and_publish(&mut mailbox, access(4, 0), &mut dma, &mut irq, &mut events),
        Ok(Reaction::Continue)
    );
    assert_eq!(events.0.len(), 2);
    assert!(dma.writes.is_empty());
    assert_eq!(irq.edges, [true, false]);
}

#[test]
fn a_host_operation_publishes_to_the_same_handlers_without_mmio() {
    use rusty_box_devices::composition::IrqMmioModel;

    let (mut mailbox, mut irq) = (Mailbox::default(), FakeIrq::default());
    let mut events = Vec::new();
    {
        let mut route = Subscriptions::<MailboxEvent>::new().on(|event| events.push(*event));
        let posted = IrqMmioModel::write(&mut mailbox, access(0, 42), &mut irq);
        assert_eq!(
            publish_events(posted, &mut route, &mut ()),
            Ok(Reaction::Continue)
        );
        let taken = mailbox.take(&mut irq);
        assert_eq!(
            publish_events(taken, &mut route, &mut ()),
            Ok(Reaction::Continue)
        );
        assert_eq!(
            publish_events(mailbox.take(&mut irq), &mut route, &mut ()),
            Ok(Reaction::Continue)
        );
    }
    assert_eq!(
        events,
        [
            MailboxEvent::Posted(MessagePosted(42)),
            MailboxEvent::Doorbell(DoorbellRequested),
            MailboxEvent::Taken(MessageTaken(42)),
        ]
    );
    assert_eq!(irq.edges, [true, false]);
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
    let mut pc = pc();
    let mut deliveries = 0;
    PcAttachment::with_model(DEVICE, IsaIrq::new(5).unwrap(), Gpu::default())
        .on_event(GpuEvent::written, move |event| {
            assert_eq!(event.pixels, TILE.to_le_bytes());
            deliveries += 1;
            assert_eq!(deliveries, 1, "the DMA notification is not replayed");
        })
        .try_on(|event| {
            assert_eq!(
                event.written().unwrap().destination,
                BxPhyAddress::new(RAM.into())
            );
            Ok(Reaction::Stop)
        })
        .on_event(GpuEvent::rejected, |_| {
            panic!("ordinary RAM DMA must succeed")
        })
        .observe_writes(|write, result| match write.offset.get() {
            0 => assert_eq!(result, Ok(Reaction::Continue)),
            4 => assert_eq!(result, Ok(Reaction::Stop)),
            _ => panic!("unexpected write"),
        })
        .install(&mut pc)
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
        PcAttachment::with_model(page, IsaIrq::new(line).unwrap(), Gpu::default())
            .on_event(GpuEvent::written, move |event| {
                assert_eq!(event.destination.get(), u64::from(destination))
            })
            .on_with_irq(|_, _| Ok(Reaction::Stop))
            .on_event(GpuEvent::rejected, |_| panic!("DMA must succeed"))
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
    let mut pc = pc();
    PcAttachment::with_model(DEVICE, IsaIrq::new(5).unwrap(), Gpu::default())
        .install(&mut pc)
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
    let mut pc = pc();
    PcAttachment::with_model(DEVICE, IsaIrq::new(5).unwrap(), Gpu::default())
        .on_event(GpuEvent::written, |_| {
            panic!("refused DMA cannot emit a completion")
        })
        .on_with_irq(|event, irq| {
            let event = event.rejected().unwrap();
            assert_eq!(event.destination, BxPhyAddress::new(0x18FFE));
            // The device raises IRQ only on success. This subscriber signals
            // the rejection through the board-assigned line.
            irq.set_level(true);
            Err(SubscriberError("consumer refused notification"))
        })
        .on(|_| panic!("an error stops delivery before the next handler"))
        .install(&mut pc)
        .unwrap();
    pc.mem_write(BxPhyAddress::new(0x18FFE), &[0x55; 4])
        .unwrap();
    program(&mut pc, 0x18FFE);
    assert_subscriber_error(&mut pc, "consumer refused notification");
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
    assert_eq!(endpoint.endpoint().read(WindowOffset(8), 4), 2);
}

#[test]
fn successful_dma_survives_subscriber_failure_and_invalidates_decoded_bytes() {
    let mut pc = pc();
    PcAttachment::with_model(DEVICE, IsaIrq::new(5).unwrap(), Gpu::default())
        .on_event(GpuEvent::rejected, |_| panic!("DMA must succeed"))
        .try_on(|_| Ok(Reaction::Stop))
        .try_on(|_| Err(SubscriberError("capture refused committed tile")))
        .on(|_| panic!("subscribers after the error are skipped"))
        .install(&mut pc)
        .unwrap();
    pc.mem_write(BxPhyAddress::new(RAM.into()), &[0xB8, 1, 0, 0, 0])
        .unwrap();
    pc.emu_start(RAM.into(), None, None, Some(1)).unwrap();
    assert_eq!(pc.regs().read(EAX), 1);
    program(&mut pc, RAM + 1);
    assert_subscriber_error(&mut pc, "capture refused committed tile");
    assert_eq!(pc.regs().read(RIP), CODE + 20);
    let mut bytes = [0; 4];
    pc.mem_read(BxPhyAddress::new(u64::from(RAM + 1)), &mut bytes)
        .unwrap();
    assert_eq!(u32::from_le_bytes(bytes), TILE);
    // Resume without rerunning the store or delivering its error again.
    pc.emu_start(CODE + 20, None, None, Some(5)).unwrap();
    assert_ne!(pc.regs().read(ECX) & (1 << 5), 0);
    pc.emu_start(RAM.into(), None, None, Some(1)).unwrap();
    assert_eq!(pc.regs().read(EAX), TILE);
}

fn assert_subscriber_error(pc: &mut Emulator, message: &'static str) {
    let error = pc.emu_start(CODE, None, None, Some(9)).unwrap_err();
    match error {
        Error::Cpu(CpuError::MmioSubscriber(error)) => assert_eq!(
            error,
            MmioSubscriberError {
                address: DEVICE + 4,
                source: SubscriberError(message),
            }
        ),
        other => panic!("unexpected execution result {other:?}"),
    }
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
        let (_, _, endpoint) =
            PcAttachment::with_model(DEVICE, IsaIrq::new(5).unwrap(), Gpu::default())
                .try_on(|event| {
                    assert!(event.rejected().is_some());
                    Ok(Reaction::Stop)
                })
                .into_parts();
        PcDevice::owned(endpoint)
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

#[test]
fn two_distinct_models_mount_together_and_a_detached_mailbox_can_move() {
    let mut pc = pc();
    let mailbox_page = DEVICE + 4096;
    PcAttachment::with_model(DEVICE, IsaIrq::new(5).unwrap(), Gpu::default())
        .on_event(GpuEvent::written, |event| {
            assert_eq!(event.pixels, TILE.to_le_bytes())
        })
        .install(&mut pc)
        .unwrap();

    let refused =
        PcAttachment::with_model(mailbox_page, IsaIrq::new(4).unwrap(), Mailbox::default())
            .try_on({
                let mut posted = false;
                move |event| match event {
                    MailboxEvent::Posted(MessagePosted(42)) => {
                        assert!(!posted);
                        posted = true;
                        Ok(Reaction::Continue)
                    }
                    MailboxEvent::Doorbell(_) => {
                        assert!(posted);
                        Ok(Reaction::Stop)
                    }
                    other => panic!("unexpected notification {other:?}"),
                }
            })
            .install(&mut pc)
            .unwrap_err();
    assert_eq!(refused.reason, PcDeviceAttachRefusal::BoardIrqInUse);
    let (page, _, endpoint) = refused.attachment.into_parts();
    PcAttachment::new(page, IsaIrq::new(10).unwrap(), endpoint)
        .install(&mut pc)
        .unwrap();

    let mut saved = Vec::new();
    let save_error = pc.save_snapshot(&mut saved).unwrap_err();
    assert_eq!(save_error.kind(), std::io::ErrorKind::Unsupported);
    assert!(saved.is_empty());
    let mut restore = std::io::Cursor::new([0; 8]);
    let restore_error = pc.restore_snapshot(&mut restore).unwrap_err();
    assert_eq!(restore_error.kind(), std::io::ErrorKind::Unsupported);
    assert_eq!(restore.position(), 0);

    program(&mut pc, RAM);
    pc.emu_start(CODE, None, None, Some(3)).unwrap();
    assert_eq!(pc.regs().read(EBX), TILE);

    // mov dword [0x91000],42; mov eax,[0x91000]
    pc.mem_write(
        BxPhyAddress::new(CODE),
        &[0xC7, 0x05, 0, 0x10, 9, 0, 42, 0, 0, 0, 0xA1, 0, 0x10, 9, 0],
    )
    .unwrap();
    assert!(matches!(
        pc.emu_start(CODE, None, None, Some(2)).unwrap(),
        EmuStopReason::Stopped
    ));
    assert_eq!(pc.regs().read(RIP), CODE + 10);
    pc.emu_start(CODE + 10, None, None, Some(1)).unwrap();
    assert_eq!(pc.regs().read(EAX), 42);

    let device = pc.detach_composed_mmio(mailbox_page).unwrap();
    pc.attach_composed_mmio(mailbox_page + 4096, IsaIrq::new(10).unwrap(), device)
        .unwrap();
    let mut device = pc.detach_composed_mmio(mailbox_page + 4096).unwrap();
    assert_eq!(device.endpoint().read(WindowOffset(0), 4), 42);
}

#[test]
fn attachment_rejects_current_pirq_and_hpet_routes() {
    // Use the full board initializer so PCI configuration ports and the
    // chipset's HPET window are installed before executing the fixture.
    let mut pc = MachineBuilder::new(EmulatorConfig::default())
        .build()
        .unwrap();
    pc.setup_cpu_mode(CpuSetupMode::FlatProtected32).unwrap();
    // Select PIIX config byte 0x60 at 0xcf8 and route PIRQA to IRQ10.
    let pirq = [
        0x66, 0xBA, 0xF8, 0x0C, 0xB8, 0x60, 8, 0, 0x80, 0xEF, 0x66, 0xBA, 0xFC, 0x0C, 0xB0, 10,
        0xEE, 0xEC,
    ];
    pc.mem_write(BxPhyAddress::new(CODE), &pirq).unwrap();
    pc.emu_start(CODE, None, None, Some(7)).unwrap();
    assert_eq!(pc.regs().read(EAX) & 0xFF, 10, "PIIX route readback");
    assert_route_claimed(&mut pc, 10);

    // Open A20 through port 0x92, then program and read HPET timer 0.
    let hpet = [
        0xB0, 2, 0xE6, 0x92, 0xC7, 0x05, 0, 1, 0xD0, 0xFE, 4, 10, 0, 0, 0xA1, 0, 1, 0xD0, 0xFE,
    ];
    pc.mem_write(BxPhyAddress::new(CODE), &hpet).unwrap();
    pc.emu_start(CODE, None, None, Some(4)).unwrap();
    assert_eq!(pc.regs().read(EAX) & 0x3E00, 0x0A00, "HPET route readback");
    assert_route_claimed(&mut pc, 5);
}

fn assert_route_claimed(pc: &mut Emulator, line: u8) {
    let result = PcAttachment::with_model(DEVICE, IsaIrq::new(line).unwrap(), Mailbox::default())
        .install(pc);
    assert!(
        matches!(result, Err(error) if error.reason == PcDeviceAttachRefusal::BoardIrqInUse),
        "board route for IRQ{line} must block attachment"
    );
}
