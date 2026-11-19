use rusty_box::cpu::CpuEmulator;
use rusty_box::cpu::registers::XMM0;

fn main() {
    let mut emu = CpuEmulator::builder().build().unwrap();
    emu.regs().write(XMM0, [0; 32]);
}
