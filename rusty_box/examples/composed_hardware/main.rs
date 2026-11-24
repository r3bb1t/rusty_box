//! Mount two independent device models and subscribe to their notifications.
mod gpu;
mod mailbox;

use gpu::{Gpu, GpuEvent};
use mailbox::{Mailbox, MailboxEvent};
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuSetupMode;
use rusty_box::cpu::instrumentation::EmuStopReason;
use rusty_box::cpu::registers::{EAX, EBX, RIP};
use rusty_box::emulator::hardware::{
    EventEndpoint, ExecutionRequest, InterruptOutput, IsaIrq, PcAttachment, PcInterrupt,
    SubscriberError,
};
use rusty_box::events::App;
use rusty_box::{Emulator, EmulatorConfig};
use rusty_box_devices::api::WindowOffset;
use rusty_box_devices::composition::{IrqMmioModel, MmioWrite, Subscriptions, publish_events};

#[derive(Default)]
struct TileLog {
    completed: usize,
}

fn record_tile(event: &GpuEvent, state: &mut TileLog) -> ExecutionRequest {
    if let Some(tile) = event.written() {
        state.completed += 1;
        println!(
            "Tile {} committed at {:#x}",
            state.completed, tile.destination
        );
        return ExecutionRequest::Stop;
    }
    if let Some(tile) = event.rejected() {
        eprintln!("Unwritable DMA destination {:#x}", tile.destination);
    }
    ExecutionRequest::None
}

fn acknowledge_tile(
    event: &GpuEvent,
    irq: &mut PcInterrupt<'_>,
) -> Result<ExecutionRequest, SubscriberError> {
    if event.written().is_some() {
        irq.set_level(false);
    }
    Ok(ExecutionRequest::None)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::fallible()
        .route(record_tile)
        .with_state(TileLog::default());
    let subscribers = Subscriptions::new().forward(app).on_with(acknowledge_tile);
    let gpu = PcAttachment::new(
        BxPhyAddress::new(0x90000),
        IsaIrq::new(5).ok_or("invalid GPU IRQ")?,
        EventEndpoint::with_subscriptions(Gpu::default(), subscribers),
    )
    .observe_writes(|access, _| println!("GPU write at offset {}", access.offset.get()));

    let mut pc = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)?;
    gpu.install(&mut pc)?;
    PcAttachment::with_model(
        BxPhyAddress::new(0x91000),
        IsaIrq::new(10).ok_or("invalid mailbox IRQ")?,
        Mailbox::default(),
    )
    .on(|event| println!("mailbox {event:?}"))
    .on_event(MailboxEvent::posted, |message| {
        println!("Posted {}", message.0)
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
    let stop = pc.emu_start(entry, None, None, Some(3))?;
    if stop != EmuStopReason::Stopped {
        return Err(std::io::Error::other(format!("expected tile stop, got {stop:?}")).into());
    }
    let resume = pc.regs().read(RIP);
    pc.emu_start(resume, None, None, Some(1))?;
    println!("Guest loaded tile data {:#x}", pc.regs().read(EBX));

    let entry = 0x10100;
    // mov dword [0x91000],42; mov eax,[0x91000]
    pc.mem_write(
        BxPhyAddress::new(entry),
        &[0xC7, 0x05, 0, 0x10, 9, 0, 42, 0, 0, 0, 0xA1, 0, 0x10, 9, 0],
    )?;
    pc.emu_start(entry, None, None, Some(2))?;
    println!("Guest read mailbox value {}", pc.regs().read(EAX));

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
    if publish_events(mailbox.write(post, &mut host_irq), &mut handlers, &mut ())?
        == ExecutionRequest::Stop
    {
        return Ok(());
    }
    let request = publish_events(mailbox.take(&mut host_irq), &mut handlers, &mut ())?;
    println!("Host mailbox IRQ is {}, delivery {request:?}", host_irq.0);
    Ok(())
}
