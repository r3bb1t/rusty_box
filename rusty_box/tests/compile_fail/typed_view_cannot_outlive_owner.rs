use rusty_box::cpu::CpuEmulator;
use rusty_box::cpu::registers::EAX;

fn main() {
    let regs = {
        let mut emu = CpuEmulator::builder().build().unwrap();
        emu.regs()
    };
    let _ = regs.read(EAX);
}
