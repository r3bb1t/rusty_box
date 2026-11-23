//! Host-only snapshots of the active task on the pinned Debian i386 build.

mod layout;
mod maps;

use layout::*;
use rusty_box::Emulator;
use rusty_box::config::BxLinAddress;
use rusty_box::cpu::registers::{CR0, CR3, CR4, CS, ESP, RIP};
use rusty_box::cpu::{CodeBreakpoint, Instrumentation};
use rusty_box_core::events::{Emit, ExecutionRequest};
use sha2::{Digest, Sha256};

use super::metadata::{self, AuxHeaders};
use super::{AnalysisError, MappedElf, MappedRegion, read_guest};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ObservationError {
    #[error("boot image does not match Debian 6.1.0-50-686 (6.1.176-1)")]
    UnknownKernel,
    #[error("running kernel build ID disagrees with the pinned layout")]
    KernelIdentity,
    #[error(
        "observation requires a single CPU with non-PAE paging at user mode or its pinned return boundary"
    )]
    UnsupportedBoundary,
    #[error("current Linux task has no user address space")]
    NoAddressSpace,
    #[error("active CR3 {cr3:#x} disagrees with the task's page-table root {pgd:#x}")]
    AddressSpaceMismatch { cr3: u64, pgd: u64 },
    #[error("current address space has an active mmap writer")]
    BusyMappings,
    #[error("the address space has no executable file")]
    NoExecutable,
    #[error("guest memory at {address:#x} is unavailable")]
    Unavailable { address: u64 },
    #[error("unsupported or invalid Linux state: {0}")]
    Invalid(&'static str),
}

/// Raw Linux IDs in the initial PID namespace and the kernel's per-task
/// start time. The identity is scoped to one boot and its forward execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskIdentity {
    /// Thread-group ID in the initial PID namespace.
    pub pid: u32,
    /// Task ID in the initial PID namespace.
    pub tid: u32,
    pub start_boottime_ns: u64,
    pub task_address: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplePoint {
    UserInstruction,
    PendingUserReturn,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProcessSample {
    pub task: TaskIdentity,
    pub mm_address: u32,
    pub cr3: u64,
    pub command: String,
    pub executable_basename: Option<String>,
    pub point: SamplePoint,
    /// Live user PC or the saved user PC of the pending return.
    pub user_ip: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ExecutableSample {
    pub task: TaskIdentity,
    pub mm_address: u32,
    pub file_address: u32,
    /// `AT_ENTRY` from this mm's saved auxiliary vector, when present.
    pub entry: Option<u64>,
    /// Executable VMAs backed by this mm's exact `exe_file`. A VMA can exist
    /// even when its pages are absent from the active hardware translation.
    pub mappings: Vec<MappedRegion>,
}

pub struct Observation {
    pub process: ProcessSample,
    pub executable: ExecutableSample,
}

impl Observation {
    /// Publish process then executable. A stop is collected across both
    /// events. The first subscriber error takes precedence over a stop.
    pub fn deliver<S, X>(&self, sink: &mut S) -> Result<ExecutionRequest, X>
    where
        S: Emit<ProcessSample, Error = X> + Emit<ExecutableSample, Error = X>,
    {
        let process = sink.emit(&self.process, &mut ())?;
        let executable = sink.emit(&self.executable, &mut ())?;
        Ok(if [process, executable].contains(&ExecutionRequest::Stop) {
            ExecutionRequest::Stop
        } else {
            ExecutionRequest::None
        })
    }
}

/// The supported i386 layout, selected by the exact boot image hash.
pub struct Observer(());

impl Observer {
    pub fn for_kernel(bzimage: &[u8]) -> Result<Self, ObservationError> {
        if format!("{:x}", Sha256::digest(bzimage)) != KERNEL_SHA256 {
            return Err(ObservationError::UnknownKernel);
        }
        Ok(Self(()))
    }

    /// A kernel boundary shared by normal userspace returns on the pinned
    /// build. No address from an observed executable selects this breakpoint.
    pub fn return_breakpoint(&self) -> CodeBreakpoint {
        CodeBreakpoint::Linear(u64::from(RETURN_TO_USER))
    }

    /// Read the active task while exclusively borrowing the stopped machine.
    /// The caller controls the run boundary and may discard a recoverable error.
    pub fn at_pause<'m, T: Instrumentation>(
        &self,
        machine: &'m mut Emulator<T>,
    ) -> Result<PausedProcess<'m, T>, ObservationError> {
        if machine.debug_cpu(1).is_some() {
            return Err(ObservationError::UnsupportedBoundary);
        }
        let regs = machine.regs();
        let rip = regs.read(RIP);
        let esp = regs.read(ESP);
        let privilege = regs.read(CS) & 3;
        let cr3 = regs.read(CR3);
        if regs.read(CR0) & (1 << 31) == 0 || regs.read(CR4) & (1 << 5) != 0 {
            return Err(ObservationError::UnsupportedBoundary);
        }
        let mut memory = GuestMemory(machine);
        if memory.bytes::<20>(BUILD_ID_ADDRESS, 0)? != *BUILD_ID {
            return Err(ObservationError::KernelIdentity);
        }
        let (point, user_ip) = match privilege {
            3 if rip < u64::from(KERNEL_BASE) => (SamplePoint::UserInstruction, rip as u32),
            0 if rip == u64::from(RETURN_TO_USER) => {
                let frame = kernel_pointer(esp)?;
                let ip = memory.word(frame, PT_IP)?;
                let cs = u16::from_le_bytes(memory.bytes(frame, PT_CS)?);
                if cs & 3 != 3 || ip >= KERNEL_BASE {
                    return Err(ObservationError::UnsupportedBoundary);
                }
                (SamplePoint::PendingUserReturn, ip)
            }
            _ => return Err(ObservationError::UnsupportedBoundary),
        };

        // Linux's SHIFT_PERCPU_PTR adds the CPU's offset in pointer width.
        let offset = memory.word(PER_CPU_OFFSETS, 0)?;
        let task = kernel_pointer(memory.word(CURRENT_TASK.wrapping_add(offset), 0)?)?;
        let mm = memory.word(task, TASK_MM)?;
        if mm == 0 {
            return Err(ObservationError::NoAddressSpace);
        }
        let mm = kernel_pointer(mm)?;
        let directory = kernel_pointer(memory.word(mm, MM_PGD)?)?;
        let pgd = memory.physical(directory)?;
        if pgd & 0xFFF != 0 || pgd != (cr3 & !0xFFF) {
            return Err(ObservationError::AddressSpaceMismatch { cr3, pgd });
        }
        let pid = memory.word(task, TASK_TGID)?;
        let tid = memory.word(task, TASK_PID)?;
        if !(1..=i32::MAX as u32).contains(&pid) || !(1..=i32::MAX as u32).contains(&tid) {
            return Err(ObservationError::Invalid("task PID or TID"));
        }
        let comm = memory.bytes::<16>(task, TASK_COMM)?;
        let comm_end = comm
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(comm.len());
        let exe = memory.word(mm, MM_EXE_FILE)?;
        let basename = (exe != 0).then(|| memory.basename(exe)).transpose()?;
        let process = ProcessSample {
            task: TaskIdentity {
                pid,
                tid,
                start_boottime_ns: u64::from_le_bytes(memory.bytes(task, TASK_START)?),
                task_address: task,
            },
            mm_address: mm,
            cr3,
            command: String::from_utf8_lossy(&comm[..comm_end]).into_owned(),
            executable_basename: basename,
            point,
            user_ip,
        };
        Ok(PausedProcess {
            memory,
            process,
            exe,
        })
    }
}

