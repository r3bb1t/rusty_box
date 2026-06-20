#![no_std]
#![no_main]

use core::panic::PanicInfo;

fn consume<T>(_value: T) {}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    consume(rusty_box::EmulatorConfig::default());
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
