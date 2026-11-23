//! Lend a caller-owned RAM slice, run x86 code, then recover the guest's bytes.
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::ProcessorActivity;
use rusty_box::cpu::builder::BxCpuBuilder;
use rusty_box::cpu::memory_session::{EntryMode, MemorySession, SessionStop};
use rusty_box::cpu::registers::EAX;
use rusty_box::memory::flat::FlatMemory;

const ENTRY: u32 = 0x1000;
const RESULT: usize = 0x2000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cpu = BxCpuBuilder::new().build()?;
    let mut bytes = [0; 0x3000];
    let mut stamps = [0; FlatMemory::required_stamps(0x3000)];
    {
        let mut memory = FlatMemory::new(&mut bytes, &mut stamps)?;
        let mut session = MemorySession::new(&mut cpu, memory.loan());
        session.reset_at(EntryMode::Protected32, ENTRY, 0x2FFC)?;
        session.memory().write_phys(
            BxPhyAddress::new(u64::from(ENTRY)),
            &[
                0x83, 0xC0, 3, // add eax, 3
                0xA3, 0, 0x20, 0, 0,    // mov [0x2000], eax
                0xF4, // hlt
            ],
        )?;
        session.regs().write(EAX, 39);
        let run = session.run_controlled(8);
        assert!(matches!(
            run.stop,
            SessionStop::Inactive(ProcessorActivity::Halted)
        ));
        assert_eq!(session.regs().read(EAX), 42);
    }

    // Ending the loan gives the caller access to the same backing bytes.
    assert_eq!(bytes[RESULT..RESULT + 4], 42u32.to_le_bytes());
    println!("guest wrote 42 to caller-owned RAM at {RESULT:#x}");
    Ok(())
}
