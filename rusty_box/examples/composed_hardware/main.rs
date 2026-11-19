//! Mount an independent DMA device and subscribe to its notifications.
mod adapter;
mod gpu;

use adapter::GpuAttachment;
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuSetupMode;
use rusty_box::cpu::registers::{EBX, RIP};
use rusty_box::emulator::hardware::{InterruptOutput, IsaIrq, Reaction};
use rusty_box::{Emulator, EmulatorConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut completed_tiles = 0;
    let gpu = GpuAttachment::new()
        .on(move |event| {
            completed_tiles += 1;
            println!(
                "tile {completed_tiles} committed at {:#x}",
                event.destination
            );
        })
        .on_with_irq(|_, irq| {
            irq.set_level(false);
            Reaction::Stop
        })
        .on_rejected(|event| eprintln!("unwritable DMA destination {:#x}", event.destination))
        .at(BxPhyAddress::new(0x90000), IsaIrq::new(5).unwrap())
        .observe_writes(|access, _| println!("GPU write at offset {}", access.offset.get()));

    let mut pc = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)?;
    gpu.install(&mut pc)?;
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
    Ok(())
}
