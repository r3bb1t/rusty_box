#![no_std]
#![no_main]

use core::panic::PanicInfo;

fn consume<T>(_value: T) {}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    consume(rusty_box::EmulatorConfig::default());
    let mut bytes = [0; 4097];
    let mut stamps = [0; 2];
    let mut ram = rusty_box::memory::flat::FlatMemory::new(&mut bytes, &mut stamps).unwrap();
    ram.write(rusty_box::config::BxPhyAddress::new(4096), &[0x90])
        .unwrap();
    consume(ram);
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
