use super::MappedRegion;
use crate::linux::{AnalysisError, ELF_HEADER, Elf, ExecutableType, PROGRAM_HEADER, ProgramHeader};

pub(super) struct AuxHeaders {
    pub(super) phdr: u64,
    pub(super) phent: u32,
    pub(super) phnum: u32,
}

/// The ELF header and program headers read from resident guest memory.
/// Segment sizes and offsets describe the ELF file. They give no residency
/// guarantee for the segment contents and no digest of the live code.
#[derive(Debug, PartialEq, Eq)]
pub struct MappedElf {
    pub executable_type: ExecutableType,
    pub link_entry: u32,
    pub entry: u64,
    pub guest_base: u64,
    pub load_bias: u64,
    pub program_headers: Vec<ProgramHeader>,
}

pub(super) fn inspect<F: FnMut(u64, &mut [u8]) -> Result<(), AnalysisError>>(
    header_mapping: Option<MappedRegion>,
    aux: AuxHeaders,
    observed_entry: u64,
    executable_regions: &[MappedRegion],
    mut read: F,
) -> Result<MappedElf, AnalysisError> {
    let mapping = header_mapping.ok_or(AnalysisError::Missing { address: aux.phdr })?;
    let base = mapping.start;
    let table = base.checked_add(ELF_HEADER as u64);
    if table != Some(aux.phdr)
        || aux.phent != PROGRAM_HEADER as u32
        || !(1..=64).contains(&aux.phnum)
    {
        return Err(AnalysisError::Invalid(
            "auxv disagrees with supported ELF table layout",
        ));
    }
    let table_end = aux.phdr + u64::from(aux.phent) * u64::from(aux.phnum);
    if table_end > mapping.end {
        return Err(AnalysisError::Invalid(
            "program header table extends beyond the readable mapping",
        ));
    }
    let elf = Elf::read(|offset, out| {
        let start = base + u64::from(offset);
        if start + out.len() as u64 > table_end {
            return Err(AnalysisError::Invalid(
                "ELF requests more headers than auxv reports",
            ));
        }
        read(start, out)
    })?;
    if elf.headers.len() != aux.phnum as usize {
        return Err(AnalysisError::Invalid(
            "ELF header count disagrees with auxv",
        ));
    }
    let load_bias = base
        .checked_sub(u64::from(elf.link_base))
        .ok_or(AnalysisError::Invalid("negative load bias"))?;
    let entry = u64::from(elf.entry) + load_bias;
    if (elf.kind == ExecutableType::Executable && load_bias != 0) || entry != observed_entry {
        return Err(AnalysisError::Invalid(
            "ELF entry disagrees with the observed entry",
        ));
    }

    for segment in elf
        .headers
        .iter()
        .filter(|header| header.is_executable_load() && header.file_size != 0)
    {
        let start = u64::from(segment.virtual_address) + load_bias;
        let end = start + u64::from(segment.file_size);
        let mapped = executable_regions.iter().any(|map| {
            map.contains(start)
                && end <= map.end
                && map.file_offset.checked_add(start - map.start)
                    == Some(u64::from(segment.file_offset))
        });
        if !mapped {
            return Err(AnalysisError::Invalid(
                "file-backed executable segment disagrees with proc maps",
            ));
        }
    }
    Ok(MappedElf {
        executable_type: elf.kind,
        link_entry: elf.entry,
        entry,
        guest_base: base,
        load_bias,
        program_headers: elf.headers,
    })
}
