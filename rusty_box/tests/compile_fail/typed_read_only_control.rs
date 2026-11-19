use rusty_box::cpu::CpuEmulator;
use rusty_box::cpu::registers::CR0;

fn main() {
    let mut emu = CpuEmulator::builder().build().unwrap();
    emu.regs().write(CR0, 0u32);
}