pub struct PausedProcess<'m, T: Instrumentation> {
    memory: GuestMemory<'m, T>,
    process: ProcessSample,
    exe: u32,
}

impl<'m, T: Instrumentation> PausedProcess<'m, T> {
    pub fn sample(&self) -> &ProcessSample {
        &self.process
    }

    /// Keep the process sample and release the machine without walking VMAs.
    pub fn into_sample(self) -> ProcessSample {
        self.process
    }

    /// Retain the pause across the VMA walk so its identity and mappings
    /// come from one execution boundary.
    pub fn executable(mut self) -> Result<PausedExecutable<'m, T>, ObservationError> {
        if self.exe == 0 {
            return Err(ObservationError::NoExecutable);
        }
        let mm = self.process.mm_address;
        // rwsem.c sets bit zero while an mmap writer owns the semaphore.
        if self.memory.word(mm, MM_LOCK_COUNT)? & 1 != 0 {
            return Err(ObservationError::BusyMappings);
        }
        let aux = AuxVector::read(self.memory.bytes(mm, MM_AUXV)?)?;
        let ranges = maps::walk(&mut self.memory, mm, self.exe, aux.phdr)?;
        let executable = ExecutableSample {
            task: self.process.task,
            mm_address: mm,
            file_address: self.exe,
            entry: (aux.entry != 0).then_some(aux.entry),
            mappings: ranges.executable,
        };
        Ok(PausedExecutable {
            memory: self.memory,
            observation: Observation {
                process: self.process,
                executable,
            },
            aux,
            header: ranges.header,
        })
    }
}

pub struct PausedExecutable<'m, T: Instrumentation> {
    memory: GuestMemory<'m, T>,
    observation: Observation,
    aux: AuxVector,
    header: Option<MappedRegion>,
}

