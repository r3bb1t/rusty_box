//! Connect a reusable IRQ-only model and handle its typed notifications.
#[path = "../composed_hardware/mailbox.rs"]
mod mailbox;

use mailbox::{Mailbox, MailboxEvent};
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuSetupMode;
use rusty_box::cpu::instrumentation::EmuStopReason;
use rusty_box::cpu::registers::{EAX, RIP};
use rusty_box::emulator::hardware::{IsaIrq, MmioWindow, Reaction};
use rusty_box::{Emulator, EmulatorConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut pc = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)?;
    MmioWindow::with_model(IsaIrq::new(5).unwrap(), Mailbox::default())
        .on(|event| {
            if let MailboxEvent::Posted(message) = event {
                println!("guest posted {}", message.0);
            }
        })
        .try_on(|event| {
            Ok(match event {
                MailboxEvent::Doorbell(_) => Reaction::Stop,
                _ => Reaction::Continue,
            })
        })
        .install(&mut pc, BxPhyAddress::new(0x90000))?;

    let entry = 0x10000;
    let guest = [
        0xC7, 0x05, 0, 0, 9, 0, 42, 0, 0, 0, // mov dword [0x90000],42
        0xA1, 0, 0, 9, 0, // mov eax,[0x90000]
    ];
    pc.mem_write(BxPhyAddress::new(entry), &guest)?;
    assert!(matches!(
        pc.emu_start(entry, None, None, Some(2))?,
        EmuStopReason::Stopped
    ));
    let resume = pc.regs().read(RIP);
    assert_eq!(resume, entry + 10);
    pc.emu_start(resume, None, None, Some(1))?;
    assert_eq!(pc.regs().read(EAX), 42);
    Ok(())
}
