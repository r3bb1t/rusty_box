//! Mount one fixed aperture with a two-page register bank and a downstream bus.
#[path = "../composed_hardware/gpu.rs"]
mod gpu;
mod register_bank;

use std::num::NonZeroU16;

use gpu::Gpu;
use register_bank::RegisterBank;
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuSetupMode;
use rusty_box::cpu::instrumentation::EmuStopReason;
use rusty_box::cpu::registers::RIP;
use rusty_box::emulator::hardware::{IsaIrq, MmioWindow, Reaction};
use rusty_box::{Emulator, EmulatorConfig};

fn store(code: &mut Vec<u8>, address: u32, value: u32) {
    code.extend_from_slice(&[0xC7, 0x05]);
    code.extend_from_slice(&address.to_le_bytes());
    code.extend_from_slice(&value.to_le_bytes());
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registers = MmioWindow::registers(
        RegisterBank::new(0),
        RegisterBank::read,
        RegisterBank::write,
    )
    .with_pages(NonZeroU16::new(2).unwrap())
    .on(|event| println!("register at {:#x} is {}", event.offset.get(), event.value));
    let gpu = |line, stop| {
        MmioWindow::with_model(IsaIrq::new(line).unwrap(), Gpu::default())
            .on(move |event| {
                if let Some(tile) = event.written() {
                    println!("GPU on IRQ {line} DMA to {:#x}", tile.destination);
                } else if let Some(failure) = event.rejected() {
                    println!("GPU on IRQ {line} refused {:#x}", failure.destination);
                }
            })
            .try_on(move |_| {
                Ok(if stop {
                    Reaction::Stop
                } else {
                    Reaction::Continue
                })
            })
    };
    let downstream = gpu(5, false).then(gpu(10, true))?;
    let root = registers.then(downstream)?;
    let mut pc = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)?;
    root.install(&mut pc, BxPhyAddress::new(0x90000))?;

    let mut code = Vec::new();
    store(&mut code, 0x91000, 42);
    store(&mut code, 0x92000, 0x18000);
    store(&mut code, 0x92004, 0x4433_2211);
    store(&mut code, 0x93000, 0x18004);
    store(&mut code, 0x93004, 0x8877_6655);
    code.push(0x90);
    let entry = 0x10000;
    pc.mem_write(BxPhyAddress::new(entry), &code)?;
    assert!(matches!(
        pc.emu_start(entry, None, None, Some(6))?,
        EmuStopReason::Stopped
    ));
    assert_eq!(pc.regs().read(RIP), entry + 50);

    let mut pixels = [0; 8];
    pc.mem_read(BxPhyAddress::new(0x18000), &mut pixels)?;
    assert_eq!(pixels, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
    pc.emu_start(entry + 50, None, None, Some(1))?;
    Ok(())
}
