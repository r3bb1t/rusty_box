//! Execute the same function as the borrowed `cpu_only` example.
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::builder::BxCpuBuilder;
use rusty_box::cpu::memory_session::{EntryMode, MemorySession, SessionStop};
use rusty_box::cpu::registers::{AL, AX, EAX, ECX, EDX, EIP};
use rusty_box::cpu::{CpuEmulator, CpuError, ProcessorActivity};
use rusty_box::memory::mapped::{MappedMemory, PhysicalRegion, RegionKind};

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

    caller_owned()?;
    Ok(())
}

fn caller_owned() -> Result<(), Box<dyn std::error::Error>> {
    // A discontiguous physical layout using caller-owned bytes and stamps.
    let mut cpu = BxCpuBuilder::new().build()?;
    let mut bytes = [0; 8192];
    let mut stamps = [0; 2];
    let regions = [
        PhysicalRegion {
            guest_start: phys(0x1000),
            arena_offset: 0,
            len: 4096,
            kind: RegionKind::Ram,
        },
        PhysicalRegion {
            guest_start: phys(0x5000),
            arena_offset: 4096,
            len: 4096,
            kind: RegionKind::Ram,
        },
    ];
    let mut memory = MappedMemory::new(&mut bytes, &mut stamps, &regions)?;
    let mut session = MemorySession::new(&mut cpu, memory.loan());
    session.reset_at(EntryMode::Real16, ENTRY, 0x5FFC)?;
    // mov [0x5000],ax; hlt
    session
        .memory()
        .write_phys(phys(ENTRY), &[0xA3, 0, 0x50, 0xF4])?;
    session.regs().write(AX, 0x1234);
    assert!(matches!(session.step().stop, SessionStop::Budget));
    let mut stored = [0; 2];
    session.memory().read_phys(phys(0x5000), &mut stored)?;
    assert_eq!(u16::from_le_bytes(stored), 0x1234);
    println!(
        "caller-owned sparse RAM received {:#06x}",
        u16::from_le_bytes(stored)
    );
    Ok(())
}
