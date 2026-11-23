//! Observe one supervised Linux/i386 exec at its first user instruction.

mod metadata;

pub use metadata::MappedElf;
use rusty_box::cpu::CodeBreakpoint;
use rusty_box::cpu::registers::{CR3, CS, ESP, RIP};
use rusty_box::emulator::StopReason;
use rusty_box_core::events::{Emit, ExecutionRequest};

use super::console::{Console, ConsoleResult};
use super::{AddressSpace, AnalysisError, next_address_space};

#[derive(Debug, PartialEq, Eq)]
pub struct ProcessObserved {
    pub pid: u32,
    pub executable: String,
    pub space: AddressSpace,
}

/// A half-open file mapping reported by `/proc/<pid>/maps` at exec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappedRegion {
    pub start: u64,
    pub end: u64,
    pub file_offset: u64,
}

impl MappedRegion {
    fn contains(self, address: u64) -> bool {
        (self.start..self.end).contains(&address)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ImageObserved {
    pub pid: u32,
    pub space: AddressSpace,
    pub entry: u64,
    pub executable_regions: Vec<MappedRegion>,
}

/// Optional inspection belongs to this observation boundary. Both success and
/// failure are delivered as an owned result shared by all metadata handlers.
#[derive(Debug, PartialEq, Eq)]
pub struct ImageMetadata {
    pub space: AddressSpace,
    pub result: Result<MappedElf, AnalysisError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataRequest {
    Skip,
    ReadMappedElf,
}

pub struct Observation {
    pub process: ProcessObserved,
    pub image: ImageObserved,
    pub metadata: Option<ImageMetadata>,
}

impl Observation {
    /// Deliver process, image, then optional metadata to the same sink.
    /// A stop is collected across the observation. The first handler error
    /// ends delivery and takes precedence over an earlier stop.
    pub fn deliver<S, X>(&self, sink: &mut S) -> Result<ExecutionRequest, X>
    where
        S: Emit<ProcessObserved, Error = X>
            + Emit<ImageObserved, Error = X>
            + Emit<ImageMetadata, Error = X>,
    {
        let process = sink.emit(&self.process, &mut ())?;
        let image = sink.emit(&self.image, &mut ())?;
        let metadata = match &self.metadata {
            Some(metadata) => sink.emit(metadata, &mut ())?,
            None => ExecutionRequest::None,
        };
        Ok(
            if [process, image, metadata].contains(&ExecutionRequest::Stop) {
                ExecutionRequest::Stop
            } else {
                ExecutionRequest::None
            },
        )
    }
}

/// An isolated uniprocessor guest booted with the observation initramfs.
pub struct Guest(Console);

impl Guest {
    pub fn boot(kernel: &[u8], initramfs: &[u8]) -> ConsoleResult<Self> {
        let mut console = Console::boot(kernel, initramfs)?;
        if console.machine().debug_cpu(1).is_some() {
            return Err("exec observation requires one guest processor".into());
        }
        Ok(Self(console))
    }

    /// Launch one test-owned static executable and prove it reached user entry.
    /// The guest is transferred to the paused result. A failed observation
    /// discards the isolated machine and its outstanding tracee.
    pub fn observe(mut self, executable: &str, request: MetadataRequest) -> ConsoleResult<Paused> {
        if !executable.starts_with("/bin/")
            || executable.len() >= 128
            || !executable
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/_.-".contains(&byte))
        {
            return Err("expected a fixture path under /bin using plain ASCII".into());
        }

        self.0.clear_output();
        self.0.command(&format!("/bin/rb-observe {executable}"))?;
        wait_record(&mut self.0, "supervised exec", |line| line == "RB_WAIT")?;
        let record = ExecRecord::parse(self.0.lines(), executable)?;
        self.0.clear_output();

        let breakpoint = CodeBreakpoint::Linear(record.entry);
        self.0.machine().set_code_breakpoints(&[breakpoint])?;
        self.0.command(match request {
            MetadataRequest::Skip => "r",
            MetadataRequest::ReadMappedElf => "m",
        })?;
        let cpu = self.wait_entry(breakpoint)?;
        let observation = record.observation(cpu, request, |address, bytes| {
            super::read_guest(self.0.machine(), address, bytes)
        })?;

        Ok(Paused {
            guest: self,
            observation,
        })
    }

    fn wait_entry(&mut self, breakpoint: CodeBreakpoint) -> ConsoleResult<EntryCpu> {
        for _ in 0..600 {
            match self.0.pump()? {
                StopReason::Breakpoint(hit) if hit.cpu == 0 && hit.breakpoint == breakpoint => {
                    let regs = self.0.machine().regs();
                    if regs.read(CS) & 3 != 3 || regs.read(RIP) != hit.linear {
                        return Err(AnalysisError::WrongBoundary.into());
                    }
                    return Ok(EntryCpu {
                        entry: hit.linear,
                        stack: u64::from(regs.read(ESP)),
                        root: regs.read(CR3),
                    });
                }
                StopReason::BudgetExhausted | StopReason::Halted => {}
                stop => return Err(format!("unexpected entry stop {stop:?}").into()),
            }
            if let Some(error) = self.0.lines().find_map(guest_error) {
                return Err(error.into());
            }
        }
        Err(format!("guest did not reach the observed entry {:?}", self.0.tail()).into())
    }
}

/// A guest stopped before its first executable instruction. Drop it to stop
/// the workflow, or finish it after handling the prepared observation.
pub struct Paused {
    guest: Guest,
    observation: Observation,
}

impl Paused {
    pub fn observation(&self) -> &Observation {
        &self.observation
    }

    pub fn finish(mut self) -> ConsoleResult<Completed> {
        let console = &mut self.guest.0;
        console.machine().set_code_breakpoints(&[])?;
        console.machine().stop_handle().reset();
        let status = wait_record(console, "child exit", |line| line.starts_with("RB_STATUS "))?;
        let [pid, raw] = hex_words("RB_STATUS", &status)?;
        if pid != self.observation.process.pid {
            return Err("waitpid returned a different child".into());
        }
        let mut lines = console.lines().skip_while(|line| *line != "RB_RUNNING");
        if lines.next().is_none() {
            return Err("missing supervisor resume record".into());
        }
        let output = lines
            .take_while(|line| !line.starts_with("RB_STATUS "))
            .map(str::to_owned)
            .collect();
        Ok(Completed {
            guest: self.guest,
            output,
            wait_status: u16::try_from(raw)?,
        })
    }
}

pub struct Completed {
    pub guest: Guest,
    pub output: Vec<String>,
    /// Raw Linux wait status. Zero means normal exit with status zero.
    pub wait_status: u16,
}

fn guest_error(line: &str) -> Option<String> {
    line.strip_prefix("RB_ERROR ")
        .map(|error| format!("guest supervisor failed during {error}"))
}

fn wait_record<F: FnMut(&str) -> bool>(
    console: &mut Console,
    label: &str,
    mut wanted: F,
) -> ConsoleResult<String> {
    let record = console.wait_for(label, 600, |line| {
        if let Some(error) = guest_error(line) {
            Some(Err(error))
        } else {
            wanted(line).then(|| Ok(line.to_owned()))
        }
    })?;
    Ok(record?)
}

fn hex_words<const N: usize>(tag: &str, line: &str) -> ConsoleResult<[u32; N]> {
    let mut fields = line.split_ascii_whitespace();
    if fields.next() != Some(tag) {
        return Err("unexpected supervisor record".into());
    }
    let mut words = [0; N];
    for word in &mut words {
        let field = fields
            .next()
            .filter(|field| field.len() == 8)
            .ok_or("invalid record width")?;
        *word = u32::from_str_radix(field, 16)?;
    }
    if fields.next().is_some() {
        return Err("extra supervisor record fields".into());
    }
    Ok(words)
}

struct EntryCpu {
    entry: u64,
    stack: u64,
    root: u64,
}

struct ExecRecord {
    pid: u32,
    executable: String,
    entry: u64,
    stack: u64,
    aux: metadata::AuxHeaders,
    header_mapping: Option<MappedRegion>,
    executable_regions: Vec<MappedRegion>,
}

impl ExecRecord {
    fn parse<'a, L: Iterator<Item = &'a str>>(
        mut lines: L,
        executable: &str,
    ) -> ConsoleResult<Self> {
        let record = lines
            .find(|line| line.starts_with("RB_EXEC "))
            .ok_or("missing exec record")?;
        let [pid, entry, stack, phdr, phent, phnum] = hex_words("RB_EXEC", record)?;
        let (entry, stack, phdr) = (u64::from(entry), u64::from(stack), u64::from(phdr));
        let observed_path = lines
            .next()
            .and_then(|line| line.strip_prefix("RB_EXE "))
            .ok_or("missing /proc executable identity")?;
        if pid == 0
            || pid > i32::MAX as u32
            || entry == 0
            || stack == 0
            || observed_path != executable
            || lines.next() != Some("RB_MAPS")
        {
            return Err("unexpected exec identity or mapping record".into());
        }

        let mut header_mapping = None;
        let mut executable_regions = Vec::new();
        let mut has_stack = false;
        let mut complete = false;
        for line in lines {
            if line == "RB_WAIT" {
                complete = true;
                break;
            }
            let mut fields = line.split_ascii_whitespace();
            let range = fields.next().ok_or("missing map range")?;
            let permissions = fields.next().ok_or("missing map permissions")?.as_bytes();
            let offset = fields.next().ok_or("missing map offset")?;
            let path = fields.nth(2);
            if (path != Some(observed_path) && path != Some("[stack]")) || fields.next().is_some() {
                continue;
            }
            let (start, end) = range.split_once('-').ok_or("invalid map range")?;
            let map = MappedRegion {
                start: u64::from_str_radix(start, 16)?,
                end: u64::from_str_radix(end, 16)?,
                file_offset: u64::from_str_radix(offset, 16)?,
            };
            if map.start >= map.end || map.end > 1 << 32 || permissions.len() != 4 {
                return Err("invalid i386 mapping".into());
            }
            if path == Some("[stack]") {
                has_stack |= map.contains(stack);
            } else {
                if permissions[0] == b'r'
                    && map.file_offset == 0
                    && map.contains(phdr)
                    && header_mapping.replace(map).is_some()
                {
                    return Err("ambiguous ELF header mapping".into());
                }
                if permissions[2] == b'x' {
                    executable_regions.push(map);
                }
            }
        }
        if !complete || !has_stack || !executable_regions.iter().any(|map| map.contains(entry)) {
            return Err("entry or initial stack lies outside the child's reported mappings".into());
        }
        Ok(Self {
            pid,
            executable: observed_path.to_owned(),
            entry,
            stack,
            aux: metadata::AuxHeaders { phdr, phent, phnum },
            header_mapping,
            executable_regions,
        })
    }

    fn observation<F: FnMut(u64, &mut [u8]) -> Result<(), AnalysisError>>(
        self,
        cpu: EntryCpu,
        request: MetadataRequest,
        read: F,
    ) -> Result<Observation, AnalysisError> {
        if cpu.root == 0 || cpu.entry != self.entry || cpu.stack != self.stack {
            return Err(AnalysisError::WrongBoundary);
        }
        let space = next_address_space(cpu.root)?;
        let metadata = match request {
            MetadataRequest::Skip => None,
            MetadataRequest::ReadMappedElf => Some(ImageMetadata {
                space,
                result: metadata::inspect(
                    self.header_mapping,
                    self.aux,
                    self.entry,
                    &self.executable_regions,
                    read,
                ),
            }),
        };
        Ok(Observation {
            process: ProcessObserved {
                pid: self.pid,
                executable: self.executable,
                space,
            },
            image: ImageObserved {
                pid: self.pid,
                space,
                entry: self.entry,
                executable_regions: self.executable_regions,
            },
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use rusty_box::Emulator;
    use rusty_box::cpu::CpuSetupMode;
    use rusty_box::emulator::EmulatorConfig;
    use rusty_box_core::events::App;

    use super::*;
    use crate::linux::tests::{BASE, ENTRY, elf};

    const EXECUTABLE: &str = "/bin/program_sum";
    const PID: u32 = 42;
    const ROOT: u64 = 0x8000;
    const STACK: u64 = 0xBFFF_0100;
    const RECORD: [&str; 6] = [
        "RB_EXEC 0000002a 00001054 bfff0100 00001034 00000020 00000001",
        "RB_EXE /bin/program_sum",
        "RB_MAPS",
        "1000-2000 r-xp 00000000 00:01 123 /bin/program_sum",
        "bfff0000-bfff2000 rw-p 00000000 00:00 0 [stack]",
        "RB_WAIT",
    ];

    fn entry_cpu(stack: u64) -> EntryCpu {
        EntryCpu {
            entry: ENTRY,
            stack,
            root: ROOT,
        }
    }

    fn record() -> ExecRecord {
        ExecRecord::parse(RECORD.into_iter(), EXECUTABLE).unwrap()
    }

    #[test]
    fn basic_observation_skips_memory_and_rejects_a_different_entry_stack() {
        let capture = |stack| {
            record().observation(entry_cpu(stack), MetadataRequest::Skip, |_, _| {
                panic!("basic observation must skip ELF reads")
            })
        };
        assert!(matches!(
            capture(STACK + 4),
            Err(AnalysisError::WrongBoundary)
        ));
        let observed = capture(STACK).unwrap();
        assert_eq!(observed.process.pid, PID);
        assert_eq!(observed.process.executable, EXECUTABLE);
        assert_eq!(observed.process.space.page_table_root, ROOT);
        assert_eq!(observed.image.entry, ENTRY);
        assert!(observed.metadata.is_none());
        assert!(ExecRecord::parse(RECORD.into_iter(), "/bin/program_text").is_err());
    }

    #[test]
    fn one_checked_preparation_supplies_both_metadata_handlers() {
        let bytes = elf();
        let reads = Cell::new(0);
        let observed = record()
            .observation(
                entry_cpu(STACK),
                MetadataRequest::ReadMappedElf,
                |address, out| {
                    reads.set(reads.get() + 1);
                    let start = usize::try_from(address - BASE).unwrap();
                    out.copy_from_slice(&bytes[start..start + out.len()]);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(reads.get(), 2, "one ELF header plus one program header");

        let mut app = App::<Vec<&str>>::new()
            .on(|event: &ProcessObserved, output: &mut Vec<_>| {
                assert_eq!(event.pid, PID);
                output.push("process");
                ExecutionRequest::Stop
            })
            .on(|_: &ImageObserved, output: &mut Vec<_>| output.push("image"))
            .on(|metadata: &ImageMetadata, output: &mut Vec<_>| {
                assert_eq!(metadata.result.as_ref().unwrap().entry, ENTRY);
                output.push("entry");
            })
            .on(|metadata: &ImageMetadata, output: &mut Vec<_>| {
                assert_eq!(metadata.result.as_ref().unwrap().program_headers.len(), 1);
                output.push("headers");
            })
            .with_state(Vec::new());
        assert_eq!(observed.deliver(&mut app), Ok(ExecutionRequest::Stop));
        assert_eq!(app.into_state(), ["process", "image", "entry", "headers"]);
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn malformed_and_unavailable_metadata_remain_distinct_results() {
        let observe = |read: fn(u64, &mut [u8]) -> Result<(), AnalysisError>| {
            record()
                .observation(entry_cpu(STACK), MetadataRequest::ReadMappedElf, read)
                .unwrap()
                .metadata
                .unwrap()
                .result
        };
        assert!(matches!(
            observe(|_, out| {
                out.fill(0);
                Ok(())
            }),
            Err(AnalysisError::Invalid(_))
        ));
        assert_eq!(
            observe(|address, _| Err(AnalysisError::Missing { address })),
            Err(AnalysisError::Missing { address: BASE })
        );

        let mut machine =
            Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatProtected32)
                .unwrap();
        let mut bytes = [0; 4];
        assert_eq!(
            crate::linux::read_guest(&mut machine, 0xFFFF_F000, &mut bytes),
            Err(AnalysisError::Missing {
                address: 0xFFFF_F000
            })
        );
    }

    #[test]
    fn oversized_auxv_is_refused_before_reading_guest_memory() {
        let mut exec = record();
        exec.aux.phnum = 65;
        let observed = exec
            .observation(entry_cpu(STACK), MetadataRequest::ReadMappedElf, |_, _| {
                panic!("a rejected header extent must perform no reads")
            })
            .unwrap();
        assert!(matches!(
            observed.metadata.unwrap().result,
            Err(AnalysisError::Invalid(_))
        ));
    }

    #[test]
    fn image_handler_error_ends_the_observation_after_an_earlier_stop() {
        let observed = record()
            .observation(
                entry_cpu(STACK),
                MetadataRequest::ReadMappedElf,
                |address, _| Err(AnalysisError::Missing { address }),
            )
            .unwrap();
        let mut app = App::<Vec<&str>, &str>::new()
            .on(|_: &ProcessObserved, output: &mut Vec<_>| {
                output.push("process");
                ExecutionRequest::Stop
            })
            .on(|_: &ImageObserved, output: &mut Vec<_>| {
                output.push("image");
                Err("image handler failed")
            })
            .on(|_: &ImageMetadata, output: &mut Vec<_>| output.push("metadata"))
            .with_state(Vec::new());
        assert_eq!(observed.deliver(&mut app), Err("image handler failed"));
        assert_eq!(app.into_state(), ["process", "image"]);
    }
}
