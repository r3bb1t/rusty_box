//! Run a small x86 function on borrowed RAM, without PC hardware or firmware.
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::builder::BxCpuBuilder;
use rusty_box::cpu::memory_session::{EntryMode, MemorySession, SessionStop};
use rusty_box::cpu::{CpuError, ProcessorActivity, X86Reg};
use rusty_box::memory::flat::FlatMemory;

const FUNCTION: u32 = 0x1000;
const RETURN: u32 = 0x1100;
const STACK: u32 = 0x2FFC;

fn physical(address: u32) -> BxPhyAddress {
    BxPhyAddress::new(u64::from(address))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cpu = BxCpuBuilder::new().build()?;
    let mut ram = [0; 0x3000];
    let mut stamps = [0; 3];
    let mut memory = FlatMemory::new(&mut ram, &mut stamps)?;
    let mut session = MemorySession::new(&mut cpu, memory.loan());

    session.reset_at(EntryMode::Protected32, FUNCTION, STACK)?;
    // lea eax,[ecx+edx*2]; add eax,edx; ret
    session.write_ram(physical(FUNCTION), &[0x8D, 0x04, 0x51, 0x01, 0xD0, 0xC3])?;
    session.write_ram(physical(STACK), &RETURN.to_le_bytes())?;
    // The return trampoline signals an I/O boundary and halts.
    session.write_ram(physical(RETURN), &[0xE6, 0x80, 0xF4])?;
    session.write_register(X86Reg::Ecx, 7)?;
    session.write_register(X86Reg::Edx, 3)?;

    let step = session.step();
    assert!(matches!(step.stop, SessionStop::Budget));
    assert_eq!(step.instructions, 1);
    let intermediate = session.read_register(X86Reg::Eax)?;
    assert_eq!(intermediate, 13);
    session.write_register(X86Reg::Edx, 10)?;

    let exit = session.run_controlled(16);
    assert_eq!(exit.instructions, 2);
    let result = session.read_register(X86Reg::Eax)?;
    assert_eq!(result, 23);
    match exit.stop {
        SessionStop::Exit(CpuError::IoUnavailable {
            port,
            len: 1,
            direction_in: false,
            string: false,
        }) if port.get() == 0x80 => {
            assert_eq!(session.read_register(X86Reg::Eip)?, u64::from(RETURN));
            println!("function returned {result} after intermediate value {intermediate}");
            println!("host received port output {result} at {port:?}");
            // OUT did not retire. This example acknowledges it by skipping
            // the known two-byte instruction in the trampoline.
            session.set_instruction_pointer(u64::from(RETURN + 2));
        }
        other => return Err(std::io::Error::other(format!("unexpected stop {other:?}")).into()),
    }
    let halt = session.step();
    assert_eq!(halt.instructions, 1);
    assert!(matches!(
        halt.stop,
        SessionStop::Inactive(ProcessorActivity::Halted)
    ));
    Ok(())
}
