use alloc::vec::Vec;

use crate::memory::{PAGE, STACK_GUARD, USER_MIN};

const ELF_HEADER: usize = 52;
const PROGRAM_HEADER: usize = 32;
const MAX_PHDRS: u16 = 64;
const MAX_MAPPED: u32 = 32 * 1024 * 1024;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_TLS: u32 = 7;
const PT_GNU_STACK: u32 = 0x6474_E551;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ElfError {
    #[error("expected a static, little-endian ELF32/i386 ET_EXEC executable")]
    Format,
    #[error("invalid or truncated ELF program headers")]
    Headers,
    #[error("unsupported dynamic linking, TLS or executable stack")]
    Runtime,
    #[error("invalid PT_LOAD bounds or alignment")]
    Segment,
    #[error("PT_LOAD pages overlap or exceed the process memory limit")]
    Layout,
    #[error("PT_LOAD must be readable and may not be writable and executable")]
    Permissions,
    #[error("ELF entry is outside the executable file-backed bytes")]
    Entry,
    #[error("program headers are not mapped by a PT_LOAD segment")]
    UnmappedHeaders,
}

pub(crate) struct Segment<'a> {
    pub start: u32,
    pub page_start: u32,
    pub page_end: u32,
    pub writable: bool,
    pub executable: bool,
    pub data: &'a [u8],
}

/// Validated fixed-address ELF32/i386 image. Parse once and construct independent
/// processes from the same immutable bytes.
pub struct Image<'a> {
    pub(crate) segments: Vec<Segment<'a>>,
    pub(crate) entry: u32,
    pub(crate) phdr: u32,
    pub(crate) phnum: u16,
}

impl<'a> Image<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ElfError> {
        let header = bytes.get(..ELF_HEADER).ok_or(ElfError::Format)?;
        if header[..7] != *b"\x7fELF\x01\x01\x01"
            || !matches!(header[7], 0 | 3)
            || header[8] != 0
            || u16_at(header, 16) != 2
            || u16_at(header, 18) != 3
            || u32_at(header, 20) != 1
            || usize::from(u16_at(header, 40)) != ELF_HEADER
        {
            return Err(ElfError::Format);
        }
        let entry = u32_at(header, 24);
        let offset = u32_at(header, 28);
        let phnum = u16_at(header, 44);
        if !(1..=MAX_PHDRS).contains(&phnum)
            || usize::from(u16_at(header, 42)) != PROGRAM_HEADER
            || offset < ELF_HEADER as u32
        {
            return Err(ElfError::Headers);
        }
        let table_end = offset
            .checked_add(u32::from(phnum) * PROGRAM_HEADER as u32)
            .ok_or(ElfError::Headers)?;
        let table = bytes
            .get(offset as usize..table_end as usize)
            .ok_or(ElfError::Headers)?;
        let mut segments = Vec::new();
        let mut mapped = 0;
        let mut phdr = None;
        for raw in table.as_chunks::<PROGRAM_HEADER>().0 {
            let kind = u32_at(raw, 0);
            let flags = u32_at(raw, 24);
            if matches!(kind, PT_DYNAMIC | PT_INTERP | PT_TLS)
                || (kind == PT_GNU_STACK && flags & PF_X != 0)
            {
                return Err(ElfError::Runtime);
            }
            if kind != PT_LOAD {
                continue;
            }
            let file_at = u32_at(raw, 4);
            let start = u32_at(raw, 8);
            let file_len = u32_at(raw, 16);
            let mem_len = u32_at(raw, 20);
            let align = u32_at(raw, 28);
            if file_len > mem_len
                || (file_at ^ start) & (PAGE - 1) != 0
                || (align > 1 && (!align.is_power_of_two() || (file_at ^ start) & (align - 1) != 0))
            {
                return Err(ElfError::Segment);
            }
            if mem_len == 0 {
                continue;
            }
            let end = start.checked_add(mem_len).ok_or(ElfError::Segment)?;
            let file_end = file_at.checked_add(file_len).ok_or(ElfError::Segment)?;
            let data = bytes
                .get(file_at as usize..file_end as usize)
                .ok_or(ElfError::Segment)?;
            if start < USER_MIN || end > STACK_GUARD {
                return Err(ElfError::Layout);
            }
            let page_start = start & !(PAGE - 1);
            let page_end = (end + PAGE - 1) & !(PAGE - 1);
            mapped += page_end - page_start;
            if mapped > MAX_MAPPED {
                return Err(ElfError::Layout);
            }
            let writable = flags & PF_W != 0;
            let executable = flags & PF_X != 0;
            if flags & PF_R == 0 || (writable && executable) {
                return Err(ElfError::Permissions);
            }
            if file_at <= offset && table_end <= file_end {
                phdr = Some(start + (offset - file_at));
            }
            segments.push(Segment {
                start,
                page_start,
                page_end,
                writable,
                executable,
                data,
            });
        }
        segments.sort_unstable_by_key(|segment| segment.page_start);
        if segments
            .windows(2)
            .any(|pair| pair[0].page_end > pair[1].page_start)
        {
            return Err(ElfError::Layout);
        }
        if !segments.iter().any(|segment| {
            segment.executable
                && (segment.start..segment.start + segment.data.len() as u32).contains(&entry)
        }) {
            return Err(ElfError::Entry);
        }
        Ok(Self {
            segments,
            entry,
            phdr: phdr.ok_or(ElfError::UnmappedHeaders)?,
            phnum,
        })
    }

    pub fn entry(&self) -> u32 {
        self.entry
    }
}

// Both callers pass fully sized ELF records from the checked slices above.
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}
