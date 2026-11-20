//! Run an x86 read, update and write against one register-only device.
mod register_bank;

use register_bank::RegisterBank;
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::CpuSetupMode;
use rusty_box::cpu::registers::EAX;
use rusty_box::emulator::hardware::MmioWindow;
use rusty_box::{Emulator, EmulatorConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut pc = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)?;
    MmioWindow::registers(
        RegisterBank::new(7),
        RegisterBank::read,
        RegisterBank::write,
    )
    .install(&mut pc, BxPhyAddress::new(0x90000))?;

    let entry = 0x10000;
    let code = [
        0xA1, 0, 0, 9, 0, // mov eax,[0x90000]
        0x83, 0xC0, 5, // add eax,5
        0xA3, 0, 0, 9, 0, // mov [0x90000],eax
        0xA1, 0, 0, 9, 0, // mov eax,[0x90000]
    ];
    pc.mem_write(BxPhyAddress::new(entry), &code)?;
    pc.emu_start(entry, None, None, Some(4))?;
    assert_eq!(pc.regs().read(EAX), 12);
    println!("guest updated the register from 7 to 12");
    Ok(())
}