impl<T: Instrumentation> PausedExecutable<'_, T> {
    /// Optional ELF inspection uses only bytes that are already mapped into
    /// the active guest. Reading the executable VMAs does not perform it.
    pub fn inspect_elf(&mut self) -> Result<MappedElf, AnalysisError> {
        metadata::inspect(
            self.header,
            AuxHeaders {
                phdr: self.aux.phdr,
                phent: self.aux.phent,
                phnum: self.aux.phnum,
            },
            self.aux.entry,
            &self.observation.executable.mappings,
            |address, buffer| read_guest(self.memory.0, address, buffer),
        )
    }

    /// Release the machine loan before delivering the owned snapshot.
    pub fn into_observation(self) -> Observation {
        self.observation
    }
}

#[derive(Default)]
struct AuxVector {
    entry: u64,
    phdr: u64,
    phent: u32,
    phnum: u32,
}

impl AuxVector {
    fn read(raw: [u8; MM_AUXV_SIZE]) -> Result<Self, ObservationError> {
        let mut fields = Self::default();
        for [a, b, c, d, e, f, g, h] in raw.as_chunks::<8>().0.iter().copied() {
            let value = u32::from_le_bytes([e, f, g, h]);
            match u32::from_le_bytes([a, b, c, d]) {
                0 => return Ok(fields),
                3 => fields.phdr = u64::from(value),
                4 => fields.phent = value,
                5 => fields.phnum = value,
                9 => fields.entry = u64::from(value),
                _ => {}
            }
        }
        Err(ObservationError::Invalid(
            "unterminated saved auxiliary vector",
        ))
    }
}

fn kernel_pointer(address: u32) -> Result<u32, ObservationError> {
    if address < KERNEL_BASE || address & 3 != 0 {
        return Err(ObservationError::Invalid("unaligned or non-kernel pointer"));
    }
    Ok(address)
}

struct GuestMemory<'m, T: Instrumentation>(&'m mut Emulator<T>);

