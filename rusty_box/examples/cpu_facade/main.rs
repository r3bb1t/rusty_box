//! Step, edit registers, and complete a port output using the owned x86 API.
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::cpu::registers::{AL, EAX, ECX, EDX, EIP};
use rusty_box::cpu::{CpuEmulator, CpuError, ProcessorActivity};

const ENTRY: u32 = 0x1000;
const RETURN: u32 = 0x1100;
const STACK: u32 = 0x2FFC;

fn phys(address: u32) -> BxPhyAddress {
    BxPhyAddress::new(u64::from(address))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut emu = CpuEmulator::builder().entry(ENTRY).stack(STACK).build()?;

    // lea eax,[ecx+edx*2]; add eax,edx; ret
    emu.memory()
        .write_phys(phys(ENTRY), &[0x8D, 0x04, 0x51, 0x01, 0xD0, 0xC3])?;
    emu.memory()
        .write_phys(phys(STACK), &RETURN.to_le_bytes())?;
    emu.memory().write_phys(phys(RETURN), &[0xE6, 0x80, 0xF4])?;
    {
        let mut regs = emu.regs();
        regs.write(ECX, 7);
        regs.write(EDX, 3);
    }

    assert!(matches!(emu.step().stop, SessionStop::Budget));
    let intermediate = emu.regs().read(EAX);
    assert_eq!(intermediate, 13);
    emu.regs().write(EDX, 10);
    let outcome = emu.run(16);
    match outcome.stop {
        SessionStop::Exit(CpuError::IoUnavailable { string: false, .. }) => {
            assert_eq!(emu.regs().read(EIP), RETURN);
            assert_eq!(emu.regs().read(EAX), 23);
            let result = emu.regs().read(AL);
            assert!(emu.service_scalar_io(|request| {
                assert_eq!(
                    (request.port.get(), request.output),
                    (0x80, Some(result.into()))
                );
                println!("function returned {result} after intermediate value {intermediate}");
                Some(0)
            })?);
        }
        other => return Err(std::io::Error::other(format!("unexpected stop {other:?}")).into()),
    }
    assert!(matches!(
        emu.step().stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));

    Ok(())
}
