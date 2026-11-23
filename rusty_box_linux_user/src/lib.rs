//! A synchronous Linux/i386 process environment over the rusty_box x86 interpreter.
//!
//! Supported programs are fixed-address, static ELF32 executables that use
//! `int 0x80` and the `exit`, `read`, or `write` system calls. The adapter owns
//! the CPU, page tables, stack and backing. The caller supplies standard streams.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod elf;
mod i386;
mod io;
mod memory;
mod stack;

pub use elf::{ElfError, Image};
pub use io::{BufferedIo, Io, IoError, Output};
pub use memory::MemoryError;
use rusty_box::cpu::memory_session::SessionStop;
use rusty_box::cpu::registers::{EAX, EBX, ECX, EDX};
use rusty_box::cpu::{CpuEmulator, CpuError};
pub use stack::StackError;

use crate::i386::Boundary;
use crate::memory::AddressSpace;
use crate::stack::Stack;

const SYS_EXIT: u32 = 1;
const SYS_READ: u32 = 3;
const SYS_WRITE: u32 = 4;
const EBADF: i32 = 9;
const EFAULT: i32 = 14;
/// A successful transfer can be short. The adapter handles at most this many
/// bytes per supported call and uses a fixed buffer independent of guest input.
const MAX_IO: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error(transparent)]
    Stack(#[from] StackError),
    #[error(transparent)]
    Memory(#[from] MemoryError),
    #[error(transparent)]
    Engine(#[from] rusty_box::Error),
    #[error("x86 bootstrap failed at IP {ip:#x} with {stop:?}")]
    Bootstrap { ip: u32, stop: SessionStop },
}

/// Initial argument and environment byte strings without their trailing NUL.
#[derive(Clone, Copy, Debug, Default)]
pub struct Launch<'a> {
    pub args: &'a [&'a [u8]],
    pub env: &'a [&'a [u8]],
}

/// An owned lifecycle event that can be published through `Emit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgramStarted {
    pub entry: u32,
    pub initial_stack: u32,
}

/// Low eight bits of Linux `exit`'s argument, equivalent to `WEXITSTATUS`.
/// This value is also a typed application event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgramExited {
    pub status: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestFault {
    pub vector: u8,
    /// Saved guest EIP. For x86 traps this may name the following instruction.
    pub instruction: u32,
    pub error_code: Option<u32>,
    /// Faulting linear address for a page fault.
    pub address: Option<u32>,
}

#[derive(Debug)]
pub enum Stop {
    Budget,
    Exited(ProgramExited),
    Fault(GuestFault),
    UnsupportedSystemCall { number: u32 },
    Engine(SessionStop),
}