impl<T: Instrumentation> GuestMemory<'_, T> {
    fn read(&mut self, address: u32, out: &mut [u8]) -> Result<(), ObservationError> {
        if address < KERNEL_BASE || u64::from(address) + out.len() as u64 > 1 << 32 {
            return Err(ObservationError::Invalid("kernel read extent"));
        }
        read_guest(self.0, u64::from(address), out).map_err(|error| match error {
            AnalysisError::Missing { address } => ObservationError::Unavailable { address },
            _ => ObservationError::Invalid("unexpected guest RAM read error"),
        })
    }

    fn bytes<const N: usize>(
        &mut self,
        base: u32,
        offset: u32,
    ) -> Result<[u8; N], ObservationError> {
        let address = base
            .checked_add(offset)
            .ok_or(ObservationError::Invalid("kernel field address"))?;
        let mut out = [0; N];
        self.read(address, &mut out)?;
        Ok(out)
    }

    fn word(&mut self, base: u32, offset: u32) -> Result<u32, ObservationError> {
        Ok(u32::from_le_bytes(self.bytes(base, offset)?))
    }

    fn physical(&mut self, address: u32) -> Result<u64, ObservationError> {
        let address = u64::from(address);
        self.0
            .debug_cpu(0)
            .and_then(|mut cpu| cpu.dbg_xlate_linear2phy(BxLinAddress::new(address)))
            .map(|physical| physical.get())
            .ok_or(ObservationError::Unavailable { address })
    }

    fn basename(&mut self, file: u32) -> Result<String, ObservationError> {
        let dentry = kernel_pointer(self.word(kernel_pointer(file)?, FILE_DENTRY)?)?;
        let name = self.word(dentry, DENTRY_NAME)?;
        let len = self.word(dentry, DENTRY_NAME_LEN)? as usize;
        let mut bytes = [0; 255];
        let data = bytes
            .get_mut(..len)
            .filter(|data| !data.is_empty())
            .ok_or(ObservationError::Invalid("dentry name length"))?;
        self.read(name, data)?;
        if data.contains(&0) || data.contains(&b'/') {
            return Err(ObservationError::Invalid("dentry basename"));
        }
        Ok(String::from_utf8_lossy(data).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use rusty_box::cpu::instrumentation::CpuSetupMode;
    use rusty_box::emulator::EmulatorConfig;
    use rusty_box_core::events::App;

    use super::super::semantic::{Guest, MetadataRequest};
    use super::*;

    #[test]
    fn rejects_unknown_kernel_boundaries_and_unavailable_memory() {
        assert!(matches!(
            Observer::for_kernel(b"unrecognized"),
            Err(ObservationError::UnknownKernel)
        ));
        let mut machine =
            Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::RealMode).unwrap();
        assert!(matches!(
            Observer(()).at_pause(&mut machine),
            Err(ObservationError::UnsupportedBoundary)
        ));
        let mut memory = GuestMemory(&mut machine);
        assert!(matches!(
            memory.word(0xF000_0000, 0),
            Err(ObservationError::Unavailable { .. })
        ));
        assert!(matches!(
            AuxVector::read([0xFF; MM_AUXV_SIZE]),
            Err(ObservationError::Invalid(_))
        ));
    }

    #[test]
    fn owned_delivery_preserves_order_stop_and_error() {
        let task = TaskIdentity {
            pid: 2,
            tid: 2,
            start_boottime_ns: 10,
            task_address: 0xC000_0000,
        };
        let sample = Observation {
            process: ProcessSample {
                task,
                mm_address: 0xC000_1000,
                cr3: 0x8000,
                command: "test".to_owned(),
                executable_basename: None,
                point: SamplePoint::PendingUserReturn,
                user_ip: 0x2000,
            },
            executable: ExecutableSample {
                task,
                mm_address: 0xC000_1000,
                file_address: 0xC000_2000,
                entry: None,
                mappings: vec![],
            },
        };
        #[derive(Default)]
        struct State(Vec<u8>);
        let handlers = App::<_, &'static str>::fallible()
            .route(|_: &ProcessSample, state: &mut State| {
                state.0.push(1);
                Ok(ExecutionRequest::Stop)
            })
            .route(|_: &ProcessSample, state: &mut State| state.0.push(2));
        let mut stopped = handlers
            .route(|_: &ExecutableSample, state: &mut State| state.0.push(3))
            .with_state(State::default());
        assert_eq!(sample.deliver(&mut stopped), Ok(ExecutionRequest::Stop));
        assert_eq!(stopped.into_state().0, [1, 2, 3]);

        let mut faulty = App::<_, &'static str>::fallible()
            .route(|_: &ProcessSample| Ok(ExecutionRequest::Stop))
            .route(|_: &ExecutableSample, state: &mut State| {
                state.0.push(3);
                Err("later executable error")
            })
            .route(|_: &ExecutableSample, state: &mut State| state.0.push(4))
            .with_state(State::default());
        assert_eq!(sample.deliver(&mut faulty), Err("later executable error"));
        assert_eq!(faulty.into_state().0, [3]);
    }

    #[test]
    #[ignore = "requires the pinned Debian kernel and the supervised Milestone 7 initramfs"]
    fn host_snapshot_matches_independent_ptrace_procfs_oracle() {
        let kernel = std::fs::read(std::env::var_os("RB_OBSERVATION_KERNEL").unwrap()).unwrap();
        let initramfs =
            std::fs::read(std::env::var_os("RB_OBSERVATION_INITRAMFS").unwrap()).unwrap();
        let observer = Observer::for_kernel(&kernel).unwrap();
        let mut guest = Guest::boot(&kernel, &initramfs).unwrap();
        let mut previous = None;
        for (program, output) in [
            ("/bin/program_sum", "sum=22"),
            ("/bin/program_text", "argv0=/bin/program_text"),
        ] {
            let mut paused = guest
                .observe(program, MetadataRequest::ReadMappedElf)
                .unwrap();
            let mut executable = observer
                .at_pause(paused.stopped_machine())
                .unwrap()
                .executable()
                .unwrap();
            let mut timing = [0; 256];
            let elf = executable.inspect_elf().unwrap();
            for ns in &mut timing {
                let start = Instant::now();
                let result = executable.inspect_elf();
                *ns = start.elapsed().as_nanos();
                assert!(std::hint::black_box(result).is_ok());
            }
            timing.sort_unstable();
            println!(
                "{program} resident ELF median {} ns",
                timing[timing.len() / 2]
            );
            let host = executable.into_observation();
            let oracle = paused.observation();
            assert_eq!(host.process.task.pid, oracle.process.pid);
            assert_eq!(host.process.task.tid, oracle.process.pid);
            assert_ne!(Some(host.process.task), previous);
            assert_eq!(
                host.process.executable_basename.as_deref(),
                program.rsplit('/').next()
            );
            assert_eq!(host.process.point, SamplePoint::UserInstruction);
            assert_eq!(host.process.cr3, oracle.process.space.page_table_root);
            assert_eq!(host.process.user_ip as u64, oracle.image.entry);
            assert_eq!(host.executable.entry, Some(oracle.image.entry));
            assert_eq!(host.executable.mappings, oracle.image.executable_regions);
            assert_eq!(
                &elf,
                oracle.metadata.as_ref().unwrap().result.as_ref().unwrap()
            );
            println!(
                "{program} host and oracle agree on PID {}, CR3 {:#x}, entry {:#x}, and {} executable VMA(s)",
                host.process.task.pid,
                host.process.cr3,
                oracle.image.entry,
                host.executable.mappings.len()
            );
            previous = Some(host.process.task);
            let completed = paused.finish().unwrap();
            assert_eq!(completed.wait_status, 0);
            assert_eq!(completed.output, [output]);
            guest = completed.guest;
        }
    }
}
