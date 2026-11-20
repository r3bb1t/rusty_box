//! Mount two independent device models and subscribe to their notifications.
mod gpu;
mod mailbox;

use gpu::{Gpu, GpuEvent};
use mailbox::{Mailbox, MailboxEvent};
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuSetupMode;
use rusty_box::cpu::registers::{EAX, EBX, RIP};
use rusty_box::emulator::hardware::{
    ExecutionRequest, InterruptOutput, IsaIrq, PcAttachment, SubscriberError,
};
use rusty_box::{Emulator, EmulatorConfig};
use rusty_box_devices::api::WindowOffset;
use rusty_box_devices::composition::{IrqMmioModel, MmioWrite, Subscriptions, publish_events};

fn stop_on_tile(
    event: &GpuEvent,
    irq: &mut impl InterruptOutput,
) -> Result<ExecutionRequest, SubscriberError> {
    if event.written().is_some() {
        irq.set_level(false);
        return Ok(ExecutionRequest::Stop);
    }
    Ok(ExecutionRequest::None)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut completed_tiles = 0;
    let gpu = PcAttachment::with_model(
        BxPhyAddress::new(0x90000),
        IsaIrq::new(5).unwrap(),
        Gpu::default(),
    )
    .on_event(GpuEvent::written, move |event| {
        completed_tiles += 1;
        println!(
            "tile {completed_tiles} committed at {:#x}",
            event.destination
        );
    })
    .on_with_irq(|event, irq| stop_on_tile(event, irq))
    .on_event(GpuEvent::rejected, |event| {
        eprintln!("unwritable DMA destination {:#x}", event.destination)
    })
    .observe_writes(|access, _| println!("GPU write at offset {}", access.offset.get()));

    let mut pc = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)?;
    gpu.install(&mut pc)?;
    PcAttachment::with_model(
        BxPhyAddress::new(0x91000),
        IsaIrq::new(10).unwrap(),
        Mailbox::default(),
    )
    .on(|event| println!("mailbox {event:?}"))
    .on_event(MailboxEvent::posted, |message| {
        assert_eq!(message.0, 42);
    })
    .install(&mut pc)?;
    // Set destination, submit tile pixels, then load back through the CPU.
    let entry = 0x10000;
    pc.mem_write(
        BxPhyAddress::new(entry),
        &[
            0xC7, 0x05, 0, 0, 9, 0, 0, 0x80, 1, 0, 0xC7, 0x05, 4, 0, 9, 0, 0x11, 0x22, 0x33, 0x44,
            0x8B, 0x1D, 0, 0x80, 1, 0,
        ],
    )?;
    pc.emu_start(entry, None, None, Some(3))?;
    assert_eq!(pc.regs().read(RIP), entry + 20);
    pc.emu_start(entry + 20, None, None, Some(1))?;
    assert_eq!(pc.regs().read(EBX), 0x4433_2211);
    println!("x86 loaded the tile data after resuming");

    let entry = 0x10100;
    // mov dword [0x91000],42; mov eax,[0x91000]
    pc.mem_write(
        BxPhyAddress::new(entry),
        &[0xC7, 0x05, 0, 0x10, 9, 0, 42, 0, 0, 0, 0xA1, 0, 0x10, 9, 0],
    )?;
    pc.emu_start(entry, None, None, Some(2))?;
    assert_eq!(pc.regs().read(EAX), 42);

    // A host-owned mailbox can use the same subscriptions outside MMIO.
    #[derive(Default)]
    struct HostIrq(bool);
    impl InterruptOutput for HostIrq {
        fn set_level(&mut self, asserted: bool) {
            self.0 = asserted;
        }
    }
    let mut host_irq = HostIrq::default();
    let mut mailbox = Mailbox::default();
    let mut handlers = Subscriptions::<MailboxEvent>::new().on(|event| {
        println!("host mailbox {event:?}");
    });
    let post = MmioWrite {
        offset: WindowOffset(0),
        len: 4,
        value: 7,
    };
    assert_eq!(
        publish_events(mailbox.write(post, &mut host_irq), &mut handlers, &mut ())?,
        ExecutionRequest::None
    );
    assert_eq!(
        publish_events(mailbox.take(&mut host_irq), &mut handlers, &mut ())?,
        ExecutionRequest::None
    );
    assert!(!host_irq.0);
    Ok(())
}
