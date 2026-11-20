//! A scoped observation of the instrumented entry point of a Linux i386 image.

use std::sync::atomic::{AtomicU64, Ordering};

use rusty_box::Emulator;
use rusty_box::config::BxLinAddress;
use rusty_box::cpu::registers::{CR3, CS, EAX, RIP};
use rusty_box::cpu::{CodeBreakpoint, CodeBreakpointHit, Instrumentation};
use rusty_box_core::events::ExecutionRequest;
use sha2::{Digest, Sha256};

const ELF_HEADER: usize = 52;
const PROGRAM_HEADER: usize = 32;
const MAX_CODE_BYTES: usize = 1024 * 1024;
const LOAD: u32 = 1;
const EXECUTE: u32 = 1;
// Linux/i386 getpid returns the process ID in EAX before returning to ring 3.
const GETPID: [u8; 7] = [0xB8, 20, 0, 0, 0, 0xCD, 0x80];
static NEXT_IMAGE: AtomicU64 = AtomicU64::new(1);

/// Scoped to this host process. Each observation gets a fresh generation
/// even when a guest reuses the PID and page-table root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressSpace {
    pub page_table_root: u64,
    pub image_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessImageMapped {
    pub pid: u32,
    pub space: AddressSpace,
    pub checkpoint: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessCreated {
    pub pid: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessExited {
    pub pid: u32,
    /// Status reported by the shell's `wait` for the launched child.
    pub wait_status: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryFormat {
    Elf32LittleEndianI386,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutableType {
    Executable,
    SharedObject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramHeader {
    pub kind: u32,
    pub file_offset: u32,
    pub virtual_address: u32,
    pub file_size: u32,
    pub memory_size: u32,
    pub flags: u32,
    pub alignment: u32,
}

impl ProgramHeader {
    pub fn is_executable_load(&self) -> bool {
        self.kind == LOAD && self.flags & EXECUTE != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnalyzedBinary {
    pub format: BinaryFormat,
    pub executable_type: ExecutableType,
    pub entry: u64,
    pub guest_base: u64,
    pub load_bias: u64,
    pub image: ProcessImageMapped,
    pub program_headers: Vec<ProgramHeader>,
    /// Header, program header table and resident file-backed executable bytes.
    pub loaded_code_sha256: [u8; 32],
}

impl AnalyzedBinary {
    /// Normalize a PC only within this image generation's measured code.
    pub fn code_offset(&self, space: AddressSpace, pc: u64) -> Option<u64> {
        if self.image.space != space {
            return None;
        }
        let link_address = pc.checked_sub(self.load_bias)?;
        self.program_headers
            .iter()
            .any(|header| {
                header.is_executable_load()
                    && link_address.wrapping_sub(u64::from(header.virtual_address))
                        < u64::from(header.file_size)
            })
            .then(|| pc.checked_sub(self.guest_base))
            .flatten()
    }
}

/// A short borrow of resolved, host-owned metadata during event delivery.
///
/// ```compile_fail
/// use rusty_box_core::events::ExecutionRequest;
/// use rusty_box_execution_workflows::linux::{AnalyzedBinary, ImageObservers, Resolved};
///
/// let mut kept: Option<Resolved<'_, AnalyzedBinary>> = None;
/// let mut observers = ImageObservers::default();
/// observers.on_binary(|_, binary| {
///     kept = Some(binary);
///     ExecutionRequest::None
/// });
/// ```
pub struct Resolved<'a, T>(&'a T);

impl<T> core::ops::Deref for Resolved<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AnalysisError {
    #[error("invalid or unsupported ELF: {0}")]
    Invalid(&'static str),
    #[error("guest image bytes at {address:#x} are unavailable")]
    Missing { address: u64 },
    #[error("the processor is outside the Linux/i386 image checkpoint")]
    WrongBoundary,
    #[error("image generation exhausted")]
    GenerationExhausted,
}

type Observer<'a> = Box<dyn FnMut(&ProcessImageMapped) + 'a>;
type BinaryObserver<'a> =
    Box<dyn FnMut(&ProcessImageMapped, Resolved<'_, AnalyzedBinary>) -> ExecutionRequest + 'a>;

enum Handler<'a> {
    Event(Observer<'a>),
    Binary(BinaryObserver<'a>),
}

/// Subscriptions for a host tool. Registration order spans both handler kinds.
#[derive(Default)]
pub struct ImageObservers<'a> {
    handlers: Vec<Handler<'a>>,
}

impl<'a> ImageObservers<'a> {
    pub fn on(&mut self, observer: impl FnMut(&ProcessImageMapped) + 'a) {
        self.handlers.push(Handler::Event(Box::new(observer)));
    }

    pub fn on_binary(
        &mut self,
        observer: impl FnMut(&ProcessImageMapped, Resolved<'_, AnalyzedBinary>) -> ExecutionRequest + 'a,
    ) {
        self.handlers.push(Handler::Binary(Box::new(observer)));
    }

    /// Resolve at most once at the paused boundary. Only binary subscribers
    /// ask for a resolution. The prepared result carries no emulator loan.
    pub fn prepare<T: Instrumentation>(
        &self,
        machine: &mut Emulator<T>,
        probe: &I386ImageProbe,
        hit: CodeBreakpointHit,
    ) -> Result<PreparedImage, AnalysisError> {
        let image = probe.observe(machine, hit)?;
        Ok(self.prepare_with(image, || {
            analyze_memory(probe.base, image, |address, out| {
                read_guest(machine, address, out)
            })
        }))
    }

    fn prepare_with(
        &self,
        image: ProcessImageMapped,
        resolve: impl FnOnce() -> Result<AnalyzedBinary, AnalysisError>,
    ) -> PreparedImage {
        PreparedImage {
            image,
            binary: self
                .handlers
                .iter()
                .any(|handler| matches!(handler, Handler::Binary(_)))
                .then(resolve),
        }
    }

    /// Publish without a machine borrow. Ordinary observers still receive a
    /// verified checkpoint when optional binary analysis reports an error.
    pub fn deliver(&mut self, prepared: &PreparedImage) -> ExecutionRequest {
        let mut request = ExecutionRequest::None;
        for handler in &mut self.handlers {
            match handler {
                Handler::Event(observer) => observer(&prepared.image),
                Handler::Binary(observer) => {
                    if let Some(Ok(binary)) = prepared.binary()
                        && observer(&prepared.image, Resolved(binary)) == ExecutionRequest::Stop
                    {
                        request = ExecutionRequest::Stop;
                    }
                }
            }
        }
        request
    }
}

pub struct PreparedImage {
    image: ProcessImageMapped,
    binary: Option<Result<AnalyzedBinary, AnalysisError>>,
}

impl PreparedImage {
    pub fn event(&self) -> &ProcessImageMapped {
        &self.image
    }

    pub fn binary(&self) -> Option<Result<&AnalyzedBinary, &AnalysisError>> {
        self.binary.as_ref().map(Result::as_ref)
    }
}

/// The fixture leaves Linux's real getpid result at a known user-mode PC.
/// The file supplies the breakpoint placement. The analysis reads guest RAM.
pub struct I386ImageProbe {
    base: u64,
    checkpoint: u64,
    supplied_code_sha256: [u8; 32],
}

impl I386ImageProbe {
    pub fn for_replacement(file: &[u8]) -> Result<Self, AnalysisError> {
        let elf = Elf::read(|offset, bytes| {
            let start = offset as usize;
            let end = start
                .checked_add(bytes.len())
                .ok_or(AnalysisError::Invalid("file range"))?;
            bytes.copy_from_slice(
                file.get(start..end)
                    .ok_or(AnalysisError::Invalid("truncated file"))?,
            );
            Ok(())
        })?;
        let entry = u64::from(elf.entry);
        let segment = elf
            .headers
            .iter()
            .find(|header| {
                header.is_executable_load()
                    && entry
                        .checked_sub(u64::from(header.virtual_address))
                        .is_some_and(|delta| {
                            delta + (GETPID.len() as u64) < u64::from(header.file_size)
                        })
            })
            .ok_or(AnalysisError::Invalid(
                "entry checkpoint is outside executable file bytes",
            ))?;
        let offset =
            (entry - u64::from(segment.virtual_address) + u64::from(segment.file_offset)) as usize;
        if elf.kind != ExecutableType::Executable
            || file.get(offset..offset + GETPID.len()) != Some(&GETPID)
        {
            return Err(AnalysisError::Invalid(
                "expected static i386 getpid entry fixture",
            ));
        }
        let mut digest = elf.digest;
        for header in elf.headers.iter().filter(|header| header.kind == LOAD) {
            let start = header.file_offset as usize;
            let end = start + header.file_size as usize;
            let bytes = file
                .get(start..end)
                .ok_or(AnalysisError::Invalid("incomplete file load segment"))?;
            if header.is_executable_load() {
                digest.update(bytes);
            }
        }
        Ok(Self {
            base: u64::from(elf.link_base),
            checkpoint: entry + GETPID.len() as u64,
            supplied_code_sha256: digest.finalize().into(),
        })
    }

    pub fn checkpoint(&self) -> u64 {
        self.checkpoint
    }

    pub fn supplied_code_sha256(&self) -> [u8; 32] {
        self.supplied_code_sha256
    }

    fn observe<T: Instrumentation>(
        &self,
        machine: &mut Emulator<T>,
        hit: CodeBreakpointHit,
    ) -> Result<ProcessImageMapped, AnalysisError> {
        if hit.cpu != 0
            || hit.linear != self.checkpoint
            || hit.breakpoint != CodeBreakpoint::Linear(self.checkpoint)
        {
            return Err(AnalysisError::WrongBoundary);
        }
        let regs = machine.regs();
        let pid = regs.read(EAX);
        let root = regs.read(CR3);
        if regs.read(RIP) != self.checkpoint
            || regs.read(CS) & 3 != 3
            || root == 0
            || pid == 0
            || pid > i32::MAX as u32
        {
            return Err(AnalysisError::WrongBoundary);
        }
        let mut actual = [0; GETPID.len()];
        read_guest(machine, self.checkpoint - GETPID.len() as u64, &mut actual)?;
        if actual != GETPID {
            return Err(AnalysisError::WrongBoundary);
        }
        let generation = NEXT_IMAGE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| AnalysisError::GenerationExhausted)?;
        Ok(ProcessImageMapped {
            pid,
            space: AddressSpace {
                page_table_root: root,
                image_generation: generation,
            },
            checkpoint: self.checkpoint,
        })
    }
}

fn read_guest<T: Instrumentation>(
    machine: &mut Emulator<T>,
    mut address: u64,
    mut out: &mut [u8],
) -> Result<(), AnalysisError> {
    while !out.is_empty() {
        let linear = BxLinAddress::new(address);
        let physical = machine
            .debug_cpu(0)
            .and_then(|mut cpu| cpu.dbg_xlate_linear2phy(linear))
            .ok_or(AnalysisError::Missing { address })?;
        let len = (4096 - linear.page_offset() as usize).min(out.len());
        machine
            .mem_read(physical, &mut out[..len])
            .map_err(|_| AnalysisError::Missing { address })?;
        address += len as u64;
        out = &mut out[len..];
    }
    Ok(())
}

fn analyze_memory(
    base: u64,
    image: ProcessImageMapped,
    mut read: impl FnMut(u64, &mut [u8]) -> Result<(), AnalysisError>,
) -> Result<AnalyzedBinary, AnalysisError> {
    let elf = Elf::read(|offset, out| read(base + u64::from(offset), out))?;
    let load_bias = base
        .checked_sub(u64::from(elf.link_base))
        .ok_or(AnalysisError::Invalid("negative load bias"))?;
    if load_bias != 0 && elf.kind == ExecutableType::Executable {
        return Err(AnalysisError::Invalid("relocated fixed executable"));
    }
    let entry = u64::from(elf.entry) + load_bias;
    if entry.checked_add(GETPID.len() as u64) != Some(image.checkpoint) {
        return Err(AnalysisError::Invalid(
            "entry disagrees with the observed checkpoint",
        ));
    }
    let mut digest = elf.digest;
    let mut budget = MAX_CODE_BYTES;
    let mut chunk = [0; 4096];
    for header in elf
        .headers
        .iter()
        .filter(|header| header.is_executable_load())
    {
        let mut remaining = header.file_size as usize;
        budget = budget.checked_sub(remaining).ok_or(AnalysisError::Invalid(
            "executable bytes exceed analysis limit",
        ))?;
        let mut address = u64::from(header.virtual_address) + load_bias;
        while remaining != 0 {
            let len = remaining.min(chunk.len());
            read(address, &mut chunk[..len])?;
            digest.update(&chunk[..len]);
            address += len as u64;
            remaining -= len;
        }
    }
    Ok(AnalyzedBinary {
        format: BinaryFormat::Elf32LittleEndianI386,
        executable_type: elf.kind,
        entry,
        guest_base: base,
        load_bias,
        image,
        program_headers: elf.headers,
        loaded_code_sha256: digest.finalize().into(),
    })
}

struct Elf {
    kind: ExecutableType,
    entry: u32,
    link_base: u32,
    headers: Vec<ProgramHeader>,
    digest: Sha256,
}

impl Elf {
    fn read(
        mut read: impl FnMut(u32, &mut [u8]) -> Result<(), AnalysisError>,
    ) -> Result<Self, AnalysisError> {
        let mut raw = [0; ELF_HEADER];
        read(0, &mut raw)?;
        if raw[..7] != *b"\x7fELF\x01\x01\x01"
            || half(&raw, 18) != 3
            || word(&raw, 20) != 1
            || half(&raw, 40) != ELF_HEADER as u16
        {
            return Err(AnalysisError::Invalid("expected ELF32 little-endian i386"));
        }
        let kind = match half(&raw, 16) {
            2 => ExecutableType::Executable,
            3 => ExecutableType::SharedObject,
            _ => return Err(AnalysisError::Invalid("unsupported ELF object type")),
        };
        let entry = word(&raw, 24);
        let phoff = word(&raw, 28);
        let count = u32::from(half(&raw, 44));
        if count == 0
            || count > 64
            || half(&raw, 42) != PROGRAM_HEADER as u16
            || phoff != ELF_HEADER as u32
        {
            return Err(AnalysisError::Invalid("unsupported program header table"));
        }
        let table_end = phoff + count * PROGRAM_HEADER as u32;
        let mut digest = Sha256::new();
        digest.update(raw);
        let mut headers = Vec::with_capacity(count as usize);
        let mut link_base = None;
        for i in 0..count {
            let mut raw = [0; PROGRAM_HEADER];
            read(phoff + i * PROGRAM_HEADER as u32, &mut raw)?;
            digest.update(raw);
            let header = ProgramHeader {
                kind: word(&raw, 0),
                file_offset: word(&raw, 4),
                virtual_address: word(&raw, 8),
                file_size: word(&raw, 16),
                memory_size: word(&raw, 20),
                flags: word(&raw, 24),
                alignment: word(&raw, 28),
            };
            if header.kind == LOAD {
                if header.file_size > header.memory_size
                    || header
                        .virtual_address
                        .checked_add(header.memory_size)
                        .is_none()
                    || header.file_offset.checked_add(header.file_size).is_none()
                    || (header.alignment > 1
                        && (!header.alignment.is_power_of_two()
                            || header.virtual_address % header.alignment
                                != header.file_offset % header.alignment))
                {
                    return Err(AnalysisError::Invalid(
                        "invalid load segment extent or alignment",
                    ));
                }
                if header.file_offset == 0
                    && header.file_size >= table_end
                    && link_base.replace(header.virtual_address).is_some()
                {
                    return Err(AnalysisError::Invalid("ambiguous ELF header mapping"));
                }
            }
            headers.push(header);
        }
        if !headers.iter().any(|header| {
            header.is_executable_load()
                && entry.wrapping_sub(header.virtual_address) < header.file_size
        }) {
            return Err(AnalysisError::Invalid(
                "entry is outside file-backed executable code",
            ));
        }
        let link_base = link_base.ok_or(AnalysisError::Invalid(
            "ELF headers are not in a load segment",
        ))?;
        Ok(Self {
            kind,
            entry,
            link_base,
            headers,
            digest,
        })
    }
}

fn half(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    const BASE: u64 = 0x1000;
    const ENTRY: u64 = BASE + (ELF_HEADER + PROGRAM_HEADER) as u64;

    fn image(generation: u64) -> ProcessImageMapped {
        ProcessImageMapped {
            pid: 42,
            space: AddressSpace {
                page_table_root: 0x2000,
                image_generation: generation,
            },
            checkpoint: ENTRY + GETPID.len() as u64,
        }
    }

    fn elf() -> [u8; 128] {
        let mut mem = [0; 128];
        mem[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
        mem[16] = 2;
        mem[18] = 3;
        mem[20] = 1;
        mem[24..28].copy_from_slice(&(ENTRY as u32).to_le_bytes());
        mem[28] = ELF_HEADER as u8;
        mem[40] = ELF_HEADER as u8;
        mem[42] = PROGRAM_HEADER as u8;
        mem[44] = 1;
        mem[ELF_HEADER] = 1;
        mem[ELF_HEADER + 8..ELF_HEADER + 12].copy_from_slice(&(BASE as u32).to_le_bytes());
        mem[ELF_HEADER + 16] = 128;
        mem[ELF_HEADER + 20] = 128;
        mem[ELF_HEADER + 24] = 5;
        mem[ELF_HEADER + 29] = 0x10;
        mem[ELF_HEADER + PROGRAM_HEADER..][..GETPID.len()].copy_from_slice(&GETPID);
        mem
    }

    fn analysis(mem: &[u8], event: ProcessImageMapped) -> Result<AnalyzedBinary, AnalysisError> {
        analyze_memory(BASE, event, |address, out| {
            let start = (address - BASE) as usize;
            let src = mem
                .get(start..start + out.len())
                .ok_or(AnalysisError::Missing { address })?;
            out.copy_from_slice(src);
            Ok(())
        })
    }

    #[test]
    fn observers_share_one_demand_driven_snapshot_and_preserve_order() {
        let actual = elf();
        let reads = Cell::new(0);
        let order = RefCell::new(Vec::new());
        let first_ptr = Cell::new(None);
        let mut subscribers = ImageObservers::default();
        subscribers.on(|_| order.borrow_mut().push(1));
        let plain =
            subscribers.prepare_with(image(1), || panic!("plain observations do not analyze ELF"));
        assert_eq!(subscribers.deliver(&plain), ExecutionRequest::None);
        assert!(plain.binary().is_none());
        order.borrow_mut().clear();
        subscribers.on_binary(|event, binary| {
            assert_eq!(binary.code_offset(event.space, event.checkpoint), Some(91));
            first_ptr.set(Some(core::ptr::from_ref(&*binary)));
            order.borrow_mut().push(2);
            ExecutionRequest::Stop
        });
        subscribers.on_binary(|_, binary| {
            assert_eq!(Some(core::ptr::from_ref(&*binary)), first_ptr.get());
            order.borrow_mut().push(3);
            ExecutionRequest::None
        });
        let prepared = subscribers.prepare_with(image(2), || {
            reads.set(reads.get() + 1);
            analysis(&actual, image(2))
        });
        assert_eq!(subscribers.deliver(&prepared), ExecutionRequest::Stop);
        assert_eq!(reads.get(), 1);
        assert_eq!(*order.borrow(), [1, 2, 3]);
    }

    #[test]
    fn analysis_refuses_invalid_headers_and_incomplete_mappings() {
        let mut invalid = elf();
        invalid[5] = 2;
        assert!(matches!(
            analysis(&invalid, image(1)),
            Err(AnalysisError::Invalid(_))
        ));
        let mut invalid = elf();
        invalid[ELF_HEADER + 20] = 1;
        assert!(matches!(
            analysis(&invalid, image(1)),
            Err(AnalysisError::Invalid(_))
        ));
        assert!(matches!(
            analysis(&elf()[..100], image(1)),
            Err(AnalysisError::Missing { .. })
        ));
        let plain_seen = Cell::new(false);
        let mut subscribers = ImageObservers::default();
        subscribers.on(|_| plain_seen.set(true));
        subscribers.on_binary(|_, _| panic!("invalid analysis does not produce Resolved"));
        let prepared = subscribers.prepare_with(image(1), || analysis(&invalid, image(1)));
        assert_eq!(subscribers.deliver(&prepared), ExecutionRequest::None);
        assert!(plain_seen.get());
        assert!(matches!(
            prepared.binary(),
            Some(Err(AnalysisError::Invalid(_)))
        ));
    }

    #[test]
    fn a_new_loaded_generation_cannot_use_an_older_resolution() {
        let mut mem = elf();
        let before = analysis(&mem, image(1)).unwrap();
        let supplied = I386ImageProbe::for_replacement(&mem).unwrap();
        assert_eq!(before.loaded_code_sha256, supplied.supplied_code_sha256());
        mem[127] = b'x';
        let after = analysis(&mem, image(2)).unwrap();
        assert_ne!(before.loaded_code_sha256, after.loaded_code_sha256);
        assert_eq!(
            before.code_offset(image(2).space, image(2).checkpoint),
            None
        );
        assert_eq!(after.code_offset(image(2).space, ENTRY), Some(84));
        assert_eq!(after.code_offset(image(2).space, BASE + 128), None);
    }

    #[test]
    #[ignore = "release measurement of host analysis and event delivery"]
    fn measure_resolution_and_delivery() {
        use std::hint::black_box;
        use std::time::Instant;

        let mem = elf();
        let event = image(1);
        let mut observers = ImageObservers::default();
        observers.on(|event| {
            black_box(event);
        });
        for _ in 0..2 {
            observers.on_binary(|_, image| {
                black_box(image.entry);
                ExecutionRequest::None
            });
        }
        let cached = observers.prepare_with(event, || analysis(&mem, event));
        for round in 1..=6 {
            let start = Instant::now();
            for _ in 0..20_000 {
                black_box(analysis(black_box(&mem), event).unwrap());
            }
            let first = start.elapsed().as_nanos() as f64 / 20_000.0;
            let start = Instant::now();
            for _ in 0..1_000_000 {
                black_box(black_box(&cached).binary());
            }
            let cache = start.elapsed().as_nanos() as f64 / 1_000_000.0;
            let start = Instant::now();
            let mut last = ExecutionRequest::None;
            for _ in 0..1_000_000 {
                last = black_box(observers.deliver(black_box(&cached)));
            }
            let delivery = start.elapsed().as_nanos() as f64 / 1_000_000.0;
            assert_eq!(last, ExecutionRequest::None);
            println!(
                "round={round} first_analysis_ns={first:.1} cached_ns={cache:.1} three_handlers_ns={delivery:.1}"
            );
        }
    }
}
