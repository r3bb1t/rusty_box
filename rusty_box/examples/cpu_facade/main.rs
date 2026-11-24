//! Run a guest function with owned x86 RAM and service its port output.
use rusty_box::config::BxPhyAddress;
use rusty_box::cpu::instrumentation::HookMask;
use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::cpu::registers::{AL, EAX, ECX, EDX, EIP};
use rusty_box::cpu::{CpuEmulator, CpuError, Instruction, Instrumentation, ProcessorActivity};

const ENTRY: u32 = 0x1000;
const RETURN: u32 = 0x1100;
const STACK: u32 = 0x2FFC;

fn phys(address: u32) -> BxPhyAddress {
    BxPhyAddress::new(u64::from(address))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut emu = CpuEmulator::builder()
        .ram(0x3000)
        .entry(ENTRY)
        .stack(STACK)
        .build_with_tracer(CountInstructions::default())?;

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

    match emu.step().stop {
        SessionStop::Budget => {}
        stop => return Err(std::io::Error::other(format!("unexpected step stop {stop:?}")).into()),
    }
    let intermediate = emu.regs().read(EAX);
    println!("After one step, EAX = {intermediate}");
    emu.regs().write(EDX, 10);
    match emu.run(16).stop {
        SessionStop::Exit(CpuError::IoUnavailable { string: false, .. }) => {}
        other => return Err(std::io::Error::other(format!("unexpected stop {other:?}")).into()),
    }
    let pc = emu.regs().read(EIP);
    println!("Port output at EIP {pc:#x}, AL = {}", emu.regs().read(AL));
    let serviced = emu.service_scalar_io(|request| {
        match (request.port.get(), request.output) {
            (0x80, Some(value)) => println!("Guest wrote {value} to port 0x80"),
            _ => return None,
        }
        Some(0)
    })?;
    if !serviced {
        return Err(std::io::Error::other("no supported port output to service").into());
    }

    match emu.run(1).stop {
        SessionStop::Inactive(ProcessorActivity::Halted) => {
            println!(
                "Halted after {} instruction callbacks",
                emu.instrumentation().observed
            );
            Ok(())
        }
        stop => Err(std::io::Error::other(format!("unexpected stop {stop:?}")).into()),
    }
}

#[derive(Default)]
struct CountInstructions {
    observed: u64,
}

impl Instrumentation for CountInstructions {
    fn active_hooks(&self) -> HookMask {
        HookMask::EXEC
    }

    fn before_execution(&mut self, _ip: u64, _instruction: &Instruction) {
        self.observed += 1;
    }
}