#[derive(Debug)]
pub struct Run {
    pub stop: Stop,
    /// Engine count delta including supervisor instructions. REP and fault
    /// accounting follows `CpuEmulator`. It can exceed the dispatch budget.
    pub engine_count: u64,
    /// Number of ABI requests acknowledged during this call.
    pub syscalls: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("the process is already finished")]
    Finished,
    #[error("the I/O provider reported {transferred} bytes for a {offered}-byte buffer")]
    IoCount { transferred: usize, offered: usize },
    #[error("the CPU did not complete the pending supervisor port instruction")]
    Unacknowledged,
    #[error(transparent)]
    Boundary(#[from] CpuError),
    #[error(transparent)]
    Engine(#[from] rusty_box::Error),
    #[error(transparent)]
    Memory(#[from] MemoryError),
}

/// One process and its I/O provider. Each launch starts with fresh CPU and
/// memory. The provider is owned by value and may itself borrow caller data.
/// Keep this value across budget returns to resume the same process.
///
/// The I/O view cannot stay borrowed across a run.
///
/// ```compile_fail,E0502
/// use rusty_box_linux_user::{Io, LinuxI386User};
///
/// fn overlapping_io_access<I: Io>(process: &mut LinuxI386User<I>) {
///     let streams = process.io();
///     let result = process.run(1);
///     let retained = (streams, result);
///     drop(retained);
/// }
/// ```
pub struct LinuxI386User<I> {
    cpu: CpuEmulator,
    space: AddressSpace,
    io: I,
    start: ProgramStarted,
    pending_syscall: bool,
    finished: bool,
}

impl Image<'_> {
    pub fn launch<I: Io>(&self, launch: Launch<'_>, io: I) -> Result<LinuxI386User<I>, BuildError> {
        let stack = Stack::build(self, launch.args, launch.env)?;
        let space = AddressSpace::new(self);
        let tables = space.tables();
        let mut cpu = CpuEmulator::builder()
            .ram(tables.ram_size())
            .entry(i386::BOOT)
            .stack(i386::BOOT_STACK)
            .build()?;
        space.load(&mut cpu, self)?;
        space.write(&mut cpu, stack.pointer(), stack.bytes())?;
        tables.install(&mut cpu)?;
        i386::enter(&mut cpu, self.entry(), stack.pointer())?;
        let start = ProgramStarted {
            entry: self.entry(),
            initial_stack: stack.pointer(),
        };
        Ok(LinuxI386User {
            cpu,
            space,
            io,
            start,
            pending_syscall: false,
            finished: false,
        })
    }
}

impl<I: Io> LinuxI386User<I> {
    pub fn started(&self) -> ProgramStarted {
        self.start
    }

    pub fn io(&self) -> &I {
        &self.io
    }

    pub fn into_io(self) -> I {
        self.io
    }

    /// Read checked, guest-readable addresses from the stable process map.
    /// Supervisor frames and unmapped guard pages are outside this view.
    pub fn read_memory(&mut self, address: u32, out: &mut [u8]) -> Result<(), MemoryError> {
        self.space.read(&mut self.cpu, address, out)
    }

    /// Run a bounded slice. Supervisor work counts against the same dispatch
    /// budget. A zero budget does no work. Budget returns preserve progress;
    /// all other stops and errors end the process and prohibit a retry.
    pub fn run(&mut self, dispatch_budget: u64) -> Result<Run, RunError> {
        if self.finished {
            return Err(RunError::Finished);
        }
        // A provider that unwinds after committing an effect leaves this
        // process terminal even if its caller catches the unwind.
        self.finished = true;
        let result = self.run_slice(dispatch_budget);
        self.finished = !matches!(
            &result,
            Ok(Run {
                stop: Stop::Budget,
                ..
            })
        );
        result
    }

    fn run_slice(&mut self, budget: u64) -> Result<Run, RunError> {
        let mut run = Run {
            stop: Stop::Budget,
            engine_count: 0,
            syscalls: 0,
        };
        while run.engine_count < budget {
            if self.pending_syscall {
                // This completes the already validated OUT exactly once and
                // preserves registers. Finish it before editing registers or RAM,
                // which invalidates a pending scalar port operation in the core.
                if !self.cpu.service_scalar_io(|_| Some(0))? {
                    return Err(RunError::Unacknowledged);
                }
                self.pending_syscall = false;
                run.engine_count += 1;
                run.syscalls += 1;
                if let Some(stop) = self.system_call()? {
                    run.stop = stop;
                    break;
                }
                continue;
            }
            let cpu_run = self.cpu.run(budget - run.engine_count);
            run.engine_count = run.engine_count.saturating_add(cpu_run.instructions);
            if matches!(cpu_run.stop, SessionStop::Budget) {
                break;
            }
            match i386::boundary(&mut self.cpu, &cpu_run.stop) {
                Some(Boundary::SystemCall) => self.pending_syscall = true,
                Some(Boundary::Fault(vector)) => {
                    run.stop = Stop::Fault(i386::fault(&mut self.cpu, vector)?);
                    break;
                }
                None => {
                    run.stop = Stop::Engine(cpu_run.stop);
                    break;
                }
            }
        }
        Ok(run)
    }

    fn system_call(&mut self) -> Result<Option<Stop>, RunError> {
        let regs = self.cpu.regs();
        let number = regs.read(EAX);
        let fd = regs.read(EBX);
        let address = regs.read(ECX);
        let len = (regs.read(EDX) as usize).min(MAX_IO);
        let result = match number {
            SYS_EXIT => return Ok(Some(Stop::Exited(ProgramExited { status: fd as u8 }))),
            SYS_READ if fd == 0 => self.read_stdin(address, len)?,
            SYS_WRITE if fd == 1 || fd == 2 => {
                let stream = if fd == 1 {
                    Output::Stdout
                } else {
                    Output::Stderr
                };
                self.write_stream(stream, address, len)?
            }
            SYS_READ | SYS_WRITE => -EBADF,
            _ => return Ok(Some(Stop::UnsupportedSystemCall { number })),
        };
        // Linux i386 returns only EAX; the architectural IRET restores the
        // original user IP, CS, flags and stack from the frame.
        self.cpu.regs().write(EAX, result as u32);
        Ok(None)
    }

    fn read_stdin(&mut self, address: u32, len: usize) -> Result<i32, RunError> {
        if len == 0 {
            return Ok(0);
        }
        if self.space.validate(address, len, true).is_err() {
            return Ok(-EFAULT);
        }
        let mut buffer = [0; MAX_IO];
        let count = match self.io.read_stdin(&mut buffer[..len]) {
            Ok(count) => checked_count(count, len)?,
            Err(error) => return Ok(-error.errno()),
        };
        self.space.write(&mut self.cpu, address, &buffer[..count])?;
        Ok(count as i32)
    }

    fn write_stream(&mut self, stream: Output, address: u32, len: usize) -> Result<i32, RunError> {
        if len == 0 {
            return Ok(0);
        }
        let mut buffer = [0; MAX_IO];
        match self.space.read(&mut self.cpu, address, &mut buffer[..len]) {
            Ok(()) => {}
            Err(MemoryError::Backing(error)) => return Err(error.into()),
            Err(_) => return Ok(-EFAULT),
        }
        match self.io.write(stream, &buffer[..len]) {
            Ok(count) => Ok(checked_count(count, len)? as i32),
            Err(error) => Ok(-error.errno()),
        }
    }
}

fn checked_count(transferred: usize, offered: usize) -> Result<usize, RunError> {
    if transferred > offered {
        return Err(RunError::IoCount {
            transferred,
            offered,
        });
    }
    Ok(transferred)
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
