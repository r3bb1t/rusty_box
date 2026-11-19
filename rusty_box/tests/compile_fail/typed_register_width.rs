use rusty_box::cpu::CpuEmulator;
use rusty_box::cpu::registers::EAX;

fn main() {
    let mut emu = CpuEmulator::builder().build().unwrap();
    emu.regs().write(EAX, 42u64);
}
