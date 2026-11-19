#![no_std]
#![no_main]

use core::panic::PanicInfo;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::memory_session::{EntryMode, MemorySession, SessionRun};
use rusty_box::cpu::{BxCpuC, X86Reg};
use rusty_box::memory::mapped::{MappedMemory, PhysicalRegion, RegionKind};

fn consume<T>(_value: T) {}

// This target checks that both the public map and its execution entry point
// compile with allocation disabled. The host selects and places the CPU.
fn execute(cpu: &mut BxCpuC) -> rusty_box::Result<SessionRun> {
    let mut bytes = [0; 4096];
    let mut stamps = [0; 1];
    let regions = [PhysicalRegion {
        guest_start: BxPhyAddress::new(0x1000),
        arena_offset: 0,
        len: bytes.len(),
        kind: RegionKind::Ram,
    }];
    let mut memory = MappedMemory::new(&mut bytes, &mut stamps, &regions).unwrap();
    let mut session = MemorySession::new(cpu, memory.loan());
    session.reset_at(EntryMode::Real16, 0x1000, 0x1FF0)?;
    session.write_register(X86Reg::Ax, 0x42).unwrap();
    session.write_ram(BxPhyAddress::new(0x1000), &[0x90])?;
    let mut code = [0];
    session.read_physical(BxPhyAddress::new(0x1000), &mut code)?;
    Ok(session.step())
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    consume(rusty_box::EmulatorConfig::default());
    let mut bytes = [0; 4097];
    let mut stamps = [0; 2];
    let mut ram = rusty_box::memory::flat::FlatMemory::new(&mut bytes, &mut stamps).unwrap();
    ram.write(rusty_box::config::BxPhyAddress::new(4096), &[0x90])
        .unwrap();
    consume(ram);
    consume(core::hint::black_box(execute as fn(&mut BxCpuC) -> _));
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
