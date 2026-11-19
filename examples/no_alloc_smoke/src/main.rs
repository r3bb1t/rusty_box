#![no_std]
#![no_main]

use core::panic::PanicInfo;

use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::BxCpuC;
use rusty_box::cpu::memory_session::MemorySession;
use rusty_box::memory::mapped::{MappedMemory, PhysicalRegion, RegionKind};

fn consume<T>(_value: T) {}

// This target checks that both the public map and its execution entry point
// compile with allocation disabled. The host selects and places the CPU.
fn execute(cpu: &mut BxCpuC) -> rusty_box::cpu::Result<u64> {
    let mut bytes = [0; 4096];
    let mut stamps = [0; 1];
    let regions = [PhysicalRegion {
        guest_start: BxPhyAddress::new(0x1000),
        arena_offset: 0,
        len: bytes.len(),
        kind: RegionKind::Ram,
    }];
    let mut memory = MappedMemory::new(&mut bytes, &mut stamps, &regions).unwrap();
    MemorySession::new(cpu, memory.loan()).run(1)
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
    consume(execute);
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
