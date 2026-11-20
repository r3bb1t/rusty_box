//! Run firmware-free x86 code and complete a scalar I/O exit.
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::cpu::registers::{EAX, ECX, EDX};
use rusty_box::cpu::{CpuEmulator, CpuError, ProcessorActivity};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cpu = CpuEmulator::builder().entry(0x1000).build()?;
    // lea eax,[ecx+edx*2]; out 0x80,al; hlt
    cpu.memory().write_phys(
        BxPhyAddress::new(0x1000),
        &[0x8D, 0x04, 0x51, 0xE6, 0x80, 0xF4],
    )?;
    cpu.regs().write(ECX, 5);
    cpu.regs().write(EDX, 3);

    let outcome = cpu.run(8);
    assert!(matches!(
        outcome.stop,
        SessionStop::Exit(CpuError::IoUnavailable { string: false, .. })
    ));
    assert_eq!(cpu.regs().read(EAX), 11);
    assert!(cpu.service_scalar_io(|request| {
        assert_eq!(
            (request.port.get(), request.len, request.output),
            (0x80, 1, Some(11))
        );
        println!("x86 sent {}", request.output.unwrap());
        Some(0)
    })?);
    assert!(matches!(
        cpu.step().stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));
    Ok(())
}
