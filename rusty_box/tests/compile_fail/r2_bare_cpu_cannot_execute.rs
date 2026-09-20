// R2 (docs/safety-doctrine.md): states are types. A CPU with no machine behind
// it cannot execute — `execute_instruction` lives on ExecCtx, so the
// wired-to-nothing state is unrepresentable rather than checked at runtime.
fn main() {
    let mut cpu = rusty_box::cpu::builder::BxCpuBuilder::new().build().unwrap();
    let instr = rusty_box::cpu::decoder::Instruction::default();
    let _ = cpu.execute_instruction(&instr);
}
