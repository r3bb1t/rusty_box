//! Recover two existing devices and their handlers after an IRQ conflict.
#[path = "../composed_hardware/mailbox.rs"]
mod mailbox;

use mailbox::{Mailbox, MailboxEvent};
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuSetupMode;
use rusty_box::cpu::registers::{EAX, EBX};
use rusty_box::emulator::hardware::{ExecutionRequest, InterruptOutput, IsaIrq, MmioWindow};
use rusty_box::{Emulator, EmulatorConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let shared = IsaIrq::new(5).unwrap();
    let left = MmioWindow::with_model(shared, Mailbox::default()).on(|event| {
        if let Some(message) = event.posted() {
            println!("left mailbox received {}", message.0);
        }
    });
    let right = MmioWindow::with_model(shared, Mailbox::default())
        .on(|event| {
            if let Some(message) = event.posted() {
                println!("right mailbox received {}", message.0);
            }
        })
        .on_with_irq(|event, irq| {
            if matches!(event, MailboxEvent::Doorbell(_)) {
                irq.set_level(false);
            }
            Ok(ExecutionRequest::None)
        });
    let failed = match left.then(right) {
        Err(failed) => failed,
        Ok(_) => panic!("the two outputs initially use the same line"),
    };
    println!("correcting duplicate wiring ({})", failed.reason);
    let root = failed
        .left
        .then(failed.right.with_irq_line(IsaIrq::new(10).unwrap()))?;

    let mut pc = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)?;
    root.install(&mut pc, BxPhyAddress::new(0x90000))?;
    let entry = 0x10000;
    let code = [
        0xC7, 0x05, 0, 0, 9, 0, 42, 0, 0, 0, // mov dword [0x90000],42
        0xC7, 0x05, 0, 0x10, 9, 0, 17, 0, 0, 0, // mov dword [0x91000],17
        0xA1, 0, 0, 9, 0, // mov eax,[0x90000]
        0x8B, 0x1D, 0, 0x10, 9, 0, // mov ebx,[0x91000]
    ];
    pc.mem_write(BxPhyAddress::new(entry), &code)?;
    pc.emu_start(entry, None, None, Some(4))?;
    assert_eq!((pc.regs().read(EAX), pc.regs().read(EBX)), (42, 17));
    Ok(())
}
