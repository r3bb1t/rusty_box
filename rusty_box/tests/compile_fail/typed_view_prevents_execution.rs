use rusty_box::cpu::CpuEmulator;
use rusty_box::cpu::registers::EAX;

fn main() {
    let mut emu = CpuEmulator::builder().build().unwrap();
    let regs = emu.regs();
    emu.step();
    let _ = regs.read(EAX);
}
